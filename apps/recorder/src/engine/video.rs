//! The video thread: capture, NV12 conversion, AMF encode and MKV mux on one
//! thread, paced at the exact profile frame rate from the QPC clock.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use capture::{Gpu, LatestFrame, MonitorCapture, Nv12Converter, list_monitors, qpc_frequency, qpc_now};
use media_io::mux::Container;
use media_io::{AmfEncoder, D3d11Device, D3d11FramePool, EncodedPacket, Encoder, HwFrame, MediaError, Muxer};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

use super::pacer::Pacer;
use super::session::{EngineError, SessionConfig, Stats, Summary, ThreadMessage, encoder_config};

/// Frames may fail in a row this many times before the session stops itself.
const MAX_CONSECUTIVE_FAILURES: u32 = 30;

/// Everything the per-frame path needs, set up once at session start.
struct Pipeline {
    capture: MonitorCapture,
    gpu: Gpu,
    converter: Nv12Converter,
    nv12: ID3D11Texture2D,
    pool: D3d11FramePool,
    encoder: AmfEncoder,
    muxer: Muxer,
    stream: usize,
}

/// Entry point of the video thread: records until told to stop, then reports.
pub(super) fn run(cfg: SessionConfig, stats: Arc<Stats>, stop: Arc<AtomicBool>, tx: Sender<ThreadMessage>) {
    let outcome = record(&cfg, &stats, &stop, &tx);
    let _ = tx.send(ThreadMessage::Finished(outcome));
}

/// Sets everything up, reports startup, then runs the paced recording loop.
fn record(cfg: &SessionConfig, stats: &Stats, stop: &AtomicBool, tx: &Sender<ThreadMessage>) -> Result<Summary, EngineError> {
    let profile = &cfg.profile;
    let gpu = Gpu::create(Some(cfg.adapter))?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let monitor = list_monitors()?.into_iter().nth(cfg.monitor).ok_or(EngineError::MonitorIndex(cfg.monitor))?;
    let capture = MonitorCapture::start(&gpu, &monitor, cfg.cursor)?;
    let first = wait_for_first_frame(&capture).ok_or_else(|| EngineError::Failed("no frame arrived from the capture session".into()))?;
    let converter = Nv12Converter::new(&gpu, first.width as u32, first.height as u32, profile.width, profile.height, profile.fps)?;
    let nv12 = converter.create_output()?;
    let pool = D3d11FramePool::new(&hw_device, profile.width, profile.height, 8)?;
    let encoder = AmfEncoder::open(&encoder_config(profile), &pool)?;
    let mut muxer = Muxer::create(&cfg.output_file, Container::Mkv)?;
    let stream = muxer.add_video(encoder.parameters(), encoder.time_base(), (profile.fps as i32, 1))?;
    muxer.start()?;
    let mut pipeline = Pipeline { capture, gpu, converter, nv12, pool, encoder, muxer, stream };

    tx.send(ThreadMessage::Started).map_err(|_| EngineError::Failed("the session was dropped before recording started".into()))?;

    let qpc_freq = qpc_frequency();
    let start_qpc = qpc_now();
    stats.started_qpc.store(start_qpc as u64, Ordering::Relaxed);
    let mut pacer = Pacer::new(qpc_freq, profile.fps, 1, start_qpc);
    // Mirrors of the pacer's shifted start and next frame index, used to sleep
    // up to the next tick without ever sleeping past it.
    let mut clock_start = start_qpc;
    let mut next_index = 0u64;
    let mut paused = false;
    let mut paused_at: Option<i64> = None;
    let mut consecutive_failures = 0u32;
    let mut frame_error: Option<EngineError> = None;

    'record: loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let want_pause = stats.paused.load(Ordering::Relaxed);
        if want_pause != paused {
            let now = qpc_now();
            if want_pause {
                pacer.pause(now);
                paused_at = Some(now);
            } else {
                if let Some(at) = paused_at.take() {
                    clock_start += now - at;
                }
                pacer.resume(now);
            }
            paused = want_pause;
        }

        let due = pacer.due_frames(qpc_now());
        if due.is_empty() {
            wait_for_next_tick(qpc_freq, profile.fps, clock_start, next_index, paused);
            continue;
        }
        for n in due {
            next_index = n + 1;
            let Some(frame) = pipeline.capture.latest() else {
                // No captured frame yet: wait without counting.
                continue;
            };
            let pts = pacer.pts_for(n);
            match encode_one_frame(&mut pipeline, stats, &frame, pts) {
                Ok(()) => {
                    consecutive_failures = 0;
                    pacer.record(n, frame.seq, true);
                    store_pacer_counters(stats, &pacer, pipeline.capture.frames_received());
                }
                Err(err) => {
                    consecutive_failures += 1;
                    stats.set_last_error(err.to_string());
                    pacer.record(n, frame.seq, false);
                    store_pacer_counters(stats, &pacer, pipeline.capture.frames_received());
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        frame_error = Some(err);
                        break 'record;
                    }
                }
            }
        }
    }

    pipeline.capture.stop();
    let Pipeline { encoder, muxer, stream, .. } = pipeline;
    match frame_error {
        // Still finish the file so the take stays playable up to the last frame.
        Some(err) => {
            let _ = finish_file(encoder, muxer, stream, stats);
            Err(err)
        }
        None => finish_file(encoder, muxer, stream, stats).and_then(|()| build_summary(&cfg.output_file, &pacer, stats, profile.fps)),
    }
}

/// Converts, encodes and muxes one due frame.
fn encode_one_frame(pipeline: &mut Pipeline, stats: &Stats, frame: &LatestFrame, pts: i64) -> Result<(), EngineError> {
    pipeline.converter.convert(&frame.texture, &pipeline.nv12)?;
    let mut hw = HwFrame::from_pool(&pipeline.pool)?;
    hw.copy_from(pipeline.gpu.context(), &pipeline.nv12)?;
    hw.set_pts(pts);
    pipeline.encoder.send(hw)?;
    let mut mux_err: Option<MediaError> = None;
    let Pipeline { encoder, muxer, stream, .. } = &mut *pipeline;
    encoder.receive(&mut |packet| write_packet(&packet, muxer, *stream, stats, &mut mux_err))?;
    if let Some(err) = mux_err {
        return Err(err.into());
    }
    Ok(())
}

/// Writes one packet into the muxer and adds its bytes to the stats.
fn write_packet(packet: &EncodedPacket, muxer: &mut Muxer, stream: usize, stats: &Stats, mux_err: &mut Option<MediaError>) {
    stats.bytes_written.fetch_add(packet.size() as u64, Ordering::Relaxed);
    if mux_err.is_some() {
        return;
    }
    if let Err(err) = muxer.write(stream, packet) {
        *mux_err = Some(err);
    }
}

/// Drains the encoder into the muxer and writes the MKV trailer.
fn finish_file(mut encoder: AmfEncoder, mut muxer: Muxer, stream: usize, stats: &Stats) -> Result<(), EngineError> {
    let mut mux_err: Option<MediaError> = None;
    encoder.finish(&mut |packet| write_packet(&packet, &mut muxer, stream, stats, &mut mux_err))?;
    if let Some(err) = mux_err {
        return Err(err.into());
    }
    muxer.finish()?;
    Ok(())
}

/// Builds the end-of-recording summary from the pacer and the stats.
fn build_summary(file: &Path, pacer: &Pacer, stats: &Stats, fps: u32) -> Result<Summary, EngineError> {
    let frames = pacer.frames();
    let duration_ms = if fps == 0 { 0 } else { frames * 1000 / u64::from(fps) };
    let bytes_written = stats.bytes_written.load(Ordering::Relaxed);
    let seconds = duration_ms as f64 / 1000.0;
    let average_mbps = if seconds > 0.0 { bytes_written as f64 * 8.0 / seconds / 1_000_000.0 } else { 0.0 };
    Ok(Summary { file: file.to_path_buf(), frames, duplicated: pacer.duplicated(), dropped: pacer.dropped(), bytes_written, duration_ms, average_mbps })
}

/// Copies the pacer's counters and the capture's received count into the stats the UI thread reads.
fn store_pacer_counters(stats: &Stats, pacer: &Pacer, received: u64) {
    stats.frames.store(pacer.frames(), Ordering::Relaxed);
    stats.duplicated.store(pacer.duplicated(), Ordering::Relaxed);
    stats.dropped.store(pacer.dropped(), Ordering::Relaxed);
    stats.frames_received.store(received, Ordering::Relaxed);
}

/// Waits up to two seconds for the first captured frame.
fn wait_for_first_frame(capture: &MonitorCapture) -> Option<LatestFrame> {
    for _ in 0..200 {
        if let Some(frame) = capture.latest() {
            return Some(frame);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    None
}

/// Sleeps until about 1 ms before the next tick, then yields until it is due.
fn wait_for_next_tick(qpc_freq: i64, fps: u32, clock_start: i64, next_index: u64, paused: bool) {
    if paused {
        std::thread::sleep(Duration::from_millis(1));
        return;
    }
    let tick = tick_time(clock_start, next_index, qpc_freq, fps);
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

/// The QPC value at which frame index `n` becomes due, from the shifted start.
fn tick_time(clock_start: i64, n: u64, qpc_freq: i64, fps: u32) -> i64 {
    let offset = i128::from(n) * i128::from(qpc_freq) / i128::from(fps);
    (i128::from(clock_start) + offset) as i64
}
