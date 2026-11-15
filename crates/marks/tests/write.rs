#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use marks::{
    AUTO_DATA_MAX_BYTES, AutoInfo, Clock, LABEL_MAX, Mark, MarkType, MediaEntry, NOTES_MAX, NewMark, SCHEMA_VERSION, SCRIPT_LINE_MAX, ScriptRef, SessionWriter,
    Sidecar, Source, WriteError, WriterInfo,
};
use serde_json::json;
use uuid::Uuid;

const BASE: &str = "session";
/// 2026-01-01T00:00:00Z in milliseconds since the epoch.
const START_MS: i64 = 1_767_225_600_000;

struct ClockState {
    elapsed_ms: i64,
    step_ms: i64,
    ids: u64,
}

/// A clock that moves forward by a fixed step on every read and hands out increasing UUIDv7 ids.
#[derive(Clone)]
struct TestClock {
    state: Arc<Mutex<ClockState>>,
}

impl TestClock {
    fn new() -> Self {
        Self { state: Arc::new(Mutex::new(ClockState { elapsed_ms: 0, step_ms: 1_000, ids: 0 })) }
    }

    fn set_step(&self, step_ms: i64) {
        self.state.lock().unwrap().step_ms = step_ms;
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        let mut state = self.state.lock().unwrap();
        let at = DateTime::<Utc>::from_timestamp_millis(START_MS + state.elapsed_ms).unwrap();
        state.elapsed_ms += state.step_ms;
        at
    }

    fn new_id(&self) -> Uuid {
        let mut state = self.state.lock().unwrap();
        state.ids += 1;
        let mut random = [0u8; 10];
        random[2..].copy_from_slice(&state.ids.to_be_bytes());
        let millis = u64::try_from(START_MS).unwrap() + state.ids;
        uuid::Builder::from_unix_timestamp_millis(millis, &random).into_uuid()
    }
}

fn writer_info() -> WriterInfo {
    WriterInfo { app: "cutmark-recorder".to_string(), version: "0.1.0".to_string() }
}

fn begin_in(dir: &Path, clock: &TestClock) -> SessionWriter {
    SessionWriter::begin(dir, BASE, writer_info(), &dir.join("session.mkv"), Box::new(clock.clone())).unwrap()
}

fn add(writer: &mut SessionWriter, mark: NewMark) -> Uuid {
    writer.add_mark(mark).unwrap().unwrap()
}

fn load_clean(path: &Path) -> Sidecar {
    let (sidecar, warnings) = marks::load(path).unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    sidecar
}

fn sidecar_path(dir: &Path) -> PathBuf {
    dir.join("session.marks.json")
}

fn journal_path(dir: &Path) -> PathBuf {
    dir.join("session.marks.journal")
}

fn tmp_path(dir: &Path) -> PathBuf {
    dir.join("session.marks.json.tmp")
}

/// A mark with no text, script or auto details and a placeholder creation time.
fn expected(id: Uuid, mark_type: MarkType, media: usize, t_ms: u64, end_ms: Option<u64>, source: Source) -> Mark {
    Mark {
        id,
        mark_type,
        media,
        t_ms,
        end_ms,
        label: None,
        notes: None,
        source,
        created_at: DateTime::<Utc>::from_timestamp_millis(0).unwrap(),
        script: None,
        auto: None,
        original_type: None,
    }
}

#[test]
fn round_trip_matches_what_was_added() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::new();
    let mut writer = begin_in(dir.path(), &clock);

    let pause = AutoInfo { kind: "recorder.pause".to_string(), data: json!({ "reason": "user" }) };
    let script = ScriptRef { line: "Welcome back to the channel.".to_string(), position: 3 };

    let highlight = add(
        &mut writer,
        NewMark {
            label: Some("Great moment".to_string()),
            notes: Some("Chat went wild".to_string()),
            ..NewMark::point(MarkType::Highlight, 1_000, Source::Ui)
        },
    );
    let cut = add(&mut writer, NewMark { media: Some(0), end_ms: Some(8_000), ..NewMark::point(MarkType::Cut, 5_000, Source::Hotkey) });
    let chapter = add(&mut writer, NewMark { label: Some("Intro".to_string()), ..NewMark::point(MarkType::Chapter, 10_000, Source::Ui) });
    let system = add(&mut writer, NewMark { auto: Some(pause.clone()), ..NewMark::point(MarkType::System, 12_000, Source::Auto) });
    let retake = add(&mut writer, NewMark { script: Some(script.clone()), ..NewMark::point(MarkType::Retake, 15_000, Source::Api) });
    assert_eq!(writer.add_media(&dir.path().join("session_2.mkv"), 60_000).unwrap(), 1);
    let note = add(&mut writer, NewMark { label: Some("Rollover note".to_string()), ..NewMark::point(MarkType::Note, 2_000, Source::Guest) });
    let b_roll = add(&mut writer, NewMark { media: Some(1), end_ms: Some(9_000), ..NewMark::point(MarkType::BRoll, 3_000, Source::Ui) });
    let early = add(&mut writer, NewMark { media: Some(0), ..NewMark::point(MarkType::Highlight, 500, Source::Api) });

    let path = writer.finish(&[60_000, 30_000]).unwrap();
    assert_eq!(path, sidecar_path(dir.path()));
    assert!(!journal_path(dir.path()).exists());
    assert!(!tmp_path(dir.path()).exists());

    let sidecar = load_clean(&path);
    assert_eq!(sidecar.schema_version, SCHEMA_VERSION);
    assert_eq!(sidecar.session.started_at, DateTime::<Utc>::from_timestamp_millis(START_MS).unwrap());
    assert!(sidecar.session.ended_at.is_some());
    assert!(!sidecar.session.recovered);
    assert_eq!(sidecar.session.writer, writer_info());
    assert_eq!(
        sidecar.media,
        vec![
            MediaEntry { index: 0, path: "session.mkv".to_string(), offset_ms: 0, duration_ms: 60_000 },
            MediaEntry { index: 1, path: "session_2.mkv".to_string(), offset_ms: 60_000, duration_ms: 30_000 },
        ]
    );

    let mut want = vec![
        Mark {
            label: Some("Great moment".to_string()),
            notes: Some("Chat went wild".to_string()),
            ..expected(highlight, MarkType::Highlight, 0, 1_000, None, Source::Ui)
        },
        expected(cut, MarkType::Cut, 0, 5_000, Some(8_000), Source::Hotkey),
        Mark { label: Some("Intro".to_string()), ..expected(chapter, MarkType::Chapter, 0, 10_000, None, Source::Ui) },
        Mark { auto: Some(pause), ..expected(system, MarkType::System, 0, 12_000, None, Source::Auto) },
        Mark { script: Some(script), ..expected(retake, MarkType::Retake, 0, 15_000, None, Source::Api) },
        Mark { label: Some("Rollover note".to_string()), ..expected(note, MarkType::Note, 1, 2_000, None, Source::Guest) },
        expected(b_roll, MarkType::BRoll, 1, 3_000, Some(9_000), Source::Ui),
        expected(early, MarkType::Highlight, 0, 500, None, Source::Api),
    ];

    // created_at is set when each mark is made: increasing in the order added, inside the session.
    let added_order: Vec<Uuid> = want.iter().map(|mark| mark.id).collect();
    let mut by_creation = sidecar.marks.clone();
    by_creation.sort_by_key(|mark| mark.created_at);
    assert_eq!(by_creation.iter().map(|mark| mark.id).collect::<Vec<_>>(), added_order);
    let ended_at = sidecar.session.ended_at.unwrap();
    for mark in &by_creation {
        assert!(mark.created_at > sidecar.session.started_at && mark.created_at < ended_at);
    }

    // Everything else matches, in session order with ties by id.
    for mark in &mut want {
        mark.created_at = sidecar.marks.iter().find(|got| got.id == mark.id).unwrap().created_at;
    }
    let offsets = [0u64, 60_000];
    want.sort_by_key(|mark| (offsets[mark.media] + mark.t_ms, mark.id));
    assert_eq!(sidecar.marks, want);
}

#[test]
fn finish_with_no_marks_loads_clean() {
    let dir = tempfile::tempdir().unwrap();
    let writer = begin_in(dir.path(), &TestClock::new());
    let path = writer.finish(&[12_345]).unwrap();
    let sidecar = load_clean(&path);
    assert!(sidecar.marks.is_empty());
    assert_eq!(sidecar.media[0].duration_ms, 12_345);
}

#[test]
fn media_paths_are_relative_with_forward_slashes() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::new();
    let first = dir.path().join("takes").join("one.mkv");
    let mut writer = SessionWriter::begin(dir.path(), BASE, writer_info(), &first, Box::new(clock)).unwrap();
    writer.add_media(&dir.path().join("takes").join("two.mkv"), 1_000).unwrap();
    let sidecar = load_clean(&writer.finish(&[1_000, 2_000]).unwrap());
    assert_eq!(sidecar.media[0].path, "takes/one.mkv");
    assert_eq!(sidecar.media[1].path, "takes/two.mkv");
}

#[test]
fn journal_lines_follow_the_format() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::new();
    let mut writer = begin_in(dir.path(), &clock);
    writer.add_media(&dir.path().join("session_2.mkv"), 5_000).unwrap();
    add(&mut writer, NewMark { label: Some("Look".to_string()), ..NewMark::point(MarkType::Note, 250, Source::Ui) });

    // The writer is still open: every line is already on disk.
    let text = fs::read_to_string(journal_path(dir.path())).unwrap();
    let lines: Vec<serde_json::Value> = text.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["journal"], 1);
    assert!(lines[0]["session"]["id"].is_string());
    assert!(lines[0]["session"]["started_at"].is_string());
    assert!(lines[0]["session"]["ended_at"].is_null());
    assert_eq!(lines[0]["session"]["recovered"], false);
    assert_eq!(lines[0]["session"]["writer"]["app"], "cutmark-recorder");
    assert_eq!(lines[0]["media"][0]["path"], "session.mkv");
    assert_eq!(lines[1]["media"]["index"], 1);
    assert_eq!(lines[1]["media"]["path"], "session_2.mkv");
    assert_eq!(lines[2]["mark"]["type"], "note");
    assert_eq!(lines[2]["mark"]["media"], 1);
    assert_eq!(lines[2]["mark"]["label"], "Look");
}

#[test]
fn sidecar_is_utf8_without_bom_and_with_lf_endings() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    let label = "Line one\r\nline two \u{e9} \u{2603}";
    add(&mut writer, NewMark { label: Some(label.to_string()), ..NewMark::point(MarkType::Note, 100, Source::Ui) });
    let path = writer.finish(&[1_000]).unwrap();

    let bytes = fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"{"));
    assert!(!bytes.starts_with(b"\xEF\xBB\xBF"));
    assert!(!bytes.contains(&b'\r'));
    assert!(bytes.ends_with(b"}\n"));
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("{\n  \"schema_version\": 1,\n  \"session\": {\n    \"id\": "));
    assert_eq!(load_clean(&path).marks[0].label.as_deref(), Some(label));
}

#[test]
fn failed_finish_keeps_the_journal_and_leaves_no_tmp() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    add(&mut writer, NewMark::point(MarkType::Highlight, 100, Source::Ui));

    // A folder where the sidecar belongs makes the final rename fail.
    fs::create_dir(sidecar_path(dir.path())).unwrap();
    let error = writer.finish(&[1_000]).unwrap_err();
    assert!(matches!(error, WriteError::Io(_)), "got {error:?}");

    assert!(journal_path(dir.path()).is_file());
    assert!(!tmp_path(dir.path()).exists());
    let text = fs::read_to_string(journal_path(dir.path())).unwrap();
    assert_eq!(text.lines().count(), 2);
}

#[test]
fn limits_are_enforced_on_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());

    // Two-byte characters show that the cut is by characters, not bytes.
    let long_text = add(
        &mut writer,
        NewMark {
            label: Some("\u{e9}".repeat(LABEL_MAX + 100)),
            notes: Some("\u{e9}".repeat(NOTES_MAX + 1)),
            script: Some(ScriptRef { line: "\u{e9}".repeat(SCRIPT_LINE_MAX + 100), position: 7 }),
            ..NewMark::point(MarkType::Note, 100, Source::Ui)
        },
    );
    let at_limit = add(
        &mut writer,
        NewMark {
            label: Some("a".repeat(LABEL_MAX)),
            auto: Some(AutoInfo { kind: "recorder.note".to_string(), data: json!({ "ok": true }) }),
            ..NewMark::point(MarkType::Note, 200, Source::Ui)
        },
    );
    let big_data = add(
        &mut writer,
        NewMark {
            auto: Some(AutoInfo { kind: "recorder.big".to_string(), data: json!({ "blob": "x".repeat(AUTO_DATA_MAX_BYTES * 2) }) }),
            ..NewMark::point(MarkType::System, 300, Source::Auto)
        },
    );
    let sidecar = load_clean(&writer.finish(&[10_000]).unwrap());
    let find = |id: Uuid| sidecar.marks.iter().find(|mark| mark.id == id).unwrap();

    let cut = find(long_text);
    assert_eq!(cut.label.as_deref().unwrap().chars().count(), LABEL_MAX);
    assert_eq!(cut.notes.as_deref().unwrap().chars().count(), NOTES_MAX);
    let script = cut.script.as_ref().unwrap();
    assert_eq!(script.line.chars().count(), SCRIPT_LINE_MAX);
    assert_eq!(script.position, 7);

    let kept = find(at_limit);
    assert_eq!(kept.label.as_deref().unwrap().chars().count(), LABEL_MAX);
    assert_eq!(kept.auto.as_ref().unwrap().data, json!({ "ok": true }));

    let emptied = find(big_data).auto.as_ref().unwrap();
    assert_eq!(emptied.kind, "recorder.big");
    assert_eq!(emptied.data, json!({}));
}

#[test]
fn key_repeat_inside_250_ms_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::new();
    let mut writer = begin_in(dir.path(), &clock);
    clock.set_step(100);

    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_000, Source::Hotkey)).unwrap().is_some());
    // 100 ms and 200 ms after the first mark: both are repeats.
    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_100, Source::Hotkey)).unwrap().is_none());
    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_200, Source::Hotkey)).unwrap().is_none());
    // A different type, or a different source, is not a repeat.
    assert!(writer.add_mark(NewMark::point(MarkType::Cut, 1_300, Source::Hotkey)).unwrap().is_some());
    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_400, Source::Ui)).unwrap().is_some());
    // 300 ms after the first hotkey highlight: kept.
    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_500, Source::Hotkey)).unwrap().is_some());

    let sidecar = load_clean(&writer.finish(&[10_000]).unwrap());
    let times: Vec<u64> = sidecar.marks.iter().map(|mark| mark.t_ms).collect();
    assert_eq!(times, vec![1_000, 1_300, 1_400, 1_500]);
}

#[test]
fn key_repeat_at_250_ms_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let clock = TestClock::new();
    let mut writer = begin_in(dir.path(), &clock);
    clock.set_step(250);

    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_000, Source::Hotkey)).unwrap().is_some());
    assert!(writer.add_mark(NewMark::point(MarkType::Highlight, 1_250, Source::Hotkey)).unwrap().is_some());
    assert_eq!(load_clean(&writer.finish(&[10_000]).unwrap()).marks.len(), 2);
}

#[test]
fn system_mark_is_stored_with_source_auto() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    let id = add(&mut writer, NewMark::point(MarkType::System, 100, Source::Hotkey));
    let sidecar = load_clean(&writer.finish(&[1_000]).unwrap());
    let mark = sidecar.marks.iter().find(|mark| mark.id == id).unwrap();
    assert_eq!(mark.source, Source::Auto);
}

#[test]
fn bad_ranges_are_dropped_on_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    add(&mut writer, NewMark { end_ms: Some(500), ..NewMark::point(MarkType::Cut, 500, Source::Ui) });
    add(&mut writer, NewMark { end_ms: Some(100), ..NewMark::point(MarkType::Cut, 600, Source::Ui) });
    add(&mut writer, NewMark { end_ms: Some(900), ..NewMark::point(MarkType::Chapter, 700, Source::Ui) });
    add(&mut writer, NewMark { end_ms: Some(900), ..NewMark::point(MarkType::System, 800, Source::Auto) });
    add(&mut writer, NewMark { end_ms: Some(900), ..NewMark::point(MarkType::Retake, 850, Source::Ui) });
    let sidecar = load_clean(&writer.finish(&[10_000]).unwrap());
    let ends: Vec<Option<u64>> = sidecar.marks.iter().map(|mark| mark.end_ms).collect();
    assert_eq!(ends, vec![None, None, None, None, Some(900)]);
}

#[test]
fn begin_fails_when_a_journal_exists() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(journal_path(dir.path()), "left over\n").unwrap();
    let result = SessionWriter::begin(dir.path(), BASE, writer_info(), &dir.path().join("session.mkv"), Box::new(TestClock::new()));
    match result.err().unwrap() {
        WriteError::JournalExists(path) => assert_eq!(path, journal_path(dir.path())),
        other => panic!("expected JournalExists, got {other:?}"),
    }
    assert_eq!(fs::read_to_string(journal_path(dir.path())).unwrap(), "left over\n");
}

#[test]
fn begin_fails_when_a_sidecar_exists() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(sidecar_path(dir.path()), "{}").unwrap();
    let result = SessionWriter::begin(dir.path(), BASE, writer_info(), &dir.path().join("session.mkv"), Box::new(TestClock::new()));
    match result.err().unwrap() {
        WriteError::SidecarExists(path) => assert_eq!(path, sidecar_path(dir.path())),
        other => panic!("expected SidecarExists, got {other:?}"),
    }
    assert!(!journal_path(dir.path()).exists());
}

#[test]
fn marks_past_the_file_duration_are_clamped_at_finish() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    let past_end = add(&mut writer, NewMark { end_ms: Some(70_000), ..NewMark::point(MarkType::Cut, 50_000, Source::Ui) });
    let long_range = add(&mut writer, NewMark { end_ms: Some(40_000), ..NewMark::point(MarkType::Cut, 20_000, Source::Ui) });
    let sidecar = load_clean(&writer.finish(&[30_000]).unwrap());
    let find = |id: Uuid| sidecar.marks.iter().find(|mark| mark.id == id).unwrap();

    // Both times land on the end, so the range collapses to a point.
    assert_eq!((find(past_end).t_ms, find(past_end).end_ms), (30_000, None));
    // Only the end is past the file, so the range is cut short.
    assert_eq!((find(long_range).t_ms, find(long_range).end_ms), (20_000, Some(30_000)));
}

#[test]
fn bad_media_index_and_duration_count_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin_in(dir.path(), &TestClock::new());
    let error = writer.add_mark(NewMark { media: Some(5), ..NewMark::point(MarkType::Note, 100, Source::Ui) }).unwrap_err();
    assert!(matches!(error, WriteError::BadMedia(5)), "got {error:?}");

    let error = writer.finish(&[1_000, 2_000]).unwrap_err();
    assert!(matches!(error, WriteError::DurationsMismatch { expected: 1, got: 2 }), "got {error:?}");
    assert!(journal_path(dir.path()).is_file());
}
