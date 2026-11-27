//! The video thread: the latest composite, NV12 conversion and AMF encode on one thread,
//! paced at the exact profile frame rate from the shared start QPC, with its
//! packets handed to the session's sink. The composite comes from the compose thread.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use capture::{Gpu, Nv12Converter, list_monitors, qpc_frequency, qpc_now};
use media_io::mux::SendParameters;
use media_io::{AmfEncoder, D3d11Device, D3d11FramePool, EncodedPacket, Encoder, HwFrame, MediaError};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::pacer::Pacer;
use super::session::{EngineError, SessionConfig, StartCommand, Stats, Summary, ThreadMessage, VideoSetup, encoder_config};
use super::sink::PacketSink;
use crate::compositor::{self, ComposeConfig, Composite, CompositorThread, PinGuard};

/// Frames may fail in a row this many times before the session stops itself.
const MAX_CONSECUTIVE_FAILURES: u32 = 30;

/// The width of one bucket of the encode-time histogram, in milliseconds.
const TIME_BUCKET_MS: f64 = 0.1;

/// How many buckets the encode-time histogram has, so it covers 0 to 100 ms; slower frames count in the average and the maximum only.
const TIME_BUCKETS: usize = 1000;

/// How long each frame took to convert, send and drain, kept as a histogram so a long take uses no more memory than a short one.
struct EncodeTimes {
    /// How many frames fell into each 0.1 ms bucket.
    buckets: Vec<u64>,
    /// How many frames were timed, slow ones included.
    count: u64,
    /// The sum of all frame times, in milliseconds.
    sum_ms: f64,
    /// The slowest frame, in milliseconds.
    max_ms: f64,
}

impl EncodeTimes {
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
        // The rank of the frame the 95th percentile falls on, counting from 1.
        let rank = (self.count * 95).div_ceil(100);
        let mut seen = 0;
        for (index, frames) in self.buckets.iter().enumerate() {
            seen += frames;
            if seen >= rank {
                return ((index + 1) as f64 * TIME_BUCKET_MS).min(self.max_ms);
            }
        }
        // The rank lies among the frames slower than the last bucket.
        self.max_ms
    }
}

/// Everything the per-frame path needs, set up once at session start.
struct Pipeline {
    /// Holds the compositor for the take; the frames of the take are its composites.
    compositor: PinGuard,
    gpu: Gpu,
    converter: Nv12Converter,
    nv12: ID3D11Texture2D,
    pool: D3d11FramePool,
    encoder: AmfEncoder,
    /// The compositor this session started itself when the window has none; it ends after the guard above.
    _private_compositor: Option<CompositorThread>,
}

/// Entry point of the video thread: sets up, waits for the start command,
/// records until told to stop, then reports.
pub(super) fn run(cfg: SessionConfig, stats: Arc<Stats>, stop: Arc<AtomicBool>, tx: Sender<ThreadMessage>, start_rx: Receiver<StartCommand>) {
    let outcome = match setup(&cfg) {
        Err(err) => {
            let _ = tx.send(ThreadMessage::Setup(Err(err)));
            return;
        }
        Ok((pipeline, facts)) => {
            if tx.send(ThreadMessage::Setup(Ok(facts))).is_err() {
                return;
            }
            match start_rx.recv() {
                Ok(command) => record(&cfg, &stats, &stop, pipeline, command),
                Err(_) => Err(EngineError::Failed("the session stopped before recording started".into())),
            }
        }
    };
    let _ = tx.send(ThreadMessage::Finished(outcome));
}

/// Pins the compositor, sets the converter, the frame pool and the encoder up on its device, and
/// reports the encoder facts the session needs for the mux stream.
fn setup(cfg: &SessionConfig) -> Result<(Pipeline, VideoSetup), EngineError> {
    let profile = &cfg.profile;
    list_monitors()?.into_iter().nth(cfg.monitor).ok_or(EngineError::MonitorIndex(cfg.monitor))?;
    // The window's compositor serves the take; without one, for example in a test, the session starts its own.
    let (handle, private_compositor) = match compositor::shared() {
        Some(handle) => (handle, None),
        None => {
            let thread = CompositorThread::start(cfg.adapter)?;
            (thread.handle().clone(), Some(thread))
        }
    };
    let config = ComposeConfig::from_profile(profile, (profile.width, profile.height), cfg.monitor, cfg.cursor);
    let compositor = handle.pin(config.layout).map_err(EngineError::Failed)?;
    compositor.set_config(config);
    let gpu = compositor.gpu().clone();
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let first = wait_for_first_frame(&compositor, profile.width, profile.height).ok_or_else(|| match compositor.error() {
        Some(error) => EngineError::Failed(format!("no composite frame arrived: {error}")),
        None => EngineError::Failed("no composite frame arrived from the compositor".into()),
    })?;
    // The composite already has the output size, so the converter only changes the pixel format.
    let converter = Nv12Converter::new(&gpu, first.width, first.height, profile.width, profile.height, profile.fps)?;
    let nv12 = converter.create_output()?;
    let pool = D3d11FramePool::new(&hw_device, profile.width, profile.height, 8)?;
    let encoder = AmfEncoder::open(&encoder_config(profile), &pool)?;
    let facts = VideoSetup { params: SendParameters(encoder.parameters()), time_base: encoder.time_base() };
    let pipeline = Pipeline { compositor, gpu, converter, nv12, pool, encoder, _private_compositor: private_compositor };
    Ok((pipeline, facts))
}

/// Runs the paced recording loop from the shared start QPC, then drains the
/// encoder into the sink.
fn record(cfg: &SessionConfig, stats: &Stats, stop: &AtomicBool, mut pipeline: Pipeline, command: StartCommand) -> Result<Summary, EngineError> {
    let profile = &cfg.profile;
    let qpc_freq = qpc_frequency();
    let start_qpc = command.start_qpc;
    let output = command.output;
    // The pacer keeps the capture's clock, which never pauses; its counters only count the take's frames.
    let mut pacer = Pacer::new(qpc_freq, profile.fps, 1, start_qpc);
    // A mirror of the pacer's next frame index, used to sleep up to the next tick without ever sleeping past it.
    let mut next_index = 0u64;
    let mut consecutive_failures = 0u32;
    let mut frame_error: Option<EngineError> = None;
    let mut times = EncodeTimes::new();

    'record: loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let due = pacer.due_frames(qpc_now());
        if due.is_empty() {
            wait_for_next_tick(qpc_freq, profile.fps, start_qpc, next_index);
            continue;
        }
        for n in due {
            next_index = n + 1;
            let Some(frame) = pipeline.compositor.latest() else {
                // No composite yet: wait without counting.
                continue;
            };
            let pts = pacer.pts_for(n);
            let plan = output.video_frame(n);
            if plan.force_keyframe {
                pipeline.encoder.force_keyframe();
            }
            let timer = Instant::now();
            let encoded = encode_one_frame(&mut pipeline, &mut |packet| output.video(packet), &frame, pts);
            times.add(timer.elapsed());
            if encoded.is_err() {
                output.video_failed(n);
            }
            if plan.in_take {
                pacer.record(n, frame.content_seq, encoded.is_ok());
            }
            store_pacer_counters(stats, &pacer, frame.screen_received);
            // About once a second, for a take that ends while the capture goes on.
            if n.is_multiple_of(u64::from(profile.fps.max(1))) {
                stats.set_encode_times(times.average_ms(), times.p95_ms());
            }
            match encoded {
                Ok(()) => {
                    consecutive_failures = 0;
                }
                Err(err) => {
                    consecutive_failures += 1;
                    stats.set_last_error(err.to_string());
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        frame_error = Some(err);
                        break 'record;
                    }
                }
            }
        }
    }

    let Pipeline { encoder, .. } = pipeline;
    match frame_error {
        // Still drain the encoder so the take stays playable to the last frame.
        Some(err) => {
            let _ = finish_file(encoder, output.as_ref());
            Err(err)
        }
        None => finish_file(encoder, output.as_ref()).and_then(|()| build_summary(&cfg.output_file, &pacer, profile.fps, &times)),
    }
}

/// Where the video thread's encoded packets go.
type VideoSink<'a> = &'a mut dyn FnMut(EncodedPacket) -> Result<(), MediaError>;

/// Converts, encodes and hands on one due frame.
fn encode_one_frame(pipeline: &mut Pipeline, sink: VideoSink<'_>, frame: &Composite, pts: i64) -> Result<(), EngineError> {
    pipeline.converter.convert(&frame.texture, &pipeline.nv12)?;
    let mut hw = HwFrame::from_pool(&pipeline.pool)?;
    hw.copy_from(pipeline.gpu.context(), &pipeline.nv12)?;
    hw.set_pts(pts);
    pipeline.encoder.send(hw)?;
    let mut sink_err: Option<MediaError> = None;
    pipeline.encoder.receive(&mut |packet| write_packet(packet, &mut *sink, &mut sink_err))?;
    if let Some(err) = sink_err {
        return Err(err.into());
    }
    Ok(())
}

/// Hands one packet on; the first failure is kept.
pub(super) fn write_packet(packet: EncodedPacket, sink: VideoSink<'_>, sink_err: &mut Option<MediaError>) {
    if sink_err.is_some() {
        return;
    }
    if let Err(err) = sink(packet) {
        *sink_err = Some(err);
    }
}

/// Drains the encoder into the sink; the mux thread writes the trailer
/// when the session finishes it.
fn finish_file(mut encoder: AmfEncoder, sink: &dyn PacketSink) -> Result<(), EngineError> {
    let mut sink_err: Option<MediaError> = None;
    encoder.finish(&mut |packet| write_packet(packet, &mut |packet| sink.video(packet), &mut sink_err))?;
    if let Some(err) = sink_err {
        return Err(err.into());
    }
    Ok(())
}

/// Builds the end-of-recording summary from the pacer and the frame times; stop() fills in the
/// muxed bytes, the per-stream packet counts and the audio peaks.
fn build_summary(file: &Path, pacer: &Pacer, fps: u32, times: &EncodeTimes) -> Result<Summary, EngineError> {
    let frames = pacer.frames();
    let duration_ms = if fps == 0 { 0 } else { frames * 1000 / u64::from(fps) };
    Ok(Summary {
        file: file.to_path_buf(),
        wav_files: Vec::new(),
        frames,
        duplicated: pacer.duplicated(),
        dropped: pacer.dropped(),
        bytes_written: 0,
        packets: Vec::new(),
        duration_ms,
        average_mbps: 0.0,
        encode_avg_ms: times.average_ms(),
        encode_p95_ms: times.p95_ms(),
        track_peaks: Vec::new(),
    })
}

/// Copies the pacer's counters and the screen capture's received count into the stats the UI thread reads.
fn store_pacer_counters(stats: &Stats, pacer: &Pacer, received: u64) {
    stats.frames.store(pacer.frames(), Ordering::Relaxed);
    stats.duplicated.store(pacer.duplicated(), Ordering::Relaxed);
    stats.dropped.store(pacer.dropped(), Ordering::Relaxed);
    stats.frames_received.store(received, Ordering::Relaxed);
}

/// Waits up to three seconds for the first composite of the output size.
fn wait_for_first_frame(compositor: &PinGuard, width: u32, height: u32) -> Option<Composite> {
    for _ in 0..300 {
        if let Some(frame) = compositor.latest()
            && frame.width == width
            && frame.height == height
        {
            return Some(frame);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Sleeps until about 1 ms before the next tick, then yields until it is due.
fn wait_for_next_tick(qpc_freq: i64, fps: u32, start_qpc: i64, next_index: u64) {
    let tick = tick_time(start_qpc, next_index, qpc_freq, fps);
    let now = qpc_now();
    let one_ms = (qpc_freq / 1000).max(1);
    if tick - now > one_ms {
        let sleep_ticks = tick - one_ms - now;
        let nanos = (i128::from(sleep_ticks) * 1_000_000_000i128 / i128::from(qpc_freq)).max(0) as u64;
        std::thread::sleep(Duration::from_nanos(nanos));
    }
    while qpc_now() <= tick {
        std::thread::yield_now();
    }
}

/// The QPC value at which frame index `n` becomes due, from the capture's start.
fn tick_time(clock_start: i64, n: u64, qpc_freq: i64, fps: u32) -> i64 {
    let offset = i128::from(n) * i128::from(qpc_freq) / i128::from(fps);
    (i128::from(clock_start) + offset) as i64
}

#[cfg(test)]
#[path = "video_amf_tests.rs"]
mod amf_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// A histogram filled with `fast` frames of 1 ms and `slow` frames of 10 ms.
    fn times(fast: u32, slow: u32) -> EncodeTimes {
        let mut times = EncodeTimes::new();
        for _ in 0..fast {
            times.add(Duration::from_micros(1000));
        }
        for _ in 0..slow {
            times.add(Duration::from_micros(10_000));
        }
        times
    }

    #[test]
    fn no_frames_give_zero() {
        let empty = EncodeTimes::new();
        assert_eq!(empty.average_ms(), 0.0);
        assert_eq!(empty.p95_ms(), 0.0);
    }

    #[test]
    fn the_average_is_the_mean_frame_time() {
        let mixed = times(90, 10);
        assert!((mixed.average_ms() - 1.9).abs() < 1e-6, "average was {}", mixed.average_ms());
    }

    #[test]
    fn the_95th_percentile_is_the_time_frame_95_of_100_took() {
        // 95 fast frames: the 95th is still fast. 94 fast frames: the 95th is a slow one.
        let fast = times(95, 5).p95_ms();
        assert!((fast - 1.0).abs() <= 0.11, "p95 was {fast}");
        let slow = times(94, 6).p95_ms();
        assert!((slow - 10.0).abs() <= 0.11, "p95 was {slow}");
    }

    #[test]
    fn frames_slower_than_the_histogram_still_count() {
        let mut times = EncodeTimes::new();
        for _ in 0..10 {
            times.add(Duration::from_millis(250));
        }
        assert!((times.p95_ms() - 250.0).abs() < 1e-6);
        assert!((times.average_ms() - 250.0).abs() < 1e-6);
    }
}
