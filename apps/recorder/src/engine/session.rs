//! The recording session: session config, live stats, and the start, pause,
//! resume and stop calls that drive the video thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use capture::{CaptureError, qpc_now};
use media_io::MediaError;
use media_io::encode::RateControl as EncoderRateControl;
use media_io::encode::{Codec as EncoderCodec, EncoderConfig, Preset};
use media_io::mux::{Container as MuxContainer, MuxSender, MuxThread, Muxer, SendParameters};

use super::video;
use crate::config::{Codec, Container, Profile, RateControl};

/// Why starting, pausing or stopping a recording failed.
#[derive(Debug, Error)]
pub enum EngineError {
    /// A capture call failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A media IO call failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The requested monitor index does not exist.
    #[error("no monitor with index {0}")]
    MonitorIndex(usize),
    /// A container or codec that arrives in a later phase.
    #[error("{0}")]
    NotYet(&'static str),
    /// Something failed inside the engine that has no tighter category.
    #[error("{0}")]
    Failed(String),
}

/// Everything the video thread needs to record one take.
pub struct SessionConfig {
    /// DXGI adapter index for the shared D3D11 device.
    pub adapter: usize,
    /// Index into the list returned by `capture::list_monitors()`.
    pub monitor: usize,
    /// Draw the mouse cursor into the captured frames.
    pub cursor: bool,
    /// The capture and encode profile in use.
    pub profile: Profile,
    /// Where the recording is written.
    pub output_file: PathBuf,
}

/// Live counters of a running session, shared with the UI thread through an Arc.
pub struct Stats {
    /// Frames written to the encoder.
    pub(super) frames: AtomicU64,
    /// Recorded frames that repeated the previous source frame.
    pub(super) duplicated: AtomicU64,
    /// Frames that never reached the encoder.
    pub(super) dropped: AtomicU64,
    /// Frames the capture received, whether the pacer used them or not.
    pub(super) frames_received: AtomicU64,
    /// Encoded bytes handed to the muxer.
    pub(super) bytes_written: AtomicU64,
    /// The QPC value at recording time 0.
    pub(super) started_qpc: AtomicU64,
    /// Whether the session is paused; the video thread watches it every tick.
    pub(super) paused: AtomicBool,
    /// The text of the last frame error, if any.
    pub(super) last_error: Mutex<Option<String>>,
}

impl Stats {
    fn new() -> Self {
        Self {
            frames: AtomicU64::new(0),
            duplicated: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            frames_received: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            started_qpc: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            last_error: Mutex::new(None),
        }
    }

    /// Reads every counter into a plain snapshot.
    pub(super) fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            duplicated: self.duplicated.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            frames_received: self.frames_received.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            started_qpc: self.started_qpc.load(Ordering::Relaxed),
            paused: self.paused.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|slot| slot.clone()),
        }
    }

    /// Records the text of the latest frame error.
    pub(super) fn set_last_error(&self, text: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(text);
        }
    }
}

/// A plain copy of the live counters at one instant.
pub struct StatsSnapshot {
    /// Frames written to the encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous source frame.
    pub duplicated: u64,
    /// Frames that never reached the encoder.
    pub dropped: u64,
    /// Frames the capture received, whether the pacer used them or not.
    pub frames_received: u64,
    /// Encoded bytes handed to the muxer.
    pub bytes_written: u64,
    /// The QPC value at recording time 0.
    pub started_qpc: u64,
    /// Whether the session is paused right now.
    pub paused: bool,
    /// The text of the last frame error, if any.
    pub last_error: Option<String>,
}

/// The totals of one finished take.
pub struct Summary {
    /// The file the recording was written to.
    pub file: PathBuf,
    /// Frames written to the encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous source frame.
    pub duplicated: u64,
    /// Frames that never reached the encoder.
    pub dropped: u64,
    /// The size of the finished file, read back by stop().
    pub bytes_written: u64,
    /// Packets written, one entry per stream index.
    pub packets: Vec<u64>,
    /// Recording time in milliseconds.
    pub duration_ms: u64,
    /// Average bitrate of the finished file, in megabits per second.
    pub average_mbps: f64,
}

/// Messages from the video thread back to the session.
pub(super) enum ThreadMessage {
    /// The video thread's setup ended: the encoder facts on success, or the
    /// error that ended the setup.
    Setup(Result<VideoSetup, EngineError>),
    /// The take ended, either with a summary or with the error that ended it.
    Finished(Result<Summary, EngineError>),
}

/// The encoder facts the session needs to add the video stream to the muxer.
pub(super) struct VideoSetup {
    /// The encoder's codec parameters, including extradata.
    pub(super) params: SendParameters,
    /// The time base the encoder stamps its packets in.
    pub(super) time_base: (i32, i32),
}

/// The command that releases a stream thread to start pacing.
pub(super) struct StartCommand {
    /// The QPC value every stream counts its recording time 0 from.
    pub(super) start_qpc: i64,
    /// The queue the thread writes its packets into.
    pub(super) mux: MuxSender,
}

/// Placeholder for the extra stream specs that phase 3d2 adds between the
/// video stream and muxer start: the mixdown and the per-track streams.
type StreamSpec = ();

/// One running recording: talks to the video thread through atomics and a channel.
pub struct RecordingSession {
    stats: Arc<Stats>,
    stop_flag: Arc<AtomicBool>,
    rx: Receiver<ThreadMessage>,
    handle: Option<std::thread::JoinHandle<()>>,
    mux: Option<MuxThread>,
}

impl RecordingSession {
    /// Starts a recording: the video thread sets up, the session builds the
    /// muxer around its streams, then every stream starts from one instant.
    pub fn start(cfg: SessionConfig) -> Result<Self, EngineError> {
        if cfg.profile.container != Container::Mkv {
            return Err(EngineError::NotYet("MP4 and MOV arrive in phase 4"));
        }
        media_io::init()?;
        let output_file = cfg.output_file.clone();
        let fps = cfg.profile.fps;
        let stats = Arc::new(Stats::new());
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel::<StartCommand>();
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stop_flag);
        let handle = std::thread::Builder::new()
            .name("video".to_owned())
            .spawn(move || video::run(cfg, thread_stats, thread_stop, tx, start_rx))
            .map_err(|err| EngineError::Failed(format!("the video thread could not be started: {err}")))?;
        // The video thread reports the encoder facts, or its setup error.
        let setup = match rx.recv() {
            Ok(ThreadMessage::Setup(Ok(setup))) => setup,
            Ok(ThreadMessage::Setup(Err(err))) => {
                let _ = handle.join();
                return Err(err);
            }
            Ok(ThreadMessage::Finished(..)) | Err(_) => {
                let _ = handle.join();
                return Err(EngineError::Failed("the video thread ended before reporting setup".into()));
            }
        };
        let mux = match build_muxer(&output_file, fps, setup) {
            Ok(mux) => mux,
            Err(err) => {
                drop(start_tx);
                wind_up(&stop_flag, &rx, handle);
                return Err(err);
            }
        };
        // One start instant for every stream: the video thread only begins
        // pacing once it receives the command.
        let start_qpc = qpc_now();
        stats.started_qpc.store(start_qpc as u64, Ordering::Relaxed);
        let sent = start_tx.send(StartCommand { start_qpc, mux: mux.sender() });
        drop(start_tx);
        if sent.is_err() {
            wind_up(&stop_flag, &rx, handle);
            let _ = mux.finish();
            return Err(EngineError::Failed("the video thread ended before recording started".into()));
        }
        Ok(Self { stats, stop_flag, rx, handle: Some(handle), mux: Some(mux) })
    }

    /// Stops feeding the pacer; recording continues from the same frame on resume.
    pub fn pause(&self) {
        self.stats.paused.store(true, Ordering::Relaxed);
    }

    /// Continues recording with no gap in the frame indexes.
    pub fn resume(&self) {
        self.stats.paused.store(false, Ordering::Relaxed);
    }

    /// A snapshot of the live counters, safe to call from the UI thread at any time.
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Finishes the file and returns the summary of the take.
    pub fn stop(mut self) -> Result<Summary, EngineError> {
        self.stop_flag.store(true, Ordering::Relaxed);
        let mut outcome = Err(EngineError::Failed("the video thread ended without reporting a summary".into()));
        while let Ok(message) = self.rx.recv() {
            if let ThreadMessage::Finished(result) = message {
                outcome = result;
                break;
            }
        }
        // The video thread is done, so the queue holds its last packets.
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Finish the mux thread and fold its totals into the summary.
        if outcome.is_ok()
            && let Some(mux) = self.mux.take()
        {
            outcome = match mux.finish() {
                Ok(mux_summary) => outcome.map(|mut summary| {
                    let seconds = summary.duration_ms as f64 / 1000.0;
                    summary.bytes_written = mux_summary.bytes_written;
                    summary.packets = mux_summary.packets;
                    summary.average_mbps = if seconds > 0.0 { summary.bytes_written as f64 * 8.0 / seconds / 1_000_000.0 } else { 0.0 };
                    summary
                }),
                Err(err) => Err(err.into()),
            };
        }
        outcome
    }
}

/// Creates the muxer, adds the video stream as stream 0 and starts it.
fn build_muxer(output_file: &Path, fps: u32, setup: VideoSetup) -> Result<MuxThread, EngineError> {
    let mut muxer = Muxer::create(output_file, MuxContainer::Mkv)?;
    // The video stream is stream 0; phase 3d2 adds the audio streams after it.
    muxer.add_video(setup.params.0, setup.time_base, (fps as i32, 1))?;
    // Hook for phase 3d2: build the mixdown and track stream specs into
    // `extra_streams` and add them here, before the muxer starts.
    let extra_streams: Vec<StreamSpec> = Vec::new();
    let _ = extra_streams;
    muxer.start()?;
    Ok(MuxThread::start(muxer)?)
}

/// Shuts the video thread down after a failed startup and waits for it.
fn wind_up(stop_flag: &Arc<AtomicBool>, rx: &Receiver<ThreadMessage>, handle: std::thread::JoinHandle<()>) {
    stop_flag.store(true, Ordering::Relaxed);
    while let Ok(message) = rx.recv() {
        if matches!(message, ThreadMessage::Finished(_)) {
            break;
        }
    }
    let _ = handle.join();
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        // A dropped session asks the video thread to wind down; the file is
        // only finished properly through stop().
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// Maps a profile onto the media-io encoder configuration (phase 2: AMF only).
pub(super) fn encoder_config(profile: &Profile) -> EncoderConfig {
    EncoderConfig {
        codec: match profile.codec {
            Codec::Av1 => EncoderCodec::Av1,
            Codec::Hevc => EncoderCodec::Hevc,
            Codec::H264 => EncoderCodec::H264,
        },
        width: profile.width,
        height: profile.height,
        fps_num: profile.fps,
        fps_den: 1,
        rate_control: match profile.rate_control {
            RateControl::Qvbr { quality, max_mbps } => EncoderRateControl::Qvbr { quality, max_mbps: max_mbps as u32 },
            RateControl::Vbr { target_mbps, max_mbps } => EncoderRateControl::VbrPeak { target_mbps: target_mbps as u32, max_mbps: max_mbps as u32 },
            RateControl::Cbr { mbps } => EncoderRateControl::Cbr { mbps: mbps as u32 },
            RateControl::Cqp { qp_i, qp_p } => EncoderRateControl::Cqp { qp_i, qp_p },
        },
        preset: Preset::Balanced,
        keyframe_seconds: 2,
    }
}
