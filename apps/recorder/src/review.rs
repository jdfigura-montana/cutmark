//! The Review scan: the takes of the current project folder, grouped by day
//! (spec "Review mode", the left Takes panel's data).
//!
//! A take is the main media file (mkv, mp4, mov) plus every file that shares
//! its stem: the camera ISO file (`_cam`), the track WAV files (`_t*_*.wav`),
//! the marks sidecar and the derived `_marks` files, session.json and the
//! `_script` files. The start time comes from the marks sidecar's
//! session start when there is one, and from the video file's modified time
//! otherwise. The duration is what media-io's header probe reads from the main
//! file, with the sidecar's length for that file as the fallback. The video
//! formats come from the same probe.
//!
//! The rename and delete actions (spec "Review mode", the right panel's
//! Actions group) live here too: renaming moves every file of a take to a new
//! stem and rewrites the file name references inside the marks sidecar and
//! session.json, deleting
//! hands the files to the Recycle Bin through the `trash` crate. Adding a mark
//! writes a Note to the sidecar through the marks crate and regenerates the
//! derived `_marks` files from the marks exports.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, NaiveDate, Utc};
use media_io::player::probe::{VideoInfo, probe};
use serde::Deserialize;

use crate::config::CompositeMode;

/// The file extensions that make a file group a take (spec "Review mode").
const MEDIA_EXTENSIONS: [&str; 3] = ["mkv", "mp4", "mov"];

/// The end of the marks sidecar's file name, after the take's stem.
const MARKS_SUFFIX: &str = ".marks.json";

/// The end of session.json's file name, after the take's stem.
const SESSION_SUFFIX: &str = "_session.json";

/// File-name suffixes whose stem is the take's stem; a plain `file_stem`
/// would keep the `.marks` or `_marks` part and split a take's files apart.
const SIDECAR_SUFFIXES: [&str; 6] = [MARKS_SUFFIX, "_marks.md", "_marks.csv", SESSION_SUFFIX, "_script.md", "_script.csv"];

/// What the camera file's stem adds to the take's stem.
const CAM_SUFFIX: &str = "_cam";

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
    /// The main file's length in milliseconds (see [`take_duration`]); None when nothing says.
    pub duration_ms: Option<u64>,
    /// The size of every file of the take together, in bytes.
    pub total_size: u64,
    /// Every file of the take, sorted by name.
    pub files: Vec<TakeFile>,
    /// The main media file.
    pub video: PathBuf,
    /// The camera ISO file, when the take has one.
    pub iso: Option<PathBuf>,
    /// The main file's video format (see [`format_video`]); None when the probe cannot read it.
    pub video_format: Option<String>,
    /// The camera ISO file's video format; None without one or when the probe cannot read it.
    pub iso_format: Option<String>,
    /// The marks from the sidecar, in timeline order; empty without a sidecar.
    pub marks: Vec<marks::Mark>,
    /// The composite mode from the take's session.json; Baked without a composite block.
    pub composite: CompositeMode,
}

/// One day of takes, newest first.
pub struct Day {
    /// The local date the takes started on.
    pub date: NaiveDate,
    /// The day's takes, newest first.
    pub takes: Vec<Take>,
}

/// Which of the project's two take folders Review lists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ReviewView {
    /// The normal takes.
    #[default]
    Takes,
    /// The saved replays.
    Replays,
}

/// What a toast's Review button opens: a take, in the view that lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    /// The view that lists the take.
    pub view: ReviewView,
    /// The take's name.
    pub take: String,
}

impl ReviewTarget {
    /// Switches Review to the target's view and selects its take.
    pub fn select(self, view: &mut ReviewView, selected: &mut Option<String>) {
        *view = self.view;
        *selected = Some(self.take);
    }
}

/// Scans a takes folder (the project folder or its replays folder) and returns its days, newest first.
///
/// The folder is read one level deep: a take lives in its own folder under
/// the scanned folder, and stray media files directly in it count too.
/// Unreadable entries are logged and skipped.
pub fn scan(folder: &Path) -> Vec<Day> {
    let mut groups: HashMap<String, Vec<TakeFile>> = HashMap::new();
    collect_folder(folder, &mut groups);
    if let Ok(entries) = fs::read_dir(folder) {
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

/// Formats a length in milliseconds as HH:MM:SS.
pub fn format_duration(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// The length of a take, in milliseconds: what the probe of the main file
/// `video` says, else the sidecar's length for that file. A zero counts as no
/// answer. The takes list, the File group and the playback all ask here.
pub fn take_duration(video: &Path, probed_ms: Option<u64>, sidecar: Option<&marks::Sidecar>) -> Option<u64> {
    let recorded = || {
        let sidecar = sidecar?;
        let index = media_index(sidecar, video);
        sidecar.media.iter().find(|entry| entry.index == index).map(|entry| entry.duration_ms)
    };
    probed_ms.filter(|ms| *ms > 0).or_else(|| recorded().filter(|ms| *ms > 0))
}

/// What sits between the parts of a video format. The task for these rows asks for the dot,
/// so this one line is let through the Recorder's middle dot check.
const FORMAT_SEPARATOR: &str = " \u{b7} "; // allow: spaced middle dot

/// A video stream as the File group shows it: "AV1 · 2560×1440 · 60 fps". The
/// frame rate keeps up to two decimals and is left out when the file has none.
pub fn format_video(video: &VideoInfo) -> String {
    let codec = match video.codec.as_str() {
        "h264" => "H.264".to_owned(),
        other => other.to_uppercase(),
    };
    let mut parts = vec![codec, format!("{}\u{d7}{}", video.width, video.height)];
    if video.fps > 0.0 {
        let fps = format!("{:.2}", video.fps);
        parts.push(format!("{} fps", fps.trim_end_matches('0').trim_end_matches('.')));
    }
    parts.join(FORMAT_SEPARATOR)
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

/// The timeline position of the next mark after `position_ms`, in
/// milliseconds. The positions must be sorted; when none is after the
/// position the search wraps to the first mark, so a press at the end
/// returns to the take's first mark.
pub fn next_mark(mark_positions: &[u64], position_ms: u64) -> Option<u64> {
    let first = mark_positions.first().copied()?;
    mark_positions.iter().find(|&&t| t > position_ms).copied().or(Some(first))
}

/// The timeline position of the previous mark before `position_ms`, in
/// milliseconds. The positions must be sorted; when none is before the
/// position the search wraps to the last mark, so a press at the start
/// returns to the take's last mark.
pub fn previous_mark(mark_positions: &[u64], position_ms: u64) -> Option<u64> {
    let last = mark_positions.last().copied()?;
    mark_positions.iter().rev().find(|&&t| t < position_ms).copied().or(Some(last))
}

/// The index into `mark_positions` of the mark under `position_ms`: the last
/// one at or before it. The positions must be sorted.
pub fn mark_under(mark_positions: &[u64], position_ms: u64) -> Option<usize> {
    mark_positions.iter().rposition(|&t| t <= position_ms)
}

/// The milliseconds a scrub at `x` points at on a track that starts at
/// `left`, is `width` px wide and spans `duration_ms`: a pointer before the
/// track or on it seeks to 0, one past its end to the take's length.
pub fn scrub_ms(x: f32, left: f32, width: f32, duration_ms: u64) -> u64 {
    if width <= 0.0 || duration_ms == 0 {
        return 0;
    }
    let fraction = f64::from(((x - left) / width).clamp(0.0, 1.0));
    (fraction * duration_ms as f64) as u64
}

/// Adds a Note mark at `t_ms` (the playback position in the take's main file)
/// to the take's marks sidecar through the marks crate, then regenerates the
/// derived `_marks.md` and `_marks.csv` files from the marks exports. A take
/// without a sidecar gets one.
pub fn add_mark(take: &Take, t_ms: u64) -> Result<(), String> {
    let sidecar_path = marks::discover(&take.video).unwrap_or_else(|| take.video.with_extension("marks.json"));
    let mut sidecar = if sidecar_path.exists() {
        let (sidecar, _) = marks::load(&sidecar_path).map_err(|err| format!("{} could not be read: {err}", sidecar_path.display()))?;
        sidecar
    } else {
        new_sidecar(take, t_ms)
    };
    let mark = marks::Mark {
        id: uuid::Uuid::now_v7(),
        mark_type: marks::MarkType::Note,
        media: media_index(&sidecar, &take.video),
        t_ms,
        end_ms: None,
        label: None,
        notes: None,
        source: marks::Source::Ui,
        created_at: Utc::now(),
        script: None,
        auto: None,
        original_type: None,
    };
    sidecar.marks.push(mark);
    // Sort by session time, the way the marks crate writes a sidecar; the
    // offsets are read first so the sort closure does not borrow the sidecar.
    let offsets: Vec<u64> = sidecar.media.iter().map(|entry| entry.offset_ms).collect();
    sidecar.marks.sort_by(|a, b| {
        let session = |mark: &marks::Mark| offsets.get(mark.media).copied().unwrap_or(0).saturating_add(mark.t_ms);
        session(a).cmp(&session(b)).then_with(|| a.id.cmp(&b.id))
    });
    let mut text = serde_json::to_string_pretty(&sidecar).map_err(|err| format!("the sidecar could not be encoded: {err}"))?;
    text.push('\n');
    write_file(&sidecar_path, text.as_bytes())?;
    let folder = sidecar_path.parent().unwrap_or_else(|| Path::new("."));
    write_file(&folder.join(format!("{}_marks.md", take.name)), marks::markdown(&sidecar).as_bytes())?;
    write_file(&folder.join(format!("{}_marks.csv", take.name)), &marks::csv(&sidecar))?;
    Ok(())
}

/// A fresh sidecar for a take that has none: one media entry for the main
/// file and a session that starts now. The entry's length is at least
/// `t_ms`, so the reader never clamps the first mark away.
fn new_sidecar(take: &Take, t_ms: u64) -> marks::Sidecar {
    marks::Sidecar {
        schema_version: marks::SCHEMA_VERSION,
        session: marks::Session {
            id: uuid::Uuid::now_v7(),
            started_at: Utc::now(),
            ended_at: Some(Utc::now()),
            recovered: false,
            writer: marks::WriterInfo { app: "cutmark-recorder".to_owned(), version: env!("CARGO_PKG_VERSION").to_owned() },
            clock: None,
        },
        media: vec![marks::MediaEntry {
            index: 0,
            path: take.video.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            offset_ms: 0,
            duration_ms: take.duration_ms.unwrap_or(0).max(t_ms),
        }],
        context: None,
        marks: Vec::new(),
    }
}

/// The index of the take's main file in the sidecar's media list, matched by
/// file name; 0 when no entry names it.
fn media_index(sidecar: &marks::Sidecar, video: &Path) -> usize {
    let name = video.file_name();
    sidecar.media.iter().find(|entry| Path::new(&entry.path).file_name() == name).map_or(0, |entry| entry.index)
}

/// Writes `bytes` to `path` through a temp file in the same folder that is
/// renamed over it (the atomic-write rule), removing the temp file on failure.
fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = path.with_file_name(format!("{name}.tmp"));
    let written = fs::write(&temp, bytes).and_then(|()| fs::rename(&temp, path));
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(|err| format!("{} could not be written: {err}", path.display()))
}

/// Adds every file directly in `folder` to its stem's group.
fn collect_folder(folder: &Path, groups: &mut HashMap<String, Vec<TakeFile>>) {
    let Ok(entries) = fs::read_dir(folder) else {
        log::warn!("could not read {}, its takes are skipped", folder.display());
        return;
    };
    let files: Vec<(String, TakeFile)> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_file()).unwrap_or(false))
        .map(|entry| {
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            (entry.file_name().to_string_lossy().into_owned(), TakeFile { path: entry.path(), size })
        })
        .collect();
    let videos: HashSet<String> = files
        .iter()
        .map(|(name, _)| Path::new(name))
        .filter(|path| is_media(path))
        .filter_map(|path| path.file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .collect();
    for (name, file) in files {
        groups.entry(take_stem(&name, &videos)).or_default().push(file);
    }
}

/// The take's stem inside a video's stem when the video is a camera file: it
/// ends in `_cam` and the folder's `videos` (their stems) hold the main video
/// it belongs to. A `_cam` video without one is a main take itself.
fn camera_base<'a>(stem: &'a str, videos: &HashSet<String>) -> Option<&'a str> {
    stem.strip_suffix(CAM_SUFFIX).filter(|base| videos.contains(*base) && camera_base(base, videos).is_none())
}

/// The stem a file belongs to: the sidecars and derived files, the camera
/// file and the track WAV files keep the take's stem, everything else uses its
/// own `file_stem`. `videos` holds the stems of the videos in the file's folder.
fn take_stem(file_name: &str, videos: &HashSet<String>) -> String {
    for suffix in SIDECAR_SUFFIXES {
        if let Some(stem) = file_name.strip_suffix(suffix) {
            return stem.to_owned();
        }
    }
    let path = Path::new(file_name);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let base = if is_media(path) {
        camera_base(&stem, videos)
    } else if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("wav")) {
        track_wav_base(&stem)
    } else {
        None
    };
    match base {
        Some(base) => base.to_owned(),
        None => stem,
    }
}

/// The take's stem inside the stem of a track WAV file, `{stem}_t{number}_{name}`;
/// None when the stem does not have that shape. The first match wins, because a
/// track name may repeat the pattern.
fn track_wav_base(stem: &str) -> Option<&str> {
    stem.match_indices("_t").find_map(|(at, marker)| {
        let rest = &stem[at + marker.len()..];
        let after_number = rest.trim_start_matches(|ch: char| ch.is_ascii_digit());
        (after_number.len() < rest.len() && after_number.starts_with('_')).then(|| &stem[..at])
    })
}

/// Whether `path` has a media file's extension.
fn is_media(path: &Path) -> bool {
    path.extension().is_some_and(|extension| MEDIA_EXTENSIONS.iter().any(|candidate| extension.eq_ignore_ascii_case(candidate)))
}

/// Builds the take of one stem's file group, or None when the group holds no
/// media file or its modified time cannot be read.
fn build_take(name: String, mut files: Vec<TakeFile>) -> Option<Take> {
    files.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
    let media_with_stem = |stem: &str| {
        files
            .iter()
            .filter(|file| is_media(&file.path) && file.path.file_stem().is_some_and(|found| found.to_string_lossy() == stem))
            .map(|file| file.path.clone())
            .min()
    };
    let video = media_with_stem(&name)?;
    let iso = media_with_stem(&format!("{name}{CAM_SUFFIX}"));
    let modified = fs::metadata(&video).and_then(|meta| meta.modified()).ok()?;
    let sidecar = load_sidecar(&video);
    let info = probe(&video).ok();
    let duration_ms = take_duration(&video, info.as_ref().map(|info| info.duration_ms), sidecar.as_ref());
    let video_format = info.and_then(|info| info.video).map(|stream| format_video(&stream));
    let iso_format = iso.as_deref().and_then(|path| probe(path).ok()).and_then(|info| info.video).map(|stream| format_video(&stream));
    let marks = sidecar.as_ref().map(|sidecar| sidecar.marks.clone()).unwrap_or_default();
    let start = match &sidecar {
        Some(sidecar) => sidecar.session.started_at.with_timezone(&Local),
        None => DateTime::<Utc>::from(modified).with_timezone(&Local),
    };
    let composite = load_composite_mode(&video.with_file_name(format!("{name}{SESSION_SUFFIX}")));
    let total_size = files.iter().map(|file| file.size).sum();
    Some(Take { name, start, duration_ms, total_size, files, video, iso, video_format, iso_format, marks, composite })
}

/// The `composite.mode` of a take's session.json; Baked when the file or the block is missing or
/// cannot be read (takes made before the block existed are all baked).
fn load_composite_mode(session_file: &Path) -> CompositeMode {
    fs::read(session_file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|doc| CompositeMode::deserialize(&doc["composite"]["mode"]).ok())
        .unwrap_or_default()
}

/// Loads the sidecar of `video`, logging problems; None when there is none or
/// it cannot be read.
pub fn load_sidecar(video: &Path) -> Option<marks::Sidecar> {
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

/// The characters Windows forbids in file and folder names.
const FORBIDDEN_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// The device names Windows reserves, with their trailing digits.
const RESERVED_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7",
    "LPT8", "LPT9",
];

/// Checks a new take name: not empty, no characters Windows forbids, no
/// reserved device name, no trailing dot or space, and no existing take with
/// the same name (Windows file names ignore case). Returns the reason the
/// name must be refused.
pub fn check_take_name(new_name: &str, existing_takes: &[String]) -> Result<(), String> {
    if new_name.trim().is_empty() {
        return Err("The name cannot be empty".to_owned());
    }
    if let Some(ch) = new_name.chars().find(|ch| FORBIDDEN_CHARS.contains(ch) || ch.is_control()) {
        return Err(format!("The name cannot contain the character {ch:?}"));
    }
    if new_name.ends_with('.') || new_name.ends_with(' ') {
        return Err("The name cannot end with a dot or a space".to_owned());
    }
    let base = new_name.split('.').next().unwrap_or(new_name);
    if RESERVED_NAMES.iter().any(|reserved| reserved.eq_ignore_ascii_case(base)) {
        return Err(format!("{base} is reserved by Windows"));
    }
    if existing_takes.iter().any(|existing| existing.eq_ignore_ascii_case(new_name)) {
        return Err("A take with this name already exists".to_owned());
    }
    Ok(())
}

/// Renames every file of the take to `new_name` and rewrites the file name
/// references inside the marks sidecar and session.json. When a rename or an
/// update fails, the changes already made are rolled back and the reason is
/// returned.
pub fn rename_take(take: &Take, new_name: &str) -> Result<(), String> {
    let mut renames: Vec<(PathBuf, PathBuf)> = Vec::new();
    for file in &take.files {
        let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(rest) = name.strip_prefix(take.name.as_str()) else {
            return Err(format!("{name} does not start with the take's name"));
        };
        renames.push((file.path.clone(), file.path.with_file_name(format!("{new_name}{rest}"))));
    }
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (from, to) in renames {
        if from == to {
            continue;
        }
        if to.exists() {
            roll_back(&done);
            return Err(format!("{} already exists", to.display()));
        }
        if let Err(err) = fs::rename(&from, &to) {
            log::warn!("could not rename {}: {err}", from.display());
            roll_back(&done);
            return Err(format!("{} could not be renamed", from.display()));
        }
        done.push((from, to));
    }
    // Both JSON files record the take's file names; point those at the new ones.
    let names: HashMap<String, String> = done.iter().filter_map(|(from, to)| Some((file_name_of(from)?, file_name_of(to)?))).collect();
    // The sidecar's file names are its `media[].path` values. session.json has
    // them in `composite.main.file`, `composite.layers[].file` and possibly
    // elsewhere, so there any value naming one of the take's files counts.
    let mut rewritten: Vec<(&Path, String)> = Vec::new();
    for (suffix, what, only_key) in [(MARKS_SUFFIX, "the marks sidecar", Some("path")), (SESSION_SUFFIX, "session.json", None)] {
        let old_name = format!("{}{suffix}", take.name);
        let Some((_, json)) = done.iter().find(|(from, _)| from.file_name().is_some_and(|name| name.to_string_lossy() == old_name)) else {
            continue;
        };
        match rewrite_json(json, &names, only_key) {
            Ok(original) => rewritten.push((json, original)),
            Err(err) => {
                log::warn!("could not rewrite {}: {err}", json.display());
                for (path, original) in &rewritten {
                    if let Err(err) = write_file(path, original.as_bytes()) {
                        log::warn!("could not roll back an update: {err}");
                    }
                }
                roll_back(&done);
                return Err(format!("{what} could not be updated"));
            }
        }
    }
    Ok(())
}

/// Renames the already renamed files back, last first, logging the ones that could not be undone.
fn roll_back(done: &[(PathBuf, PathBuf)]) {
    for (from, to) in done.iter().rev() {
        if let Err(err) = fs::rename(to, from) {
            log::warn!("could not roll back the rename of {}: {err}", to.display());
        }
    }
}

/// The file name of `path` as text.
fn file_name_of(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}

/// Gives `text` its new name when it names a renamed file, either bare or as
/// the last part of a path. Any other text is left alone.
fn rename_file_name(text: &mut String, names: &HashMap<String, String>) {
    let start = text.rfind(['/', '\\']).map_or(0, |at| at + 1);
    if let Some(new_name) = names.get(&text[start..]) {
        text.replace_range(start.., new_name);
    }
}

/// Renames the file names in the string values of the JSON `text`, only those
/// under the key `only_key` when there is one. Everything else stays byte for
/// byte, so the writer's key order and formatting survive (a round trip through
/// `Value` would sort the keys). `text` must be valid JSON.
fn rename_json_strings(text: &str, names: &HashMap<String, String>, only_key: Option<&str>) -> Result<String, serde_json::Error> {
    let mut updated = String::with_capacity(text.len());
    let mut copied = 0;
    let mut key = String::new();
    let mut open: Option<usize> = None;
    let mut escaped = false;
    for (at, byte) in text.bytes().enumerate() {
        let Some(start) = open else {
            if byte == b'"' {
                open = Some(at);
            }
            continue;
        };
        if escaped || byte == b'\\' {
            escaped = !escaped;
        } else if byte == b'"' {
            open = None;
            let mut value: String = serde_json::from_str(&text[start..=at])?;
            if text[at + 1..].trim_start().starts_with(':') {
                key = value;
            } else if only_key.is_none_or(|only| only == key) {
                let old = value.clone();
                rename_file_name(&mut value, names);
                if value != old {
                    updated.push_str(&text[copied..start]);
                    updated.push_str(&serde_json::to_string(&value)?);
                    copied = at + 1;
                }
            }
        }
    }
    updated.push_str(&text[copied..]);
    Ok(updated)
}

/// Parses the JSON file, renames its file name fields (see
/// [`rename_json_strings`]), writes the result atomically and returns the old
/// text for a rollback.
fn rewrite_json(path: &Path, names: &HashMap<String, String>, only_key: Option<&str>) -> Result<String, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("{} could not be read: {err}", path.display()))?;
    let invalid = |err: serde_json::Error| format!("{} is not valid JSON: {err}", path.display());
    serde_json::from_str::<serde_json::Value>(&text).map_err(invalid)?;
    let updated = rename_json_strings(&text, names, only_key).map_err(invalid)?;
    write_file(path, updated.as_bytes())?;
    Ok(text)
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
        sidecar_json_media(started_at, duration_ms, marks, "take.mkv")
    }

    /// [`sidecar_json`] with a media path of its own, like a real take's sidecar recording its file name.
    fn sidecar_json_media(started_at: &str, duration_ms: u64, marks: usize, media_path: &str) -> String {
        let entries: Vec<String> = (0..marks)
            .map(|index| {
                format!(
                    "    {{\n      \"id\": \"0196f3a2-9d44-7e02-8c15-6a0b3e7d2f9{index}\",\n      \"type\": \"highlight\",\n      \"media\": 0,\n      \"t_ms\": {},\n      \"source\": \"hotkey\",\n      \"created_at\": \"{started_at}\"\n    }}",
                    1000 * (index as u64 + 1)
                )
            })
            .collect();
        format!(
            "{{\n  \"schema_version\": 1,\n  \"session\": {{\n    \"id\": \"0196f3a2-7c10-7b3e-9a41-2d5e8f1c6b07\",\n    \"started_at\": \"{started_at}\",\n    \"ended_at\": \"{started_at}\",\n    \"recovered\": false,\n    \"writer\": {{ \"app\": \"cutmark-recorder\", \"version\": \"1.2.0\" }}\n  }},\n  \"media\": [\n    {{\n      \"index\": 0,\n      \"path\": \"{media_path}\",\n      \"offset_ms\": 0,\n      \"duration_ms\": {duration_ms}\n    }}\n  ],\n  \"marks\": [\n{}\n  ]\n}}",
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
        fs::write(one.join("2026-11-20_10-00-00_take01_cam.mkv"), b"camera")?;
        fs::write(one.join("2026-11-20_10-00-00_take01_t1_Mic_t2_a.wav"), b"wav")?;
        for suffix in ["_session.json", "_script.md", "_script.csv"] {
            fs::write(one.join(format!("2026-11-20_10-00-00_take01{suffix}")), b"data")?;
        }
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
        assert_eq!(take.files.len(), 9, "camera, wav, sidecar, session.json and the derived files belong to the take");
        assert_eq!(take.video, one.join("2026-11-20_10-00-00_take01.mkv"));
        assert_eq!(take.iso, Some(one.join("2026-11-20_10-00-00_take01_cam.mkv")));
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

    #[test]
    fn video_formats_are_built_from_the_probe() {
        let format = |codec: &str, width, height, fps| format_video(&VideoInfo { codec: codec.to_owned(), width, height, fps });
        let joined = |parts: &[&str]| parts.join(FORMAT_SEPARATOR);
        assert_eq!(FORMAT_SEPARATOR.trim(), "\u{b7}");
        assert_eq!(format("av1", 2560, 1440, 60.0), joined(&["AV1", "2560\u{d7}1440", "60 fps"]));
        assert_eq!(format("h264", 1920, 1080, 30_000.0 / 1_001.0), joined(&["H.264", "1920\u{d7}1080", "29.97 fps"]));
        assert_eq!(format("hevc", 1280, 720, 12.5), joined(&["HEVC", "1280\u{d7}720", "12.5 fps"]));
        assert_eq!(format("av1", 640, 368, 0.0), joined(&["AV1", "640\u{d7}368"]), "no frame rate, no fps part");
    }

    /// The probe of the main file wins; the sidecar answers only when the probe cannot, and
    /// only with the main file's own entry, never the entries added up.
    #[test]
    fn the_duration_comes_from_the_probe_before_the_sidecar() -> Result<(), Box<dyn std::error::Error>> {
        let mut sidecar: marks::Sidecar = serde_json::from_str(&sidecar_json_media("2026-11-20T12:00:00.000Z", 11_000, 0, "take.mkv"))?;
        let mut camera = sidecar.media[0].clone();
        camera.index = 1;
        camera.path = "take_cam.mkv".to_owned();
        camera.duration_ms = 36_000;
        sidecar.media.push(camera);
        let video = Path::new("take.mkv");
        assert_eq!(take_duration(video, Some(47_000), Some(&sidecar)), Some(47_000));
        assert_eq!(take_duration(video, None, Some(&sidecar)), Some(11_000));
        assert_eq!(take_duration(video, Some(0), Some(&sidecar)), Some(11_000), "a probe without a length is no answer");
        assert_eq!(take_duration(video, None, None), None);
        Ok(())
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

    #[test]
    fn renaming_a_take_moves_every_file_and_updates_the_sidecar() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-rename")?;
        let project = root.join("Show");
        let folder = project.join("2026-11-20_10-00-00_take01");
        fs::create_dir_all(&folder)?;
        let stem = "2026-11-20_10-00-00_take01";
        let suffixes = [".mp4", "_cam.mp4", "_t1_Mic.wav", ".marks.json", "_marks.md", "_marks.csv", "_session.json", "_script.md", "_script.csv"];
        for suffix in suffixes {
            fs::write(folder.join(format!("{stem}{suffix}")), b"data")?;
        }
        fs::write(folder.join(format!("{stem}.marks.json")), sidecar_json_media("2026-11-20T12:00:00.000Z", 1_000, 1, &format!("{stem}.mp4")))?;
        let session = format!(r#"{{"composite":{{"mode":"clean","main":{{"file":"{stem}.mp4"}},"layers":[{{"file":"{stem}_cam.mp4"}}]}}}}"#);
        fs::write(folder.join(format!("{stem}_session.json")), session)?;
        let days = scan(&project);
        assert_eq!(days[0].takes.len(), 1);
        assert_eq!(days[0].takes[0].name, stem);
        assert_eq!(days[0].takes[0].composite, CompositeMode::Clean);

        let new_name = "2026-11-20_10-05-00_take01b";
        rename_take(&days[0].takes[0], new_name)?;
        for suffix in suffixes {
            assert!(!folder.join(format!("{stem}{suffix}")).exists(), "{suffix} was moved");
            assert!(folder.join(format!("{new_name}{suffix}")).exists(), "{suffix} has the new name");
        }
        let sidecar = fs::read_to_string(folder.join(format!("{new_name}.marks.json")))?;
        assert!(sidecar.contains(&format!("\"path\": \"{new_name}.mp4\"")), "the sidecar's media path follows the rename");
        assert!(!sidecar.contains(stem), "no old file name reference is left in the sidecar");
        let session: serde_json::Value = serde_json::from_slice(&fs::read(folder.join(format!("{new_name}_session.json")))?)?;
        assert_eq!(session["composite"]["main"]["file"], format!("{new_name}.mp4"));
        assert_eq!(session["composite"]["layers"][0]["file"], format!("{new_name}_cam.mp4"));
        let days = scan(&project);
        assert_eq!(days[0].takes.len(), 1);
        let take = &days[0].takes[0];
        assert_eq!(take.composite, CompositeMode::Clean, "a Clean take stays Clean");
        assert_eq!(take.files.len(), suffixes.len());
        assert_eq!(take.iso, Some(folder.join(format!("{new_name}_cam.mp4"))));
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    /// A short stem also turns up in timestamps, IDs and labels; only the file name fields may change.
    #[test]
    fn renaming_a_short_stem_changes_only_the_file_name_fields() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-rename-short")?;
        let project = root.join("Show");
        fs::create_dir_all(&project)?;
        for name in ["01.mkv", "01_cam.mkv", "01_t1_Mic.wav"] {
            fs::write(project.join(name), b"data")?;
        }
        let sidecar = sidecar_json_media("2026-11-01T01:01:01.001Z", 1_001, 2, "01.mkv");
        fs::write(project.join("01.marks.json"), &sidecar)?;
        let session = r#"{"id":"0196f3a2-0101","started_at":"2026-11-01T01:01:01.001Z","label":"take 01","frames":101,"composite":{"mode":"clean","main":{"file":"01.mkv"},"layers":[{"file":"01_cam.mkv"}]},"tracks":[{"name":"Mic 01","file":"01_t1_Mic.wav"}]}"#;
        fs::write(project.join("01_session.json"), session)?;
        let days = scan(&project);

        rename_take(&days[0].takes[0], "intro")?;

        // Compared as text: apart from the file names, not one byte may differ.
        let expected = sidecar.replace(r#""01.mkv""#, r#""intro.mkv""#);
        assert_eq!(fs::read_to_string(project.join("intro.marks.json"))?, expected);
        let expected = session
            .replace(r#""01.mkv""#, r#""intro.mkv""#)
            .replace(r#""01_cam.mkv""#, r#""intro_cam.mkv""#)
            .replace(r#""01_t1_Mic.wav""#, r#""intro_t1_Mic.wav""#);
        assert_eq!(fs::read_to_string(project.join("intro_session.json"))?, expected);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_cam_video_without_its_main_video_is_its_own_take() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-lone-cam")?;
        let project = root.join("Show");
        fs::create_dir_all(&project)?;
        fs::write(project.join("x_cam.mkv"), b"video")?;
        let days = scan(&project);
        let takes: Vec<&Take> = days.iter().flat_map(|day| &day.takes).collect();
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].name, "x_cam");
        assert_eq!(takes[0].video, project.join("x_cam.mkv"));
        assert_eq!(takes[0].iso, None);
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_name_another_take_uses_or_windows_forbids_is_refused() {
        let existing = ["2026-11-20_10-00-00_take01".to_owned()];
        assert!(check_take_name("2026-11-20_10-00-00_take02", &existing).is_ok());
        assert!(check_take_name("2026-11-20_10-00-00_take01", &existing).is_err(), "an existing take collides");
        assert!(check_take_name("2026-11-20_10-00-00_TAKE01", &existing).is_err(), "file names on Windows ignore case");
        assert!(check_take_name("", &existing).is_err(), "an empty name is refused");
        assert!(check_take_name("take:01", &existing).is_err(), "a colon is forbidden");
        assert!(check_take_name("take01 ", &existing).is_err(), "a trailing space is refused");
        assert!(check_take_name("take01.", &existing).is_err(), "a trailing dot is refused");
        assert!(check_take_name("com1", &existing).is_err(), "a reserved device name is refused");
    }

    /// The previous and next mark jump at every edge: no marks at all,
    /// before, on and past each end, with the wrap-around the functions define.
    #[test]
    fn next_and_previous_mark_select_at_the_edges() {
        let marks = [1_000, 2_000, 3_000];
        assert_eq!(next_mark(&[], 0), None);
        assert_eq!(previous_mark(&[], 0), None);
        assert_eq!(next_mark(&marks, 0), Some(1_000), "the first mark is next at the start");
        assert_eq!(next_mark(&marks, 1_000), Some(2_000), "a mark is skipped, not repeated");
        assert_eq!(next_mark(&marks, 2_999), Some(3_000));
        assert_eq!(next_mark(&marks, 3_000), Some(1_000), "next wraps to the first mark at the end");
        assert_eq!(next_mark(&marks, 9_999), Some(1_000));
        assert_eq!(previous_mark(&marks, 5_000), Some(3_000));
        assert_eq!(previous_mark(&marks, 3_000), Some(2_000), "a mark is skipped, not repeated");
        assert_eq!(previous_mark(&marks, 2_000), Some(1_000));
        assert_eq!(previous_mark(&marks, 1_000), Some(3_000), "previous wraps to the last mark at the start");
        assert_eq!(previous_mark(&marks, 0), Some(3_000));
    }

    /// The mark the playhead sits on: the last one at or before the position,
    /// none before the first mark.
    #[test]
    fn the_mark_under_the_playhead_is_the_last_one_at_or_before_it() {
        let marks = [1_000, 2_000, 3_000];
        assert_eq!(mark_under(&marks, 0), None);
        assert_eq!(mark_under(&marks, 999), None);
        assert_eq!(mark_under(&marks, 1_000), Some(0));
        assert_eq!(mark_under(&marks, 2_500), Some(1));
        assert_eq!(mark_under(&marks, 3_000), Some(2));
        assert_eq!(mark_under(&marks, u64::MAX), Some(2));
        assert_eq!(mark_under(&[], 0), None);
    }

    /// The scrub position math: clamped to the track on both sides, linear in
    /// between, and 0 without a track or a duration.
    #[test]
    fn the_scrub_position_maps_to_milliseconds() {
        assert_eq!(scrub_ms(50.0, 100.0, 400.0, 10_000), 0, "before the track seeks to 0");
        assert_eq!(scrub_ms(100.0, 100.0, 400.0, 10_000), 0);
        assert_eq!(scrub_ms(300.0, 100.0, 400.0, 10_000), 5_000, "the middle of the track is half the length");
        assert_eq!(scrub_ms(500.0, 100.0, 400.0, 10_000), 10_000);
        assert_eq!(scrub_ms(900.0, 100.0, 400.0, 10_000), 10_000, "past the track seeks to the length");
        assert_eq!(scrub_ms(300.0, 100.0, 0.0, 10_000), 0, "no track is position 0");
        assert_eq!(scrub_ms(300.0, 100.0, 400.0, 0), 0, "no duration is position 0");
    }

    #[test]
    fn an_added_mark_is_written_to_the_sidecar_and_the_exports() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-add-mark")?;
        let project = root.join("Show");
        let folder = project.join("2026-11-21_09-00-00_take01");
        fs::create_dir_all(&folder)?;
        let stem = "2026-11-21_09-00-00_take01";
        fs::write(folder.join(format!("{stem}.mkv")), b"video")?;
        fs::write(folder.join(format!("{stem}.marks.json")), sidecar_json("2026-11-21T12:00:00.000Z", 60_000, 1))?;
        let days = scan(&project);
        let take = &days[0].takes[0];

        add_mark(take, 5_000)?;
        let (sidecar, _) = marks::load(&folder.join(format!("{stem}.marks.json")))?;
        assert_eq!(sidecar.marks.len(), 2, "the mark joined the sidecar's own marks");
        let added = sidecar.marks.last().expect("a mark was added");
        assert_eq!(added.mark_type, marks::MarkType::Note);
        assert_eq!(added.source, marks::Source::Ui);
        assert_eq!(added.t_ms, 5_000);
        assert_eq!(added.media, 0);
        assert!(
            sidecar.marks.windows(2).all(|pair| marks::session_ms(&sidecar, &pair[0]) <= marks::session_ms(&sidecar, &pair[1])),
            "the marks stay sorted by timeline position"
        );
        let md = fs::read_to_string(folder.join(format!("{stem}_marks.md")))?;
        assert!(md.contains("00:00:05.000"), "the markdown log lists the new mark: {md}");
        let csv = fs::read_to_string(folder.join(format!("{stem}_marks.csv")))?;
        assert!(csv.contains(",5000,"), "the csv log lists the new mark: {csv}");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn an_added_mark_creates_a_sidecar_when_the_take_has_none() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-add-mark-fresh")?;
        let project = root.join("Show");
        let folder = project.join("2026-11-21_09-00-00_take01");
        fs::create_dir_all(&folder)?;
        let stem = "2026-11-21_09-00-00_take01";
        fs::write(folder.join(format!("{stem}.mkv")), b"video")?;
        let days = scan(&project);
        let take = &days[0].takes[0];

        add_mark(take, 1_200)?;
        let (sidecar, _) = marks::load(&folder.join(format!("{stem}.marks.json")))?;
        assert_eq!(sidecar.marks.len(), 1);
        assert_eq!(sidecar.marks[0].mark_type, marks::MarkType::Note);
        assert_eq!(sidecar.marks[0].t_ms, 1_200);
        assert_eq!(sidecar.session.writer.app, "cutmark-recorder");
        assert_eq!(sidecar.media[0].path, format!("{stem}.mkv"), "the fresh sidecar names the take's main file");
        assert!(folder.join(format!("{stem}_marks.md")).exists(), "the markdown log was written");
        assert!(folder.join(format!("{stem}_marks.csv")).exists(), "the csv log was written");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_failed_rename_rolls_the_files_already_moved_back() -> Result<(), Box<dyn std::error::Error>> {
        let root = fresh_dir("review-rollback")?;
        let project = root.join("Show");
        let folder = project.join("2026-11-20_10-00-00_take01");
        fs::create_dir_all(&folder)?;
        let stem = "2026-11-20_10-00-00_take01";
        for suffix in [".mkv", ".iso", ".wav", ".marks.json"] {
            fs::write(folder.join(format!("{stem}{suffix}")), b"data")?;
        }
        fs::write(folder.join(format!("{stem}.marks.json")), sidecar_json_media("2026-11-20T12:00:00.000Z", 1_000, 0, &format!("{stem}.mkv")))?;
        // A directory where a later target would go makes that rename fail once
        // the earlier files have already moved (files sort .iso, .marks.json,
        // .mkv, .wav, so .wav is the blocked one).
        fs::create_dir_all(folder.join("new_name.wav"))?;
        let days = scan(&project);
        let result = rename_take(&days[0].takes[0], "new_name");
        assert!(result.is_err(), "the blocked rename must fail");
        let reason = result.err().unwrap_or_default();
        assert!(reason.contains("new_name.wav"), "the reason names the blocked file: {reason}");
        for suffix in [".mkv", ".iso", ".wav", ".marks.json"] {
            assert!(folder.join(format!("{stem}{suffix}")).exists(), "{suffix} was rolled back");
        }
        // The blocking directory stays; no file was left under the new name.
        assert!(folder.join("new_name.wav").is_dir(), "the blocking directory is untouched");
        for suffix in [".mkv", ".iso", ".marks.json"] {
            assert!(!folder.join(format!("new_name{suffix}")).exists(), "{suffix} was not left half-renamed");
        }
        fs::remove_dir_all(&root)?;
        Ok(())
    }
}
