//! Tests for the marks reader: discovery, loading, validation and warnings.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use marks::{AUTO_DATA_MAX_BYTES, LABEL_MAX, LoadError, MARKS_MAX, Mark, MarkType, NOTES_MAX, SCRIPT_LINE_MAX, Sidecar, Source, Warning, discover, load};
use serde_json::{Value, json};
use uuid::Uuid;

const EXAMPLE: &str = include_str!("fixtures/example.marks.json");

const EXAMPLE_ID: &str = "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f91";
const SECOND_ID: &str = "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f92";
const BAD_ID: &str = "0196f3a2-9d44-7e02-8c15-6a0b3e7d2fa0";
/// The example's only media file lasts this long.
const DURATION_MS: u64 = 3_698_784;

fn example() -> Value {
    serde_json::from_str(EXAMPLE).unwrap()
}

fn mark_with(id: &str, t_ms: u64) -> Value {
    let mut mark = example()["marks"][0].clone();
    mark["id"] = json!(id);
    mark["t_ms"] = json!(t_ms);
    mark
}

fn with_marks(marks: Vec<Value>) -> Value {
    let mut doc = example();
    doc["marks"] = Value::Array(marks);
    doc
}

fn load_bytes(bytes: &[u8]) -> Result<(Sidecar, Vec<Warning>), LoadError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.marks.json");
    std::fs::write(&path, bytes).unwrap();
    load(&path)
}

fn load_value(doc: &Value) -> Result<(Sidecar, Vec<Warning>), LoadError> {
    load_bytes(&serde_json::to_vec_pretty(doc).unwrap())
}

fn load_ok(doc: &Value) -> (Sidecar, Vec<Warning>) {
    load_value(doc).unwrap()
}

fn find<'a>(sidecar: &'a Sidecar, id: &str) -> &'a Mark {
    let id = Uuid::parse_str(id).unwrap();
    sidecar.marks.iter().find(|mark| mark.id == id).unwrap()
}

/// Loads a file holding `bad` and then a valid second mark, and checks that only `bad` was skipped, with one warning.
fn assert_skipped(bad: Value, expected: Warning) {
    let (sidecar, warnings) = load_ok(&with_marks(vec![bad, mark_with(SECOND_ID, 2000)]));
    assert_eq!(warnings, vec![expected]);
    assert_eq!(sidecar.marks.len(), 1);
    assert_eq!(sidecar.marks[0].id, Uuid::parse_str(SECOND_ID).unwrap());
}

#[test]
fn unmodified_example_loads_with_no_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("example.marks.json");
    std::fs::write(&path, EXAMPLE).unwrap();
    let (sidecar, warnings) = load(&path).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sidecar.schema_version, 1);
    assert_eq!(sidecar.session.writer.app, "cutmark-recorder");
    assert_eq!(sidecar.media.len(), 1);
    assert_eq!(sidecar.media[0].duration_ms, DURATION_MS);
    assert_eq!(sidecar.context.as_ref().and_then(|context| context.game.as_deref()), Some("Valorant"));
    assert_eq!(sidecar.marks.len(), 1);
    let mark = &sidecar.marks[0];
    assert_eq!(mark.id, Uuid::parse_str(EXAMPLE_ID).unwrap());
    assert_eq!(mark.mark_type, MarkType::Highlight);
    assert_eq!(mark.source, Source::Hotkey);
    assert_eq!(mark.t_ms, 754_300);
    assert_eq!(mark.label.as_deref(), Some("Ace"));
    assert_eq!(mark.original_type, None);
    // Loading never writes the file.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXAMPLE);
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let result = load(&dir.path().join("nothing.marks.json"));
    assert!(matches!(result, Err(LoadError::Io(_))));
}

#[test]
fn missing_required_field_skips_the_mark_and_warns() {
    for field in ["id", "type", "media", "t_ms", "source", "created_at"] {
        let mut bad = mark_with(BAD_ID, 1000);
        bad.as_object_mut().unwrap().remove(field);
        assert_skipped(bad, Warning::MissingField { mark: 0, field });
    }
}

#[test]
fn wrong_type_in_a_required_field_skips_the_mark_and_warns() {
    let cases = [
        ("id", json!("not-a-uuid")),
        ("type", json!(7)),
        ("media", json!("0")),
        ("t_ms", json!(1.5)),
        ("source", json!(null)),
        ("created_at", json!("yesterday")),
    ];
    for (field, value) in cases {
        let mut bad = mark_with(BAD_ID, 1000);
        bad[field] = value;
        assert_skipped(bad, Warning::MissingField { mark: 0, field });
    }
}

#[test]
fn marks_that_are_not_objects_are_skipped_without_panic() {
    let doc = with_marks(vec![json!(42), json!(null), json!("x"), json!([]), mark_with(SECOND_ID, 2000)]);
    let (sidecar, warnings) = load_ok(&doc);
    let expected: Vec<Warning> = (0..4).map(|mark| Warning::MissingField { mark, field: "id" }).collect();
    assert_eq!(warnings, expected);
    assert_eq!(sidecar.marks.len(), 1);
}

#[test]
fn media_index_out_of_range_skips_the_mark_and_warns() {
    let mut bad = mark_with(BAD_ID, 1000);
    bad["media"] = json!(5);
    assert_skipped(bad, Warning::MediaOutOfRange { mark: 0, media: 5 });

    let mut negative = mark_with(BAD_ID, 1000);
    negative["media"] = json!(-1);
    assert_skipped(negative, Warning::MediaOutOfRange { mark: 0, media: -1 });
}

#[test]
fn negative_time_skips_the_mark_and_warns() {
    let mut bad = mark_with(BAD_ID, 1000);
    bad["t_ms"] = json!(-5);
    assert_skipped(bad, Warning::NegativeTime { mark: 0 });
}

#[test]
fn end_ms_not_after_t_ms_is_dropped_with_a_warning() {
    for end in [1000, 999] {
        let mut bad = mark_with(BAD_ID, 1000);
        bad["end_ms"] = json!(end);
        let (sidecar, warnings) = load_ok(&with_marks(vec![bad, mark_with(SECOND_ID, 2000)]));
        assert_eq!(warnings, vec![Warning::BadRange { mark: 0 }]);
        assert_eq!(sidecar.marks.len(), 2);
        assert_eq!(find(&sidecar, BAD_ID).end_ms, None);
        assert_eq!(find(&sidecar, BAD_ID).t_ms, 1000);
    }
}

#[test]
fn end_ms_on_a_chapter_or_system_mark_is_dropped_with_a_warning() {
    for mark_type in ["chapter", "system"] {
        let mut bad = mark_with(BAD_ID, 1000);
        bad["type"] = json!(mark_type);
        bad["end_ms"] = json!(5000);
        let (sidecar, warnings) = load_ok(&with_marks(vec![bad, mark_with(SECOND_ID, 2000)]));
        assert_eq!(warnings, vec![Warning::BadRange { mark: 0 }]);
        assert_eq!(sidecar.marks.len(), 2);
        assert_eq!(find(&sidecar, BAD_ID).end_ms, None);
    }
}

#[test]
fn a_valid_range_is_kept() {
    let mut ranged = mark_with(BAD_ID, 1000);
    ranged["type"] = json!("cut");
    ranged["end_ms"] = json!(5000);
    let (sidecar, warnings) = load_ok(&with_marks(vec![ranged]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sidecar.marks[0].end_ms, Some(5000));
}

#[test]
fn t_ms_past_the_duration_is_clamped_with_a_warning() {
    let mut bad = mark_with(BAD_ID, DURATION_MS + 1000);
    bad["media"] = json!(0);
    let (sidecar, warnings) = load_ok(&with_marks(vec![bad, mark_with(SECOND_ID, 2000)]));
    assert_eq!(warnings, vec![Warning::Clamped { mark: 0, field: "t_ms" }]);
    assert_eq!(sidecar.marks.len(), 2);
    assert_eq!(find(&sidecar, BAD_ID).t_ms, DURATION_MS);
}

#[test]
fn end_ms_past_the_duration_is_clamped_with_a_warning() {
    let mut bad = mark_with(BAD_ID, 1000);
    bad["end_ms"] = json!(DURATION_MS + 5000);
    let (sidecar, warnings) = load_ok(&with_marks(vec![bad, mark_with(SECOND_ID, 2000)]));
    assert_eq!(warnings, vec![Warning::Clamped { mark: 0, field: "end_ms" }]);
    assert_eq!(sidecar.marks.len(), 2);
    assert_eq!(find(&sidecar, BAD_ID).end_ms, Some(DURATION_MS));
}

#[test]
fn clamping_that_leaves_a_range_with_no_length_drops_end_ms() {
    let mut bad = mark_with(BAD_ID, DURATION_MS + 1000);
    bad["end_ms"] = json!(DURATION_MS + 5000);
    let (sidecar, warnings) = load_ok(&with_marks(vec![bad]));
    assert_eq!(warnings, vec![Warning::Clamped { mark: 0, field: "t_ms" }, Warning::Clamped { mark: 0, field: "end_ms" }, Warning::BadRange { mark: 0 }]);
    assert_eq!(sidecar.marks[0].t_ms, DURATION_MS);
    assert_eq!(sidecar.marks[0].end_ms, None);
}

#[test]
fn duplicate_id_keeps_the_first_and_warns() {
    let mut first = mark_with(BAD_ID, 1000);
    first["label"] = json!("first");
    let mut again = mark_with(BAD_ID, 3000);
    again["label"] = json!("again");
    let (sidecar, warnings) = load_ok(&with_marks(vec![first, again, mark_with(SECOND_ID, 2000)]));
    assert_eq!(warnings, vec![Warning::DuplicateId { id: Uuid::parse_str(BAD_ID).unwrap() }]);
    assert_eq!(sidecar.marks.len(), 2);
    assert_eq!(find(&sidecar, BAD_ID).label.as_deref(), Some("first"));
    assert_eq!(find(&sidecar, BAD_ID).t_ms, 1000);
}

#[test]
fn over_length_text_is_truncated_by_characters_with_warnings() {
    let mut long = mark_with(BAD_ID, 1000);
    // Two-byte characters, so a byte-based cut would land on the wrong length.
    long["label"] = json!("é".repeat(LABEL_MAX + 10));
    long["notes"] = json!("n".repeat(NOTES_MAX + 1));
    long["script"] = json!({ "line": "l".repeat(SCRIPT_LINE_MAX + 1), "position": 3 });
    long["auto"] = json!({ "kind": "recorder.pause", "data": { "blob": "x".repeat(AUTO_DATA_MAX_BYTES + 1) } });
    let mut doc = with_marks(vec![long, mark_with(SECOND_ID, 2000)]);
    doc["context"]["notes"] = json!("c".repeat(NOTES_MAX + 1));

    let (sidecar, warnings) = load_ok(&doc);
    assert_eq!(
        warnings,
        vec![
            Warning::Truncated { mark: None, field: "context.notes" },
            Warning::Truncated { mark: Some(0), field: "label" },
            Warning::Truncated { mark: Some(0), field: "notes" },
            Warning::Truncated { mark: Some(0), field: "script.line" },
            Warning::Truncated { mark: Some(0), field: "auto.data" },
        ]
    );
    assert_eq!(sidecar.marks.len(), 2);
    let mark = find(&sidecar, BAD_ID);
    assert_eq!(mark.label.as_deref(), Some("é".repeat(LABEL_MAX).as_str()));
    assert_eq!(mark.notes.as_deref().map(|text| text.chars().count()), Some(NOTES_MAX));
    let script = mark.script.as_ref().unwrap();
    assert_eq!(script.line.chars().count(), SCRIPT_LINE_MAX);
    assert_eq!(script.position, 3);
    let auto = mark.auto.as_ref().unwrap();
    assert_eq!(auto.kind, "recorder.pause");
    assert_eq!(auto.data, json!({}));
    let context_notes = sidecar.context.as_ref().and_then(|context| context.notes.as_deref());
    assert_eq!(context_notes.map(|text| text.chars().count()), Some(NOTES_MAX));
}

#[test]
fn text_and_data_at_the_limits_are_kept_whole() {
    let mut full = mark_with(BAD_ID, 1000);
    full["label"] = json!("é".repeat(LABEL_MAX));
    full["notes"] = json!("n".repeat(NOTES_MAX));
    full["script"] = json!({ "line": "l".repeat(SCRIPT_LINE_MAX), "position": 0 });
    // `{"k":"..."}` is 8 bytes around the text.
    full["auto"] = json!({ "kind": "recorder.pause", "data": { "k": "x".repeat(AUTO_DATA_MAX_BYTES - 8) } });
    let (sidecar, warnings) = load_ok(&with_marks(vec![full]));
    assert!(warnings.is_empty(), "{warnings:?}");
    let mark = &sidecar.marks[0];
    assert_eq!(mark.label.as_deref().map(|text| text.chars().count()), Some(LABEL_MAX));
    assert_eq!(mark.auto.as_ref().unwrap().data["k"].as_str().map(str::len), Some(AUTO_DATA_MAX_BYTES - 8));
}

#[test]
fn more_than_the_marks_limit_keeps_the_first_ones_and_warns() {
    let template = mark_with(BAD_ID, 1000);
    let marks: Vec<Value> = (1..=MARKS_MAX as u128 + 1)
        .map(|number| {
            let mut mark = template.clone();
            mark["id"] = json!(Uuid::from_u128(number).to_string());
            mark
        })
        .collect();
    let (sidecar, warnings) = load_ok(&with_marks(marks));
    assert_eq!(warnings, vec![Warning::TooManyMarks { kept: MARKS_MAX }]);
    assert_eq!(sidecar.marks.len(), MARKS_MAX);
    // The last mark in the file is the one that was dropped.
    let dropped = Uuid::from_u128(MARKS_MAX as u128 + 1);
    assert!(sidecar.marks.iter().all(|mark| mark.id != dropped));
}

#[test]
fn missing_marks_key_gives_an_empty_list_and_a_warning() {
    let mut doc = example();
    doc.as_object_mut().unwrap().remove("marks");
    let (sidecar, warnings) = load_ok(&doc);
    assert_eq!(warnings, vec![Warning::MarksMissing]);
    assert!(sidecar.marks.is_empty());
}

#[test]
fn schema_version_99_is_refused_with_the_newer_version_message() {
    let mut doc = example();
    doc["schema_version"] = json!(99);
    let error = load_value(&doc).unwrap_err();
    assert_eq!(error.to_string(), "This marks file was made by a newer CutMark (1.2.0). Update to open it.");
    assert!(matches!(error, LoadError::NewerVersion { found: 99, writer_version: Some(ref version) } if version == "1.2.0"));
}

#[test]
fn newer_version_without_a_writer_version_says_unknown_version() {
    let mut doc = example();
    doc["schema_version"] = json!(2);
    doc["session"].as_object_mut().unwrap().remove("writer");
    let error = load_value(&doc).unwrap_err();
    assert_eq!(error.to_string(), "This marks file was made by a newer CutMark (unknown version). Update to open it.");
    assert!(matches!(error, LoadError::NewerVersion { found: 2, writer_version: None }));
}

#[test]
fn schema_version_below_1_or_not_an_integer_is_invalid() {
    for version in [json!(0), json!(-1), json!("1"), json!(1.5), json!(null)] {
        let mut doc = example();
        doc["schema_version"] = version.clone();
        let result = load_value(&doc);
        assert!(matches!(result, Err(LoadError::InvalidSchemaVersion)), "{version}: {result:?}");
    }
    let mut doc = example();
    doc.as_object_mut().unwrap().remove("schema_version");
    assert!(matches!(load_value(&doc), Err(LoadError::InvalidSchemaVersion)));
}

#[test]
fn not_json_is_an_error() {
    assert!(matches!(load_bytes(b"this is not json"), Err(LoadError::NotJson(_))));
    assert!(matches!(load_bytes(b""), Err(LoadError::NotJson(_))));
    assert!(matches!(load_bytes(&[0xff, 0xfe, 0x00]), Err(LoadError::NotJson(_))));
}

#[test]
fn json_that_is_not_an_object_is_an_error() {
    assert!(matches!(load_bytes(b"[1, 2, 3]"), Err(LoadError::NotAnObject)));
    assert!(matches!(load_bytes(b"null"), Err(LoadError::NotAnObject)));
}

#[test]
fn missing_or_invalid_session_is_an_error() {
    let mut doc = example();
    doc.as_object_mut().unwrap().remove("session");
    assert!(matches!(load_value(&doc), Err(LoadError::MissingSession)));

    let mut doc = example();
    doc["session"] = json!("not an object");
    assert!(matches!(load_value(&doc), Err(LoadError::MissingSession)));

    let mut doc = example();
    doc["session"]["id"] = json!("not-a-uuid");
    assert!(matches!(load_value(&doc), Err(LoadError::MissingSession)));
}

#[test]
fn missing_empty_or_invalid_media_is_an_error() {
    let mut doc = example();
    doc["media"] = json!([]);
    assert!(matches!(load_value(&doc), Err(LoadError::MissingMedia)));

    let mut doc = example();
    doc.as_object_mut().unwrap().remove("media");
    assert!(matches!(load_value(&doc), Err(LoadError::MissingMedia)));

    let mut doc = example();
    doc["media"] = json!([{ "index": 0, "path": "a.mkv", "offset_ms": 0 }]);
    assert!(matches!(load_value(&doc), Err(LoadError::MissingMedia)));

    let mut doc = example();
    doc["media"].as_array_mut().unwrap().push(json!("not an entry"));
    assert!(matches!(load_value(&doc), Err(LoadError::MissingMedia)));
}

#[test]
fn unknown_keys_are_ignored_everywhere() {
    let baseline = load_ok(&example()).0;
    let mut doc = example();
    doc["future_root"] = json!({ "a": 1 });
    doc["session"]["future_session"] = json!(true);
    doc["session"]["writer"]["future_writer"] = json!("x");
    doc["media"][0]["future_media"] = json!([1, 2]);
    doc["context"]["future_context"] = json!(null);
    doc["marks"][0]["future_mark"] = json!("later");
    let (sidecar, warnings) = load_ok(&doc);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sidecar, baseline);
}

#[test]
fn unknown_type_becomes_a_note_keeps_the_original_and_warns_once_per_name() {
    let mut flag = mark_with(BAD_ID, 1000);
    flag["type"] = json!("flag");
    let mut flag_again = mark_with(SECOND_ID, 2000);
    flag_again["type"] = json!("flag");
    let mut shout = mark_with("0196f3a2-9d44-7e02-8c15-6a0b3e7d2fa1", 3000);
    shout["type"] = json!("shout");
    let (sidecar, warnings) = load_ok(&with_marks(vec![flag, flag_again, shout]));
    assert_eq!(warnings, vec![Warning::UnknownType { value: "flag".into() }, Warning::UnknownType { value: "shout".into() }]);
    assert_eq!(sidecar.marks.len(), 3);
    assert!(sidecar.marks.iter().all(|mark| mark.mark_type == MarkType::Note));
    assert_eq!(find(&sidecar, BAD_ID).original_type.as_deref(), Some("flag"));
    assert_eq!(find(&sidecar, SECOND_ID).original_type.as_deref(), Some("flag"));
    assert_eq!(find(&sidecar, "0196f3a2-9d44-7e02-8c15-6a0b3e7d2fa1").original_type.as_deref(), Some("shout"));
}

#[test]
fn unknown_source_becomes_auto_without_a_warning() {
    let mut odd = mark_with(BAD_ID, 1000);
    odd["source"] = json!("carrier-pigeon");
    let (sidecar, warnings) = load_ok(&with_marks(vec![odd]));
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sidecar.marks[0].source, Source::Auto);
}

#[test]
fn a_byte_order_mark_is_accepted() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(EXAMPLE.as_bytes());
    let (sidecar, warnings) = load_bytes(&bytes).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(sidecar, load_ok(&example()).0);
}

#[test]
fn an_unsorted_marks_array_is_sorted_by_session_time_then_id_without_a_warning() {
    let ids = [
        "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f01",
        "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f02",
        "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f03",
        "0196f3a2-9d44-7e02-8c15-6a0b3e7d2f04",
    ];
    let mut doc = example();
    doc["media"] = json!([
        { "index": 0, "path": "a.mkv", "offset_ms": 0, "duration_ms": 3_698_784 },
        { "index": 1, "path": "b.mkv", "offset_ms": 3_698_784, "duration_ms": 1_000_000 },
    ]);
    // File order: 4th on the timeline, a tie broken by id, the 3rd, the other half of the tie.
    let mut on_second_file = mark_with(ids[3], 100);
    on_second_file["media"] = json!(1);
    doc["marks"] = json!([on_second_file, mark_with(ids[1], 1000), mark_with(ids[2], 2000), mark_with(ids[0], 1000)]);
    let (sidecar, warnings) = load_ok(&doc);
    assert!(warnings.is_empty(), "{warnings:?}");
    let order: Vec<String> = sidecar.marks.iter().map(|mark| mark.id.to_string()).collect();
    assert_eq!(order, ids);
}

#[test]
fn discover_finds_the_sidecar_beside_the_media_file() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("clip.mkv");
    assert_eq!(discover(&media), None);

    let sidecar: PathBuf = dir.path().join("clip.marks.json");
    std::fs::write(&sidecar, EXAMPLE).unwrap();
    assert_eq!(discover(&media), Some(sidecar));
    // No guessing: another media file does not match.
    assert_eq!(discover(&dir.path().join("other.mkv")), None);
}

#[test]
fn discover_drops_only_the_final_extension() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("take.2026.mkv");
    assert_eq!(discover(&media), None);
    std::fs::write(dir.path().join("take.marks.json"), EXAMPLE).unwrap();
    assert_eq!(discover(&media), None);
    let sidecar = dir.path().join("take.2026.marks.json");
    std::fs::write(&sidecar, EXAMPLE).unwrap();
    assert_eq!(discover(&media), Some(sidecar));
}

#[test]
fn every_warning_has_a_message() {
    let warnings = [
        Warning::MissingField { mark: 1, field: "id" },
        Warning::MediaOutOfRange { mark: 1, media: 4 },
        Warning::NegativeTime { mark: 1 },
        Warning::BadRange { mark: 1 },
        Warning::Clamped { mark: 1, field: "t_ms" },
        Warning::DuplicateId { id: Uuid::nil() },
        Warning::Truncated { mark: Some(1), field: "label" },
        Warning::Truncated { mark: None, field: "context.notes" },
        Warning::UnknownType { value: "flag".into() },
        Warning::TooManyMarks { kept: 3 },
        Warning::MarksMissing,
    ];
    for warning in warnings {
        let text = warning.to_string();
        assert!(!text.is_empty());
        assert!(!text.contains('\u{2014}'), "{text}");
    }
}
