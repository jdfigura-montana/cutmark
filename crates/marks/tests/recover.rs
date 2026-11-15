//! Crash recovery: rebuilding a sidecar from a journal a dead session left behind.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use marks::{Clock, MarkType, NewMark, SessionWriter, Source, WriteError, WriterInfo, find_journals, load};
use uuid::Uuid;

/// A UUIDv7-shaped base; the clock adds a counter to the low bits.
const ID_BASE: u128 = 0x0190_0000_0000_7000_8000_0000_0000_0000;

/// A clock that always reads the same moment and hands out counting ids.
struct TestClock {
    next_id: AtomicU64,
}

impl TestClock {
    fn boxed() -> Box<dyn Clock> {
        Box::new(Self { next_id: AtomicU64::new(1) })
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn new_id(&self) -> Uuid {
        Uuid::from_u128(ID_BASE + u128::from(self.next_id.fetch_add(1, Ordering::Relaxed)))
    }
}

fn writer_info() -> WriterInfo {
    WriterInfo { app: "recover-test".to_string(), version: "0.0.0".to_string() }
}

fn begin(dir: &Path, base: &str) -> SessionWriter {
    SessionWriter::begin(dir, base, writer_info(), &dir.join(format!("{base}.mkv")), TestClock::boxed()).unwrap()
}

fn add_highlight(writer: &mut SessionWriter, t_ms: u64) {
    writer.add_mark(NewMark::point(MarkType::Highlight, t_ms, Source::Ui)).unwrap();
}

#[test]
fn journal_cut_off_mid_line_recovers_every_complete_mark() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin(dir.path(), "take");
    for t_ms in [1_000, 2_000, 3_000] {
        add_highlight(&mut writer, t_ms);
    }
    drop(writer);

    let journal = dir.path().join("take.marks.journal");
    let mut file = OpenOptions::new().append(true).open(&journal).unwrap();
    file.write_all(br#"{"mark": {"id": "#).unwrap();
    drop(file);

    let recovered = SessionWriter::recover(dir.path(), "take").unwrap();
    let sidecar_path = dir.path().join("take.marks.json");
    assert_eq!(recovered, Some(sidecar_path.clone()));

    let (sidecar, warnings) = load(&sidecar_path).unwrap();
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    assert!(sidecar.session.recovered);
    assert_eq!(sidecar.session.ended_at, None);
    let times: Vec<u64> = sidecar.marks.iter().map(|mark| mark.t_ms).collect();
    assert_eq!(times, vec![1_000, 2_000, 3_000]);
    assert_eq!(sidecar.media.len(), 1);
    assert_eq!(sidecar.media[0].duration_ms, 3_000);
    assert!(!journal.exists());
    assert!(!dir.path().join("take.marks.json.tmp").exists());
}

#[test]
fn journal_next_to_a_finished_sidecar_is_deleted_and_the_sidecar_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = begin(dir.path(), "done");
    add_highlight(&mut writer, 4_000);
    let journal = dir.path().join("done.marks.journal");
    let journal_bytes = fs::read(&journal).unwrap();
    let sidecar_path = writer.finish(&[10_000]).unwrap();
    assert!(!journal.exists());

    // A crash after the rename but before the journal delete leaves both files.
    fs::write(&journal, journal_bytes).unwrap();
    let before = fs::read(&sidecar_path).unwrap();

    let recovered = SessionWriter::recover(dir.path(), "done").unwrap();

    assert_eq!(recovered, Some(sidecar_path.clone()));
    assert!(!journal.exists());
    assert_eq!(fs::read(&sidecar_path).unwrap(), before);
}

#[test]
fn no_journal_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let recovered = SessionWriter::recover(dir.path(), "nothing").unwrap();
    assert_eq!(recovered, None);
    assert!(!dir.path().join("nothing.marks.json").exists());
}

#[test]
fn broken_header_is_an_error_and_keeps_the_journal() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("bad.marks.journal");
    fs::write(&journal, "this is not a header\n{\"mark\": {}}\n").unwrap();

    let result = SessionWriter::recover(dir.path(), "bad");

    assert!(matches!(result, Err(WriteError::Invalid(_))), "got {result:?}");
    assert!(journal.exists());
    assert!(!dir.path().join("bad.marks.json").exists());

    let empty = dir.path().join("empty.marks.journal");
    fs::write(&empty, "").unwrap();
    let result = SessionWriter::recover(dir.path(), "empty");
    assert!(matches!(result, Err(WriteError::Invalid(_))), "got {result:?}");
    assert!(empty.exists());
}

#[test]
fn find_journals_lists_journals_sorted_and_ignores_other_files() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["b.marks.journal", "a.marks.journal", "c.marks.json", "d.txt", "e.marks.json.tmp", ".marks.journal"] {
        fs::write(dir.path().join(name), "x").unwrap();
    }
    fs::create_dir(dir.path().join("f.marks.journal")).unwrap();

    let found = find_journals(dir.path()).unwrap();

    assert_eq!(found, vec!["a".to_string(), "b".to_string()]);
}
