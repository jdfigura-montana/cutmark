//! Plain data types for the marks sidecar. Field order is the on-disk key order.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The schema version this crate writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Maximum characters in a mark label.
pub const LABEL_MAX: usize = 200;
/// Maximum characters in mark or context notes.
pub const NOTES_MAX: usize = 4000;
/// Maximum characters in a script line.
pub const SCRIPT_LINE_MAX: usize = 500;
/// Maximum serialized size of `auto.data` in bytes.
pub const AUTO_DATA_MAX_BYTES: usize = 1024;
/// Maximum number of marks in one sidecar.
pub const MARKS_MAX: usize = 100_000;

/// Serde helpers: UTC ISO 8601 with exactly three millisecond digits and a `Z`.
mod timestamp {
    use chrono::{DateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    const FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

    fn render(value: &DateTime<Utc>) -> String {
        value.format(FORMAT).to_string()
    }

    fn parse(text: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(text).map(|parsed| parsed.with_timezone(&Utc))
    }

    pub fn serialize<S: Serializer>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&render(value))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse(&text).map_err(serde::de::Error::custom)
    }

    /// The same format for `Option<DateTime<Utc>>`, with `None` as JSON null.
    pub mod option {
        use chrono::{DateTime, Utc};
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(value: &Option<DateTime<Utc>>, serializer: S) -> Result<S::Ok, S::Error> {
            match value {
                Some(inner) => serializer.serialize_str(&super::render(inner)),
                None => serializer.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error> {
            Option::<String>::deserialize(deserializer)?.map(|text| super::parse(&text)).transpose().map_err(serde::de::Error::custom)
        }
    }
}

/// An empty JSON object, the default for `AutoInfo::data`.
fn empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// The whole sidecar file: one recording session and its marks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Schema version of this file.
    pub schema_version: u32,
    /// Who wrote the file, and when.
    pub session: Session,
    /// One entry per media file in the session.
    pub media: Vec<MediaEntry>,
    /// Optional free-text hints such as game and title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
    /// The marks, sorted by timeline position.
    pub marks: Vec<Mark>,
}

/// Identity and timing of the recording session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    /// Identifies the session across all its files.
    pub id: Uuid,
    /// UTC wall clock when recording began.
    #[serde(with = "timestamp")]
    pub started_at: DateTime<Utc>,
    /// UTC wall clock when recording ended, null for a crash recovery.
    #[serde(with = "timestamp::option", default)]
    pub ended_at: Option<DateTime<Utc>>,
    /// True if the file was rebuilt from the journal after an unclean stop.
    pub recovered: bool,
    /// The app that wrote the file.
    pub writer: WriterInfo,
    /// How marks were timed; missing means capture with zero error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<ClockInfo>,
}

/// The app that wrote the sidecar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterInfo {
    /// App name, for example `cutmark-recorder`.
    pub app: String,
    /// App version string.
    pub version: String,
}

/// How mark times were measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockInfo {
    /// Where mark times came from.
    pub source: ClockSource,
    /// Estimated timing error in milliseconds, null when unknown.
    pub error_ms: Option<u64>,
}

/// The clock that mark times were taken from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockSource {
    /// Times come from the capture pipeline.
    Capture,
    /// Times come from OBS output timing.
    ObsOutput,
}

/// One media file on the session timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEntry {
    /// Zero-based position in the media array.
    pub index: usize,
    /// Path relative to the sidecar's folder, with forward slashes.
    pub path: String,
    /// Where this file starts on the session timeline, in milliseconds.
    pub offset_ms: u64,
    /// Length of this file as recorded, in milliseconds.
    pub duration_ms: u64,
}

/// Optional free-text hints about the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// Game being played.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// Working title of the recording.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Free-form notes about the session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A single mark on the timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// Unique within the file.
    pub id: Uuid,
    /// What kind of mark this is.
    #[serde(rename = "type")]
    pub mark_type: MarkType,
    /// Index into the media array.
    pub media: usize,
    /// Milliseconds from the start of that media file.
    pub t_ms: u64,
    /// End of the range in the same clock as `t_ms`; absent means a point.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    /// Short title, also used as the chapter name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Longer free text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// How the mark was made.
    pub source: Source,
    /// UTC wall clock when the mark was made.
    #[serde(with = "timestamp")]
    pub created_at: DateTime<Utc>,
    /// The script line being read when the mark was made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptRef>,
    /// Details for automatically written marks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto: Option<AutoInfo>,
    /// The unknown type name the reader replaced with `note`, never serialized.
    #[serde(skip)]
    pub original_type: Option<String>,
}

/// The kind of a mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkType {
    /// Something worth keeping.
    Highlight,
    /// Remove this span, or flag a spot for review.
    Cut,
    /// A failed take that was redone.
    Retake,
    /// A section start, point only.
    Chapter,
    /// A reminder for the editor.
    Note,
    /// A place to cover with other footage.
    BRoll,
    /// Written by the app itself, point only.
    System,
}

impl MarkType {
    /// Returns true if marks of this type may carry an `end_ms`.
    pub fn allows_range(&self) -> bool {
        !matches!(self, MarkType::Chapter | MarkType::System)
    }
}

/// How a mark was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A global hotkey.
    Hotkey,
    /// The app's own interface.
    Ui,
    /// The local control API.
    Api,
    /// A remote guest.
    Guest,
    /// Written automatically by the app.
    Auto,
}

/// The script line being read when a mark was made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptRef {
    /// Text of the script line.
    pub line: String,
    /// Position of the line within the script.
    pub position: u64,
}

/// Details attached to automatically written marks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoInfo {
    /// Dotted lowercase namespace owned by the writer, such as `recorder.pause`.
    pub kind: String,
    /// Opaque data owned by the writer; defaults to an empty object.
    #[serde(default = "empty_object")]
    pub data: serde_json::Value,
}
