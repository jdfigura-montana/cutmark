//! The pre-flight verdict: a finished 5 second test take goes in, a pass, warn or fail with one line per check comes out.
//!
//! Pure logic with no capture, files or UI, so every rule and boundary is unit tested.

use super::cam::CamSummary;
use super::session::{Summary, TrackPeak, TrackSnapshot};
use super::stats::GIB;

/// The text the controller state shows while a pre-flight runs.
pub const RUNNING: &str = "Pre-flight running";

/// A track whose peak stays below this many dBFS counts as silent.
pub const SILENT_BELOW_DBFS: f32 = -60.0;

/// A track whose peak reaches this many dBFS or more counts as clipping.
pub const CLIPPING_FROM_DBFS: f32 = -0.1;

/// The label the Do Not Disturb check uses in the report.
pub const DND_LABEL: &str = "Do Not Disturb";

/// The result of one check, or of the whole pre-flight; Info is a neutral note and the later variants are worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// A neutral note that never moves the overall result.
    Info,
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
            Self::Info => "Info",
            Self::Pass => "Pass",
            Self::Warn => "Warn",
            Self::Fail => "Fail",
        }
    }
}

/// One measured number of a check, as the window shows it in a stat block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    /// The number with its unit, such as "8.0 ms".
    pub value: String,
    /// What the number is, such as "Budget".
    pub label: String,
}

impl Stat {
    /// A number and what it is.
    fn new(value: String, label: &str) -> Self {
        Self { value, label: label.to_owned() }
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
    /// The same numbers one by one; empty for a line without numbers.
    pub stats: Vec<Stat>,
}

/// The verdict on a pre-flight take: an overall result and one line per check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    /// The worst status of any line.
    pub overall: Status,
    /// The checks in display order: dropped, duplicated, encode time, the camera encode when a camera file ran, then one per audio track.
    pub lines: Vec<CheckLine>,
}

/// For each session track, the app name when it records or leaves out an app that is not attached.
/// `apps` names each track's app, None for a device track.
pub fn waiting_apps(apps: &[Option<String>], tracks: &[TrackSnapshot]) -> Vec<Option<String>> {
    apps.iter().enumerate().map(|(n, app)| app.clone().filter(|_| !tracks.get(n).is_some_and(|track| track.attached))).collect()
}

/// Judges a finished pre-flight take recorded at `fps` frames per second, with the camera file's totals when one was written.
/// A track named in `waiting` (see [`waiting_apps`]) gets an Info line instead of a judgement of its level.
///
/// Fail when any frame was dropped or the 95th percentile encode time is over the frame budget (1000 / fps ms).
/// Warn when that percentile is over 80% of the budget, or a track's peak is under -60 dBFS (silent) or at least -0.1 dBFS (clipping).
/// The camera file gets one "Camera encode" line with the same rules at the camera's own frame rate.
pub fn judge_with_cam(summary: &Summary, cam: Option<&CamSummary>, waiting: &[Option<String>], fps: u32) -> PreflightReport {
    let budget_ms = 1000.0 / f64::from(fps.max(1));
    let mut lines = vec![
        dropped_line(summary.dropped),
        CheckLine {
            label: "Duplicated frames".to_owned(),
            status: Status::Pass,
            detail: summary.duplicated.to_string(),
            stats: vec![Stat::new(summary.duplicated.to_string(), "Frames")],
        },
        encode_line(summary.encode_avg_ms, summary.encode_p95_ms, budget_ms),
    ];
    lines.extend(cam.map(cam_line));
    lines.extend(summary.track_peaks.iter().enumerate().map(|(n, track)| match waiting.get(n) {
        Some(Some(app_name)) => waiting_line(track, app_name),
        _ => track_line(track),
    }));
    let overall = lines.iter().map(|line| line.status).max().unwrap_or(Status::Pass);
    PreflightReport { overall, lines }
}

/// The Do Not Disturb reminder line: Windows has no API that reports the state, so it stays a neutral note.
pub fn dnd_line() -> CheckLine {
    CheckLine {
        label: DND_LABEL.to_owned(),
        status: Status::Info,
        detail: "Reminder: turn on Do Not Disturb (Win+N) so notifications stay out of your recording".to_owned(),
        stats: Vec::new(),
    }
}

/// One output folder the pre-flight checked: whether it can be written to and how much free space its drive has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderCheck {
    /// The row's label, such as "Recordings folder".
    pub label: String,
    /// The folder that was checked.
    pub path: String,
    /// Why the folder cannot be written to, or none when it can.
    pub not_writable: Option<String>,
    /// Free space on the folder's drive in bytes, or none when it could not be read.
    pub free_bytes: Option<u64>,
}

/// Below this much free space on an output folder's drive the folder check warns.
pub const LOW_FREE_BYTES: u64 = 10 * GIB;

/// The line for one checked output folder: a folder that cannot be written to fails, and low free space warns.
pub fn folder_line(check: &FolderCheck) -> CheckLine {
    let status = match &check.not_writable {
        Some(_) => Status::Fail,
        None if check.free_bytes.is_some_and(|free| free < LOW_FREE_BYTES) => Status::Warn,
        None => Status::Pass,
    };
    let detail = match &check.not_writable {
        Some(reason) => format!("Can't save to {}: {reason}", check.path),
        None => match check.free_bytes {
            Some(free) => format!("{:.1} GB free on {}", free as f64 / GIB as f64, check.path),
            None => format!("{}: the free space could not be read", check.path),
        },
    };
    CheckLine { label: check.label.clone(), status, detail, stats: Vec::new() }
}

/// The lines for the checked output folders, in the order they are given.
pub fn folder_lines(checks: &[FolderCheck]) -> Vec<CheckLine> {
    checks.iter().map(folder_line).collect()
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
    CheckLine { label: "Dropped frames".to_owned(), status, detail: dropped.to_string(), stats: vec![Stat::new(dropped.to_string(), "Frames")] }
}

/// The line for encode time: the average and the 95th percentile against the budget.
fn encode_line(avg_ms: f64, p95_ms: f64, budget_ms: f64) -> CheckLine {
    CheckLine {
        label: "Encode time".to_owned(),
        status: encode_status(p95_ms, budget_ms),
        detail: format!("average {avg_ms:.1} ms, 95th percentile {p95_ms:.1} ms, budget {budget_ms:.1} ms"),
        stats: encode_stats(avg_ms, p95_ms, budget_ms),
    }
}

/// The stats of an encode line: the average, the 95th percentile and the budget.
fn encode_stats(avg_ms: f64, p95_ms: f64, budget_ms: f64) -> Vec<Stat> {
    [(avg_ms, "Average"), (p95_ms, "95th percentile"), (budget_ms, "Budget")].map(|(ms, label)| Stat::new(format!("{ms:.1} ms"), label)).into()
}

/// The line for the camera file: the main video's rules, with any dropped camera frame failing it, at the camera's frame rate.
fn cam_line(cam: &CamSummary) -> CheckLine {
    let budget_ms = 1000.0 / cam.fps.max(1.0);
    let mut status = encode_status(cam.encode_p95_ms, budget_ms);
    if cam.dropped > 0 {
        status = Status::Fail;
    }
    let mut stats = encode_stats(cam.encode_avg_ms, cam.encode_p95_ms, budget_ms);
    stats.push(Stat::new(cam.dropped.to_string(), "Dropped"));
    CheckLine {
        label: "Camera encode".to_owned(),
        status,
        detail: format!(
            "average {:.1} ms, 95th percentile {:.1} ms, budget {:.1} ms, dropped {}",
            cam.encode_avg_ms, cam.encode_p95_ms, budget_ms, cam.dropped
        ),
        stats,
    }
}

/// The line for a track whose app is not running: a neutral note, whatever its level was.
fn waiting_line(track: &TrackPeak, app_name: &str) -> CheckLine {
    let detail = if app_name.is_empty() {
        "No app is selected for this track".to_owned()
    } else {
        format!("{app_name} is not running; the track will attach when it opens")
    };
    CheckLine { label: format!("Track {}", track.name), status: Status::Info, detail, stats: Vec::new() }
}

/// The line for one track's peak level.
fn track_line(track: &TrackPeak) -> CheckLine {
    let status = track_status(track.peak_dbfs);
    let (note, stat_label) = match status {
        Status::Pass => ("", "Peak"),
        _ if track.peak_dbfs < SILENT_BELOW_DBFS => (" (silent)", "Peak, silent"),
        _ => (" (clipping)", "Peak, clipping"),
    };
    CheckLine {
        label: format!("Track {}", track.name),
        status,
        detail: format!("peak {:.1} dBFS{note}", track.peak_dbfs),
        stats: vec![Stat::new(format!("{:.1} dBFS", track.peak_dbfs), stat_label)],
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// Judges a take that has no camera file.
    fn judge(summary: &Summary, fps: u32) -> PreflightReport {
        judge_with_cam(summary, None, &[], fps)
    }

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

    /// A camera file at `fps` with this 95th percentile encode time and dropped count.
    fn cam(fps: f64, p95_ms: f64, dropped: u64) -> CamSummary {
        CamSummary {
            file: PathBuf::new(),
            frames: 150,
            duplicated: 1,
            dropped,
            bytes_written: 0,
            duration_ms: 5000,
            fps,
            encode_avg_ms: p95_ms / 2.0,
            encode_p95_ms: p95_ms,
            lost: false,
        }
    }

    #[test]
    fn a_camera_file_adds_one_camera_encode_line_before_the_tracks() {
        let take = summary(0, 5.0, &[-12.0]);
        assert_eq!(judge(&take, 60).lines.len(), 4);
        let report = judge_with_cam(&take, Some(&cam(30.0, 5.0, 0)), &[], 60);
        assert_eq!(report.lines.len(), 5);
        assert_eq!(report.lines[3].label, "Camera encode");
        assert_eq!(report.lines[3].status, Status::Pass);
        assert_eq!(report.lines[4].label, "Track T0");
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(judge_with_cam(&take, None, &[], 60), judge(&take, 60));
    }

    #[test]
    fn the_camera_encode_follows_the_main_video_rules_at_the_camera_frame_rate() {
        let take = summary(0, 5.0, &[]);
        // 25 fps gives a budget of exactly 40 ms: warn above 32 ms, fail above 40 ms.
        assert_eq!(judge_with_cam(&take, Some(&cam(25.0, 32.0, 0)), &[], 60).overall, Status::Pass);
        assert_eq!(judge_with_cam(&take, Some(&cam(25.0, 33.0, 0)), &[], 60).overall, Status::Warn);
        assert_eq!(judge_with_cam(&take, Some(&cam(25.0, 41.0, 0)), &[], 60).overall, Status::Fail);
        // The same time passes at 30 fps (33.3 ms budget) only below 26.7 ms.
        assert_eq!(judge_with_cam(&take, Some(&cam(30.0, 30.0, 0)), &[], 60).overall, Status::Warn);
        // One dropped camera frame fails, like a dropped frame of the main video.
        assert_eq!(judge_with_cam(&take, Some(&cam(30.0, 5.0, 1)), &[], 60).overall, Status::Fail);
    }

    #[test]
    fn the_camera_encode_line_names_the_numbers_and_the_budget() {
        let report = judge_with_cam(&summary(0, 5.0, &[]), Some(&cam(25.0, 8.0, 2)), &[], 60);
        assert_eq!(report.lines[3].detail, "average 4.0 ms, 95th percentile 8.0 ms, budget 40.0 ms, dropped 2");
    }

    #[test]
    fn every_check_line_carries_its_numbers_as_stats() {
        let report = judge_with_cam(&summary(0, 8.0, &[-75.0]), Some(&cam(25.0, 8.0, 2)), &[], 50);
        let stats: Vec<Vec<(&str, &str)>> =
            report.lines.iter().map(|line| line.stats.iter().map(|stat| (stat.value.as_str(), stat.label.as_str())).collect()).collect();
        let expected: [Vec<(&str, &str)>; 5] = [
            vec![("0", "Frames")],
            vec![("2", "Frames")],
            vec![("4.0 ms", "Average"), ("8.0 ms", "95th percentile"), ("20.0 ms", "Budget")],
            vec![("4.0 ms", "Average"), ("8.0 ms", "95th percentile"), ("40.0 ms", "Budget"), ("2", "Dropped")],
            vec![("-75.0 dBFS", "Peak, silent")],
        ];
        assert_eq!(stats, expected);
    }

    #[test]
    fn the_encode_line_names_the_numbers_and_the_budget() {
        let report = judge(&summary(0, 8.0, &[]), 50);
        assert_eq!(report.lines[2].detail, "average 4.0 ms, 95th percentile 8.0 ms, budget 20.0 ms");
    }

    #[test]
    fn the_dnd_line_is_an_info_reminder() {
        let line = dnd_line();
        assert_eq!(line.label, DND_LABEL);
        assert_eq!(line.status, Status::Info);
        assert_eq!(line.detail, "Reminder: turn on Do Not Disturb (Win+N) so notifications stay out of your recording");
    }

    #[test]
    fn the_dnd_reminder_never_counts_toward_the_overall_result() {
        assert_ne!(dnd_line().status, Status::Pass);
        assert_ne!(dnd_line().status, Status::Warn);
        assert_ne!(dnd_line().status, Status::Fail);
        // Info sorts below Pass, so an Info line can never raise the overall result.
        assert!(Status::Info < Status::Pass);
    }

    #[test]
    fn a_waiting_app_track_is_info_and_never_a_failure() {
        // The track recorded silence, which would warn for a device track.
        let take = summary(0, 5.0, &[-100.0, -12.0]);
        let report = judge_with_cam(&take, None, &[Some("Discord".to_owned()), None], 60);
        assert_eq!(report.lines[3].status, Status::Info);
        assert_eq!(report.lines[3].detail, "Discord is not running; the track will attach when it opens");
        assert_eq!(report.lines[4].status, Status::Pass);
        assert_eq!(report.overall, Status::Pass);
    }

    #[test]
    fn only_app_tracks_that_are_not_attached_are_waiting() {
        let track = |attached| TrackSnapshot {
            name: String::new(),
            peak_dbfs: -100.0,
            clipped: false,
            packets: 0,
            overflows: 0,
            device_lost: false,
            attached,
            instances: Default::default(),
        };
        let apps = [None, Some("Discord".to_owned()), Some("Spotify".to_owned())];
        assert_eq!(waiting_apps(&apps, &[track(false), track(false), track(true)]), [None, Some("Discord".to_owned()), None]);
    }

    #[test]
    fn the_folder_rows_fail_on_a_bad_folder_warn_on_low_free_space_and_name_the_numbers() {
        let ok = FolderCheck { label: "Recordings folder".to_owned(), path: "C:\\Videos\\CutMark".to_owned(), not_writable: None, free_bytes: Some(50 * GIB) };
        let line = folder_line(&ok);
        assert_eq!(line.status, Status::Pass);
        assert_eq!(line.detail, "50.0 GB free on C:\\Videos\\CutMark");
        // Free space below the threshold warns.
        let low = FolderCheck { free_bytes: Some(9 * GIB), ..ok.clone() };
        assert_eq!(folder_line(&low).status, Status::Warn);
        // A folder that cannot be written to fails and names the path.
        let bad = FolderCheck { not_writable: Some("access denied".to_owned()), ..ok.clone() };
        let line = folder_line(&bad);
        assert_eq!(line.status, Status::Fail);
        assert_eq!(line.detail, "Can't save to C:\\Videos\\CutMark: access denied");
        // Free space that cannot be read is a pass with a note.
        let unknown = FolderCheck { free_bytes: None, ..ok.clone() };
        assert_eq!(folder_line(&unknown).status, Status::Pass);
        assert_eq!(folder_line(&unknown).detail, "C:\\Videos\\CutMark: the free space could not be read");
        assert_eq!(folder_lines(&[ok, bad, low]).len(), 3);
    }
}
