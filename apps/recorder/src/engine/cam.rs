//! The camera ISO file: a second video pipeline on its own thread that writes the raw camera, unkeyed and at its
//! native size, to `{base}_cam.{ext}` with its own encoder, pacer and mux thread.
//!
//! The camera is already NV12, so a frame is copied into a hardware frame and encoded with no conversion.
//! Each frame is placed on the constant frame rate timeline by its own QPC time plus the webcam's sync offset.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture::{qpc_frequency, qpc_now};
use ffmpeg_next::codec::Parameters;
use media_io::encode::{Codec, ComboError, EncoderConfig, aligned_1080_value, check_combo_with};
use media_io::mux::{MuxThread, Muxer, SendParameters};
use media_io::{AmfEncoder, AudioCodec, D3d11Device, D3d11FramePool, EncodedPacket, Encoder, HwFrame, MediaError};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::pacer::Pacer;
use super::session::{ENCODER_TAG, SessionConfig, StartCommand, Stats, encoder_config, take_title};
use super::sink::{CaptureOutput, PacketSink};
use crate::config::{CompositeMode, Container};
use crate::webcam::CamShare;

/// How long the loop sleeps while it waits for the next camera frame.
const POLL: Duration = Duration::from_millis(1);

/// Frames may fail in a row this many times before the camera file is ended.
const MAX_CONSECUTIVE_FAILURES: u32 = 30;

/// The width of one bucket of the encode-time histogram, in milliseconds.
const TIME_BUCKET_MS: f64 = 0.1;

/// How many buckets the encode-time histogram has, so it covers 0 to 100 ms.
const TIME_BUCKETS: usize = 1000;

/// The audio codec the container check is run with: the camera file has no audio, so any codec every container takes will do.
fn no_audio() -> AudioCodec {
    AudioCodec::Aac { kbps: 320 }
}

/// The status bar line for a camera file that was encoded as HEVC because the container cannot hold its AV1 here.
static INFO_LINE: Mutex<Option<String>> = Mutex::new(None);

/// Whether the HEVC fallback has been logged yet; it is logged once per run.
static FALLBACK_LOGGED: AtomicBool = AtomicBool::new(false);

/// The info line the status bar shows for the camera file, if the last take plan made one.
pub fn info_line() -> Option<String> {
    INFO_LINE.lock().ok().and_then(|line| line.clone())
}

/// Replaces the info line.
fn set_info_line(line: Option<String>) {
    if let Ok(mut slot) = INFO_LINE.lock() {
        *slot = line;
    }
}

/// The codec the camera file is encoded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CameraCodec {
    /// The profile's codec, which the container holds at the camera's height.
    Profile,
    /// HEVC in place of AV1; the text is the info line that says why.
    HevcFallback(String),
}

/// Decides the camera file's codec. In MP4 or MOV, AV1 at a height the GPU cannot code exactly falls back to HEVC in the
/// same container; every other refusal stays an error.
pub(super) fn camera_codec(container: Container, codec: Codec, height: u32, aligned_1080: bool) -> Result<CameraCodec, ComboError> {
    match check_combo_with(container, codec, height, no_audio(), aligned_1080) {
        Ok(()) => Ok(CameraCodec::Profile),
        Err(ComboError::Av1Mp4Height { height }) if container.needs_finalize() => {
            check_combo_with(container, Codec::Hevc, height, no_audio(), aligned_1080)?;
            let name = container.extension().to_uppercase();
            Ok(CameraCodec::HevcFallback(format!("Camera ISO: HEVC (AV1 {height}p not supported in {name} on this GPU)")))
        }
        Err(err) => Err(err),
    }
}

/// The constant frame rate pacer of the camera file.
///
/// It is driven by the camera's own frame times instead of the wall clock: a frame stamped `qpc` plus the sync
/// offset claims every tick up to half a frame after its time, so each camera frame lands on the nearest tick.
pub struct CamPacer {
    /// The shared pacer that counts frames, duplicates and drops.
    pacer: Pacer,
    /// The sync offset in QPC ticks; positive delays the camera in the file, negative advances it.
    offset_qpc: i64,
    /// Half a frame in QPC ticks.
    half_tick: i64,
}

impl CamPacer {
    /// A pacer at `fps_num` over `fps_den` frames per second from the session's `start_qpc`, with the sync offset in ms.
    pub fn new(qpc_freq: i64, fps_num: u32, fps_den: u32, start_qpc: i64, offset_ms: i32) -> Self {
        let offset_qpc = (i128::from(offset_ms) * i128::from(qpc_freq) / 1000) as i64;
        let half_tick = (i128::from(qpc_freq) * i128::from(fps_den) / (2 * i128::from(fps_num.max(1)))) as i64;
        Self { pacer: Pacer::new(qpc_freq, fps_num, fps_den, start_qpc), offset_qpc, half_tick }
    }

    /// The tick indexes that a camera frame stamped `frame_qpc` fills; empty while paused or while the frame is too early.
    pub fn on_frame(&mut self, frame_qpc: i64) -> Range<u64> {
        self.pacer.due_frames(frame_qpc.saturating_add(self.offset_qpc).saturating_add(self.half_tick))
    }

    /// The ticks that the wall clock has passed and no camera frame filled, for the end of the take.
    pub fn tail(&mut self, now_qpc: i64) -> Range<u64> {
        self.pacer.due_frames(now_qpc)
    }

    /// The presentation timestamp of tick `n`.
    pub fn pts_for(&self, n: u64) -> i64 {
        self.pacer.pts_for(n)
    }

    /// Counts one sent or dropped frame; a repeated source sequence is a duplicate.
    pub fn record(&mut self, n: u64, source_seq: u64, sent: bool) {
        self.pacer.record(n, source_seq, sent);
    }

    /// Total frames recorded so far.
    pub fn frames(&self) -> u64 {
        self.pacer.frames()
    }

    /// Total recorded frames that repeated the previous camera frame.
    pub fn duplicated(&self) -> u64 {
        self.pacer.duplicated()
    }

    /// Total recorded frames that never reached the encoder.
    pub fn dropped(&self) -> u64 {
        self.pacer.dropped()
    }
}

/// How long each camera frame took to copy, send and drain, kept as a histogram so a long take uses no more memory than a short one.
struct CamTimes {
    /// How many frames fell into each 0.1 ms bucket.
    buckets: Vec<u64>,
    /// How many frames were timed, slow ones included.
    count: u64,
    /// The sum of all frame times, in milliseconds.
    sum_ms: f64,
    /// The slowest frame, in milliseconds.
    max_ms: f64,
}

impl CamTimes {
    /// An empty histogram.
    fn new() -> Self {
        Self { buckets: vec![0; TIME_BUCKETS], count: 0, sum_ms: 0.0, max_ms: 0.0 }
    }

    /// Adds one frame's time.
    fn add(&mut self, elapsed: Duration) {
        let ms = elapsed.as_secs_f64() * 1000.0;
        self.count += 1;
        self.sum_ms += ms;
        self.max_ms = self.max_ms.max(ms);
        if let Some(bucket) = self.buckets.get_mut((ms / TIME_BUCKET_MS) as usize) {
            *bucket += 1;
        }
    }

    /// The average frame time in milliseconds; 0 with no frames.
    fn average_ms(&self) -> f64 {
        if self.count == 0 { 0.0 } else { self.sum_ms / self.count as f64 }
    }

    /// The 95th percentile of the frame times in milliseconds, to the bucket width; 0 with no frames.
    fn p95_ms(&self) -> f64 {
        if self.count == 0 {
            return 0.0;
        }
        let rank = (self.count * 95).div_ceil(100);
        let mut seen = 0;
        for (index, frames) in self.buckets.iter().enumerate() {
            seen += frames;
            if seen >= rank {
                return ((index + 1) as f64 * TIME_BUCKET_MS).min(self.max_ms);
            }
        }
        self.max_ms
    }
}

/// The live counters of the camera file, shared with the UI thread through the session's stats.
pub(super) struct CamCounters {
    /// Whether the camera file has started recording in this take.
    started: AtomicBool,
    /// Whether the camera file is being written right now.
    running: AtomicBool,
    /// Whether the camera was lost during the take.
    lost: AtomicBool,
    /// Frames written to the camera encoder.
    frames: AtomicU64,
    /// Recorded frames that repeated the previous camera frame.
    duplicated: AtomicU64,
    /// Frames that never reached the camera encoder.
    dropped: AtomicU64,
    /// Packet bytes the camera mux thread has written.
    bytes: AtomicU64,
}

impl CamCounters {
    /// Counters for a take with no camera file yet.
    pub(super) fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            running: AtomicBool::new(false),
            lost: AtomicBool::new(false),
            frames: AtomicU64::new(0),
            duplicated: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    /// A plain copy of the counters, or none when the take has no camera file.
    pub(super) fn snapshot(&self) -> Option<CamSnapshot> {
        if !self.started.load(Ordering::Relaxed) {
            return None;
        }
        Some(CamSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            duplicated: self.duplicated.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            running: self.running.load(Ordering::Relaxed),
            lost: self.lost.load(Ordering::Relaxed),
        })
    }

    /// Packet bytes the camera mux thread has written so far.
    pub(super) fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Copies the pacer's counters and the mux byte count in.
    fn store(&self, pacer: &CamPacer, bytes: u64) {
        self.frames.store(pacer.frames(), Ordering::Relaxed);
        self.duplicated.store(pacer.duplicated(), Ordering::Relaxed);
        self.dropped.store(pacer.dropped(), Ordering::Relaxed);
        self.bytes.store(bytes, Ordering::Relaxed);
    }
}

/// A plain copy of the camera file's live counters at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CamSnapshot {
    /// Frames written to the camera encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous camera frame.
    pub duplicated: u64,
    /// Frames that never reached the camera encoder.
    pub dropped: u64,
    /// Whether the camera file is being written right now.
    pub running: bool,
    /// Whether the camera was lost during the take.
    pub lost: bool,
}

/// The totals of the finished camera file.
#[derive(Debug, Clone, PartialEq)]
pub struct CamSummary {
    /// The camera file.
    pub file: PathBuf,
    /// Frames written to the camera encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous camera frame.
    pub duplicated: u64,
    /// Frames that never reached the camera encoder.
    pub dropped: u64,
    /// The size of the finished file.
    pub bytes_written: u64,
    /// The length of the file in milliseconds.
    pub duration_ms: u64,
    /// The camera's frame rate, which sets the encode time budget.
    pub fps: f64,
    /// The average time per camera frame to copy, send and drain, in milliseconds.
    pub encode_avg_ms: f64,
    /// The 95th percentile of that per-frame time, in milliseconds.
    pub encode_p95_ms: f64,
    /// Whether the camera was lost before the take ended.
    pub lost: bool,
}

/// What a take needs to record the camera file, decided before the session starts its other threads.
pub(super) struct CamPlan {
    /// The open camera and its graphics device.
    share: CamShare,
    /// The encoder settings: the profile's codec at the camera's size and rate, with the webcam's rate control.
    encoder: EncoderConfig,
    /// The webcam's sync offset in ms.
    offset_ms: i32,
}

/// Works out whether the take records a camera file.
///
/// None when the profile has no webcam, or its ISO is off and the take is not Clean: a Clean take keeps the
/// camera out of the main video, so it records the camera file whatever the saved ISO switch says. The error is the text of the banner for a take that
/// could not have the camera file: the camera is not open, or the container cannot hold the codec.
pub(super) fn plan(cfg: &SessionConfig) -> Result<Option<CamPlan>, String> {
    set_info_line(None);
    let clean = cfg.profile.effective_composite_mode() == CompositeMode::Clean;
    let Some(webcam) = cfg.profile.webcam.as_ref().filter(|webcam| webcam.iso || clean) else { return Ok(None) };
    let Some(share) = crate::webcam::shared() else {
        return Err("The camera is not open, so this take has no camera file.".to_owned());
    };
    let format = share.capture.format();
    let mut encoder = EncoderConfig {
        width: format.width,
        height: format.height,
        fps_num: format.fps_num,
        fps_den: format.fps_den,
        rate_control: webcam.iso_rc,
        ..encoder_config(&cfg.profile)
    };
    encoder.validate().map_err(|err| format!("The camera file was skipped: {err}"))?;
    let codec = camera_codec(cfg.profile.container, encoder.codec, encoder.height, aligned_1080_value().is_some())
        .map_err(|err| format!("The camera file was skipped: {err}"))?;
    if let CameraCodec::HevcFallback(line) = codec {
        encoder.codec = Codec::Hevc;
        if !FALLBACK_LOGGED.swap(true, Ordering::Relaxed) {
            log::info!("{line}");
        }
        set_info_line(Some(line));
    }
    Ok(Some(CamPlan { share, encoder, offset_ms: webcam.offset_clamped() }))
}

/// The camera encoder's facts, which a take needs to open its camera file.
pub(super) struct CamSetup {
    /// The encoder's codec parameters, including extradata.
    pub(super) params: SendParameters,
    /// The time base the encoder stamps its packets in.
    pub(super) time_base: (i32, i32),
    /// The camera's frame rate as (numerator, denominator).
    pub(super) fps: (u32, u32),
}

/// What the camera thread reports back to the session.
enum CamMessage {
    /// The setup ended: the encoder is ready, with its facts, or this is why not.
    Setup(Result<CamSetup, String>),
    /// The camera file ended, with its totals or the reason it could not be finished.
    Finished(Result<CamSummary, String>),
}

/// The session's handle on the running camera thread.
pub(super) struct CamHandle {
    /// Releases the thread to start pacing.
    start_tx: Sender<StartCommand>,
    /// The thread's messages.
    rx: Receiver<CamMessage>,
    /// The thread, joined by `finish` or `abort`.
    handle: JoinHandle<()>,
}

impl CamHandle {
    /// Releases the thread to record; false when the thread has already ended.
    pub(super) fn begin(&self, command: StartCommand) -> bool {
        self.start_tx.send(command).is_ok()
    }

    /// Waits for the thread to finish the file and returns its totals; the stop flag must be set.
    pub(super) fn finish(self) -> Result<CamSummary, String> {
        let mut outcome = Err("the camera thread ended without reporting a summary".to_owned());
        while let Ok(message) = self.rx.recv() {
            if let CamMessage::Finished(result) = message {
                outcome = result;
                break;
            }
        }
        let _ = self.handle.join();
        outcome
    }

    /// Ends a camera thread that never started recording, and waits for it.
    pub(super) fn abort(self) {
        drop(self.start_tx);
        let _ = self.handle.join();
    }
}

/// Starts the "cam-video" thread, waits for it to open the encoder, and returns the handle with the encoder's facts.
/// The take opens the camera file, and the capture's output hands it the camera packets.
///
/// The error is the text of the banner for a camera file that could not be started.
pub(super) fn spawn(plan: CamPlan, stats: Arc<Stats>, stop: Arc<AtomicBool>) -> Result<(CamHandle, CamSetup), String> {
    let (tx, rx) = mpsc::channel();
    let (start_tx, start_rx) = mpsc::channel::<StartCommand>();
    let handle = std::thread::Builder::new()
        .name("cam-video".to_owned())
        .spawn(move || run(plan, &stats, &stop, &tx, &start_rx))
        .map_err(|err| format!("The camera file was skipped: its thread could not be started: {err}"))?;
    match rx.recv() {
        Ok(CamMessage::Setup(Ok(setup))) => Ok((CamHandle { start_tx, rx, handle }, setup)),
        Ok(CamMessage::Setup(Err(err))) => {
            let _ = handle.join();
            Err(format!("The camera file was skipped: {err}"))
        }
        Ok(CamMessage::Finished(_)) | Err(_) => {
            let _ = handle.join();
            Err("The camera file was skipped: its thread ended before it was ready.".to_owned())
        }
    }
}

/// Everything the per-frame path owns.
struct Pipeline {
    /// The hardware frames the camera textures are copied into; ends before the encoder.
    pool: D3d11FramePool,
    /// The camera file's encoder.
    encoder: AmfEncoder,
}

/// Entry point of the camera thread: sets up, waits for the start command, records until told to stop, then reports.
fn run(plan: CamPlan, stats: &Stats, stop: &AtomicBool, tx: &Sender<CamMessage>, start_rx: &Receiver<StartCommand>) {
    let (pipeline, facts) = match setup(&plan) {
        Ok(ready) => ready,
        Err(err) => {
            let _ = tx.send(CamMessage::Setup(Err(err)));
            return;
        }
    };
    if tx.send(CamMessage::Setup(Ok(facts))).is_err() {
        return;
    }
    let outcome = match start_rx.recv() {
        Ok(command) => record(&plan, stats, stop, pipeline, command),
        Err(_) => Err("the session stopped before recording started".to_owned()),
    };
    let _ = tx.send(CamMessage::Finished(outcome));
}

/// Opens the frame pool and the encoder for the camera, and reports the encoder's facts.
fn setup(plan: &CamPlan) -> Result<(Pipeline, CamSetup), String> {
    let device = D3d11Device::from_device(plan.share.gpu.device()).map_err(|err| err.to_string())?;
    let pool = D3d11FramePool::new(&device, plan.encoder.width, plan.encoder.height, 8).map_err(|err| err.to_string())?;
    let encoder = AmfEncoder::open(&plan.encoder, &pool).map_err(|err| err.to_string())?;
    let fps = (plan.encoder.fps_num, plan.encoder.fps_den);
    let facts = CamSetup { params: SendParameters(encoder.parameters()), time_base: encoder.time_base(), fps };
    Ok((Pipeline { pool, encoder }, facts))
}

/// The name of a take's camera ISO file: `{base_name}_cam` with the container's extension, in the take folder.
pub(super) fn file_path(take_folder: &Path, base_name: &str, container: Container) -> PathBuf {
    take_folder.join(format!("{base_name}_cam.{}", container.extension()))
}

/// Creates the camera file's muxer with its one video stream and starts its mux thread.
pub(super) fn open_muxer(file: &Path, container: Container, params: Parameters, time_base: (i32, i32), fps: (i32, i32)) -> Result<MuxThread, String> {
    let mut muxer = Muxer::create(file, container).map_err(|err| err.to_string())?;
    muxer.set_tags(&take_title(file), ENCODER_TAG);
    muxer.add_video_titled(params, time_base, fps, "Camera").map_err(|err| err.to_string())?;
    muxer.start().map_err(|err| err.to_string())?;
    MuxThread::start(muxer).map_err(|err| err.to_string())
}

/// The recording loop's state: the pipeline, its pacer and the per-frame bookkeeping.
struct Recorder<'a> {
    pipeline: Pipeline,
    output: Arc<CaptureOutput>,
    pacer: CamPacer,
    times: CamTimes,
    plan: &'a CamPlan,
    stats: &'a Stats,
    /// How many frames failed in a row.
    failures: u32,
}

impl Recorder<'_> {
    /// Encodes the camera texture for tick `n`; false once too many frames in a row failed.
    fn emit(&mut self, n: u64, texture: &ID3D11Texture2D, seq: u64) -> bool {
        let take = self.output.camera_frame(n);
        if take.force_keyframe {
            self.pipeline.encoder.force_keyframe();
        }
        let timer = Instant::now();
        let sent = encode_one_frame(&mut self.pipeline, self.output.as_ref(), self.plan, texture, self.pacer.pts_for(n));
        self.times.add(timer.elapsed());
        match &sent {
            Ok(()) => self.failures = 0,
            Err(err) => {
                self.failures += 1;
                self.stats.set_last_error(format!("Camera encode: {err}"));
            }
        }
        if sent.is_err() {
            self.output.camera_failed(n);
        }
        // The pacer keeps the capture's clock; its counters only count the take's frames.
        if take.in_take {
            self.pacer.record(n, seq, sent.is_ok());
        }
        self.stats.cam.store(&self.pacer, self.output.camera_bytes());
        self.failures < MAX_CONSECUTIVE_FAILURES
    }
}

/// Runs the paced recording loop from the shared start QPC, then finishes the camera file.
fn record(plan: &CamPlan, stats: &Stats, stop: &AtomicBool, pipeline: Pipeline, command: StartCommand) -> Result<CamSummary, String> {
    let StartCommand { start_qpc, output } = command;
    let capture = &plan.share.capture;
    let counters = &stats.cam;
    let pacer = CamPacer::new(qpc_frequency(), plan.encoder.fps_num, plan.encoder.fps_den, start_qpc, plan.offset_ms);
    let mut rec = Recorder { pipeline, output, pacer, times: CamTimes::new(), plan, stats, failures: 0 };
    counters.started.store(true, Ordering::Relaxed);
    counters.running.store(true, Ordering::Relaxed);
    let mut last_seq = 0u64;
    let mut lost = false;
    let mut broken = false;

    while !stop.load(Ordering::Relaxed) {
        if let Some(error) = capture.error() {
            log::warn!("the camera was lost: {error}");
            lost = true;
            break;
        }
        let Some(frame) = capture.latest().filter(|frame| frame.seq != last_seq) else {
            std::thread::sleep(POLL);
            continue;
        };
        last_seq = frame.seq;
        for n in rec.pacer.on_frame(frame.qpc) {
            if !rec.emit(n, &frame.texture, frame.seq) {
                broken = true;
                break;
            }
        }
        if broken {
            break;
        }
    }

    counters.running.store(false, Ordering::Relaxed);
    if lost {
        counters.lost.store(true, Ordering::Relaxed);
    } else if broken {
        stats.set_last_error("The camera file stopped: its frames kept failing to encode.".to_owned());
    } else if capture.error().is_none()
        && let Some(frame) = capture.latest()
    {
        // The take ended: the ticks the wall clock passed after the last camera frame repeat it, so the file is as long as the take.
        for n in rec.pacer.tail(qpc_now()) {
            if !rec.emit(n, &frame.texture, frame.seq) {
                break;
            }
        }
    }
    finish_file(rec, plan, lost)
}

/// Copies one camera texture into a pool frame, encodes it and hands on the packets.
fn encode_one_frame(pipeline: &mut Pipeline, sink: &dyn PacketSink, plan: &CamPlan, texture: &ID3D11Texture2D, pts: i64) -> Result<(), MediaError> {
    let mut hw = HwFrame::from_pool(&pipeline.pool)?;
    hw.copy_from(plan.share.gpu.context(), texture)?;
    hw.set_pts(pts);
    pipeline.encoder.send(hw)?;
    let mut sink_err: Option<MediaError> = None;
    pipeline.encoder.receive(&mut |packet| write_packet(packet, sink, &mut sink_err))?;
    sink_err.map_or(Ok(()), Err)
}

/// Hands one camera packet on; the first failure is kept.
pub(super) fn write_packet(packet: EncodedPacket, sink: &dyn PacketSink, sink_err: &mut Option<MediaError>) {
    if sink_err.is_some() {
        return;
    }
    if let Err(err) = sink.camera(packet) {
        *sink_err = Some(err);
    }
}

/// Drains the encoder, writes the trailer and builds the totals.
fn finish_file(rec: Recorder<'_>, plan: &CamPlan, lost: bool) -> Result<CamSummary, String> {
    let Recorder { pipeline, output, pacer, times, stats, .. } = rec;
    let Pipeline { pool, mut encoder } = pipeline;
    let mut sink_err: Option<MediaError> = None;
    let drained = encoder.finish(&mut |packet| write_packet(packet, output.as_ref(), &mut sink_err));
    let closed = output.close_camera();
    drop(output);
    drop(encoder);
    drop(pool);
    drained.map_err(|err| err.to_string())?;
    if let Some(err) = sink_err {
        return Err(err.to_string());
    }
    let closed = closed.ok_or_else(|| "the take has no camera file".to_owned())?;
    let bytes_written = closed.bytes?;
    stats.cam.store(&pacer, bytes_written);
    let (fps_num, fps_den) = (u64::from(plan.encoder.fps_num.max(1)), u64::from(plan.encoder.fps_den));
    Ok(CamSummary {
        file: closed.file,
        frames: pacer.frames(),
        duplicated: pacer.duplicated(),
        dropped: pacer.dropped(),
        bytes_written,
        duration_ms: pacer.frames().saturating_mul(1000).saturating_mul(fps_den) / fps_num,
        fps: fps_num as f64 / fps_den.max(1) as f64,
        encode_avg_ms: times.average_ms(),
        encode_p95_ms: times.p95_ms(),
        lost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn av1_in_mp4_at_a_height_the_gpu_cannot_code_exactly_falls_back_to_hevc() {
        let line = "Camera ISO: HEVC (AV1 1080p not supported in MP4 on this GPU)".to_owned();
        assert_eq!(camera_codec(Container::Mp4, Codec::Av1, 1080, false), Ok(CameraCodec::HevcFallback(line)));
        // The GPU codes 1080 exactly, or the height is a multiple of 16: the profile's codec stays.
        assert_eq!(camera_codec(Container::Mp4, Codec::Av1, 1080, true), Ok(CameraCodec::Profile));
        assert_eq!(camera_codec(Container::Mp4, Codec::Av1, 1088, false), Ok(CameraCodec::Profile));
        // Matroska takes AV1 at any height, and other codecs never fall back.
        assert_eq!(camera_codec(Container::Mkv, Codec::Av1, 1080, false), Ok(CameraCodec::Profile));
        assert_eq!(camera_codec(Container::Mp4, Codec::Hevc, 1080, false), Ok(CameraCodec::Profile));
        // MOV cannot hold AV1 at all, which is not a height problem, so that refusal stays.
        assert_eq!(camera_codec(Container::Mov, Codec::Av1, 1080, false), Err(ComboError::MovCodec));
    }

    /// A real QPC frequency: 100 ns ticks.
    const QPC_FREQ: i64 = 10_000_000;

    /// A pacer at 30 fps from QPC 0 with this sync offset.
    fn pacer(offset_ms: i32) -> CamPacer {
        CamPacer::new(QPC_FREQ, 30, 1, 0, offset_ms)
    }

    /// The QPC time of camera frame `k` of a perfect 30 fps camera that started at 0.
    fn frame_time(k: i64) -> i64 {
        k * QPC_FREQ / 30
    }

    /// Feeds frames `from..to` and returns, for each frame, the ticks it filled.
    fn feed(pacer: &mut CamPacer, from: i64, to: i64) -> Vec<Vec<u64>> {
        (from..to).map(|k| pacer.on_frame(frame_time(k)).collect()).collect()
    }

    #[test]
    fn with_no_offset_every_camera_frame_fills_its_own_tick() {
        let mut pacer = pacer(0);
        let ticks = feed(&mut pacer, 0, 90);
        for (k, filled) in ticks.iter().enumerate() {
            assert_eq!(filled, &vec![k as u64], "frame {k}");
        }
    }

    #[test]
    fn a_positive_offset_delays_the_camera_and_pads_the_start_with_its_first_frame() {
        // 100 ms is 3 frames at 30 fps: the first frame fills ticks 0 to 3, and every later frame lands 3 ticks late.
        let mut pacer = pacer(100);
        let ticks = feed(&mut pacer, 0, 10);
        assert_eq!(ticks[0], vec![0, 1, 2, 3]);
        for (k, filled) in ticks.iter().enumerate().skip(1) {
            assert_eq!(filled, &vec![k as u64 + 3], "frame {k}");
        }
    }

    #[test]
    fn a_negative_offset_advances_the_camera_and_drops_its_first_frames() {
        // -100 ms: the first 3 frames fall before the start of the file, and frame 3 becomes tick 0.
        let mut pacer = pacer(-100);
        let ticks = feed(&mut pacer, 0, 10);
        for filled in &ticks[..3] {
            assert!(filled.is_empty());
        }
        for (k, filled) in ticks.iter().enumerate().skip(3) {
            assert_eq!(filled, &vec![k as u64 - 3], "frame {k}");
        }
    }

    #[test]
    fn an_offset_in_ms_scales_with_the_qpc_frequency() {
        // A 1 kHz style clock: 1 tick is 1 ms, so 100 ms is 100 ticks and a frame is 33 ticks.
        let mut pacer = CamPacer::new(1000, 30, 1, 0, 100);
        assert_eq!(pacer.on_frame(0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_camera_that_skips_a_frame_repeats_the_one_before_it_in_the_count() {
        let mut pacer = pacer(0);
        let first: Vec<u64> = pacer.on_frame(frame_time(0)).collect();
        assert_eq!(first, vec![0]);
        // Frame 1 never arrives: frame 2 fills ticks 1 and 2, so one tick is a repeat of the same source frame.
        let late: Vec<u64> = pacer.on_frame(frame_time(2)).collect();
        assert_eq!(late, vec![1, 2]);
        pacer.record(0, 1, true);
        pacer.record(1, 3, true);
        pacer.record(2, 3, true);
        assert_eq!((pacer.frames(), pacer.duplicated(), pacer.dropped()), (3, 1, 0));
    }

    #[test]
    fn the_tail_fills_the_ticks_the_wall_clock_passed() {
        let mut pacer = pacer(0);
        feed(&mut pacer, 0, 30);
        // The take ends at 1.1 s: ticks 30 to 32 are due on the wall clock.
        let tail: Vec<u64> = pacer.tail(QPC_FREQ + QPC_FREQ / 10).collect();
        assert_eq!(tail, vec![30, 31, 32]);
        assert!(pacer.tail(QPC_FREQ + QPC_FREQ / 10).is_empty());
    }

    #[test]
    fn a_tick_is_never_given_out_twice() {
        let mut pacer = pacer(-40);
        let mut seen = Vec::new();
        for k in 0..300 {
            seen.extend(pacer.on_frame(frame_time(k)));
        }
        for pair in seen.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn the_encode_times_give_an_average_and_a_95th_percentile() {
        let empty = CamTimes::new();
        assert_eq!((empty.average_ms(), empty.p95_ms()), (0.0, 0.0));
        let mut times = CamTimes::new();
        for _ in 0..95 {
            times.add(Duration::from_micros(1000));
        }
        for _ in 0..5 {
            times.add(Duration::from_micros(10_000));
        }
        assert!((times.average_ms() - 1.45).abs() < 1e-6);
        assert!((times.p95_ms() - 1.0).abs() <= 0.11);
    }
}
