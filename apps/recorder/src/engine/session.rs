//! The recording session: session config, live stats, and the start, pause,
//! resume and stop calls that drive the video thread.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use capture::CaptureError;
use media_io::MediaError;
use media_io::encode::RateControl as EncoderRateControl;
use media_io::encode::{Codec as EncoderCodec, EncoderConfig, Preset};

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
    /// Encoded bytes handed to the muxer.
    pub bytes_written: u64,
    /// Recording time in milliseconds.
    pub duration_ms: u64,
    /// Average encoded bitrate in megabits per second.
    pub average_mbps: f64,
}

/// Messages from the video thread back to the session.
pub(super) enum ThreadMessage {
    /// Setup succeeded; the take is running.
    Started,
    /// The take ended, either with a summary or with the error that ended it.
    Finished(Result<Summary, EngineError>),
}

/// One running recording: talks to the video thread through atomics and a channel.
pub struct RecordingSession {
    stats: Arc<Stats>,
    stop_flag: Arc<AtomicBool>,
    rx: Receiver<ThreadMessage>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl RecordingSession {
    /// Starts a recording on a new video thread and waits for its setup.
    pub fn start(cfg: SessionConfig) -> Result<Self, EngineError> {
        if cfg.profile.container != Container::Mkv {
            return Err(EngineError::NotYet("MP4 and MOV arrive in phase 4"));
        }
        media_io::init()?;
        let stats = Arc::new(Stats::new());
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stop_flag);
        let handle = std::thread::Builder::new()
            .name("video".to_owned())
            .spawn(move || video::run(cfg, thread_stats, thread_stop, tx))
            .map_err(|err| EngineError::Failed(format!("the video thread could not be started: {err}")))?;
        match rx.recv() {
            Ok(ThreadMessage::Started) => Ok(Self { stats, stop_flag, rx, handle: Some(handle) }),
            Ok(ThreadMessage::Finished(Err(err))) => Err(err),
            Ok(ThreadMessage::Finished(Ok(..))) => Err(EngineError::Failed("the video thread finished before reporting startup".into())),
            Err(_) => Err(EngineError::Failed("the video thread ended before reporting startup".into())),
        }
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
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        outcome
    }
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
