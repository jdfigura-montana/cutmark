//! The pre-flight verdict: a finished 5 second test take goes in, a pass, warn or fail with one line per check comes out.
//!
//! Pure logic with no capture, files or UI, so every rule and boundary is unit tested.

use super::session::{Summary, TrackPeak};

/// The text the controller state shows while a pre-flight runs.
pub const RUNNING: &str = "Pre-flight running";

/// A track whose peak stays below this many dBFS counts as silent.
pub const SILENT_BELOW_DBFS: f32 = -60.0;

/// A track whose peak reaches this many dBFS or more counts as clipping.
pub const CLIPPING_FROM_DBFS: f32 = -0.1;

/// The result of one check, or of the whole pre-flight; later variants are worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Nothing to worry about.
    Pass,
    /// Worth a look, but the recording would work.
    Warn,
    /// The recording would lose frames or fall behind.
    Fail,
}

impl Status {
    /// The word the window shows for this result.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "Pass",
            Self::Warn => "Warn",
            Self::Fail => "Fail",
        }
    }
}

/// One line of the report: what was checked, how it went and the numbers behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckLine {
    /// What was checked, such as "Dropped frames" or "Track Mic".
    pub label: String,
    /// How the check went.
    pub status: Status,
    /// The measured numbers in words.
    pub detail: String,
}

/// The verdict on a pre-flight take: an overall result and one line per check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    /// The worst status of any line.
    pub overall: Status,
    /// The checks in display order: dropped, duplicated, encode time, then one per audio track.
    pub lines: Vec<CheckLine>,
}

/// Judges a finished pre-flight take recorded at `fps` frames per second.
///
/// Fail when any frame was dropped or the 95th percentile encode time is over the frame budget (1000 / fps ms).
/// Warn when that percentile is over 80% of the budget, or a track's peak is under -60 dBFS (silent) or at least -0.1 dBFS (clipping).
pub fn judge(summary: &Summary, fps: u32) -> PreflightReport {
    let budget_ms = 1000.0 / f64::from(fps.max(1));
    let mut lines = vec![
        dropped_line(summary.dropped),
        CheckLine { label: "Duplicated frames".to_owned(), status: Status::Pass, detail: summary.duplicated.to_string() },
        encode_line(summary.encode_avg_ms, summary.encode_p95_ms, budget_ms),
    ];
    lines.extend(summary.track_peaks.iter().map(track_line));
    let overall = lines.iter().map(|line| line.status).max().unwrap_or(Status::Pass);
    PreflightReport { overall, lines }
}

/// The status of the 95th percentile encode time against the frame budget, both in milliseconds.
fn encode_status(p95_ms: f64, budget_ms: f64) -> Status {
    // The warn edge is multiplied out so it is exact: p95 > 0.8 * budget.
    if p95_ms > budget_ms {
        Status::Fail
    } else if p95_ms * 5.0 > budget_ms * 4.0 {
        Status::Warn
    } else {
        Status::Pass
    }
}

/// The status of one track's peak level in dBFS.
fn track_status(peak_dbfs: f32) -> Status {
    if !(SILENT_BELOW_DBFS..CLIPPING_FROM_DBFS).contains(&peak_dbfs) { Status::Warn } else { Status::Pass }
}

/// The line for dropped frames: any drop fails.
fn dropped_line(dropped: u64) -> CheckLine {
    let status = if dropped > 0 { Status::Fail } else { Status::Pass };
    CheckLine { label: "Dropped frames".to_owned(), status, detail: dropped.to_string() }
}

/// The line for encode time: the average and the 95th percentile against the budget.
fn encode_line(avg_ms: f64, p95_ms: f64, budget_ms: f64) -> CheckLine {
    CheckLine {
        label: "Encode time".to_owned(),
        status: encode_status(p95_ms, budget_ms),
        detail: format!("average {avg_ms:.1} ms, 95th percentile {p95_ms:.1} ms, budget {budget_ms:.1} ms"),
    }
}

/// The line for one track's peak level.
fn track_line(track: &TrackPeak) -> CheckLine {
    let status = track_status(track.peak_dbfs);
    let note = match status {
        Status::Pass => "",
        _ if track.peak_dbfs < SILENT_BELOW_DBFS => " (silent)",
        _ => " (clipping)",
    };
    CheckLine { label: format!("Track {}", track.name), status, detail: format!("peak {:.1} dBFS{note}", track.peak_dbfs) }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A summary with these numbers; the average is half the percentile.
    fn summary(dropped: u64, p95_ms: f64, peaks: &[f32]) -> Summary {
        Summary {
            file: PathBuf::new(),
            frames: 300,
            duplicated: 2,
            dropped,
            bytes_written: 0,
            packets: Vec::new(),
            duration_ms: 5000,
            average_mbps: 0.0,
            wav_files: Vec::new(),
            encode_avg_ms: p95_ms / 2.0,
            encode_p95_ms: p95_ms,
            track_peaks: peaks.iter().enumerate().map(|(n, peak)| TrackPeak { name: format!("T{n}"), peak_dbfs: *peak }).collect(),
        }
    }

    #[test]
    fn a_clean_take_passes_with_one_line_per_check() {
        let report = judge(&summary(0, 5.0, &[-12.0, -20.0]), 60);
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(report.lines.len(), 5);
        assert!(report.lines.iter().all(|line| line.status == Status::Pass));
        assert_eq!(report.lines[0].label, "Dropped frames");
        assert_eq!(report.lines[3].label, "Track T0");
    }

    #[test]
    fn one_dropped_frame_fails() {
        let report = judge(&summary(1, 5.0, &[-12.0]), 60);
        assert_eq!(report.overall, Status::Fail);
        assert_eq!(report.lines[0].status, Status::Fail);
        assert_eq!(judge(&summary(0, 5.0, &[-12.0]), 60).lines[0].status, Status::Pass);
    }

    #[test]
    fn the_encode_time_edges_are_80_percent_and_100_percent_of_the_budget() {
        // 50 fps gives a budget of exactly 20 ms.
        assert_eq!(encode_status(16.0, 20.0), Status::Pass);
        assert_eq!(encode_status(16.01, 20.0), Status::Warn);
        assert_eq!(encode_status(20.0, 20.0), Status::Warn);
        assert_eq!(encode_status(20.01, 20.0), Status::Fail);
        assert_eq!(judge(&summary(0, 16.0, &[]), 50).overall, Status::Pass);
        assert_eq!(judge(&summary(0, 18.0, &[]), 50).overall, Status::Warn);
        assert_eq!(judge(&summary(0, 21.0, &[]), 50).overall, Status::Fail);
    }

    #[test]
    fn the_budget_follows_the_frame_rate() {
        // 10 ms is fine at 60 fps (16.7 ms budget) and over the budget at 120 fps (8.3 ms).
        assert_eq!(judge(&summary(0, 10.0, &[]), 60).overall, Status::Pass);
        assert_eq!(judge(&summary(0, 10.0, &[]), 120).overall, Status::Fail);
        // A frame rate of zero cannot divide by zero.
        assert_eq!(judge(&summary(0, 10.0, &[]), 0).overall, Status::Pass);
    }

    #[test]
    fn a_silent_track_warns_below_minus_60_dbfs_and_passes_at_it() {
        assert_eq!(track_status(-60.0), Status::Pass);
        assert_eq!(track_status(-60.1), Status::Warn);
        assert_eq!(track_status(-100.0), Status::Warn);
        let report = judge(&summary(0, 5.0, &[-75.0]), 60);
        assert_eq!(report.overall, Status::Warn);
        assert!(report.lines[3].detail.ends_with("(silent)"));
    }

    #[test]
    fn a_clipping_track_warns_from_minus_0_1_dbfs() {
        assert_eq!(track_status(-0.11), Status::Pass);
        assert_eq!(track_status(CLIPPING_FROM_DBFS), Status::Warn);
        assert_eq!(track_status(0.0), Status::Warn);
        let report = judge(&summary(0, 5.0, &[0.0]), 60);
        assert_eq!(report.overall, Status::Warn);
        assert!(report.lines[3].detail.ends_with("(clipping)"));
    }

    #[test]
    fn the_overall_result_is_the_worst_line() {
        // A warning from a track and a failure from a drop: the failure wins.
        assert_eq!(judge(&summary(3, 5.0, &[-80.0]), 60).overall, Status::Fail);
        // A warning from the encode time and one from a track stay a warning.
        assert_eq!(judge(&summary(0, 14.0, &[-80.0]), 60).overall, Status::Warn);
    }

    #[test]
    fn the_encode_line_names_the_numbers_and_the_budget() {
        let report = judge(&summary(0, 8.0, &[]), 50);
        assert_eq!(report.lines[2].detail, "average 4.0 ms, 95th percentile 8.0 ms, budget 20.0 ms");
    }
}
