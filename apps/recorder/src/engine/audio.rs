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
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use capture::{AppLoopback, AppTarget, AudioApp, AudioCapture, AudioPacket, CaptureError, DeviceKind, LoopbackMode, find_device, list_audio_apps, qpc_now};
use hound::{SampleFormat, WavSpec, WavWriter};
use media_io::mux::{MuxSender, SendParameters};
use media_io::{AudioCodec, AudioEncoder};

use super::audio_track::{AppChange, AppLink, AppSource, StartApp, TrackPipeline, app_pids};
use super::session::{AppEvent, AudioStreamSpec, EngineError, Stats, TrackSpec};

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
/// How often the running apps are listed for the app tracks.
const APP_POLL_INTERVAL: Duration = Duration::from_secs(1);

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
    /// The peak level of each track over the whole take in dBFS, in track order.
    pub(super) track_peaks_dbfs: Vec<f32>,
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
            // An app track has no device; its capture comes and goes with the app.
            None if track.app.is_some() => None,
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
    // Apps that already run are attached before the take starts; the thread started here keeps listing them.
    let mut apps = app_links(tracks);
    let app_listings = apps.iter().any(Option::is_some).then(|| {
        match list_audio_apps(true) {
            Ok(listing) => {
                poll_links(&mut apps, &listing, &mut start_loopback);
            }
            Err(err) => log::warn!("the running apps could not be listed: {err}"),
        }
        spawn_app_lister()
    });
    let _ = tx.send(AudioMessage::Setup(Ok(AudioSetup { streams })));
    // The session releases the thread once the muxer holds every stream.
    let start = match start_rx.recv() {
        Ok(start) => start,
        // The session gave up; leave quietly without writing any files.
        Err(_) => return Ok(AudioSummary { wav_files: Vec::new(), track_peaks_dbfs: Vec::new() }),
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
    rig.attach_at_start(apps, stats);
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
        // After the drain, so a replaced capture's queued packets are already in. Pauses are polled too.
        if let Some(listing) = app_listings.as_ref().and_then(|listings| listings.try_iter().last()) {
            let t_ms = take_ms(paused_at.unwrap_or(now), start_100ns);
            rig.poll_apps(&listing, t_ms, stats, &mut start_loopback);
        }
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
    let track_peaks_dbfs = rig.pipelines.iter().map(TrackPipeline::peak_hold_dbfs).collect();
    Ok(AudioSummary { wav_files: rig.finalize_wavs(), track_peaks_dbfs })
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
    /// Whether each track's device was lost, or its app is not attached, in track order.
    lost: Vec<bool>,
    /// The app link of each app track, in track order; None for a device track.
    apps: Vec<Option<AppLink>>,
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
        let apps = app_links(&tracks);
        // A track with no capture starts lost.
        let lost = captures.iter().map(Option::is_none).collect();
        Ok(Self {
            captures,
            pipelines,
            taken: vec![0; tracks.len()],
            lost,
            apps,
            wavs,
            mix_encoder,
            track_encoders,
            mixdown_on,
            first_track_slot: usize::from(mixdown_on),
            tracks,
            stream_indexes,
        })
    }
    /// Takes over the app links polled before the start, and reports the attached ones as attached at time 0.
    fn attach_at_start(&mut self, apps: Vec<Option<AppLink>>, stats: &Stats) {
        self.apps = apps;
        for (index, (track, link)) in self.tracks.iter().zip(&self.apps).enumerate() {
            let Some((app, link)) = track.app.as_ref().zip(link.as_ref()) else {
                continue;
            };
            if link.source().is_some() {
                self.lost[index] = false;
                stats.set_track_attached(index, true);
                stats.push_app_event(AppEvent { track: index, app_name: app.app_name.clone(), attached: true, t_ms: 0, mark: false });
            }
        }
    }

    /// Attaches, detaches or restarts every app track from a listing of the running apps, at `t_ms` on the take clock.
    fn poll_apps(&mut self, listing: &[AudioApp], t_ms: u64, stats: &Stats, start: &mut StartApp<'_>) {
        for (index, change) in poll_links(&mut self.apps, listing, start) {
            let attached = change == AppChange::Attached;
            self.lost[index] = !attached;
            stats.set_track_attached(index, attached);
            let Some(app) = &self.tracks[index].app else {
                continue;
            };
            // A rare event, logged once per change, never per tick.
            log::info!("audio track {}: {} {}", self.tracks[index].name, app.exe, if attached { "attached" } else { "detached, recording silence" });
            stats.push_app_event(AppEvent { track: index, app_name: app.app_name.clone(), attached, t_ms, mark: true });
        }
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
        for (link, pipeline) in self.apps.iter().zip(&mut self.pipelines) {
            if let Some(source) = link.as_ref().and_then(AppLink::source) {
                while let Ok(packet) = source.receiver().try_recv() {
                    pipeline.push(&packet);
                }
            }
        }
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
            // App tracks come back through poll_apps.
            if !*lost || track.app.is_some() {
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
        for (n, ((capture, pipeline), lost)) in self.captures.iter().zip(&mut self.pipelines).zip(&self.lost).enumerate() {
            let peak = pipeline.peak_since_last_read();
            let clipped = pipeline.clipped();
            // A detached app is not a lost device: it has its own marks and state.
            let (packets, overflows, device_lost) = match &self.apps[n] {
                Some(link) => {
                    let (packets, overflows) = link.source().map_or((0, 0), |source| (source.packets(), source.overflows()));
                    (packets, overflows, false)
                }
                None => {
                    let (packets, overflows) = capture.as_ref().map_or((0, 0), |capture| (capture.packets(), capture.overflows()));
                    stats.set_track_attached(n, !*lost);
                    (packets, overflows, *lost)
                }
            };
            stats.fold_audio_track(n, peak, clipped, packets, overflows, device_lost);
        }
    }

    /// Keeps every track's silence lead filled on the timeline.
    fn fill_silence(&mut self, now_index: u64) {
        for ((track, pipeline), lost) in self.tracks.iter().zip(&mut self.pipelines).zip(&self.lost) {
            let kind = track.device.as_ref().map(|device| device.kind);
            // An app that makes no sound may send no packets, like an idle output device.
            if kind == Some(DeviceKind::Output) || track.app.is_some() || *lost {
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

/// A detached link for every app track, in track order.
fn app_links(tracks: &[TrackSpec]) -> Vec<Option<AppLink>> {
    tracks.iter().map(|track| track.app.as_ref().map(|app| AppLink::new(&app.exe))).collect()
}

/// Polls every app link against a listing and returns the tracks that attached or detached.
fn poll_links(links: &mut [Option<AppLink>], listing: &[AudioApp], start: &mut StartApp<'_>) -> Vec<(usize, AppChange)> {
    let mut changes = Vec::new();
    for (index, link) in links.iter_mut().enumerate() {
        let Some(link) = link else {
            continue;
        };
        let running = app_pids(listing, link.exe());
        if let Some(change) = link.poll(running, start) {
            changes.push((index, change));
        }
    }
    changes
}

/// Starts the real capture of an app and its child processes.
fn start_loopback(exe: &str) -> Result<Box<dyn AppSource>, CaptureError> {
    Ok(Box::new(AppLoopback::start(AppTarget::new(exe), LoopbackMode::Include)?))
}

impl AppSource for AppLoopback {
    fn receiver(&self) -> &Receiver<AudioPacket> {
        AppLoopback::receiver(self)
    }
    fn lost(&self) -> bool {
        AppLoopback::lost(self)
    }
    fn instance_pids(&self) -> &[u32] {
        AppLoopback::instance_pids(self)
    }
    fn packets(&self) -> u64 {
        AppLoopback::packets(self)
    }
    fn overflows(&self) -> u64 {
        AppLoopback::overflows(self)
    }
}

/// Lists the running apps once a second on a thread of its own, so a slow listing never delays the audio tick.
///
/// The thread ends within about a second of the receiver being dropped. If it cannot start, the receiver stays empty.
fn spawn_app_lister() -> Receiver<Vec<AudioApp>> {
    let (sender, receiver) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("app-list".to_owned()).spawn(move || {
        loop {
            std::thread::sleep(APP_POLL_INTERVAL);
            match list_audio_apps(true) {
                Ok(listing) => {
                    if sender.send(listing).is_err() {
                        break;
                    }
                }
                Err(err) => log::debug!("the running apps could not be listed: {err}"),
            }
        }
    });
    if let Err(err) = spawned {
        log::warn!("the app list thread could not be started, app tracks will not attach or detach: {err}");
    }
    receiver
}

/// The time on the take's clock in ms of a QPC stamp in 100 ns units; `start_100ns` is already shifted by earlier pauses.
fn take_ms(now_100ns: i64, start_100ns: i64) -> u64 {
    (now_100ns - start_100ns).max(0) as u64 / 10_000
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

#[cfg(test)]
mod tests {
    use super::super::audio_track::fake::FakeSystem;
    use super::super::clock::MediaClock;
    use super::super::marks::{MarksWarning, TakeMarks};
    use super::super::session::AppSpec;
    use super::*;

    /// A rig with one app track for Discord, no encoders and no files, and its stats.
    fn discord_rig() -> Result<(AudioRig, Stats), EngineError> {
        let app = AppSpec { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned() };
        let tracks = vec![TrackSpec {
            name: "Discord".to_owned(),
            device: None,
            device_id: String::new(),
            device_name: String::new(),
            gain_db: 0.0,
            muted: false,
            offset_ms: 0,
            in_mixdown: true,
            app: Some(app),
        }];
        let stats = Stats::new(&tracks);
        let rig = AudioRig::new(tracks, vec![None], None, vec![None], false, false, Path::new("."), "take", 0, Vec::new())?;
        Ok((rig, stats))
    }

    fn discord_running() -> Vec<AudioApp> {
        vec![AudioApp { exe: "discord.exe".to_owned(), display_name: "Discord".to_owned(), pids: vec![40, 41], active: true }]
    }

    #[test]
    fn silence_is_written_while_an_app_track_is_detached() -> Result<(), EngineError> {
        let (mut rig, stats) = discord_rig()?;
        let system = FakeSystem::default();
        let mut start = |_: &str| system.start();
        rig.fill_silence(48_000);
        let before = rig.pipelines[0].take(96_000);
        assert_eq!(before.len() as u64 / 2, 48_000 - OUTPUT_LEAD_FRAMES);
        assert!(before.iter().all(|sample| *sample == 0.0));

        // Attached: the app's packets reach the track.
        rig.poll_apps(&discord_running(), 1_000, &stats, &mut start);
        let packet =
            AudioPacket { qpc_100ns: 10_000_000, frames: 480, channels: 2, sample_rate: OUT_RATE, samples: vec![0.5; 960], discontinuity: true, silent: false };
        assert!(system.sender.borrow().as_ref().is_some_and(|sender| sender.send(packet).is_ok()));
        rig.drain_captures();
        assert!(rig.pipelines[0].take(96_000).iter().any(|sample| *sample > 0.1));

        // Detached again: silence up to the timeline, and the track is not reported as a lost device.
        rig.poll_apps(&[], 2_000, &stats, &mut start);
        rig.pipelines[0].take(96_000);
        rig.fill_silence(144_000);
        let after = rig.pipelines[0].take(144_000);
        assert!(!after.is_empty() && after.iter().all(|sample| *sample == 0.0));
        assert_eq!(rig.pipelines[0].produced_frames(), 144_000 - OUTPUT_LEAD_FRAMES);
        rig.fold_stats(&stats);
        let snapshot = stats.snapshot();
        assert!(!snapshot.tracks[0].attached && !snapshot.tracks[0].device_lost);
        Ok(())
    }

    #[test]
    fn polling_marks_each_attach_and_detach_at_its_time() -> Result<(), Box<dyn std::error::Error>> {
        let (mut rig, stats) = discord_rig()?;
        let system = FakeSystem::default();
        let mut start = |_: &str| system.start();
        for (t_ms, running) in [(1_000, false), (2_000, true), (3_000, true), (65_500, false), (66_500, false), (70_000, true)] {
            rig.poll_apps(&if running { discord_running() } else { Vec::new() }, t_ms, &stats, &mut start);
        }
        let events = stats.take_app_events();
        assert_eq!(
            events.iter().map(|event| (event.attached, event.t_ms, event.mark)).collect::<Vec<_>>(),
            [(true, 2_000, true), (false, 65_500, true), (true, 70_000, true)]
        );

        let dir = std::env::temp_dir().join(format!("cutmark-app-marks-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut marks = TakeMarks::begin(&dir, "take", &dir.join("take.mkv"), 60, MediaClock::new(0, 10_000_000), MarksWarning::default());
        for event in &events {
            marks.app(&event.app_name, event.attached, event.t_ms);
        }
        let listed: Vec<_> = marks.recent().iter().cloned().collect();
        let expected = [(2_000, "App attached: Discord"), (65_500, "App detached: Discord"), (70_000, "App attached: Discord")]
            .map(|(t_ms, label)| (::marks::format_tc(t_ms), "System".to_owned(), Some(label.to_owned())));
        assert_eq!(listed, expected);
        drop(marks);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
