//! Saving the replay buffer: the writer thread and its queue, the rule that merges two quick saves, and the memory the
//! buffer is expected to take.
//!
//! See the replay buffer spec: How it behaves, and Under the hood.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Local};
use media_io::AudioCodec;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

use super::audio::{OUT_CHANNELS, OUT_RATE};
use super::replay::{self, ReplaySnapshot, ReplayTake};
use super::session::{EngineError, SessionConfig};
use crate::config::{RateControl, Settings};
use crate::takes;

/// Saves pressed closer together than this count as one.
const SAVE_MERGE: Duration = Duration::from_secs(2);

/// What a video stream with no bitrate ceiling, which is constant QP, is assumed to take, in megabits per second.
const UNCAPPED_VIDEO_MBPS: f32 = 40.0;

/// Remembers the last save that was queued, so a second one right after it is left out.
#[derive(Debug, Default)]
pub(super) struct SaveMerge {
    saved_qpc: Option<i64>,
}

impl SaveMerge {
    /// Whether a save pressed at `press_qpc` is too close to the last queued one to count; `frequency` is in ticks per
    /// second.
    pub(super) fn merges(&self, press_qpc: i64, frequency: i64) -> bool {
        let window = i128::from(frequency) * i128::from(SAVE_MERGE.as_secs());
        self.saved_qpc.is_some_and(|last| (i128::from(press_qpc) - i128::from(last)).abs() < window)
    }

    /// Notes that a save pressed at `press_qpc` was queued.
    pub(super) fn saved(&mut self, press_qpc: i64) {
        self.saved_qpc = Some(press_qpc);
    }
}

/// A writer thread that works through its jobs one at a time, in the order they came.
pub(super) struct SaveQueue<J, R> {
    jobs: Sender<J>,
    results: Receiver<R>,
    thread: JoinHandle<()>,
}

impl<J: Send + 'static, R: Send + 'static> SaveQueue<J, R> {
    /// Starts the "replay-writer" thread, which turns every job into a result with `write`.
    pub(super) fn start(write: impl Fn(J) -> R + Send + 'static) -> std::io::Result<Self> {
        let (jobs, queued) = mpsc::channel();
        let (done, results) = mpsc::channel();
        let thread = std::thread::Builder::new().name("replay-writer".to_owned()).spawn(move || {
            for job in queued {
                if done.send(write(job)).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self { jobs, results, thread })
    }

    /// Queues a job without waiting; false when the thread has ended.
    pub(super) fn push(&self, job: J) -> bool {
        self.jobs.send(job).is_ok()
    }

    /// The oldest result that has not been taken yet, if a job has finished.
    pub(super) fn try_result(&self) -> Option<R> {
        self.results.try_recv().ok()
    }

    /// Waits for every queued job and returns the results that have not been taken yet.
    pub(super) fn finish(self) -> Vec<R> {
        let Self { jobs, results, thread } = self;
        drop(jobs);
        if thread.join().is_err() {
            log::error!("the replay writer thread panicked");
        }
        results.try_iter().collect()
    }
}

/// One save on its way to disk.
pub(super) struct SaveJob {
    /// The buffer as it was at the press.
    pub(super) snapshot: ReplaySnapshot,
    /// What the capture was opened with, with the gains and mutes of the press.
    pub(super) cfg: SessionConfig,
    /// The settings that name the take.
    pub(super) settings: Settings,
    /// When the save was pressed, for the take's name.
    pub(super) pressed_at: DateTime<Local>,
}

/// The writer thread of the replay buffer.
pub(super) type ReplayWriter = SaveQueue<SaveJob, Result<ReplayTake, EngineError>>;

/// Picks the next take by the naming template and writes the saved buffer into it. The number is picked here, when the
/// save's turn comes, so every earlier save already has its folder.
pub(super) fn write_job(job: SaveJob) -> Result<ReplayTake, EngineError> {
    let take = takes::next_take_for(&job.settings, job.pressed_at, job.cfg.profile.container)
        .map_err(|err| EngineError::Failed(format!("could not make the take folder: {err}")))?;
    let paths = replay::TakePaths { folder: take.folder, base_name: take.base_name, output_file: take.video_file };
    replay::write_replay(job.snapshot, &job.cfg, &paths)
}

/// The bytes a video stream takes per second. A rate control with a ceiling and no target counts at its ceiling.
fn video_bytes_per_s(rate_control: RateControl) -> f64 {
    let mbps = match rate_control {
        RateControl::ConstantQuality { max_mbps, .. } => max_mbps,
        RateControl::ConstantQp { .. } => UNCAPPED_VIDEO_MBPS,
        RateControl::Cbr { mbps } => mbps,
        RateControl::VbrPeak { target_mbps, .. } => target_mbps,
    };
    f64::from(mbps.max(0.0)) * 1_000_000.0 / 8.0
}

/// The memory a full replay buffer of `length` is expected to take for a capture of `cfg`, in bytes: every stream a
/// take writes, for the length plus the one keyframe interval a ring can run over. The PCM kept for WAV sidecars is
/// 32-bit float.
pub(super) fn ram_estimate(cfg: &SessionConfig, length: Duration) -> u64 {
    let pcm_bytes_per_s = |bytes_per_sample: u32| f64::from(OUT_RATE) * f64::from(OUT_CHANNELS) * f64::from(bytes_per_sample);
    let video = video_bytes_per_s(cfg.profile.rate_control);
    let camera = cfg.profile.webcam.as_ref().filter(|webcam| webcam.iso).map_or(0.0, |webcam| video_bytes_per_s(webcam.iso_rc));
    let streams = usize::from(cfg.mixdown && !cfg.tracks.is_empty()) + if cfg.track_streams { cfg.tracks.len() } else { 0 };
    let stream = match cfg.audio_codec {
        AudioCodec::Aac { kbps } => f64::from(kbps) * 1000.0 / 8.0,
        AudioCodec::Pcm24 => pcm_bytes_per_s(3),
    };
    let wav_tracks = if cfg.wav_sidecars { cfg.tracks.len() } else { 0 };
    let per_second = video + camera + streams as f64 * stream + wav_tracks as f64 * pcm_bytes_per_s(4);
    let seconds = length.as_secs_f64() + f64::from(cfg.profile.keyframe_secs.max(0.0));
    (per_second * seconds) as u64
}

/// Whether a buffer of `need` bytes is more than 25% of the `free` bytes of physical memory.
pub(super) fn over_ram_limit(need: u64, free: u64) -> bool {
    u128::from(need) * 4 > u128::from(free)
}

/// The physical memory that is free right now, in bytes; None when Windows does not say.
#[allow(unsafe_code)]
pub(super) fn free_physical_memory() -> Option<u64> {
    let mut status = MEMORYSTATUSEX { dwLength: u32::try_from(size_of::<MEMORYSTATUSEX>()).ok()?, ..MEMORYSTATUSEX::default() };
    // SAFETY: `status` is a local MEMORYSTATUSEX with its length set, which is all the call asks for.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(status.ullAvailPhys)
}

#[cfg(test)]
#[path = "replay_save_tests.rs"]
mod tests;
