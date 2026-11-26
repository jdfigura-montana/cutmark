//! Tests for the generated design tokens: spec coverage, spot checks and the
//! committed `design/tokens.css`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::Color32;
use serde_json::Value;
use std::path::PathBuf;
use ui_kit::tokens;

/// Every token named in the Design Language spec's tables.
const SPEC_TOKENS: [&str; 57] = [
    // Color: surfaces
    "bg.ground",
    "bg.side",
    "bg.panel",
    "bg.stage.inner",
    "bg.group",
    "bg.control",
    "bg.hover",
    "bg.selected",
    "bg.row.selected",
    "bg.track.selected",
    "bg.track.header",
    "bg.track.header.selected",
    "bg.track.alt",
    "bg.track.empty",
    "bg.clip",
    "stroke.hairline",
    "stroke.separator",
    "stroke.separator.strong",
    "stroke.track",
    "stroke.track.header",
    "stroke.clip",
    // Color: text
    "text.primary",
    "text.secondary",
    "text.tertiary",
    "text.disabled",
    // Color: meaning
    "sem.record",
    "sem.meter.ok",
    "sem.meter.warn",
    "sem.meter.clip",
    "sem.success",
    "sem.pending",
    // Marker colors
    "marker.note",
    "marker.retake",
    "marker.cut",
    "marker.broll",
    "marker.system",
    // Accent (default Blue values)
    "accent",
    "accent.hover",
    "accent.pressed",
    "accent.on",
    "accent.tint",
    "accent.content",
    // Typography
    "type.title",
    "type.headline",
    "type.body",
    "type.caption",
    "type.micro",
    "type.time.large",
    "type.value",
    // Radii
    "radius.window",
    "radius.panel",
    "radius.side",
    "radius.group",
    "radius.control",
    "radius.segment",
    "radius.clip",
    "radius.capsule",
];

/// Reads `design/tokens.json` from the repo root.
fn tokens_json() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../design/tokens.json");
    let raw = std::fs::read_to_string(path).expect("design/tokens.json must be readable");
    serde_json::from_str(&raw).expect("design/tokens.json must be valid JSON")
}

/// True when the token name appears as a key anywhere in the JSON tree.
fn contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map.contains_key(key) || map.values().any(|v| contains_key(v, key)),
        Value::Array(items) => items.iter().any(|v| contains_key(v, key)),
        _ => false,
    }
}

#[test]
fn every_spec_token_exists_in_tokens_json() {
    let json = tokens_json();
    for token in SPEC_TOKENS {
        assert!(contains_key(&json, token), "token {token} is missing in design/tokens.json");
    }
}

#[test]
fn spot_checks_match_the_spec() {
    // A neutral's constant carries the default Blue hue.
    assert_eq!(tokens::neutral::BG_GROUND, (0.141, 0.006, 1.0));
    assert_eq!(tokens::neutral::STROKE_HAIRLINE, (0.963, 0.0075, 0.07));
    assert_eq!(tokens::accent::ACCENT, Color32::from_rgb(0x5B, 0x9D, 0xFF));
    assert_eq!(tokens::color::SEM_METER_CLIP, Color32::from_rgb(0xFF, 0x4D, 0x3D));
    assert!((tokens::radius::PANEL - 18.0).abs() < f32::EPSILON);
    assert!((tokens::radius::SIDE - 12.0).abs() < f32::EPSILON);
    assert!((tokens::radius::GROUP - 8.0).abs() < f32::EPSILON);
    // rgba(255,255,255,.025) premultiplied: 0.025 * 255 rounds to 6.
    assert_eq!(tokens::color::BG_TRACK_ALT.a(), 6);
    assert_eq!(tokens::color::STROKE_HAIRLINE.a(), 18);
    assert_eq!(tokens::color::STROKE_SEPARATOR.a(), 13);
    // rgba(255,255,255,.10) premultiplied: alpha 26 (0.10 * 255 = 25.5 rounds up).
    assert_eq!(tokens::color::STROKE_SEPARATOR_STRONG.a(), 26);
    assert_eq!(tokens::color::STROKE_TRACK.a(), 18);
    assert_eq!(tokens::color::STROKE_TRACK_HEADER.a(), 20);
    // Rev 3: the title bar is 44 tall and the caption buttons are 46 x 44.
    assert!((tokens::size::TOOLBAR_MAIN_HEIGHT - 44.0).abs() < f32::EPSILON);
    assert!((tokens::size::CAPTION_BUTTON_HEIGHT - 44.0).abs() < f32::EPSILON);
}

#[test]
fn committed_tokens_css_matches_the_generated_css() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../design/tokens.css");
    if std::env::var("UPDATE_TOKENS_CSS").is_ok_and(|value| value == "1") {
        std::fs::write(&path, tokens::TOKENS_CSS).expect("design/tokens.css must be writable");
        return;
    }
    let committed = std::fs::read_to_string(&path).expect("design/tokens.css must be readable");
    assert_eq!(committed, tokens::TOKENS_CSS, "design/tokens.css is stale; regenerate with UPDATE_TOKENS_CSS=1");
}
