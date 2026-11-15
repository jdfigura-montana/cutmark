//! Cutmark marks sidecar: the only code that reads or writes `*.marks.json`.
//!
//! This is the S0 skeleton. The schema, reader, writer and exports arrive in
//! Marks Sidecar phases 1 to 3 (see docs/suite).

pub mod model;

pub use model::{
    AUTO_DATA_MAX_BYTES, AutoInfo, ClockInfo, ClockSource, Context, LABEL_MAX, MARKS_MAX, Mark, MarkType, MediaEntry, NOTES_MAX, SCHEMA_VERSION,
    SCRIPT_LINE_MAX, ScriptRef, Session, Sidecar, Source, WriterInfo,
};

#[cfg(test)]
mod tests {
    #[test]
    fn dependencies_link() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(id.get_version_num(), 7);
        let when = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(0).unwrap();
        let json = serde_json::to_string(&(id, when)).unwrap();
        let back: (uuid::Uuid, chrono::DateTime<chrono::Utc>) = serde_json::from_str(&json).unwrap();
        assert_eq!(back, (id, when));
    }
}
