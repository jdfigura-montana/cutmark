//! The live numbers of a running take: bytes, bitrate, free disk space and the time left, sampled twice a second.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::session::{StatsSnapshot, TakeCounters};

/// How often a [`TakeStats`] is built while a take runs.
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);

/// How far back the current bitrate looks.
const RATE_WINDOW: Duration = Duration::from_secs(2);

/// One gibibyte, the unit of the disk thresholds.
pub const GIB: u64 = 1024 * 1024 * 1024;

/// The environment variable that fakes the free space, in GB, for testing the disk warnings.
pub const FAKE_FREE_ENV: &str = "CUTMARK_FAKE_FREE_GB";

/// A data rate below this many bytes a second counts as no data, so no time remaining is worked out.
const MIN_RATE: f64 = 1.0;

/// The live numbers of one take at one instant.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TakeStats {
    /// How much of the take was recorded, in milliseconds; a pause does not count.
    pub elapsed_ms: u64,
    /// Frames written to the encoder.
    pub frames: u64,
    /// Bytes of all the take's files so far: the video file and the WAV sidecars.
    pub bytes: u64,
    /// The current data rate of all the take's files in bytes a second, over the last 2 seconds.
    pub bytes_per_sec: f64,
    /// Free space on the take's volume in bytes; none when it could not be read.
    pub disk_free_bytes: Option<u64>,
    /// Seconds until the disk is full at the current data rate; none when free space or the rate is unknown.
    pub remaining_secs: Option<u64>,
    /// Frames repeated because no new one arrived in time.
    pub duplicated: u64,
    /// Frames that never reached the encoder.
    pub dropped: u64,
    /// Packets waiting for the mux thread.
    pub queue_depth: usize,
}

impl TakeStats {
    /// The current data rate in kilobits a second.
    pub fn bitrate_kbps(&self) -> f64 {
        self.bytes_per_sec * 8.0 / 1000.0
    }
}

/// The data rate over a sliding window of byte counts.
#[derive(Debug, Clone)]
pub struct RollingRate {
    window: Duration,
    /// Time since the take started and the byte count then, oldest first.
    samples: VecDeque<(Duration, u64)>,
}

impl RollingRate {
    /// A rate that looks back `window`.
    pub fn new(window: Duration) -> Self {
        Self { window, samples: VecDeque::new() }
    }

    /// Adds the byte count `bytes` seen `at` after the start, and forgets samples that fell out of the window.
    pub fn push(&mut self, at: Duration, bytes: u64) {
        self.samples.push_back((at, bytes));
        while self.samples.len() > 1 && self.samples.front().is_some_and(|(then, _)| *then + self.window < at) {
            self.samples.pop_front();
        }
    }

    /// Bytes a second between the oldest and the newest sample; 0 until there are two samples at different times.
    pub fn bytes_per_sec(&self) -> f64 {
        let (Some((first_at, first_bytes)), Some((last_at, last_bytes))) = (self.samples.front(), self.samples.back()) else {
            return 0.0;
        };
        let seconds = last_at.saturating_sub(*first_at).as_secs_f64();
        if seconds <= 0.0 { 0.0 } else { last_bytes.saturating_sub(*first_bytes) as f64 / seconds }
    }
}

/// Seconds until `free_bytes` is used up at `bytes_per_sec`; none when no data is being written.
pub fn time_remaining_secs(free_bytes: u64, bytes_per_sec: f64) -> Option<u64> {
    if bytes_per_sec < MIN_RATE {
        return None;
    }
    Some((free_bytes as f64 / bytes_per_sec) as u64)
}

/// The free space the `CUTMARK_FAKE_FREE_GB` value `raw` asks for, minus the bytes written this take; none when unset or not a number.
pub fn fake_free_bytes(raw: Option<&str>, written: u64) -> Option<u64> {
    let gb: f64 = raw?.trim().parse().ok()?;
    if !gb.is_finite() || gb < 0.0 {
        return None;
    }
    Some(((gb * GIB as f64) as u64).saturating_sub(written))
}

/// Free space for the volume of `folder`, or the fake value when `CUTMARK_FAKE_FREE_GB` is set.
///
/// `written` is what this take has written so far, which the fake value counts down by.
pub fn free_space(folder: &Path, written: u64) -> Option<u64> {
    let fake = std::env::var(FAKE_FREE_ENV).ok();
    fake_free_bytes(fake.as_deref(), written).or_else(|| folder.ancestors().find_map(disk_free_bytes))
}

/// Free bytes available to the user on the volume that holds `path`; none when the path is not there or the call fails.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn disk_free_bytes(path: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::HSTRING;
    let wide = HSTRING::from(path.as_os_str());
    let mut free = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the call, `free` is a valid u64 the call writes
    // to, and the other two outputs are optional and passed as None.
    unsafe { GetDiskFreeSpaceExW(&wide, Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// Free bytes available to the user on the volume that holds `path`; always none off Windows.
#[cfg(not(windows))]
pub fn disk_free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Plays the Windows exclamation sound without waiting for it to finish.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn play_warning_sound() {
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC};
    use windows::core::w;
    // SAFETY: `w!` gives a static NUL-terminated wide string that SND_ALIAS reads as a system sound alias, and
    // SND_ASYNC returns at once, so nothing the call uses is freed early; the module handle is None.
    let played = unsafe { PlaySoundW(w!("SystemExclamation"), None, SND_ALIAS | SND_ASYNC) };
    if !played.as_bool() {
        log::debug!("the warning sound could not be played");
    }
}

/// Plays the Windows exclamation sound; does nothing off Windows.
#[cfg(not(windows))]
pub fn play_warning_sound() {}

/// Builds a [`TakeStats`] from the session's counters, keeping the rolling rate between samples.
pub struct StatsTracker {
    /// The take folder, whose volume the free space is read from.
    folder: PathBuf,
    /// When the take started.
    started: Instant,
    /// When the last sample was built.
    last: Option<Instant>,
    /// The data rate of all the take's files.
    rate: RollingRate,
}

impl StatsTracker {
    /// A tracker for a take that started at `now` and writes into `folder`.
    pub fn new(folder: PathBuf, now: Instant) -> Self {
        Self { folder, started: now, last: None, rate: RollingRate::new(RATE_WINDOW) }
    }

    /// Whether a sample is due at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.last.is_none_or(|last| now.saturating_duration_since(last) >= SAMPLE_INTERVAL)
    }

    /// Builds the numbers at `now` from the session's counters; `fps` turns frames into the elapsed time.
    pub fn sample(&mut self, now: Instant, snapshot: &StatsSnapshot, counters: TakeCounters, fps: u32) -> TakeStats {
        self.last = Some(now);
        let bytes = counters.mux_bytes.saturating_add(counters.wav_bytes);
        self.rate.push(now.saturating_duration_since(self.started), bytes);
        let bytes_per_sec = self.rate.bytes_per_sec();
        let disk_free_bytes = free_space(&self.folder, bytes);
        TakeStats {
            elapsed_ms: snapshot.frames.saturating_mul(1000) / u64::from(fps.max(1)),
            frames: snapshot.frames,
            bytes,
            bytes_per_sec,
            disk_free_bytes,
            remaining_secs: disk_free_bytes.and_then(|free| time_remaining_secs(free, bytes_per_sec)),
            duplicated: snapshot.duplicated,
            dropped: snapshot.dropped,
            queue_depth: counters.queue_depth,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    #[test]
    fn the_rate_is_the_byte_growth_over_the_window() {
        let mut rate = RollingRate::new(secs(2.0));
        assert_eq!(rate.bytes_per_sec(), 0.0);
        rate.push(secs(0.0), 0);
        assert_eq!(rate.bytes_per_sec(), 0.0);
        rate.push(secs(0.5), 500);
        rate.push(secs(1.0), 1000);
        assert!((rate.bytes_per_sec() - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn the_rate_forgets_what_fell_out_of_the_window() {
        let mut rate = RollingRate::new(secs(2.0));
        for step in 0u32..=4 {
            rate.push(secs(f64::from(step) * 0.5), u64::from(step) * 1000);
        }
        assert!((rate.bytes_per_sec() - 2000.0).abs() < 1e-9);
        // The writing stops; after more than 2 seconds the old growth is gone.
        for step in 5u32..=10 {
            rate.push(secs(f64::from(step) * 0.5), 4000);
        }
        assert_eq!(rate.bytes_per_sec(), 0.0);
    }

    #[test]
    fn the_time_remaining_follows_the_rate() {
        assert_eq!(time_remaining_secs(10_000, 100.0), Some(100));
        assert_eq!(time_remaining_secs(10_000, 0.0), None);
        assert_eq!(time_remaining_secs(10_000, 0.5), None);
        assert_eq!(time_remaining_secs(0, 100.0), Some(0));
    }

    #[test]
    fn the_fake_free_space_counts_down_with_the_bytes_written() {
        assert_eq!(fake_free_bytes(Some("10"), 0), Some(10 * GIB));
        assert_eq!(fake_free_bytes(Some(" 1.5 "), GIB), Some(GIB / 2));
        assert_eq!(fake_free_bytes(Some("1"), 5 * GIB), Some(0));
        assert_eq!(fake_free_bytes(None, 0), None);
        assert_eq!(fake_free_bytes(Some("lots"), 0), None);
        assert_eq!(fake_free_bytes(Some("-3"), 0), None);
    }
}
