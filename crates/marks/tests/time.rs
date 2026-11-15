//! Tests for session time, timecode text and frame snapping.

use chrono::{DateTime, Utc};
use marks::{Mark, MarkType, MediaEntry, SCHEMA_VERSION, Session, Sidecar, Source, WriterInfo, format_tc, parse_tc, session_ms, snap_to_frame};
use uuid::Uuid;

fn media(index: usize, offset_ms: u64, duration_ms: u64) -> MediaEntry {
    MediaEntry { index, path: format!("take_{index:03}.mkv"), offset_ms, duration_ms }
}

fn mark(media: usize, t_ms: u64) -> Mark {
    Mark {
        id: Uuid::nil(),
        mark_type: MarkType::Highlight,
        media,
        t_ms,
        end_ms: None,
        label: None,
        notes: None,
        source: Source::Hotkey,
        created_at: DateTime::<Utc>::default(),
        script: None,
        auto: None,
        original_type: None,
    }
}

fn sidecar(media: Vec<MediaEntry>) -> Sidecar {
    Sidecar {
        schema_version: SCHEMA_VERSION,
        session: Session {
            id: Uuid::nil(),
            started_at: DateTime::<Utc>::default(),
            ended_at: None,
            recovered: false,
            writer: WriterInfo { app: "test".to_owned(), version: "0".to_owned() },
            clock: None,
        },
        media,
        context: None,
        marks: Vec::new(),
    }
}

/// Three files in a gapless rollover: offsets 0, 600000 and 900000.
fn rollover() -> Sidecar {
    sidecar(vec![media(0, 0, 600_000), media(1, 600_000, 300_000), media(2, 900_000, 120_000)])
}

#[test]
fn session_ms_adds_the_media_offset() {
    let s = rollover();
    assert_eq!(session_ms(&s, &mark(0, 0)), 0);
    assert_eq!(session_ms(&s, &mark(0, 12_345)), 12_345);
    assert_eq!(session_ms(&s, &mark(1, 0)), 600_000);
    assert_eq!(session_ms(&s, &mark(1, 5_000)), 605_000);
    assert_eq!(session_ms(&s, &mark(2, 0)), 900_000);
    assert_eq!(session_ms(&s, &mark(2, 119_999)), 1_019_999);
}

#[test]
fn session_ms_uses_zero_offset_for_a_bad_media_index() {
    let s = rollover();
    assert_eq!(session_ms(&s, &mark(3, 7_000)), 7_000);
    assert_eq!(session_ms(&s, &mark(usize::MAX, 1)), 1);
}

#[test]
fn session_ms_saturates() {
    let s = sidecar(vec![media(0, u64::MAX - 10, 1)]);
    assert_eq!(session_ms(&s, &mark(0, 100)), u64::MAX);
}

#[test]
fn format_tc_pads_every_field() {
    assert_eq!(format_tc(0), "00:00:00.000");
    assert_eq!(format_tc(1), "00:00:00.001");
    assert_eq!(format_tc(999), "00:00:00.999");
    assert_eq!(format_tc(1_000), "00:00:01.000");
    assert_eq!(format_tc(59_999), "00:00:59.999");
    assert_eq!(format_tc(60_000), "00:01:00.000");
    assert_eq!(format_tc(3_599_999), "00:59:59.999");
    assert_eq!(format_tc(3_600_000), "01:00:00.000");
    assert_eq!(format_tc(605_000), "00:10:05.000");
    assert_eq!(format_tc(86_399_999), "23:59:59.999");
}

#[test]
fn format_tc_allows_hours_above_99() {
    assert_eq!(format_tc(360_000_000), "100:00:00.000");
    assert_eq!(format_tc(360_000_000 + 3_661_001), "101:01:01.001");
}

#[test]
fn format_tc_handles_the_largest_value() {
    // u64::MAX ms is 5124095576030 whole hours plus 1551615 ms (25 min 51.615 s).
    let text = format_tc(u64::MAX);
    assert_eq!(text, "5124095576030:25:51.615");
    assert_eq!(parse_tc(&text), Some(u64::MAX));
}

#[test]
fn format_tc_round_trips_through_parse_tc() {
    let samples = [0, 1, 999, 1_000, 59_999, 60_000, 3_599_999, 3_600_000, 605_000, 86_399_999, 360_000_000, 123_456_789_012, u64::MAX];
    for ms in samples {
        assert_eq!(parse_tc(&format_tc(ms)), Some(ms), "round trip of {ms}");
    }
}

#[test]
fn parse_tc_reads_known_values() {
    assert_eq!(parse_tc("00:00:00.000"), Some(0));
    assert_eq!(parse_tc("00:00:12.000"), Some(12_000));
    assert_eq!(parse_tc("00:10:05.000"), Some(605_000));
    assert_eq!(parse_tc("01:02:03.004"), Some(3_723_004));
    assert_eq!(parse_tc("100:00:00.000"), Some(360_000_000));
}

#[test]
fn parse_tc_rejects_bad_input() {
    let bad = [
        "",
        " ",
        "00:00:00",
        "00:00:00.00",
        "00:00:00.0000",
        "00:00:00,000",
        "0:00:00.000",
        "00:0:00.000",
        "00:00:0.000",
        "00:60:00.000",
        "00:00:60.000",
        "00:00.000",
        "00:00:00:00.000",
        "aa:00:00.000",
        "00:00:00.00a",
        "-1:00:00.000",
        "+1:00:00.000",
        "00:+1:00.000",
        "00:00:00.+10",
        " 00:00:00.000",
        "00:00:00.000 ",
        "00:00:00.000\n",
        "00:00:00.000.000",
        ":00:00.000",
        "00::00.000",
        "００:00:00.000",
        // Too big for a u64 of milliseconds: the hours overflow, the hour digits overflow,
        // and the last one is exactly 1 ms past u64::MAX.
        "5124095576031:00:00.000",
        "99999999999999999999999:00:00.000",
        "5124095576030:25:51.616",
    ];
    for text in bad {
        assert_eq!(parse_tc(text), None, "{text:?} should be rejected");
    }
}

#[test]
fn snap_to_frame_at_30000_over_1001() {
    // One frame is 1001 / 30 = 33.3667 ms.
    assert_eq!(snap_to_frame(0, 30_000, 1_001), 0);
    assert_eq!(snap_to_frame(16, 30_000, 1_001), 0);
    assert_eq!(snap_to_frame(17, 30_000, 1_001), 33);
    assert_eq!(snap_to_frame(33, 30_000, 1_001), 33);
    assert_eq!(snap_to_frame(50, 30_000, 1_001), 33);
    assert_eq!(snap_to_frame(51, 30_000, 1_001), 67);
    assert_eq!(snap_to_frame(100, 30_000, 1_001), 100);
    // Frame 30 starts at exactly 1001 ms, so one second snaps to it.
    assert_eq!(snap_to_frame(1_000, 30_000, 1_001), 1_001);
    assert_eq!(snap_to_frame(1_001, 30_000, 1_001), 1_001);
}

#[test]
fn snap_to_frame_at_60_over_1() {
    // One frame is 16.6667 ms.
    assert_eq!(snap_to_frame(0, 60, 1), 0);
    assert_eq!(snap_to_frame(8, 60, 1), 0);
    assert_eq!(snap_to_frame(9, 60, 1), 17);
    assert_eq!(snap_to_frame(17, 60, 1), 17);
    assert_eq!(snap_to_frame(50, 60, 1), 50);
    assert_eq!(snap_to_frame(1_000, 60, 1), 1_000);
    assert_eq!(snap_to_frame(3_600_000, 60, 1), 3_600_000);
}

#[test]
fn snap_to_frame_rounds_half_a_frame_up() {
    // 25 ms is exactly 1.5 frames at 60 fps, so it goes up to frame 2 (33.33 ms, shown as 33).
    assert_eq!(snap_to_frame(25, 60, 1), 33);
    // 75 ms is exactly 4.5 frames, so it goes up to frame 5 (83.33 ms, shown as 83).
    assert_eq!(snap_to_frame(75, 60, 1), 83);
    // 50 ms is exactly 1.5 frames at 30 fps, so it goes up to frame 2 (66.67 ms, shown as 67).
    assert_eq!(snap_to_frame(50, 30, 1), 67);
    // 500 ms is exactly 0.5 frames at 1 fps, so it goes up to frame 1 (1000 ms).
    assert_eq!(snap_to_frame(500, 1, 1), 1_000);
    assert_eq!(snap_to_frame(499, 1, 1), 0);
}

#[test]
fn snap_to_frame_leaves_the_value_alone_for_a_zero_rate() {
    assert_eq!(snap_to_frame(12_345, 0, 1), 12_345);
    assert_eq!(snap_to_frame(12_345, 30, 0), 12_345);
}

#[test]
fn snap_to_frame_does_not_overflow() {
    assert_eq!(snap_to_frame(u64::MAX, u32::MAX, 1), u64::MAX);
    // The nearest frame can start up to half a frame (about 17 ms) below the input.
    assert!(snap_to_frame(u64::MAX, 30_000, 1_001) >= u64::MAX - 17);
}
