//! Model round trip and type rules.

use marks::{ClockSource, MarkType, Sidecar};

const FIXTURE: &str = include_str!("fixtures/example.marks.json");

#[test]
fn example_round_trips_exactly() -> Result<(), serde_json::Error> {
    let sidecar: Sidecar = serde_json::from_str(FIXTURE)?;
    let mut out = serde_json::to_string_pretty(&sidecar)?;
    out.push('\n');
    assert_eq!(out, FIXTURE);
    Ok(())
}

#[test]
fn allows_range_for_all_seven_types() {
    assert!(MarkType::Highlight.allows_range());
    assert!(MarkType::Cut.allows_range());
    assert!(MarkType::Retake.allows_range());
    assert!(!MarkType::Chapter.allows_range());
    assert!(MarkType::Note.allows_range());
    assert!(MarkType::BRoll.allows_range());
    assert!(!MarkType::System.allows_range());
}

#[test]
fn multi_word_enums_use_snake_case_names() -> Result<(), serde_json::Error> {
    assert_eq!(serde_json::to_string(&MarkType::BRoll)?, "\"b_roll\"");
    assert_eq!(serde_json::to_string(&ClockSource::ObsOutput)?, "\"obs_output\"");
    Ok(())
}
