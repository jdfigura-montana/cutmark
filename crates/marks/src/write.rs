//! Writes a marks sidecar: every mark goes to a journal first, then one atomic finish builds the file.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::model::{
    AUTO_DATA_MAX_BYTES, AutoInfo, LABEL_MAX, MARKS_MAX, Mark, MarkType, MediaEntry, NOTES_MAX, SCHEMA_VERSION, SCRIPT_LINE_MAX, ScriptRef, Session, Sidecar,
    Source, WriterInfo,
};
use crate::read;

/// A repeat of the same hotkey closer than this many milliseconds is not a second mark.
const KEY_REPEAT_MS: i64 = 250;
/// The journal format version written in the header line.
const JOURNAL_VERSION: u32 = 1;

/// Supplies the current time and fresh ids, so tests can fix both.
pub trait Clock: Send {
    /// Returns the current UTC time.
    fn now(&self) -> DateTime<Utc>;
    /// Returns a new unique id.
    fn new_id(&self) -> Uuid;
}

/// The real clock: system time and UUIDv7 ids.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        // chrono's `clock` feature is off in this crate, so `Utc::now` is not available; read the system time directly.
        let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let secs = i64::try_from(since_epoch.as_secs()).unwrap_or(i64::MAX);
        DateTime::from_timestamp(secs, since_epoch.subsec_nanos()).unwrap_or_default()
    }

    fn new_id(&self) -> Uuid {
        Uuid::now_v7()
    }
}

/// A mark as the app describes it, before the writer gives it an id and a creation time.
#[derive(Debug, Clone, PartialEq)]
pub struct NewMark {
    /// What kind of mark this is.
    pub mark_type: MarkType,
    /// Index into the media list, or `None` for the newest media entry.
    pub media: Option<usize>,
    /// Milliseconds from the start of that media file.
    pub t_ms: u64,
    /// End of the range in the same clock as `t_ms`; `None` means a point.
    pub end_ms: Option<u64>,
    /// Short title, also used as the chapter name.
    pub label: Option<String>,
    /// Longer free text.
    pub notes: Option<String>,
    /// How the mark was made.
    pub source: Source,
    /// The script line being read when the mark was made.
    pub script: Option<ScriptRef>,
    /// Details for automatically written marks.
    pub auto: Option<AutoInfo>,
}

impl NewMark {
    /// Builds a point mark on the newest media entry, with no text, script or auto details.
    pub fn point(mark_type: MarkType, t_ms: u64, source: Source) -> Self {
        Self { mark_type, media: None, t_ms, end_ms: None, label: None, notes: None, source, script: None, auto: None }
    }
}

/// Why a session could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// A file could not be created, written, renamed or deleted.
    #[error("Could not write the marks file: {0}")]
    Io(#[from] io::Error),
    /// A journal for this recording already exists, so another session may be using it or needs recovery.
    #[error("A marks journal already exists at {}.", .0.display())]
    JournalExists(PathBuf),
    /// A finished marks file already exists for this recording.
    #[error("A marks file already exists at {}.", .0.display())]
    SidecarExists(PathBuf),
    /// A mark asked for a media entry that does not exist.
    #[error("Media entry {0} does not exist.")]
    BadMedia(usize),
    /// The number of durations does not match the number of media entries.
    #[error("Expected {expected} media durations but got {got}.")]
    DurationsMismatch {
        /// How many media entries the session has.
        expected: usize,
        /// How many durations were given.
        got: usize,
    },
    /// The session already holds the largest number of marks a file may have.
    #[error("The session already has the largest number of marks a file can hold.")]
    TooManyMarks,
    /// The marks data could not be turned into JSON.
    #[error("Could not encode the marks data: {0}")]
    Serialize(#[from] serde_json::Error),
    /// An input or the finished file failed its checks; the text says what was wrong.
    #[error("{0}")]
    Invalid(String),
}

/// The first journal line: format version, session and the media known at the start.
#[derive(Serialize)]
struct JournalHeader<'a> {
    journal: u32,
    session: &'a Session,
    media: &'a [MediaEntry],
}

/// A journal line that adds a media entry.
#[derive(Serialize)]
struct MediaLine<'a> {
    media: &'a MediaEntry,
}

/// A journal line that adds a mark.
#[derive(Serialize)]
struct MarkLine<'a> {
    mark: &'a Mark,
}

/// The first journal line as read back during recovery.
#[derive(Deserialize)]
struct JournalHeaderIn {
    journal: u32,
    session: Session,
    media: Vec<MediaEntry>,
}

/// A later journal line as read back during recovery: it holds a media entry, a mark, or neither.
#[derive(Deserialize)]
struct JournalLineIn {
    #[serde(default)]
    media: Option<MediaEntry>,
    #[serde(default)]
    mark: Option<Mark>,
}

/// Records one recording session: marks go to a journal as they are made, and `finish` writes the sidecar.
pub struct SessionWriter {
    dir: PathBuf,
    base_name: String,
    journal_path: PathBuf,
    journal: File,
    session: Session,
    media: Vec<MediaEntry>,
    marks: Vec<Mark>,
    last_hotkey: Option<(MarkType, DateTime<Utc>)>,
    clock: Box<dyn Clock>,
}

impl SessionWriter {
    /// Starts a session in `dir`, creating `{base_name}.marks.journal` with its header line.
    pub fn begin(dir: &Path, base_name: &str, writer: WriterInfo, first_media: &Path, clock: Box<dyn Clock>) -> Result<Self, WriteError> {
        if base_name.is_empty() || base_name.contains(['/', '\\']) {
            return Err(WriteError::Invalid("The base name must be a file name without folders.".to_string()));
        }
        let journal_path = dir.join(format!("{base_name}.marks.journal"));
        let sidecar_path = dir.join(format!("{base_name}.marks.json"));
        if journal_path.exists() {
            return Err(WriteError::JournalExists(journal_path));
        }
        if sidecar_path.exists() {
            return Err(WriteError::SidecarExists(sidecar_path));
        }
        let first = MediaEntry { index: 0, path: relative_media_path(dir, first_media)?, offset_ms: 0, duration_ms: 0 };
        let mut journal = match OpenOptions::new().write(true).create_new(true).open(&journal_path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Err(WriteError::JournalExists(journal_path)),
            Err(error) => return Err(error.into()),
        };
        let media = vec![first];
        let session = Session { id: clock.new_id(), started_at: clock.now(), ended_at: None, recovered: false, writer, clock: None };
        let header = JournalHeader { journal: JOURNAL_VERSION, session: &session, media: &media };
        if let Err(error) = write_line(&mut journal, &header) {
            drop(journal);
            let _ = fs::remove_file(&journal_path);
            return Err(error);
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            base_name: base_name.to_string(),
            journal_path,
            journal,
            session,
            media,
            marks: Vec::new(),
            last_hotkey: None,
            clock,
        })
    }

    /// Adds a media file that starts `offset_ms` into the session, as after a file rollover, and returns its index.
    pub fn add_media(&mut self, path: &Path, offset_ms: u64) -> Result<usize, WriteError> {
        let index = self.media.len();
        let entry = MediaEntry { index, path: relative_media_path(&self.dir, path)?, offset_ms, duration_ms: 0 };
        write_line(&mut self.journal, &MediaLine { media: &entry })?;
        self.media.push(entry);
        Ok(index)
    }

    /// Records a mark and syncs it to the journal; returns its id, or `None` for a repeated hotkey press.
    pub fn add_mark(&mut self, mark: NewMark) -> Result<Option<Uuid>, WriteError> {
        let media = mark.media.unwrap_or_else(|| self.media.len().saturating_sub(1));
        if media >= self.media.len() {
            return Err(WriteError::BadMedia(media));
        }
        let source = if mark.mark_type == MarkType::System { Source::Auto } else { mark.source };
        let now = self.clock.now();
        if source == Source::Hotkey && self.is_key_repeat(mark.mark_type, now) {
            return Ok(None);
        }
        if self.marks.len() >= MARKS_MAX {
            return Err(WriteError::TooManyMarks);
        }
        let end_ms = mark.end_ms.filter(|end| mark.mark_type.allows_range() && *end > mark.t_ms);
        let record = Mark {
            id: self.clock.new_id(),
            mark_type: mark.mark_type,
            media,
            t_ms: mark.t_ms,
            end_ms,
            label: mark.label.map(|text| truncate_chars(text, LABEL_MAX)),
            notes: mark.notes.map(|text| truncate_chars(text, NOTES_MAX)),
            source,
            created_at: now,
            script: mark.script.map(|script| ScriptRef { line: truncate_chars(script.line, SCRIPT_LINE_MAX), position: script.position }),
            auto: mark.auto.map(limit_auto_data),
            original_type: None,
        };
        write_line(&mut self.journal, &MarkLine { mark: &record })?;
        if source == Source::Hotkey {
            self.last_hotkey = Some((record.mark_type, now));
        }
        let id = record.id;
        self.marks.push(record);
        Ok(Some(id))
    }

    /// Writes `{base}.marks.json` atomically from the session so far and deletes the journal.
    pub fn finish(self, durations_ms: &[u64]) -> Result<PathBuf, WriteError> {
        if durations_ms.len() != self.media.len() {
            return Err(WriteError::DurationsMismatch { expected: self.media.len(), got: durations_ms.len() });
        }
        let Self { dir, base_name, journal_path, journal, mut session, mut media, marks, clock, .. } = self;
        drop(journal);

        for (entry, duration) in media.iter_mut().zip(durations_ms) {
            entry.duration_ms = *duration;
        }
        session.ended_at = Some(clock.now());
        publish_sidecar(&dir, &base_name, &journal_path, session, media, marks)
    }

    /// Rebuilds a sidecar from a journal a crashed session left behind; returns its path, or `None` if there is no journal.
    ///
    /// A journal next to a finished sidecar (a crash after the rename) is just deleted and the sidecar is left alone.
    /// Otherwise every complete journal line is applied and any line that does not parse is skipped. The result is marked
    /// `recovered` with no `ended_at`, and each media duration is the latest mark time on it, since the real length is unknown.
    /// On any failure the journal stays in place.
    pub fn recover(dir: &Path, base_name: &str) -> Result<Option<PathBuf>, WriteError> {
        let journal_path = dir.join(format!("{base_name}.marks.journal"));
        let sidecar_path = dir.join(format!("{base_name}.marks.json"));
        if !journal_path.exists() {
            return Ok(None);
        }
        if sidecar_path.exists() {
            fs::remove_file(&journal_path)?;
            return Ok(Some(sidecar_path));
        }

        let bytes = fs::read(&journal_path)?;
        let (mut session, mut media, marks) = replay_journal(&bytes)?;
        session.recovered = true;
        session.ended_at = None;
        for (position, entry) in media.iter_mut().enumerate() {
            entry.duration_ms = marks.iter().filter(|mark| mark.media == position).map(|mark| mark.t_ms.max(mark.end_ms.unwrap_or(0))).max().unwrap_or(0);
        }
        publish_sidecar(dir, base_name, &journal_path, session, media, marks).map(Some)
    }

    /// Returns true if the last hotkey mark had this type and was made less than the repeat window ago.
    fn is_key_repeat(&self, mark_type: MarkType, now: DateTime<Utc>) -> bool {
        match self.last_hotkey {
            Some((last_type, last_at)) if last_type == mark_type => {
                let gap = now.signed_duration_since(last_at);
                gap >= Duration::zero() && gap < Duration::milliseconds(KEY_REPEAT_MS)
            }
            _ => false,
        }
    }
}

/// Returns the base names of the `*.marks.journal` files directly in `dir`, sorted.
pub fn find_journals(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        if let Some(base) = file_name.to_str().and_then(|name| name.strip_suffix(".marks.journal")).filter(|base| !base.is_empty()) {
            names.push(base.to_string());
        }
    }
    names.sort();
    Ok(names)
}

/// Reads a journal: the header line gives the session and first media, then each media and mark line is applied in order.
///
/// Works on bytes because a crash can cut the last line inside a multi-byte character.
fn replay_journal(bytes: &[u8]) -> Result<(Session, Vec<MediaEntry>, Vec<Mark>), WriteError> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let header = lines
        .next()
        .and_then(|line| serde_json::from_slice::<JournalHeaderIn>(line).ok())
        .filter(|header| header.journal == JOURNAL_VERSION)
        .ok_or_else(|| WriteError::Invalid("The marks journal has no readable first line.".to_string()))?;

    let mut media = header.media;
    let mut marks = Vec::new();
    for line in lines {
        let Ok(parsed) = serde_json::from_slice::<JournalLineIn>(line) else { continue };
        media.extend(parsed.media);
        marks.extend(parsed.mark);
    }
    Ok((header.session, media, marks))
}

/// Clamps and sorts the marks, then writes `{base}.marks.json` atomically (temp file, sync, check, rename) and deletes the journal.
///
/// On failure the journal is kept and the temp file is removed.
fn publish_sidecar(
    dir: &Path,
    base_name: &str,
    journal_path: &Path,
    session: Session,
    media: Vec<MediaEntry>,
    mut marks: Vec<Mark>,
) -> Result<PathBuf, WriteError> {
    clamp_marks(&mut marks, &media);
    marks.sort_by(|a, b| session_time(&media, a).cmp(&session_time(&media, b)).then_with(|| a.id.cmp(&b.id)));

    let sidecar = Sidecar { schema_version: SCHEMA_VERSION, session, media, context: None, marks };
    let mut text = serde_json::to_string_pretty(&sidecar)?;
    text.push('\n');

    let final_path = dir.join(format!("{base_name}.marks.json"));
    let tmp_path = dir.join(format!("{base_name}.marks.json.tmp"));
    let placed = write_and_validate(&tmp_path, text.as_bytes()).and_then(|()| fs::rename(&tmp_path, &final_path).map_err(WriteError::from));
    if let Err(error) = placed {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }
    fs::remove_file(journal_path)?;
    Ok(final_path)
}

/// Appends one compact JSON line to the journal, then flushes and syncs it to disk.
fn write_line<T: Serialize>(journal: &mut File, value: &T) -> Result<(), WriteError> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    journal.write_all(line.as_bytes())?;
    journal.flush()?;
    journal.sync_data()?;
    Ok(())
}

/// Turns a media path into the form stored in the file: relative to `dir`, with forward slashes.
fn relative_media_path(dir: &Path, media: &Path) -> Result<String, WriteError> {
    let relative = media.strip_prefix(dir).unwrap_or(media);
    let mut parts: Vec<&str> = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str().ok_or_else(|| WriteError::Invalid("The media path is not valid text.".to_string()))?),
            Component::ParentDir => parts.push(".."),
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(WriteError::Invalid("The media file must be inside the marks file's folder.".to_string()));
            }
        }
    }
    if parts.is_empty() {
        return Err(WriteError::Invalid("The media path is empty.".to_string()));
    }
    Ok(parts.join("/"))
}

/// Cuts text to at most `max` characters, never inside a character.
fn truncate_chars(mut text: String, max: usize) -> String {
    if let Some((end, _)) = text.char_indices().nth(max) {
        text.truncate(end);
    }
    text
}

/// Replaces `auto.data` with an empty object if its serialized form is over the byte limit.
fn limit_auto_data(auto: AutoInfo) -> AutoInfo {
    let fits = serde_json::to_string(&auto.data).is_ok_and(|text| text.len() <= AUTO_DATA_MAX_BYTES);
    if fits { auto } else { AutoInfo { kind: auto.kind, data: Value::Object(Map::new()) } }
}

/// Where a mark falls on the session timeline, in milliseconds.
fn session_time(media: &[MediaEntry], mark: &Mark) -> u64 {
    media.get(mark.media).map_or(mark.t_ms, |entry| entry.offset_ms.saturating_add(mark.t_ms))
}

/// Clamps each mark's times to its file's duration, dropping an end that is no longer after the start.
fn clamp_marks(marks: &mut [Mark], media: &[MediaEntry]) {
    for mark in marks {
        let Some(entry) = media.get(mark.media) else { continue };
        let t_ms = mark.t_ms.min(entry.duration_ms);
        mark.t_ms = t_ms;
        mark.end_ms = mark.end_ms.map(|end| end.min(entry.duration_ms)).filter(|end| *end > t_ms);
    }
}

/// Writes the temp file, syncs it, and loads it back; any warning or error means the file is not good enough to keep.
fn write_and_validate(tmp: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    let mut file = File::create(tmp)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);

    let (_, warnings) = read::load(tmp).map_err(|error| WriteError::Invalid(format!("The new marks file failed its check: {error}")))?;
    if warnings.is_empty() {
        return Ok(());
    }
    let details: Vec<String> = warnings.iter().map(ToString::to_string).collect();
    Err(WriteError::Invalid(format!("The new marks file failed its check: {}", details.join(" "))))
}
