//! The audio thread: every track's capture and pipeline, the PCM for the WAV
//! sidecars, the mixdown and per-track AAC streams, all handed to the sink.
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

use capture::{
    AppLoopback, AppTarget, AudioApp, AudioCapture, AudioPacket, CaptureError, DeviceKind, LoopbackMode, find_device, list_audio_apps, list_audio_devices,
    qpc_now,
};
use hound::{SampleFormat, WavSpec, WavWriter};
use media_io::mux::SendParameters;
use media_io::{AudioCodec, AudioEncoder};

use super::audio_track::{AppChange, AppSource, AppSystem, TrackLink, TrackPipeline, app_pids};
use super::session::{AppEvent, AudioStreamSpec, EngineError, Instances, Stats, TrackSpec};
use super::sink::{CaptureOutput, PacketSink};

/// The audio thread's tick, about one AAC frame.
const TICK: Duration = Duration::from_millis(10);
/// The sample rate of every track, stream and WAV sidecar.
pub(super) const OUT_RATE: u32 = 48_000;
/// The channel count of every track, stream and WAV sidecar.
pub(super) const OUT_CHANNELS: u16 = 2;
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
    /// Where the thread's packets and PCM go.
    pub(super) output: Arc<CaptureOutput>,
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
            // An app or desktop-except track has no device of its own; its link starts and switches its captures.
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
    // Apps that already run are attached, and desktop-except tracks get their first capture, before the
    // take starts; the thread started here keeps listing the apps.
    let mut apps = app_links(tracks);
    let app_listings = apps.iter().any(Option::is_some).then(|| {
        match list_audio_apps(true) {
            Ok(listing) => {
                poll_links(&mut apps, &listing, &mut WindowsApps);
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
    let output = start.output.as_ref();
    let sink: &dyn PacketSink = output;
    sink.open_pcm()?;
    // Closes the PCM output however the take ends.
    let _pcm = PcmGuard(sink);
    let mut rig = AudioRig::new(tracks.clone(), captures, mix_encoder, track_encoders, mixdown_on, start.start_qpc);
    rig.attach_at_start(apps, stats);
    // The audio timeline is the capture's, which never pauses: the output removes the take's paused time.
    let start_100ns = start.start_qpc;
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
        rig.apply_controls(stats);
        let t_ms = output.take_ms(now - start_100ns);
        rig.drain_captures(now, t_ms, stats, &mut WindowsApps);
        // After the drain, so a replaced capture's queued packets are already in.
        if let Some(listing) = app_listings.as_ref().and_then(|listings| listings.try_iter().last()) {
            rig.poll_apps(&listing, t_ms, stats, &mut WindowsApps);
        }
        rig.fold_stats(stats);
        if last_reconnect.elapsed() >= RECONNECT_INTERVAL {
            rig.reconnect_lost();
            last_reconnect = Instant::now();
        }
        let now_index = timeline_index(now, start_100ns);
        rig.fill_silence(now_index);
        if let Some(frames) = rig.ready_frames(MAX_BLOCK_FRAMES)
            && let Err(err) = rig.emit_block(frames, sink, stats)
        {
            stats.set_last_error(format!("audio: {err}"));
            return Err(err);
        }
        if last_flush.elapsed() >= WAV_FLUSH_INTERVAL {
            sink.flush_pcm();
            last_flush = Instant::now();
        }
    }
    // Fill every track with silence up to the stop moment on the timeline, so
    // every stream and WAV ends at the same length as the video.
    let stop_index = timeline_index(qpc_now(), start_100ns);
    rig.fill_silence(stop_index);
    while let Some(frames) = rig.ready_frames(MAX_BLOCK_FRAMES) {
        rig.emit_block(frames, sink, stats)?;
    }
    rig.finish_streams(sink)?;
    sink.flush_pcm();
    let track_peaks_dbfs = rig.pipelines.iter().map(TrackPipeline::peak_hold_dbfs).collect();
    Ok(AudioSummary { wav_files: sink.close_pcm(), track_peaks_dbfs })
}

/// Closes a sink's PCM output when dropped, so an audio thread that fails still leaves finished WAV files.
struct PcmGuard<'a>(&'a dyn PacketSink);

impl Drop for PcmGuard<'_> {
    fn drop(&mut self) {
        self.0.close_pcm();
    }
}

/// One WAV sidecar: the hound writer and the file it lands in.
pub(super) struct SidecarWav {
    /// The file the track is written to.
    path: PathBuf,
    /// The 48 kHz 24-bit stereo writer.
    writer: WavWriter<BufWriter<File>>,
}

impl SidecarWav {
    /// The bytes one sample of one channel takes in the file.
    pub(super) const BYTES_PER_SAMPLE: u64 = (WAV_BITS / 8) as u64;

    /// Creates the sidecar of the track at `index` in track order, named `{base_name}_t{n}_{track}.wav`.
    pub(super) fn create(take_folder: &Path, base_name: &str, index: usize, track_name: &str) -> Result<Self, EngineError> {
        let path = take_folder.join(format!("{base_name}_t{}_{}.wav", index + 1, safe_file_name(track_name)));
        let writer = WavWriter::create(&path, wav_spec()).map_err(|err| EngineError::Failed(format!("could not create {}: {err}", path.display())))?;
        Ok(Self { path, writer })
    }

    /// Appends interleaved stereo samples.
    pub(super) fn write(&mut self, samples: &[f32]) -> Result<(), EngineError> {
        for &sample in samples {
            let scaled = (sample.clamp(-1.0, 1.0) * WAV_FULL_SCALE) as i32;
            self.writer.write_sample(scaled).map_err(|err| EngineError::Failed(format!("the WAV sidecar write failed: {err}")))?;
        }
        Ok(())
    }

    /// Refreshes the header and flushes, so a crash leaves a usable file; a failure is left for the next write.
    pub(super) fn flush(&mut self) {
        let _ = self.writer.flush();
    }

    /// Drops the writer, which completes the header if it can, and returns the file written.
    pub(super) fn close(self) -> PathBuf {
        self.path
    }

    /// Completes the header and returns the file written.
    pub(super) fn finalize(self) -> Result<PathBuf, EngineError> {
        self.writer.finalize().map_err(|err| EngineError::Failed(format!("the WAV sidecar could not be finished: {err}")))?;
        Ok(self.path)
    }
}

/// Everything the audio thread owns while a take runs.
struct AudioRig {
    /// One capture per track, in track order; None while the track's device is missing.
    captures: Vec<Option<AudioCapture>>,
    /// One timeline-locked pipeline per track, in track order.
    pipelines: Vec<TrackPipeline>,
    /// Frames already taken from each pipeline, in track order.
    taken: Vec<u64>,
    /// Whether each device track's device was lost, in track order; app links keep their own state.
    lost: Vec<bool>,
    /// The link of each app or desktop-except track, in track order; None for a device track.
    apps: Vec<Option<TrackLink>>,
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
    /// How many audio streams the take has: the mixdown when present, then one per track encoder.
    stream_count: usize,
}

impl AudioRig {
    /// Builds the rig, with the pipelines at the take's timeline start.
    fn new(
        tracks: Vec<TrackSpec>,
        captures: Vec<Option<AudioCapture>>,
        mix_encoder: Option<AudioEncoder>,
        track_encoders: Vec<Option<AudioEncoder>>,
        mixdown_on: bool,
        start_100ns: i64,
    ) -> Self {
        let mut pipelines = Vec::with_capacity(tracks.len());
        for track in &tracks {
            let mut pipeline = TrackPipeline::new(start_100ns, OUT_RATE);
            pipeline.set_gain_db(track.gain_db);
            pipeline.set_muted(track.muted);
            pipeline.set_offset_ms(track.offset_ms as f32);
            pipelines.push(pipeline);
        }
        let stream_count = usize::from(mix_encoder.is_some()) + track_encoders.iter().flatten().count();
        let apps = app_links(&tracks);
        // A track with no capture starts lost.
        let lost = captures.iter().map(Option::is_none).collect();
        Self {
            captures,
            pipelines,
            taken: vec![0; tracks.len()],
            lost,
            apps,
            mix_encoder,
            track_encoders,
            mixdown_on,
            first_track_slot: usize::from(mixdown_on),
            tracks,
            stream_count,
        }
    }
    /// Takes over the app links polled before the start, and reports the attached ones as attached at time 0.
    fn attach_at_start(&mut self, apps: Vec<Option<TrackLink>>, stats: &Stats) {
        self.apps = apps;
        for (index, (track, link)) in self.tracks.iter().zip(&self.apps).enumerate() {
            let Some((app, link)) = track.app.as_ref().zip(link.as_ref()) else {
                continue;
            };
            if link.attached() {
                stats.set_track_attached(index, true);
                stats.push_app_event(AppEvent { track: index, app_name: app.app_name.clone(), except: app.except, attached: true, t_ms: 0, mark: false });
            }
        }
    }

    /// Attaches, detaches, switches or restarts every app link from a listing of the running apps, at `t_ms` on the take clock.
    fn poll_apps(&mut self, listing: &[AudioApp], t_ms: u64, stats: &Stats, system: &mut dyn AppSystem) {
        for (index, change) in poll_links(&mut self.apps, listing, system) {
            self.report_app(index, change, t_ms, stats);
        }
    }

    /// Records one track's app change in the live stats and queues its event.
    fn report_app(&self, index: usize, change: AppChange, t_ms: u64, stats: &Stats) {
        let attached = change == AppChange::Attached;
        stats.set_track_attached(index, attached);
        let Some(app) = &self.tracks[index].app else {
            return;
        };
        let state = match (app.except, attached) {
            (false, true) => "attached",
            (false, false) => "detached, recording silence",
            (true, true) => "excluded",
            (true, false) => "closed, capturing all desktop audio",
        };
        // A rare event, logged once per change, never per tick.
        log::info!("audio track {}: {} {state}", self.tracks[index].name, app.exe);
        stats.push_app_event(AppEvent { track: index, app_name: app.app_name.clone(), except: app.except, attached, t_ms, mark: true });
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

    /// Moves every queued packet from each capture into its pipeline. A desktop-except link also completes
    /// its switches here, at `now_100ns`, and may find that its app has closed.
    fn drain_captures(&mut self, now_100ns: i64, t_ms: u64, stats: &Stats, system: &mut dyn AppSystem) {
        let mut changes = Vec::new();
        for (index, (link, pipeline)) in self.apps.iter_mut().zip(&mut self.pipelines).enumerate() {
            if let Some(link) = link
                && let Some(change) = link.drain(now_100ns, system, &mut |packet| pipeline.push(packet))
            {
                changes.push((index, change));
            }
        }
        for (index, change) in changes {
            self.report_app(index, change, t_ms, stats);
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
            // A detached app is not a lost device: it has its own marks and state. A desktop-except track
            // that should hear the desktop and cannot is one.
            let (packets, overflows, device_lost) = match &self.apps[n] {
                Some(link) => {
                    let (packets, overflows) = link.source().map_or((0, 0), |source| (source.packets(), source.overflows()));
                    (packets, overflows, link.desktop_missing())
                }
                None => {
                    let (packets, overflows) = capture.as_ref().map_or((0, 0), |capture| (capture.packets(), capture.overflows()));
                    stats.set_track_attached(n, !*lost);
                    (packets, overflows, *lost)
                }
            };
            stats.set_track_instances(n, self.apps[n].as_ref().map(TrackLink::instances).unwrap_or_default());
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
    /// The audio stream slot of the track at `index`.
    fn track_slot(&self, index: usize) -> Result<usize, EngineError> {
        let slot = self.first_track_slot + index;
        if slot < self.stream_count { Ok(slot) } else { Err(EngineError::Failed("a track stream index is missing".into())) }
    }

    /// Takes the same number of frames from every track, hands on their PCM,
    /// sums the mixdown and hands on every stream's packets.
    fn emit_block(&mut self, frames: usize, sink: &dyn PacketSink, stats: &Stats) -> Result<(), EngineError> {
        let mut blocks: Vec<Vec<f32>> = self.pipelines.iter_mut().map(|pipeline| pipeline.take(frames)).collect();
        let common = blocks.iter().map(|block| block.len() / 2).min().unwrap_or(0);
        if common == 0 {
            return Ok(());
        }
        for block in &mut blocks {
            block.truncate(common * 2);
        }
        let mut wav_bytes = 0;
        for (index, (block, taken)) in blocks.iter().zip(self.taken.iter_mut()).enumerate() {
            wav_bytes += sink.pcm(index, *taken, block)?;
            *taken += common as u64;
        }
        stats.wav_bytes.fetch_add(wav_bytes, Ordering::Relaxed);
        if self.mixdown_on {
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
            // The mixdown is always the first stream.
            encode_stream(self.mix_encoder.as_mut(), 0, &mix, sink)?;
        }
        for (index, block) in blocks.iter().enumerate() {
            let slot = self.track_slot(index)?;
            let encoder = self.track_encoders.get_mut(index).and_then(Option::as_mut);
            encode_stream(encoder, slot, block, sink)?;
        }
        Ok(())
    }

    /// Finishes the encoders and hands on their last packets.
    fn finish_streams(&mut self, sink: &dyn PacketSink) -> Result<(), EngineError> {
        // A take without a mixdown has no mixdown stream, and may have no stream at all.
        if let Some(encoder) = self.mix_encoder.as_mut() {
            finish_stream(Some(encoder), 0, sink)?;
        }
        for index in 0..self.track_encoders.len() {
            let slot = self.track_slot(index)?;
            let encoder = self.track_encoders.get_mut(index).and_then(Option::as_mut);
            finish_stream(encoder, slot, sink)?;
        }
        Ok(())
    }
}

/// A link with no capture for every app and desktop-except track, in track order.
fn app_links(tracks: &[TrackSpec]) -> Vec<Option<TrackLink>> {
    tracks.iter().map(|track| track.app.as_ref().map(|app| TrackLink::new(&app.exe, app.except))).collect()
}

/// Polls every app link against a listing and returns the tracks that attached or detached.
fn poll_links(links: &mut [Option<TrackLink>], listing: &[AudioApp], system: &mut dyn AppSystem) -> Vec<(usize, AppChange)> {
    let mut changes = Vec::new();
    for (index, link) in links.iter_mut().enumerate() {
        let Some(link) = link else {
            continue;
        };
        let running = app_pids(listing, link.exe());
        if let Some(change) = link.poll(running, system) {
            changes.push((index, change));
        }
    }
    changes
}

/// The real captures behind the app links.
pub(super) struct WindowsApps;

impl AppSystem for WindowsApps {
    fn start_app(&mut self, exe: &str, mode: LoopbackMode) -> Result<Box<dyn AppSource>, CaptureError> {
        Ok(Box::new(AppLoopback::start(AppTarget::new(exe), mode)?))
    }

    fn start_desktop(&mut self) -> Result<Box<dyn AppSource>, CaptureError> {
        let device = list_audio_devices()?.into_iter().find(|device| device.kind == DeviceKind::Output && device.is_default);
        let device = device.ok_or(CaptureError::Missing("there is no default output device"))?;
        Ok(Box::new(AudioCapture::start(&device)?))
    }

    fn top_level_pids(&mut self, exe: &str) -> Vec<u32> {
        capture::top_level_pids(exe)
    }
}

impl AppSource for AudioCapture {
    fn receiver(&self) -> &Receiver<AudioPacket> {
        AudioCapture::receiver(self)
    }
    fn lost(&self) -> bool {
        self.device_lost()
    }
    fn instance_pids(&self) -> &[u32] {
        &[]
    }
    fn packets(&self) -> u64 {
        AudioCapture::packets(self)
    }
    fn overflows(&self) -> u64 {
        AudioCapture::overflows(self)
    }
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
    fn instances(&self) -> Instances {
        Instances { excluded: self.excluded_instances(), total: self.total_instances() }
    }
}

/// Lists the running apps once a second on a thread of its own, so a slow listing never delays the audio tick.
///
/// The thread ends within about a second of the receiver being dropped. If it cannot start, the receiver stays empty.
pub(super) fn spawn_app_lister() -> Receiver<Vec<AudioApp>> {
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

/// Feeds one interleaved block into its encoder and hands on the packets of the stream in `slot`.
fn encode_stream(encoder: Option<&mut AudioEncoder>, slot: usize, block: &[f32], sink: &dyn PacketSink) -> Result<(), EngineError> {
    let Some(encoder) = encoder else {
        return Ok(());
    };
    encoder.send(block)?;
    let mut send_error = None;
    encoder.receive(&mut |packet| {
        if let Err(err) = sink.audio(slot, packet) {
            send_error = Some(err);
        }
    })?;
    send_error.map_or(Ok(()), Err)
}

/// Pads the encoder's last frame and hands on the final packets of the stream in `slot`.
fn finish_stream(encoder: Option<&mut AudioEncoder>, slot: usize, sink: &dyn PacketSink) -> Result<(), EngineError> {
    let Some(encoder) = encoder else {
        return Ok(());
    };
    let mut send_error = None;
    encoder.finish(&mut |packet| {
        if let Err(err) = sink.audio(slot, packet) {
            send_error = Some(err);
        }
    })?;
    send_error.map_or(Ok(()), Err)
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
    use super::super::sink::fake::{FakeSink, Output};
    use super::super::{cam, video};
    use super::*;

    /// A track with no device, which the rig fills with silence.
    fn silent_track(name: &str) -> TrackSpec {
        TrackSpec {
            name: name.to_owned(),
            device: None,
            device_id: String::new(),
            device_name: String::new(),
            gain_db: 0.0,
            muted: false,
            offset_ms: 0,
            in_mixdown: true,
            app: None,
        }
    }

    #[test]
    fn a_fake_sink_receives_every_stream_a_take_writes() -> Result<(), EngineError> {
        media_io::init()?;
        let tracks = vec![silent_track("Mic"), silent_track("Desktop")];
        let stats = Stats::new(&tracks);
        let open = || AudioEncoder::open(AudioCodec::Aac { kbps: 320 }, OUT_RATE, OUT_CHANNELS);
        let mut rig = AudioRig::new(tracks, vec![None, None], Some(open()?), vec![Some(open()?), Some(open()?)], true, 0);
        let sink = FakeSink::default();
        rig.fill_silence(48_000);
        // Blocks of one audio tick, 10 ms, as a running take hands them on.
        while let Some(frames) = rig.ready_frames(480) {
            rig.emit_block(frames, &sink, &stats)?;
        }
        rig.finish_streams(&sink)?;
        // The video encoders need the GPU, so their packets enter where each thread takes them from its encoder.
        let packet = || media_io::EncodedPacket::from_raw_packet(ffmpeg_next::Packet::copy(&[0; 4]));
        let (mut video_err, mut cam_err) = (None, None);
        video::write_packet(packet(), &mut |packet| sink.video(packet), &mut video_err);
        cam::write_packet(packet(), &sink, &mut cam_err);
        assert!(video_err.is_none() && cam_err.is_none());

        let received = sink.received();
        let outputs = [Output::Video, Output::Camera, Output::Audio(0), Output::Audio(1), Output::Audio(2), Output::Pcm(0), Output::Pcm(1)];
        assert_eq!(received.keys().copied().collect::<Vec<_>>(), outputs);
        assert!(received.values().all(|count| *count > 0));
        let frames = 48_000 - OUTPUT_LEAD_FRAMES;
        assert_eq!((received.get(&Output::Pcm(0)), received.get(&Output::Pcm(1))), (Some(&frames), Some(&frames)));
        Ok(())
    }

    /// A rig with one app track for Discord, no encoders and no files, and its stats.
    fn discord_rig() -> Result<(AudioRig, Stats), EngineError> {
        let app = AppSpec { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned(), except: false };
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
        let rig = AudioRig::new(tracks, vec![None], None, vec![None], false, 0);
        Ok((rig, stats))
    }

    fn discord_running() -> Vec<AudioApp> {
        vec![AudioApp { exe: "discord.exe".to_owned(), display_name: "Discord".to_owned(), pids: vec![40, 41], active: true }]
    }

    #[test]
    fn silence_is_written_while_an_app_track_is_detached() -> Result<(), EngineError> {
        let (mut rig, stats) = discord_rig()?;
        let system = FakeSystem::default();
        let mut apps = system.clone();
        rig.fill_silence(48_000);
        let before = rig.pipelines[0].take(96_000);
        assert_eq!(before.len() as u64 / 2, 48_000 - OUTPUT_LEAD_FRAMES);
        assert!(before.iter().all(|sample| *sample == 0.0));

        // Attached: the app's packets reach the track.
        rig.poll_apps(&discord_running(), 1_000, &stats, &mut apps);
        let packet =
            AudioPacket { qpc_100ns: 10_000_000, frames: 480, channels: 2, sample_rate: OUT_RATE, samples: vec![0.5; 960], discontinuity: true, silent: false };
        assert!(system.latest_sender().is_some_and(|sender| sender.send(packet).is_ok()));
        rig.drain_captures(10_100_000, 1_000, &stats, &mut apps);
        assert!(rig.pipelines[0].take(96_000).iter().any(|sample| *sample > 0.1));

        // Detached again: silence up to the timeline, and the track is not reported as a lost device.
        rig.poll_apps(&[], 2_000, &stats, &mut apps);
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
        let mut apps = FakeSystem::default();
        for (t_ms, running) in [(1_000, false), (2_000, true), (3_000, true), (65_500, false), (66_500, false), (70_000, true)] {
            rig.poll_apps(&if running { discord_running() } else { Vec::new() }, t_ms, &stats, &mut apps);
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
            marks.app(&event.app_name, event.except, event.attached, event.t_ms);
        }
        marks.app("Spotify", true, true, 80_000);
        marks.app("Spotify", true, false, 90_000);
        let listed: Vec<_> = marks.recent().iter().cloned().collect();
        let expected = [
            (2_000, "App attached: Discord"),
            (65_500, "App detached: Discord"),
            (70_000, "App attached: Discord"),
            (80_000, "Now excluding Spotify"),
            (90_000, "Spotify closed, capturing all desktop audio"),
        ]
        .map(|(t_ms, label)| (::marks::format_tc(t_ms), "System".to_owned(), Some(label.to_owned())));
        assert_eq!(listed, expected);
        drop(marks);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
