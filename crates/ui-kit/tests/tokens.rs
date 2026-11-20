//! Tests for the generated design tokens: spec coverage, spot checks and the
//! committed `design/tokens.css`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::Color32;
use serde_json::Value;
use std::path::PathBuf;
use ui_kit::tokens;

/// Every token named in the Design Language spec's tables.
const SPEC_TOKENS: [&str; 48] = [
    // Color: surfaces
    "bg.ground",
    "bg.panel",
    "bg.group",
    "bg.control",
    "bg.hover",
    "bg.selected",
    "bg.row.selected",
    "bg.track.selected",
    "bg.track.alt",
    "bg.clip",
    "stroke.hairline",
    "stroke.separator",
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
    assert_eq!(tokens::color::BG_GROUND, Color32::from_rgb(0x0A, 0x0A, 0x0B));
    assert_eq!(tokens::accent::ACCENT, Color32::from_rgb(0x5B, 0x9D, 0xFF));
    assert_eq!(tokens::color::SEM_METER_CLIP, Color32::from_rgb(0xFF, 0x45, 0x3A));
    assert!((tokens::radius::PANEL - 18.0).abs() < f32::EPSILON);
    // rgba(255,255,255,.015) premultiplied: 0.015 * 255 rounds to 4.
    assert_eq!(tokens::color::BG_TRACK_ALT.a(), 4);
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
