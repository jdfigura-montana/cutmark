//! Property tests: random valid mark lists survive write then load, and `load` never panics.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};
use marks::{Clock, MarkType, NewMark, SessionWriter, Source, WriterInfo, load};
use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError};
use uuid::Uuid;

/// The length of the file every generated mark falls inside.
const FILE_MS: u64 = 600_000;
/// A UUIDv7-shaped base; the clock adds a counter to the low bits, so ids grow in insertion order.
const ID_BASE: u128 = 0x0190_0000_0000_7000_8000_0000_0000_0000;

/// A clock that always reads the same moment and hands out counting ids.
struct TestClock {
    next_id: AtomicU64,
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).unwrap()
    }

    fn new_id(&self) -> Uuid {
        Uuid::from_u128(ID_BASE + u128::from(self.next_id.fetch_add(1, Ordering::Relaxed)))
    }
}

/// What the test controls about a mark: the rest is fixed.
type Fields = (MarkType, u64, Option<u64>, Option<String>);

fn config() -> Config {
    // No failure files: a failing case must not leave stray files in the crate.
    Config { cases: 64, failure_persistence: None, ..Config::default() }
}

fn mark_type() -> impl Strategy<Value = MarkType> {
    prop_oneof![
        Just(MarkType::Highlight),
        Just(MarkType::Cut),
        Just(MarkType::Retake),
        Just(MarkType::Chapter),
        Just(MarkType::Note),
        Just(MarkType::BRoll),
        Just(MarkType::System),
    ]
}

/// A valid mark: the end exists only on types that allow a range, and falls after the start and inside the file.
fn mark_fields() -> impl Strategy<Value = Fields> {
    (mark_type(), 0..=FILE_MS, proptest::option::of(1..=FILE_MS), proptest::option::of("[A-Za-z0-9]([A-Za-z0-9 ]{0,18}[A-Za-z0-9])?")).prop_map(
        |(mark_type, t_ms, span, label)| {
            let end_ms = span.map(|span| (t_ms + span).min(FILE_MS)).filter(|end| mark_type.allows_range() && *end > t_ms);
            (mark_type, t_ms, end_ms, label)
        },
    )
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn written_marks_load_back_sorted_and_unchanged(input in proptest::collection::vec(mark_fields(), 0..30)) {
        let dir = tempfile::tempdir().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let writer_info = WriterInfo { app: "props-test".to_string(), version: "0.0.0".to_string() };
        let clock = Box::new(TestClock { next_id: AtomicU64::new(1) });
        let mut writer = SessionWriter::begin(dir.path(), "take", writer_info, &dir.path().join("take.mkv"), clock)
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        for (mark_type, t_ms, end_ms, label) in &input {
            let mark = NewMark { end_ms: *end_ms, label: label.clone(), ..NewMark::point(*mark_type, *t_ms, Source::Ui) };
            writer.add_mark(mark).map_err(|error| TestCaseError::fail(error.to_string()))?;
        }
        let path = writer.finish(&[FILE_MS]).map_err(|error| TestCaseError::fail(error.to_string()))?;

        let (sidecar, warnings) = load(&path).map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assert!(warnings.is_empty(), "unexpected warnings: {:?}", warnings);

        let times: Vec<u64> = sidecar.marks.iter().map(|mark| mark.t_ms).collect();
        prop_assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "marks are not sorted: {:?}", times);

        // Ids grow in insertion order and ties sort by id, so a stable sort of the input gives the expected order.
        let mut expected = input;
        expected.sort_by_key(|fields| fields.1);
        let loaded: Vec<Fields> = sidecar.marks.iter().map(|mark| (mark.mark_type, mark.t_ms, mark.end_ms, mark.label.clone())).collect();
        prop_assert_eq!(loaded, expected);
    }

    #[test]
    fn load_never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let dir = tempfile::tempdir().map_err(|error| TestCaseError::fail(error.to_string()))?;
        let path = dir.path().join("junk.marks.json");
        fs::write(&path, &bytes).map_err(|error| TestCaseError::fail(error.to_string()))?;
        // An error is fine; a panic is not.
        let _ = load(&path);
    }
}
