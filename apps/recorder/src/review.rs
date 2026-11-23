//! The Review scan: the takes of the current project folder, grouped by day
//! (spec "Review mode", the left Takes panel's data).
//!
//! A take is the main media file (mkv, mp4, mov) plus every file that shares
//! its stem: the camera ISO file, the WAV sidecars, the marks sidecar and the
//! derived `_marks` files. The start time comes from the marks sidecar's
//! session start when there is one, and from the video file's modified time
//! otherwise. media-io exposes no file probe, so the duration comes from the
//! sidecar's recording length and a take without a sidecar shows none; no
//! new dependency was added for this.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, Utc};

/// The file extensions that make a file group a take (spec "Review mode").
const MEDIA_EXTENSIONS: [&str; 3] = ["mkv", "mp4", "mov"];

/// File-name suffixes whose stem is the take's stem; a plain `file_stem`
/// would keep the `.marks` or `_marks` part and split a take's files apart.
const SIDECAR_SUFFIXES: [&str; 3] = [".marks.json", "_marks.md", "_marks.csv"];

/// One file of a take.
pub struct TakeFile {
    /// The full path of the file.
    pub path: PathBuf,
    /// The file's size in bytes.
    pub size: u64,
}

/// One take of the project: its media file, every file that shares the stem,
/// and what the sidecar says about it.
pub struct Take {
    /// The take's name: the stem every file of the take shares.
    pub name: String,
    /// When the take started: the sidecar's session start, else the media file's modified time.
    pub start: DateTime<Local>,
    /// The recording length from the sidecar, in milliseconds; None without a sidecar.
    pub duration_ms: Option<u64>,
    /// The size of every file of the take together, in bytes.
    pub total_size: u64,
    /// Every file of the take, sorted by name.
    pub files: Vec<TakeFile>,
    /// The main media file.
    pub video: PathBuf,
    /// The camera ISO file, when the take has one.
    pub iso: Option<PathBuf>,
    /// The marks from the sidecar, in timeline order; empty without a sidecar.
    pub marks: Vec<marks::Mark>,
}

/// One day of takes, newest first.
pub struct Day {
    /// The local date the takes started on.
    pub date: NaiveDate,
    /// The day's takes, newest first.
    pub takes: Vec<Take>,
}

/// Scans the project folder for takes and returns its days, newest first.
///
/// The folder is read one level deep: a take lives in its own folder under
/// the project folder, and stray media files directly in the project folder
/// count too. Unreadable entries are logged and skipped.
pub fn scan(project_folder: &Path) -> Vec<Day> {
    let mut groups: HashMap<String, Vec<TakeFile>> = HashMap::new();
    collect_folder(project_folder, &mut groups);
    if let Ok(entries) = fs::read_dir(project_folder) {
        for entry in entries.flatten() {
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                collect_folder(&entry.path(), &mut groups);
            }
        }
    }
    let mut takes: Vec<Take> = groups.into_iter().filter_map(|(name, files)| build_take(name, files)).collect();
    takes.sort_by(|a, b| b.start.cmp(&a.start).then_with(|| b.name.cmp(&a.name)));
    group_by_day(takes)
}

/// The take of `selected`, by name, across every day.
pub fn find<'a>(days: &'a [Day], selected: Option<&str>) -> Option<&'a Take> {
    let selected = selected?;
    days.iter().flat_map(|day| &day.takes).find(|take| take.name == selected)
}

/// The takes of today with their total length: the count, the summed duration
/// in milliseconds, and how many of the takes had a duration at all.
pub fn today_summary(days: &[Day]) -> (usize, u64, usize) {
    let today = Local::now().date_naive();
    let mut count = 0;
    let mut total_ms = 0;
    let mut timed = 0;
    for take in days.iter().flat_map(|day| &day.takes) {
        if take.start.date_naive() == today {
            count += 1;
            if let Some(duration) = take.duration_ms {
                total_ms += duration;
                timed += 1;
            }
        }
    }
    (count, total_ms, timed)
}

/// Formats a length in milliseconds as HH:MM:SS.
pub fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// Formats a size in bytes in the largest unit that keeps one decimal digit.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

/// Adds every file directly in `folder` to its stem's group.
fn collect_folder(folder: &Path, groups: &mut HashMap<String, Vec<TakeFile>>) {
    let Ok(entries) = fs::read_dir(folder) else {
        log::warn!("could not read {}, its takes are skipped", folder.display());
        return;
    };
    for entry in entries.flatten().filter(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false)) {
        let path = entry.path();
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let name = entry.file_name().to_string_lossy().into_owned();
        groups.entry(take_stem(&name)).or_default().push(TakeFile { path, size });
    }
}

/// The stem a file belongs to: the sidecar and its derived files keep the
/// take's stem, everything else uses its own `file_stem`.
fn take_stem(file_name: &str) -> String {
    for suffix in SIDECAR_SUFFIXES {
        if let Some(stem) = file_name.strip_suffix(suffix) {
            return stem.to_owned();
        }
    }
    Path::new(file_name).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Whether `path` is a main media file.
fn is_media(path: &Path) -> bool {
    path.extension().is_some_and(|extension| MEDIA_EXTENSIONS.iter().any(|candidate| extension.eq_ignore_ascii_case(candidate)))
}

/// Builds the take of one stem's file group, or None when the group holds no
/// media file or its modified time cannot be read.
fn build_take(name: String, mut files: Vec<TakeFile>) -> Option<Take> {
    files.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    let video = files.iter().filter(|file| is_media(&file.path)).map(|file| file.path.clone()).min()?;
    let modified = fs::metadata(&video).and_then(|meta| meta.modified()).ok()?;
    let sidecar = load_sidecar(&video);
    let duration_ms = sidecar.as_ref().map(|sidecar| sidecar.media.iter().map(|media| media.duration_ms).sum());
    let marks = sidecar.as_ref().map(|sidecar| sidecar.marks.clone()).unwrap_or_default();
    let start = match &sidecar {
        Some(sidecar) => sidecar.session.started_at.with_timezone(&Local),
        None => DateTime::<Utc>::from(modified).with_timezone(&Local),
    };
    let iso = files.iter().find(|file| file.path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("iso"))).map(|file| file.path.clone());
    Some(Take { name, start, duration_ms, total_size: files.iter().map(|file| file.size).sum(), files, video, iso, marks })
}

/// Loads the sidecar of `video`, logging problems; None when there is none or
/// it cannot be read.
fn load_sidecar(video: &Path) -> Option<marks::Sidecar> {
    let sidecar_path = marks::discover(video)?;
    match marks::load(&sidecar_path) {
        Ok((sidecar, warnings)) => {
            if !warnings.is_empty() {
                log::warn!("{} loaded with {} warning(s)", sidecar_path.display(), warnings.len());
            }
            Some(sidecar)
        }
        Err(err) => {
            log::warn!("could not load {}: {err}", sidecar_path.display());
            None
        }
    }
}

/// Splits the newest-first takes into one day per local start date, keeping
/// the order (so the days come out newest first too).
fn group_by_day(takes: Vec<Take>) -> Vec<Day> {
    let mut days: Vec<Day> = Vec::new();
    for take in takes {
        let date = take.start.date_naive();
        match days.last_mut() {
            Some(day) if day.date == date => day.takes.push(take),
            _ => days.push(Day { date, takes: vec![take] }),
        }
    }
    days
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    /// A fresh, empty folder under the system temp folder.
    fn fresh_dir(name: &str) -> Result<PathBuf, io::Error> {
        let dir = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-{name}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// A minimal valid sidecar: `marks` marks, started at `started_at`, one media file of `duration_ms`.
    fn sidecar_json(started_at: &str, duration_ms: u64, marks: usize) -> String {
        let entries: Vec<String> = (0..marks)
            .map(|index| {
                format!(
                    "    {{\n      \"id\": \"0196f3a2-9d44-7e02-8c15-6a0b3e7d2f9{index}\",\n      \"type\": \"highlight\",\n      \"media\": 0,\n      \"t_ms\": {},\n      \"source\": \"hotkey\",\n      \"created_at\": \"{started_at}\"\n    }}",
                    1000 * (index as u64 + 1)
                )
            })
            .collect();
        format!(
            "{{\n  \"schema_version\": 1,\n  \"session\": {{\n    \"id\": \"0196f3a2-7c10-7b3e-9a41-2d5e8f1c6b07\",\n    \"started_at\": \"{started_at}\",\n    \"ended_at\": \"{started_at}\",\n    \"recovered\": false,\n    \"writer\": {{ \"app\": \"cutmark-recorder\", \"version\": \"1.2.0\" }}\n  }},\n  \"media\": [\n    {{\n      \"index\": 0,\n      \"path\": \"take.mkv\",\n      \"offset_ms\": 0,\n      \"duration_ms\": {duration_ms}\n    }}\n  ],\n  \"marks\": [\n{}\n  ]\n}}",
            entries.join(",\n")
        )
    }

    #[test]
    fn takes_group_by_stem_and_carry_their_files_marks_and_duration() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-groups")?;
        let project = root.join("Show");
        let one = project.join("2026-11-20_10-00-00_take01");
        fs::create_dir_all(&one)?;
        fs::write(one.join("2026-11-20_10-00-00_take01.mkv"), b"video")?;
        fs::write(one.join("2026-11-20_10-00-00_take01.iso"), b"iso")?;
        fs::write(one.join("2026-11-20_10-00-00_take01.wav"), b"wav")?;
        fs::write(one.join("2026-11-20_10-00-00_take01.marks.json"), sidecar_json("2026-11-20T12:00:00.000Z", 3_698_784, 1))?;
        fs::write(one.join("2026-11-20_10-00-00_take01_marks.md"), b"md")?;
        fs::write(one.join("2026-11-20_10-00-00_take01_marks.csv"), b"csv")?;
        let two = project.join("2026-11-21_09-00-00_take02");
        fs::create_dir_all(&two)?;
        fs::write(two.join("2026-11-21_09-00-00_take02.mkv"), b"video")?;
        fs::write(two.join("2026-11-21_09-00-00_take02.marks.json"), sidecar_json("2026-11-21T12:00:00.000Z", 1_000, 2))?;

        let days = scan(&project);
        // Two sessions 24 hours apart: one day each, the newer day first.
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].takes[0].name, "2026-11-21_09-00-00_take02");
        assert_eq!(days[1].takes[0].name, "2026-11-20_10-00-00_take01");

        let take = &days[1].takes[0];
        assert_eq!(take.files.len(), 6, "iso, wav, sidecar, md and csv belong to the take");
        assert!(take.iso.as_ref().is_some_and(|iso| iso.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("iso"))));
        assert!(take.files.iter().any(|file| file.path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("wav"))));
        assert_eq!(take.marks.len(), 1);
        assert_eq!(take.duration_ms, Some(3_698_784));
        let newer = &days[0].takes[0];
        assert_eq!(newer.marks.len(), 2);
        assert_eq!(newer.duration_ms, Some(1_000));
        assert_eq!(newer.files.len(), 2);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn takes_of_one_day_come_newest_first() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-order")?;
        let project = root.join("Show");
        for (folder, started) in [
            ("2026-11-21_09-00-00_take01", "2026-11-21T12:00:00.000Z"),
            ("2026-11-21_11-00-00_take02", "2026-11-21T13:00:00.000Z"),
            ("2026-11-21_10-00-00_take03", "2026-11-21T12:30:00.000Z"),
        ] {
            let take = project.join(folder);
            fs::create_dir_all(&take)?;
            fs::write(take.join(format!("{folder}.mkv")), b"video")?;
            fs::write(take.join(format!("{folder}.marks.json")), sidecar_json(started, 1_000, 0))?;
        }
        let days = scan(&project);
        assert_eq!(days.len(), 1);
        let names: Vec<&str> = days[0].takes.iter().map(|take| take.name.as_str()).collect();
        assert_eq!(names, ["2026-11-21_11-00-00_take02", "2026-11-21_10-00-00_take03", "2026-11-21_09-00-00_take01"]);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_take_with_no_sidecar_shows_zero_marks_and_no_duration() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-no-sidecar")?;
        let project = root.join("Show");
        let take = project.join("2026-11-21_09-00-00_take01");
        fs::create_dir_all(&take)?;
        fs::write(take.join("2026-11-21_09-00-00_take01.mkv"), b"video")?;
        fs::write(take.join("2026-11-21_09-00-00_take01.iso"), b"iso")?;
        let days = scan(&project);
        assert_eq!(days.len(), 1);
        let take = &days[0].takes[0];
        assert_eq!(take.marks.len(), 0);
        assert_eq!(take.duration_ms, None);
        assert_eq!(take.files.len(), 2);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn groups_without_a_media_file_are_not_takes() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-no-media")?;
        let project = root.join("Show");
        fs::create_dir_all(project.join("notes"))?;
        fs::write(project.join("notes").join("notes.txt"), b"text")?;
        fs::write(project.join("loose.marks.json"), b"{}")?;
        assert!(scan(&project).is_empty());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    /// A take for the summary tests, with only the fields they read.
    fn take_named(name: &str, start: DateTime<Local>, duration_ms: Option<u64>) -> Take {
        Take { name: name.to_owned(), start, duration_ms, total_size: 0, files: Vec::new(), video: PathBuf::new(), iso: None, marks: Vec::new() }
    }

    #[test]
    fn the_today_summary_counts_todays_takes_and_their_length() {
        let yesterday = Local::now() - chrono::Duration::days(1);
        let days = vec![
            Day { date: yesterday.date_naive(), takes: vec![take_named("old", yesterday, Some(60_000))] },
            Day { date: Local::now().date_naive(), takes: vec![take_named("a", Local::now(), Some(30_000)), take_named("b", Local::now(), None)] },
        ];
        assert_eq!(today_summary(&days), (2, 30_000, 1));
    }

    #[test]
    fn durations_and_sizes_are_formatted() {
        assert_eq!(format_duration(0), "00:00:00");
        assert_eq!(format_duration(3_698_784), "01:01:38");
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1_500_000_000), "1.5 GB");
        assert_eq!(format_size(2_500_000), "2.5 MB");
        assert_eq!(format_size(1_500), "1.5 KB");
    }
}
