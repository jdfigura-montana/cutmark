//! Reads a marks sidecar from disk, fixing or skipping bad marks and reporting each fix as a warning.

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::IntoDeserializer;
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::model::{
    AUTO_DATA_MAX_BYTES, AutoInfo, ClockInfo, ClockSource, Context, LABEL_MAX, MARKS_MAX, Mark, MarkType, MediaEntry, NOTES_MAX, SCHEMA_VERSION,
    SCRIPT_LINE_MAX, ScriptRef, Session, Sidecar, Source, WriterInfo,
};

/// The UTF-8 byte order mark that some Windows editors put at the start of a file.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Why a sidecar could not be loaded at all.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("Could not read the marks file: {0}")]
    Io(#[from] std::io::Error),
    /// The file is not valid JSON.
    #[error("The marks file is not valid JSON: {0}")]
    NotJson(#[from] serde_json::Error),
    /// The top level of the file is not a JSON object.
    #[error("The marks file does not contain a JSON object.")]
    NotAnObject,
    /// The session block is missing or invalid.
    #[error("The marks file has no valid session.")]
    MissingSession,
    /// The media list is missing, empty or has an invalid entry.
    #[error("The marks file has no valid media list.")]
    MissingMedia,
    /// The schema version is missing, not an integer, or below 1.
    #[error("The marks file has no valid schema version.")]
    InvalidSchemaVersion,
    /// The file was made by a newer schema than this reader understands.
    #[error("This marks file was made by a newer CutMark ({}). Update to open it.", writer_label(.writer_version))]
    NewerVersion {
        /// The schema version found in the file.
        found: u64,
        /// The version of the app that wrote the file, if the file says.
        writer_version: Option<String>,
    },
}

/// The writer version for display, or a stand-in when the file does not say.
fn writer_label(version: &Option<String>) -> &str {
    version.as_deref().unwrap_or("unknown version")
}

/// Something the reader fixed or skipped while loading a file that is otherwise usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A mark was skipped because a required field is missing or has the wrong type.
    MissingField {
        /// Zero-based position of the mark in the file's marks array.
        mark: usize,
        /// Name of the missing field.
        field: &'static str,
    },
    /// A mark was skipped because its media index does not exist.
    MediaOutOfRange {
        /// Zero-based position of the mark in the file's marks array.
        mark: usize,
        /// The media index the mark asked for.
        media: i64,
    },
    /// A mark was skipped because its `t_ms` is negative.
    NegativeTime {
        /// Zero-based position of the mark in the file's marks array.
        mark: usize,
    },
    /// A mark lost its `end_ms` because it is not a valid range end.
    BadRange {
        /// Zero-based position of the mark in the file's marks array.
        mark: usize,
    },
    /// A time was past the end of its media file and was clamped to the duration.
    Clamped {
        /// Zero-based position of the mark in the file's marks array.
        mark: usize,
        /// Name of the field that was clamped.
        field: &'static str,
    },
    /// A mark was skipped because an earlier mark has the same id.
    DuplicateId {
        /// The id that appears more than once.
        id: Uuid,
    },
    /// Some text or data was over its length limit and was cut down.
    Truncated {
        /// Zero-based position of the mark, or `None` for the session context.
        mark: Option<usize>,
        /// Name of the field that was truncated.
        field: &'static str,
    },
    /// A mark had a type this reader does not know and was read as a note.
    UnknownType {
        /// The type name as written in the file.
        value: String,
    },
    /// The file has more marks than the limit and the rest were dropped.
    TooManyMarks {
        /// How many marks were kept.
        kept: usize,
    },
    /// The file has no marks list and was read as having no marks.
    MarksMissing,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Warning::MissingField { mark, field } => {
                write!(f, "Skipped the mark at position {mark}: the required field {field} is missing or has the wrong type.")
            }
            Warning::MediaOutOfRange { mark, media } => {
                write!(f, "Skipped the mark at position {mark}: it points at media {media}, which does not exist.")
            }
            Warning::NegativeTime { mark } => write!(f, "Skipped the mark at position {mark}: its time is negative."),
            Warning::BadRange { mark } => {
                write!(f, "The mark at position {mark} has an end time that is not valid, so it was read as a single point.")
            }
            Warning::Clamped { mark, field } => {
                write!(f, "The mark at position {mark} has a {field} past the end of its media file, so it was set to the end.")
            }
            Warning::DuplicateId { id } => write!(f, "Skipped a mark because its id {id} was already used. The first one was kept."),
            Warning::Truncated { mark: Some(mark), field } => write!(f, "The {field} of the mark at position {mark} was too long and was shortened."),
            Warning::Truncated { mark: None, field } => write!(f, "The {field} was too long and was shortened."),
            Warning::UnknownType { value } => write!(f, "Unknown mark type \"{value}\" was read as a note."),
            Warning::TooManyMarks { kept } => write!(f, "The file has too many marks. Only the first {kept} were kept."),
            Warning::MarksMissing => write!(f, "The file has no marks list, so it was read as having no marks."),
        }
    }
}

/// Finds the sidecar for a media file, which sits beside it and shares its base name.
pub fn discover(media: &Path) -> Option<PathBuf> {
    let mut name = media.file_stem()?.to_os_string();
    name.push(".marks.json");
    let candidate = media.with_file_name(name);
    candidate.is_file().then_some(candidate)
}

/// Reads a sidecar without locking or writing it, returning the marks that survived and a warning for each problem.
pub fn load(sidecar: &Path) -> Result<(Sidecar, Vec<Warning>), LoadError> {
    let bytes = std::fs::read(sidecar)?;
    let json = bytes.strip_prefix(UTF8_BOM).unwrap_or(&bytes);
    let value: Value = serde_json::from_slice(json)?;
    let version = read_schema_version(&value)?;
    let value = migrate(value, version);
    build(&value)
}

/// Checks `schema_version` and refuses files from a newer schema.
fn read_schema_version(value: &Value) -> Result<u64, LoadError> {
    let root = value.as_object().ok_or(LoadError::NotAnObject)?;
    let version = root.get("schema_version").and_then(Value::as_u64).filter(|version| *version >= 1).ok_or(LoadError::InvalidSchemaVersion)?;
    if version > u64::from(SCHEMA_VERSION) {
        let writer_version = value.get("session").and_then(|session| session.get("writer")).and_then(|writer| writer.get("version")).and_then(Value::as_str);
        return Err(LoadError::NewerVersion { found: version, writer_version: writer_version.map(str::to_owned) });
    }
    Ok(version)
}

/// Upgrades an older file's value to the current shape, in memory only.
fn migrate(value: Value, from: u64) -> Value {
    match from {
        // Version 1 is the current shape, so there is nothing to change.
        1 => value,
        // Later versions add one arm each, upgrading the value by one step.
        _ => value,
    }
}

/// Builds the sidecar from a value of the current shape.
fn build(value: &Value) -> Result<(Sidecar, Vec<Warning>), LoadError> {
    let root = value.as_object().ok_or(LoadError::NotAnObject)?;
    let session = read_session(root.get("session")).ok_or(LoadError::MissingSession)?;
    let media = read_media(root.get("media")).ok_or(LoadError::MissingMedia)?;
    let mut warnings = Vec::new();
    let context = read_context(root.get("context"), &mut warnings);
    let mut marks = read_marks(root.get("marks"), &media, &mut warnings);
    marks.sort_by_key(|mark| (session_time(&media, mark), mark.id));
    Ok((Sidecar { schema_version: SCHEMA_VERSION, session, media, context, marks }, warnings))
}

/// Position of a mark on the whole session timeline, in milliseconds.
fn session_time(media: &[MediaEntry], mark: &Mark) -> u64 {
    media.get(mark.media).map_or(0, |entry| entry.offset_ms).saturating_add(mark.t_ms)
}

/// Reads the session block, or `None` if a required part is missing or invalid.
fn read_session(value: Option<&Value>) -> Option<Session> {
    let fields = value?.as_object()?;
    let writer = fields.get("writer")?.as_object()?;
    Some(Session {
        id: read_uuid(fields.get("id"))?,
        started_at: read_time(fields.get("started_at"))?,
        ended_at: read_time(fields.get("ended_at")),
        recovered: fields.get("recovered").and_then(Value::as_bool).unwrap_or(false),
        writer: WriterInfo { app: read_string(writer.get("app"))?, version: read_string(writer.get("version"))? },
        clock: read_clock(fields.get("clock")),
    })
}

/// Reads the clock block, or `None` if it is absent or unusable.
fn read_clock(value: Option<&Value>) -> Option<ClockInfo> {
    let fields = value?.as_object()?;
    let source = fields.get("source").and_then(Value::as_str).and_then(read_name::<ClockSource>)?;
    Some(ClockInfo { source, error_ms: fields.get("error_ms").and_then(Value::as_u64) })
}

/// Reads the media list, or `None` if it is missing, empty or has an invalid entry.
fn read_media(value: Option<&Value>) -> Option<Vec<MediaEntry>> {
    let entries = value?.as_array()?;
    if entries.is_empty() {
        return None;
    }
    entries
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let fields = entry.as_object()?;
            // Marks refer to media by array position, so the position is the index whatever the file says.
            Some(MediaEntry {
                index: position,
                path: read_string(fields.get("path"))?,
                offset_ms: fields.get("offset_ms")?.as_u64()?,
                duration_ms: fields.get("duration_ms")?.as_u64()?,
            })
        })
        .collect()
}

/// Reads the optional context block, truncating its notes if they are too long.
fn read_context(value: Option<&Value>, warnings: &mut Vec<Warning>) -> Option<Context> {
    let fields = value?.as_object()?;
    let notes = read_string(fields.get("notes")).map(|text| truncate(text, NOTES_MAX, None, "context.notes", warnings));
    Some(Context { game: read_string(fields.get("game")), title: read_string(fields.get("title")), notes })
}

/// Reads every mark, skipping bad ones, and keeps at most `MARKS_MAX` valid marks in file order.
fn read_marks(value: Option<&Value>, media: &[MediaEntry], warnings: &mut Vec<Warning>) -> Vec<Mark> {
    let Some(entries) = value.and_then(Value::as_array) else {
        warnings.push(Warning::MarksMissing);
        return Vec::new();
    };
    let mut reader = MarkReader { media, warnings, seen_ids: HashSet::new(), seen_unknown_types: HashSet::new() };
    let mut marks = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let Some(mark) = reader.read(position, entry) else {
            continue;
        };
        if marks.len() >= MARKS_MAX {
            reader.warn(Warning::TooManyMarks { kept: MARKS_MAX });
            break;
        }
        marks.push(mark);
    }
    marks
}

/// State shared while reading the marks of one file.
struct MarkReader<'a> {
    media: &'a [MediaEntry],
    warnings: &'a mut Vec<Warning>,
    seen_ids: HashSet<Uuid>,
    seen_unknown_types: HashSet<String>,
}

impl MarkReader<'_> {
    fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Passes a required value through, warning that the field is missing when it is not there.
    fn require<T>(&mut self, mark: usize, field: &'static str, value: Option<T>) -> Option<T> {
        if value.is_none() {
            self.warn(Warning::MissingField { mark, field });
        }
        value
    }

    /// Reads one mark, or returns `None` after warning if the mark has to be skipped.
    fn read(&mut self, position: usize, value: &Value) -> Option<Mark> {
        let Some(fields) = value.as_object() else {
            self.warn(Warning::MissingField { mark: position, field: "id" });
            return None;
        };

        // Required fields, reporting the first one that is missing or has the wrong type.
        let id = self.require(position, "id", read_uuid(fields.get("id")))?;
        let type_name = self.require(position, "type", fields.get("type").and_then(Value::as_str))?;
        let media_number = self.require(position, "media", read_integer(fields.get("media")))?;
        let time_number = self.require(position, "t_ms", read_integer(fields.get("t_ms")))?;
        let source_name = self.require(position, "source", fields.get("source").and_then(Value::as_str))?;
        let created_at = self.require(position, "created_at", read_time(fields.get("created_at")))?;

        // The media index has to exist, and the time cannot be negative.
        let found = usize::try_from(media_number).ok().and_then(|index| self.media.get(index).map(|entry| (index, entry.duration_ms)));
        let Some((media_index, duration_ms)) = found else {
            self.warn(Warning::MediaOutOfRange { mark: position, media: i64::try_from(media_number).unwrap_or(i64::MAX) });
            return None;
        };
        let Ok(mut t_ms) = u64::try_from(time_number) else {
            self.warn(Warning::NegativeTime { mark: position });
            return None;
        };

        // An unknown type becomes a note, with one warning per distinct name.
        let (mark_type, original_type) = match read_name::<MarkType>(type_name) {
            Some(known) => (known, None),
            None => {
                if self.seen_unknown_types.insert(type_name.to_owned()) {
                    self.warn(Warning::UnknownType { value: type_name.to_owned() });
                }
                (MarkType::Note, Some(type_name.to_owned()))
            }
        };

        // An unknown source becomes auto, quietly.
        let source = read_name::<Source>(source_name).unwrap_or(Source::Auto);

        // A range end has to come after the start, and only some types may have one.
        let mut end_ms = match fields.get("end_ms") {
            None | Some(Value::Null) => None,
            Some(raw) => match read_integer(Some(raw)).and_then(|end| u64::try_from(end).ok()) {
                Some(end) if end > t_ms && mark_type.allows_range() => Some(end),
                _ => {
                    self.warn(Warning::BadRange { mark: position });
                    None
                }
            },
        };

        // Times past the end of the media file are clamped to its duration.
        if t_ms > duration_ms {
            t_ms = duration_ms;
            self.warn(Warning::Clamped { mark: position, field: "t_ms" });
        }
        if end_ms.is_some_and(|end| end > duration_ms) {
            end_ms = Some(duration_ms);
            self.warn(Warning::Clamped { mark: position, field: "end_ms" });
        }
        // Clamping can leave a range with no length, which is the same problem as a bad range.
        if end_ms.is_some_and(|end| end <= t_ms) {
            end_ms = None;
            self.warn(Warning::BadRange { mark: position });
        }

        // The first mark with an id wins.
        if !self.seen_ids.insert(id) {
            self.warn(Warning::DuplicateId { id });
            return None;
        }

        let label = self.read_text(position, fields.get("label"), LABEL_MAX, "label");
        let notes = self.read_text(position, fields.get("notes"), NOTES_MAX, "notes");
        let script = self.read_script(position, fields.get("script"));
        let auto = self.read_auto(position, fields.get("auto"));

        Some(Mark { id, mark_type, media: media_index, t_ms, end_ms, label, notes, source, created_at, script, auto, original_type })
    }

    /// Reads an optional text field, cutting it to `max` characters.
    fn read_text(&mut self, mark: usize, value: Option<&Value>, max: usize, field: &'static str) -> Option<String> {
        read_string(value).map(|text| truncate(text, max, Some(mark), field, self.warnings))
    }

    /// Reads the optional script reference, dropping it if it is not usable.
    fn read_script(&mut self, mark: usize, value: Option<&Value>) -> Option<ScriptRef> {
        let fields = value?.as_object()?;
        let line = read_string(fields.get("line"))?;
        let position = fields.get("position")?.as_u64()?;
        Some(ScriptRef { line: truncate(line, SCRIPT_LINE_MAX, Some(mark), "script.line", self.warnings), position })
    }

    /// Reads the optional auto block, emptying `data` if it is too big and dropping the block if it has no kind.
    fn read_auto(&mut self, mark: usize, value: Option<&Value>) -> Option<AutoInfo> {
        let fields = value?.as_object()?;
        let kind = read_string(fields.get("kind"))?;
        let mut data = match fields.get("data") {
            None | Some(Value::Null) => empty_object(),
            Some(data) => data.clone(),
        };
        let size = serde_json::to_string(&data).map_or(usize::MAX, |text| text.len());
        if size > AUTO_DATA_MAX_BYTES {
            data = empty_object();
            self.warn(Warning::Truncated { mark: Some(mark), field: "auto.data" });
        }
        Some(AutoInfo { kind, data })
    }
}

/// An empty JSON object.
fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// Cuts `text` to `max` characters, warning when it was cut.
fn truncate(mut text: String, max: usize, mark: Option<usize>, field: &'static str, warnings: &mut Vec<Warning>) -> String {
    if let Some((end, _)) = text.char_indices().nth(max) {
        text.truncate(end);
        warnings.push(Warning::Truncated { mark, field });
    }
    text
}

/// Reads a string value.
fn read_string(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

/// Reads a UUID written as a string.
fn read_uuid(value: Option<&Value>) -> Option<Uuid> {
    Uuid::parse_str(value?.as_str()?).ok()
}

/// Reads an ISO 8601 timestamp written as a string.
fn read_time(value: Option<&Value>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?.as_str()?).ok().map(|time| time.with_timezone(&Utc))
}

/// Reads a whole number, negative or not, without losing range.
fn read_integer(value: Option<&Value>) -> Option<i128> {
    match value? {
        Value::Number(number) => number.as_u64().map(i128::from).or_else(|| number.as_i64().map(i128::from)),
        _ => None,
    }
}

/// Reads a snake_case enum name with the model's own serde rules, or `None` if the name is unknown.
fn read_name<'a, T: Deserialize<'a>>(name: &'a str) -> Option<T> {
    let deserializer: serde::de::value::StrDeserializer<'a, serde::de::value::Error> = name.into_deserializer();
    T::deserialize(deserializer).ok()
}
