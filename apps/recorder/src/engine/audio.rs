//! The audio thread: every track's capture and pipeline, WAV sidecars, the
//! mixdown and per-track AAC streams, and their packets into the muxer.
//!
//! See the recorder spec: Audio capture and tracks (Format, Mixdown track,
//! WAV sidecars) and Sync and reliability rules (Audio, Pause, Crash safety,
//! Device loss).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use capture::{AudioCapture, DeviceKind, find_device, qpc_now};
use hound::{SampleFormat, WavSpec, WavWriter};
use media_io::mux::{MuxSender, SendParameters};
use media_io::{AudioCodec, AudioEncoder};

use super::audio_track::TrackPipeline;
use super::session::{AudioStreamSpec, EngineError, Stats, TrackSpec};

/// The audio thread's tick, about one AAC frame.
const TICK: Duration = Duration::from_millis(10);
/// The sample rate of every track, stream and WAV sidecar.
const OUT_RATE: u32 = 48_000;
/// The channel count of every track, stream and WAV sidecar.
const OUT_CHANNELS: u16 = 2;
/// The WAV sidecars' bit depth.
const WAV_BITS: u16 = 24;
/// Full scale of a 24-bit sample, the target of every WAV write.
const WAV_FULL_SCALE: f32 = 8_388_607.0;
/// How often a WAV sidecar's header is refreshed and its bytes flushed.
const WAV_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
/// Silence lead for output and lost tracks, so loopback never runs dry.
const OUTPUT_LEAD_FRAMES: u64 = 4_800;
/// Silence safety net for input tracks, so a stalled mic never runs dry.
const INPUT_LEAD_FRAMES: u64 = 24_000;
/// Largest block taken per tick, so one burst cannot build a huge encode.
const MAX_BLOCK_FRAMES: usize = 4_800;
/// How often every lost track looks for its device again.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);

/// Messages from the audio thread back to the session.
pub(super) enum AudioMessage {
    /// The audio thread's setup ended: the stream facts on success, or the
    /// error that ended the setup.
    Setup(Result<AudioSetup, EngineError>),
    /// The take's audio ended, with the WAV files or the error that ended it.
    Finished(Result<AudioSummary, EngineError>),
}

/// The audio facts the session needs to add the audio streams to the muxer.
pub(super) struct AudioSetup {
    /// One entry per stream: the mixdown first when present, then per-track.
    pub(super) streams: Vec<AudioStreamSpec>,
}

/// The totals of one finished take's audio.
pub(super) struct AudioSummary {
    /// The WAV sidecar files written, one per track.
    pub(super) wav_files: Vec<PathBuf>,
}

/// The command that releases the audio thread to start.
pub(super) struct AudioStart {
    /// The QPC value recording time 0 maps to, in 100 nanosecond units.
    pub(super) start_qpc: i64,
    /// The queue the thread writes its packets into.
    pub(super) mux: MuxSender,
    /// The mux stream index per encoder slot: the mixdown first when present,
    /// then one per track.
    pub(super) streams: Vec<usize>,
}
/// The audio side of a take's configuration, handed to the audio thread.
pub(super) struct AudioJob {
    /// One entry per audio track, in stream order.
    pub(super) tracks: Vec<TrackSpec>,
    /// The codec every audio stream is encoded with.
    pub(super) codec: AudioCodec,
    /// Whether the mixdown stream is wanted.
    pub(super) mixdown: bool,
    /// Whether every track also gets its own stream in the video file.
    pub(super) track_streams: bool,
    /// Whether every track is written as a WAV sidecar.
    pub(super) wav_sidecars: bool,
    /// The take folder the WAV sidecars are written into.
    pub(super) take_folder: PathBuf,
    /// The base name for the WAV sidecar file names.
    pub(super) base_name: String,
}

/// The audio thread's entry point: sets up, waits for the start command,
/// records until the stop flag, and always reports one Finished message.
pub(super) fn run(job: AudioJob, stats: Arc<Stats>, stop_flag: Arc<AtomicBool>, tx: Sender<AudioMessage>, start_rx: Receiver<AudioStart>) {
    let outcome = record(&job, &stats, &stop_flag, &tx, start_rx);
    let _ = tx.send(AudioMessage::Finished(outcome));
}

/// Runs the audio thread: setup, the record loop, then the stop path.
fn record(
    job: &AudioJob,
    stats: &Stats,
    stop_flag: &AtomicBool,
    tx: &Sender<AudioMessage>,
    start_rx: Receiver<AudioStart>,
) -> Result<AudioSummary, EngineError> {
    let tracks = &job.tracks;
    // One capture per track; a device that will not open ends the setup, but a
    // device that is missing does not: that track starts lost and writes silence.
    let mut captures = Vec::with_capacity(tracks.len());
    for track in tracks {
        let capture = match &track.device {
            Some(device) => Some(AudioCapture::start(device)?),
            None => {
                log::warn!("audio track {}: its device is missing, recording silence until it returns", track.name);
                None
            }
        };
        captures.push(capture);
    }
    let mixdown_on = job.mixdown && tracks.iter().any(|track| track.in_mixdown);
    // The mixdown encoder comes first so its stream lands right after the video.
    let mix_encoder = if mixdown_on { Some(AudioEncoder::open(job.codec, OUT_RATE, OUT_CHANNELS)?) } else { None };
    let mut track_encoders = Vec::with_capacity(tracks.len());
    for _ in tracks {
        track_encoders.push(if job.track_streams { Some(AudioEncoder::open(job.codec, OUT_RATE, OUT_CHANNELS)?) } else { None });
    }
    let mut streams = Vec::new();
    if let Some(encoder) = &mix_encoder {
        streams.push(AudioStreamSpec {
            params: SendParameters(encoder.parameters()),
            time_base: encoder.time_base(),
            title: "Mixdown".to_string(),
            default: true,
        });
    }
    for (encoder, track) in track_encoders.iter().zip(tracks) {
        if let Some(encoder) = encoder {
            streams.push(AudioStreamSpec {
                params: SendParameters(encoder.parameters()),
                time_base: encoder.time_base(),
                title: track.name.clone(),
                default: false,
            });
        }
    }
    let _ = tx.send(AudioMessage::Setup(Ok(AudioSetup { streams })));
    // The session releases the thread once the muxer holds every stream.
    let start = match start_rx.recv() {
        Ok(start) => start,
        // The session gave up; leave quietly without writing any files.
        Err(_) => return Ok(AudioSummary { wav_files: Vec::new() }),
    };
    let mut rig = AudioRig::new(
        tracks.clone(),
        captures,
        mix_encoder,
        track_encoders,
        mixdown_on,
        job.wav_sidecars,
        &job.take_folder,
        &job.base_name,
        start.start_qpc,
        start.streams,
    )?;
    let mut start_100ns = start.start_qpc;
    let mut paused_at: Option<i64> = None;
    let mut was_paused = false;
    let mut next_tick = Instant::now();
    let mut last_flush = Instant::now();
    let mut last_reconnect = Instant::now();
    while !stop_flag.load(Ordering::Relaxed) {
        let now_instant = Instant::now();
        if next_tick > now_instant {
            std::thread::sleep(next_tick - now_instant);
        }
        next_tick += TICK;
        let now = qpc_now();
        // A pause shifts every pipeline's timeline, exactly like the video pacer.
        let paused = stats.paused.load(Ordering::Relaxed);
        if paused != was_paused {
            if paused {
                for pipeline in &mut rig.pipelines {
                    pipeline.pause(now);
                }
                paused_at = Some(now);
            } else {
                let at = paused_at.take().unwrap_or(now);
                start_100ns += now - at;
                for pipeline in &mut rig.pipelines {
                    pipeline.resume(now);
                }
            }
            was_paused = paused;
        }
        rig.apply_controls(stats);
        rig.drain_captures();
        rig.fold_stats(stats);
        if last_reconnect.elapsed() >= RECONNECT_INTERVAL {
            rig.reconnect_lost();
            last_reconnect = Instant::now();
        }
        if paused {
            continue;
        }
        let now_index = timeline_index(now, start_100ns);
        rig.fill_silence(now_index);
        if let Some(frames) = rig.ready_frames(MAX_BLOCK_FRAMES)
            && let Err(err) = rig.emit_block(frames, &start.mux, stats)
        {
            stats.set_last_error(format!("audio: {err}"));
            return Err(err);
        }
        if last_flush.elapsed() >= WAV_FLUSH_INTERVAL {
            rig.flush_wavs();
            last_flush = Instant::now();
        }
    }
    // Fill every track with silence up to the stop moment on the timeline, so
    // every stream and WAV ends at the same length as the video.
    let stop_index = timeline_index(qpc_now(), start_100ns);
    rig.fill_silence(stop_index);
    while let Some(frames) = rig.ready_frames(MAX_BLOCK_FRAMES) {
        rig.emit_block(frames, &start.mux, stats)?;
    }
    // Finish the encoders and queue their last packets for the muxer.
    // A take without a mixdown has no mixdown stream, and may have no stream at all.
    if let Some(encoder) = rig.mix_encoder.as_mut() {
        let mix_stream = rig.stream_indexes.first().copied().ok_or(EngineError::Failed("the mixdown stream index is missing".into()))?;
        finish_stream(Some(encoder), mix_stream, &start.mux)?;
    }
    for (slot, encoder) in rig.track_encoders.iter_mut().enumerate() {
        let Some(stream) = rig.stream_indexes.get(rig.first_track_slot + slot).copied() else {
            return Err(EngineError::Failed("a track stream index is missing".into()));
        };
        finish_stream(encoder.as_mut(), stream, &start.mux)?;
    }
    rig.flush_wavs();
    Ok(AudioSummary { wav_files: rig.finalize_wavs() })
}

/// One WAV sidecar: the hound writer and the file it lands in.
struct SidecarWav {
    /// The file the track is written to.
    path: PathBuf,
    /// The 48 kHz 24-bit stereo writer.
    writer: WavWriter<BufWriter<File>>,
}
/// Everything the audio thread owns while a take runs.
struct AudioRig {
    /// One capture per track, in track order; None while the track's device is missing.
    captures: Vec<Option<AudioCapture>>,
    /// One timeline-locked pipeline per track, in track order.
    pipelines: Vec<TrackPipeline>,
    /// Frames already taken from each pipeline, in track order.
    taken: Vec<u64>,
    /// Whether each track's device was lost, in track order.
    lost: Vec<bool>,
    /// One WAV sidecar per track when sidecars are on, in track order.
    wavs: Vec<Option<SidecarWav>>,
    /// The mixdown encoder, when the mixdown is on and a track feeds it.
    mix_encoder: Option<AudioEncoder>,
    /// One per-track encoder when track streams are on, in track order.
    track_encoders: Vec<Option<AudioEncoder>>,
    /// The track specs, in track order.
    tracks: Vec<TrackSpec>,
    /// Whether the mixdown stream exists.
    mixdown_on: bool,
    /// The first track stream's mux slot: 1 with a mixdown, 0 without.
    first_track_slot: usize,
    /// The mux stream index per encoder slot: the mixdown first when present,
    /// then one per track.
    stream_indexes: Vec<usize>,
}

impl AudioRig {
    /// Builds the rig: pipelines at the take's timeline start, and the WAV sidecars.
    #[allow(clippy::too_many_arguments)]
    fn new(
        tracks: Vec<TrackSpec>,
        captures: Vec<Option<AudioCapture>>,
        mix_encoder: Option<AudioEncoder>,
        track_encoders: Vec<Option<AudioEncoder>>,
        mixdown_on: bool,
        wav_sidecars: bool,
        take_folder: &Path,
        base_name: &str,
        start_100ns: i64,
        stream_indexes: Vec<usize>,
    ) -> Result<Self, EngineError> {
        let mut pipelines = Vec::with_capacity(tracks.len());
        for track in &tracks {
            let mut pipeline = TrackPipeline::new(start_100ns, OUT_RATE);
            pipeline.set_gain_db(track.gain_db);
            pipeline.set_muted(track.muted);
            pipeline.set_offset_ms(track.offset_ms as f32);
            pipelines.push(pipeline);
        }
        let mut wavs = Vec::with_capacity(tracks.len());
        if wav_sidecars {
            let _ = std::fs::create_dir_all(take_folder);
        }
        for (n, track) in tracks.iter().enumerate() {
            let wav = if wav_sidecars {
                let path = take_folder.join(format!("{base_name}_t{}_{}.wav", n + 1, safe_file_name(&track.name)));
                let writer = WavWriter::create(&path, wav_spec()).map_err(|err| EngineError::Failed(format!("could not create {}: {err}", path.display())))?;
                Some(SidecarWav { path, writer })
            } else {
                None
            };
            wavs.push(wav);
        }
        // A track with no capture starts lost.
        let lost = captures.iter().map(Option::is_none).collect();
        Ok(Self {
            captures,
            pipelines,
            taken: vec![0; tracks.len()],
            lost,
            wavs,
            mix_encoder,
            track_encoders,
            mixdown_on,
            first_track_slot: usize::from(mixdown_on),
            tracks,
            stream_indexes,
        })
    }
    /// Applies the gain and mute the UI set since the last tick to each track's pipeline.
    fn apply_controls(&mut self, stats: &Stats) {
        for (index, (track, pipeline)) in self.tracks.iter_mut().zip(&mut self.pipelines).enumerate() {
            let Some((gain_db, muted)) = stats.track_controls(index) else {
                continue;
            };
            if gain_db != track.gain_db {
                track.gain_db = gain_db;
                pipeline.set_gain_db(gain_db);
            }
            if muted != track.muted {
                track.muted = muted;
                pipeline.set_muted(muted);
            }
        }
    }

    /// Moves every queued packet from each capture into its pipeline.
    fn drain_captures(&mut self) {
        for (((capture, pipeline), lost), track) in self.captures.iter().zip(&mut self.pipelines).zip(self.lost.iter_mut()).zip(&self.tracks) {
            // A track with no capture stays lost until reconnect_lost finds its device.
            let Some(capture) = capture else {
                continue;
            };
            while let Ok(packet) = capture.receiver().try_recv() {
                pipeline.push(&packet);
            }
            let now_lost = capture.device_lost();
            if now_lost && !*lost {
                // A rare event, logged once per loss, never per tick.
                log::warn!("audio track {}: its device was lost, recording silence until it returns", track.name);
            }
            *lost = now_lost;
        }
    }

    /// Looks for the device of every lost track and starts a new capture for each one found.
    ///
    /// The pipeline's resync places the first new packet on the timeline, so
    /// the gap in between stays silence.
    fn reconnect_lost(&mut self) {
        for ((track, capture), lost) in self.tracks.iter_mut().zip(&mut self.captures).zip(&mut self.lost) {
            if !*lost {
                continue;
            }
            let Some(device) = find_device(&track.device_id, &track.device_name) else {
                continue;
            };
            match AudioCapture::start(&device) {
                Ok(new_capture) => {
                    log::info!("audio track {}: its device {} is back, recording again", track.name, device.name);
                    // Replacing the capture drops the old one, whose thread has already ended.
                    *capture = Some(new_capture);
                    track.device = Some(device);
                    *lost = false;
                }
                Err(err) => log::debug!("audio track {}: its device {} is listed but would not start yet: {err}", track.name, device.name),
            }
        }
    }

    /// Folds each track's capture counters and meter into the live stats.
    fn fold_stats(&mut self, stats: &Stats) {
        for (n, ((capture, pipeline), lost)) in self.captures.iter().zip(&mut self.pipelines).zip(self.lost.iter_mut()).enumerate() {
            let peak = pipeline.peak_since_last_read();
            let clipped = pipeline.clipped();
            let (packets, overflows) = capture.as_ref().map_or((0, 0), |capture| (capture.packets(), capture.overflows()));
            stats.fold_audio_track(n, peak, clipped, packets, overflows, *lost);
        }
    }

    /// Keeps every track's silence lead filled on the timeline.
    fn fill_silence(&mut self, now_index: u64) {
        for ((track, pipeline), lost) in self.tracks.iter().zip(&mut self.pipelines).zip(&self.lost) {
            let kind = track.device.as_ref().map(|device| device.kind);
            if kind == Some(DeviceKind::Output) || *lost {
                pipeline.fill_silence_to(now_index.saturating_sub(OUTPUT_LEAD_FRAMES));
            }
            if kind == Some(DeviceKind::Input) {
                pipeline.fill_silence_to(now_index.saturating_sub(INPUT_LEAD_FRAMES));
            }
        }
    }

    /// The same number of frames every track can give, capped at max_frames.
    fn ready_frames(&self, max_frames: usize) -> Option<usize> {
        let mut ready = u64::MAX;
        for (pipeline, taken) in self.pipelines.iter().zip(&self.taken) {
            ready = ready.min(pipeline.produced_frames() - taken);
        }
        if ready == 0 || ready == u64::MAX { None } else { Some((ready as usize).min(max_frames)) }
    }
    /// Takes the same number of frames from every track, writes the WAVs,
    /// sums the mixdown and queues every stream's packets.
    fn emit_block(&mut self, frames: usize, mux: &MuxSender, stats: &Stats) -> Result<(), EngineError> {
        let mut blocks: Vec<Vec<f32>> = self.pipelines.iter_mut().map(|pipeline| pipeline.take(frames)).collect();
        let common = blocks.iter().map(|block| block.len() / 2).min().unwrap_or(0);
        if common == 0 {
            return Ok(());
        }
        for block in &mut blocks {
            block.truncate(common * 2);
        }
        for ((block, wav), taken) in blocks.iter().zip(self.wavs.iter_mut()).zip(self.taken.iter_mut()) {
            if let Some(wav) = wav {
                for &sample in block {
                    let scaled = (sample.clamp(-1.0, 1.0) * WAV_FULL_SCALE) as i32;
                    wav.writer.write_sample(scaled).map_err(|err| EngineError::Failed(format!("the WAV sidecar write failed: {err}")))?;
                }
            }
            *taken += common as u64;
        }
        if self.wavs.iter().any(Option::is_some) {
            let wav_count = self.wavs.iter().flatten().count() as u64;
            stats.wav_bytes.fetch_add(wav_count * common as u64 * u64::from(OUT_CHANNELS) * u64::from(WAV_BITS / 8), Ordering::Relaxed);
        }
        if self.mixdown_on {
            let stream = self.stream_indexes.first().copied().ok_or(EngineError::Failed("the mixdown stream index is missing".into()))?;
            let mut mix = vec![0.0f32; common * 2];
            for (block, track) in blocks.iter().zip(&self.tracks) {
                if track.in_mixdown {
                    for (dst, src) in mix.iter_mut().zip(block.iter()) {
                        *dst += src;
                    }
                }
            }
            for sample in mix.iter_mut() {
                *sample = sample.clamp(-1.0, 1.0);
            }
            encode_stream(self.mix_encoder.as_mut(), stream, &mix, mux, stats)?;
        }
        for (slot, (encoder, block)) in self.track_encoders.iter_mut().zip(&blocks).enumerate() {
            let Some(stream) = self.stream_indexes.get(self.first_track_slot + slot).copied() else {
                return Err(EngineError::Failed("a track stream index is missing".into()));
            };
            encode_stream(encoder.as_mut(), stream, block, mux, stats)?;
        }
        Ok(())
    }

    /// Refreshes every WAV header flush so a crash leaves a usable file.
    fn flush_wavs(&mut self) {
        for wav in self.wavs.iter_mut().flatten() {
            let _ = wav.writer.flush();
        }
    }

    /// Finishes every WAV sidecar and returns the files written.
    fn finalize_wavs(&mut self) -> Vec<PathBuf> {
        let wavs = std::mem::take(&mut self.wavs);
        wavs.into_iter().flatten().map(|wav| wav.path).collect()
    }
}

/// Feeds one interleaved block into its encoder and queues the packets.
fn encode_stream(encoder: Option<&mut AudioEncoder>, stream: usize, block: &[f32], mux: &MuxSender, stats: &Stats) -> Result<(), EngineError> {
    let Some(encoder) = encoder else {
        return Ok(());
    };
    encoder.send(block)?;
    let mut send_error = None;
    encoder.receive(&mut |packet| {
        if let Err(err) = mux.send(stream, packet) {
            send_error = Some(err);
        }
    })?;
    stats.audio_frames.fetch_add(block.len() as u64 / u64::from(OUT_CHANNELS), Ordering::Relaxed);
    send_error.map_or(Ok(()), |err| Err(err.into()))
}

/// Pads the encoder's last frame and queues the final packets for its stream.
fn finish_stream(encoder: Option<&mut AudioEncoder>, stream: usize, mux: &MuxSender) -> Result<(), EngineError> {
    let Some(encoder) = encoder else {
        return Ok(());
    };
    let mut send_error = None;
    encoder.finish(&mut |packet| {
        if let Err(err) = mux.send(stream, packet) {
            send_error = Some(err);
        }
    })?;
    send_error.map_or(Ok(()), |err| Err(err.into()))
}

/// The timeline index a QPC stamp in 100 ns units lands on, in output frames.
fn timeline_index(now_100ns: i64, start_100ns: i64) -> u64 {
    let elapsed = (now_100ns - start_100ns).max(0) as u64;
    elapsed * u64::from(OUT_RATE) / 10_000_000
}

/// The WAV sidecar format: 48 kHz, 24-bit stereo.
fn wav_spec() -> WavSpec {
    WavSpec { channels: OUT_CHANNELS, sample_rate: OUT_RATE, bits_per_sample: WAV_BITS, sample_format: SampleFormat::Int }
}

/// Maps a track name onto a file-system-safe fragment for the WAV file name.
fn safe_file_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if safe.is_empty() { "track".to_string() } else { safe }
}
