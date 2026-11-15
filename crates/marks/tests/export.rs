//! Golden tests for the derived outputs, plus the edges the golden fixture does not reach.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use marks::{Chapter, MarkType, Sidecar, Source, chapters, csv, load, markdown};

type TestResult = Result<(), Box<dyn Error>>;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

/// Loads the full fixture and checks that the reader has nothing to say about it.
fn load_full() -> Result<Sidecar, Box<dyn Error>> {
    let (sidecar, warnings) = load(&fixture_path("full.marks.json"))?;
    assert!(warnings.is_empty(), "the full fixture should load without warnings: {warnings:?}");
    Ok(sidecar)
}

/// Compares bytes, and on a mismatch prints both sides with escapes so CR, LF and the BOM are visible.
fn assert_bytes_eq(actual: &[u8], expected: &[u8], what: &str) {
    assert!(
        actual == expected,
        "{what} differs from its golden file\n  actual: {:?}\nexpected: {:?}",
        String::from_utf8_lossy(actual),
        String::from_utf8_lossy(expected)
    );
}

#[test]
fn markdown_matches_the_golden_file() -> TestResult {
    let sidecar = load_full()?;
    let golden = fs::read(fixture_path("full_marks.md"))?;
    assert_bytes_eq(markdown(&sidecar).as_bytes(), &golden, "markdown");
    Ok(())
}

#[test]
fn csv_matches_the_golden_file() -> TestResult {
    let sidecar = load_full()?;
    let golden = fs::read(fixture_path("full_marks.csv"))?;
    assert_bytes_eq(&csv(&sidecar), &golden, "csv");
    Ok(())
}

#[test]
fn outputs_are_deterministic() -> TestResult {
    let sidecar = load_full()?;
    assert_eq!(markdown(&sidecar), markdown(&sidecar));
    assert_eq!(csv(&sidecar), csv(&sidecar));
    Ok(())
}

#[test]
fn chapters_for_media_0() -> TestResult {
    let sidecar = load_full()?;
    assert_eq!(chapters(&sidecar, 0), vec![Chapter { start_ms: 12_000, end_ms: 600_000, title: "Warm-up".to_owned() }]);
    Ok(())
}

#[test]
fn chapters_for_media_1() -> TestResult {
    let sidecar = load_full()?;
    assert_eq!(chapters(&sidecar, 1), vec![Chapter { start_ms: 5_000, end_ms: 300_000, title: "Chapter 1".to_owned() }]);
    Ok(())
}

#[test]
fn chapters_for_a_media_index_that_does_not_exist_is_empty() -> TestResult {
    let sidecar = load_full()?;
    assert_eq!(chapters(&sidecar, 2), Vec::<Chapter>::new());
    Ok(())
}

#[test]
fn chapters_are_sorted_chain_their_ends_and_number_from_one() -> TestResult {
    let mut sidecar = load_full()?;
    // Added out of order: the unlabeled one first, then a labeled one that sorts before it.
    let mut unlabeled = sidecar.marks[0].clone();
    unlabeled.t_ms = 400_000;
    unlabeled.label = Some(String::new());
    let mut second = sidecar.marks[0].clone();
    second.t_ms = 300_000;
    second.label = Some("Second".to_owned());
    sidecar.marks.push(unlabeled);
    sidecar.marks.push(second);

    assert_eq!(
        chapters(&sidecar, 0),
        vec![
            Chapter { start_ms: 12_000, end_ms: 300_000, title: "Warm-up".to_owned() },
            Chapter { start_ms: 300_000, end_ms: 400_000, title: "Second".to_owned() },
            Chapter { start_ms: 400_000, end_ms: 600_000, title: "Chapter 3".to_owned() },
        ]
    );
    // Media 1 is untouched, and its numbering starts again at 1.
    assert_eq!(chapters(&sidecar, 1).len(), 1);
    Ok(())
}

#[test]
fn markdown_title_falls_back_to_the_media_base_name() -> TestResult {
    let mut sidecar = load_full()?;
    sidecar.context = None;
    let text = markdown(&sidecar);
    assert_eq!(text.lines().nth(1), Some("# 2026-10-28_ranked-grind"));
    Ok(())
}

#[test]
fn markdown_without_scripts_has_no_script_column() -> TestResult {
    let mut sidecar = load_full()?;
    for mark in &mut sidecar.marks {
        mark.script = None;
    }
    let text = markdown(&sidecar);
    assert!(text.contains("\n| # | Time | Type | Label | Notes |\n| --- | --- | --- | --- | --- |\n"), "{text}");
    assert!(text.contains("\n| 5 | 00:04:10.000 | Retake | Café take |  |\n"), "{text}");
    assert!(!text.contains("Script"), "{text}");

    let csv_text = String::from_utf8(csv(&sidecar))?;
    assert!(csv_text.starts_with("\u{feff}index,session_ms,media,t_ms,end_ms,timecode,end_timecode,type,label,notes,source\r\n"), "{csv_text:?}");
    Ok(())
}

#[test]
fn markdown_without_chapters_has_no_chapters_section() -> TestResult {
    let mut sidecar = load_full()?;
    sidecar.marks.retain(|mark| mark.mark_type != MarkType::Chapter);
    let text = markdown(&sidecar);
    assert!(!text.contains("## Chapters"), "{text}");
    assert!(text.contains("- Duration: 00:15:00.000\n\n## Marks\n\n| # |"), "{text}");
    // The rows renumber from 1 without the chapter marks.
    assert!(text.contains("\n| 1 | 00:00:30.000 to 00:01:00.000 | Highlight |"), "{text}");
    Ok(())
}

#[test]
fn an_original_type_replaces_the_type_name_in_both_outputs() -> TestResult {
    let mut sidecar = load_full()?;
    sidecar.marks[3].original_type = Some("clip".to_owned());
    assert!(markdown(&sidecar).contains("\n| 4 | 00:02:00.000 | clip | Fix audio |"));
    let csv_text = String::from_utf8(csv(&sidecar))?;
    assert!(csv_text.contains("\r\n3,120000,0,120000,,00:02:00.000,,clip,Fix audio,"), "{csv_text:?}");
    Ok(())
}

#[test]
fn csv_type_and_source_names_match_the_serde_names() -> TestResult {
    let base = load_full()?;
    let types = [MarkType::Highlight, MarkType::Cut, MarkType::Retake, MarkType::Chapter, MarkType::Note, MarkType::BRoll, MarkType::System];
    let sources = [Source::Hotkey, Source::Ui, Source::Api, Source::Guest, Source::Auto];
    for mark_type in types {
        for source in sources {
            let mut mark = base.marks[0].clone();
            mark.mark_type = mark_type;
            mark.source = source;
            let mut sidecar = base.clone();
            sidecar.marks = vec![mark];

            let text = String::from_utf8(csv(&sidecar))?;
            let row = text.lines().nth(1).ok_or("the csv has no data row")?;
            let fields: Vec<&str> = row.split(',').collect();
            assert_eq!(serde_json::to_value(mark_type)?.as_str(), fields.get(7).copied(), "type name of {mark_type:?}");
            assert_eq!(serde_json::to_value(source)?.as_str(), fields.get(10).copied(), "source name of {source:?}");
        }
    }
    Ok(())
}
