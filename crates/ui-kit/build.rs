//! Build script for ui-kit: turns `design/tokens.json` into the generated
//! token modules (`tokens.rs`) and the CSS custom properties (`tokens.css`),
//! both written to OUT_DIR only. Nothing is ever written into `src/` or
//! `design/` from here.

use serde_json::{Map, Value};
use std::fmt::Write as _;
use std::path::PathBuf;

/// The OKLCH math the runtime uses too, so the generated Blue neutrals match it.
#[path = "src/oklch.rs"]
mod oklch;

/// The design data file, relative to this crate's manifest directory.
const TOKENS_JSON: &str = "../../design/tokens.json";

/// Reports a fatal build-script error and stops the build without panicking.
fn fail(message: &str) -> ! {
    eprintln!("ui-kit build.rs: {message}");
    std::process::exit(1);
}

/// Reads an environment variable or fails the build.
fn env_or_fail(name: &str) -> PathBuf {
    match std::env::var_os(name) {
        Some(value) => PathBuf::from(value),
        None => fail(&format!("{name} is not set")),
    }
}

fn main() {
    let manifest_dir = env_or_fail("CARGO_MANIFEST_DIR");
    let out_dir = env_or_fail("OUT_DIR");
    let source = manifest_dir.join(TOKENS_JSON);
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/oklch.rs");

    let raw = match std::fs::read_to_string(&source) {
        Ok(raw) => raw,
        Err(error) => fail(&format!("cannot read {}: {error}", source.display())),
    };
    let tokens: Value = match serde_json::from_str(&raw) {
        Ok(tokens) => tokens,
        Err(error) => fail(&format!("cannot parse {}: {error}", source.display())),
    };

    if let Err(error) = std::fs::write(out_dir.join("tokens.rs"), generate_rust(&tokens)) {
        fail(&format!("cannot write tokens.rs: {error}"));
    }
    if let Err(error) = std::fs::write(out_dir.join("tokens.css"), generate_css(&tokens)) {
        fail(&format!("cannot write tokens.css: {error}"));
    }
}

/// Returns the object for one top-level group, or fails the build.
fn group<'a>(tokens: &'a Value, name: &str) -> &'a Map<String, Value> {
    match tokens.get(name).and_then(Value::as_object) {
        Some(map) => map,
        None => fail(&format!("group {name} is missing in {TOKENS_JSON}")),
    }
}

/// Builds the Rust constant name for a token: drops the group prefix, turns
/// dots and hyphens into underscores and uppercases.
fn const_name(group_name: &str, token: &str) -> String {
    let body = token.strip_prefix(&format!("{group_name}.")).unwrap_or(token);
    body.replace(['.', '-'], "_").to_uppercase()
}

/// Builds the CSS custom property name for a token: `--<group>-<token>` with
/// dots turned into hyphens.
fn css_name(group_name: &str, token: &str) -> String {
    let body = token.strip_prefix(&format!("{group_name}.")).unwrap_or(token);
    format!("--{}-{}", group_name.replace('.', "-"), body.replace('.', "-"))
}

/// Returns the `(value, use text)` pair of a leaf token entry.
fn leaf(entry: &Value) -> (&Value, &str) {
    let map = match entry.as_object() {
        Some(map) => map,
        None => fail("token entry is not an object"),
    };
    let value = match map.get("value") {
        Some(value) => value,
        None => fail("token entry is missing its value"),
    };
    (value, map.get("use").and_then(Value::as_str).unwrap_or(""))
}

/// Renders one doc comment line for a token.
fn doc_line(token: &str, use_text: &str, indent: usize) -> String {
    let clean = use_text.replace(['\n', '\r'], " ");
    format!("{}/// `{token}` - {clean}\n", "    ".repeat(indent))
}

/// Formats a number as a Rust f32 literal.
fn f32_literal(value: f64) -> String {
    if value.fract() == 0.0 { format!("{value}.0") } else { format!("{value}") }
}

/// Parses a spec color: `#RRGGBB` or `rgba(r,g,b,a)`. Returns premultiplied
/// RGBA8 so the generated `Color32` constructors stay const.
fn parse_color(text: &str) -> (u8, u8, u8, u8) {
    if let Some(hex) = text.strip_prefix('#') {
        let byte = |range: std::ops::Range<usize>| -> u8 {
            match hex.get(range).and_then(|part| u8::from_str_radix(part, 16).ok()) {
                Some(byte) => byte,
                None => fail(&format!("bad hex color {text:?}")),
            }
        };
        (byte(0..2), byte(2..4), byte(4..6), 255)
    } else if let Some(inner) = text.strip_prefix("rgba(").and_then(|rest| rest.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            fail(&format!("bad rgba color {text:?}"));
        }
        let number = |index: usize| -> f64 {
            match parts[index].parse::<f64>() {
                Ok(value) => value,
                Err(error) => fail(&format!("bad rgba color {text:?}: {error}")),
            }
        };
        let (r, g, b, a) = (number(0), number(1), number(2), number(3));
        let alpha = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        let premultiplied = |channel: f64| (channel * a).round().clamp(0.0, 255.0) as u8;
        (premultiplied(r), premultiplied(g), premultiplied(b), alpha)
    } else {
        fail(&format!("unsupported color {text:?}"))
    }
}

/// Emits `Color32::from_rgb(..)` or `Color32::from_rgba_premultiplied(..)`.
fn color_expr(text: &str) -> String {
    let (r, g, b, a) = parse_color(text);
    rgba_expr([r, g, b, a])
}

/// The OKLCH hue of the default Blue preset: the hue of the generated neutral
/// constants and CSS properties.
fn default_hue(tokens: &Value) -> f32 {
    let preset = match group(tokens, "accent").get("preset.blue") {
        Some(entry) => leaf(entry).0,
        None => fail("accent token preset.blue is missing"),
    };
    let Some(text) = preset.as_str() else { fail("accent token preset.blue is not a string") };
    let (r, g, b, _) = parse_color(text);
    oklch::rgb_to_oklch([r, g, b]).h
}

/// Reads a neutral color value as `(l, c, alpha)`: either `{ l, c }`, or
/// `{ base, alpha }`, which takes `l` and `c` from the base neutral.
fn neutral_lca(tokens: &Value, token: &str, value: &Map<String, Value>) -> (f32, f32, f32) {
    let number = |map: &Map<String, Value>, name: &str| -> f32 {
        match map.get(name).and_then(Value::as_f64) {
            Some(number) => number as f32,
            None => fail(&format!("neutral token {token} has no numeric {name}")),
        }
    };
    let Some(base) = value.get("base").and_then(Value::as_str) else {
        return (number(value, "l"), number(value, "c"), 1.0);
    };
    let base_value = group(tokens, "color").values().find_map(|entries| entries.get(base)).map(|entry| leaf(entry).0);
    match base_value.and_then(Value::as_object) {
        Some(map) => (number(map, "l"), number(map, "c"), number(value, "alpha")),
        None => fail(&format!("neutral token {token} has no neutral base {base}")),
    }
}

/// Emits `Color32::from_rgb(..)` or `Color32::from_rgba_premultiplied(..)`
/// from premultiplied bytes.
fn rgba_expr([r, g, b, a]: [u8; 4]) -> String {
    if a == 255 { format!("Color32::from_rgb({r}, {g}, {b})") } else { format!("Color32::from_rgba_premultiplied({r}, {g}, {b}, {a})") }
}

/// Generates the whole Rust module tree from the token groups.
fn generate_rust(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("// Generated by crates/ui-kit/build.rs from design/tokens.json. Do not edit by hand.\n\n");

    // Color: nested subgroup maps, every key is a full token name. A string
    // value is a fixed color. An object value is a neutral: its constant here
    // has the Blue hue, and its (l, c, alpha) goes into the `neutral` module.
    let hue = default_hue(tokens);
    let mut neutral = String::new();
    neutral.push_str("/// The neutral tokens as OKLCH `(l, c, alpha)`; the hue comes from the accent at runtime.\npub mod neutral {\n");
    let mut color = String::new();
    color.push_str("/// Color tokens: surfaces, text, meaning, marker categories and glass.\npub mod color {\n    use ::egui::Color32;\n\n");
    for (subgroup, entries) in group(tokens, "color") {
        color.push_str(&format!("    // {subgroup}\n"));
        let map = match entries.as_object() {
            Some(map) => map,
            None => fail(&format!("color subgroup {subgroup} is not an object")),
        };
        for (token, entry) in map {
            let (value, use_text) = leaf(entry);
            let name = const_name("color", token);
            let expr = match value {
                Value::String(text) => color_expr(text),
                Value::Object(map) => {
                    let lca = neutral_lca(tokens, token, map);
                    neutral.push_str(&doc_line(token, use_text, 1));
                    let _ = writeln!(neutral, "    pub const {name}: (f32, f32, f32) = ({:?}, {:?}, {:?});", lca.0, lca.1, lca.2);
                    rgba_expr(oklch::neutral_rgba(lca, Some(hue)))
                }
                _ => fail(&format!("color token {token} is not a string or a neutral")),
            };
            color.push_str(&doc_line(token, use_text, 1));
            let _ = writeln!(color, "    pub const {name}: Color32 = {expr};");
        }
    }
    color.push_str("}\n\n");
    out.push_str(&color);
    neutral.push_str("}\n\n");
    out.push_str(&neutral);

    // Accent: flat map of colors.
    let mut accent = String::new();
    accent.push_str("/// Accent tokens: the default Blue values and the picker presets.\npub mod accent {\n    use ::egui::Color32;\n\n");
    for (token, entry) in group(tokens, "accent") {
        let (value, use_text) = leaf(entry);
        let text = match value.as_str() {
            Some(text) => text,
            None => fail(&format!("accent token {token} is not a string")),
        };
        accent.push_str(&doc_line(token, use_text, 1));
        let _ = writeln!(accent, "    pub const {}: Color32 = {};", const_name("accent", token), color_expr(text));
    }
    accent.push_str("}\n\n");
    out.push_str(&accent);

    // Type: family, size, weight and letter spacing per style. The group is
    // named `typography` in Rust because `type` is a keyword.
    let mut typography = String::new();
    typography.push_str("/// Type tokens: family, size, weight and letter spacing per text style.\n");
    typography.push_str("/// The JSON group is named \"type\"; the Rust module is `typography` because `type` is a keyword.\npub mod typography {\n");
    for (token, entry) in group(tokens, "type") {
        let (value, use_text) = leaf(entry);
        let map = match value.as_object() {
            Some(map) => map,
            None => fail(&format!("type token {token} is not an object")),
        };
        let field = |name: &str| -> &Value {
            match map.get(name) {
                Some(field) => field,
                None => fail(&format!("type token {token} is missing {name}")),
            }
        };
        let family = match field("family").as_str() {
            Some(family) => family,
            None => fail(&format!("type token {token} has no family string")),
        };
        let number = |name: &str| -> f64 {
            match field(name).as_f64() {
                Some(number) => number,
                None => fail(&format!("type token {token} has no numeric {name}")),
            }
        };
        let name = const_name("type", token);
        typography.push_str(&doc_line(token, use_text, 1));
        let _ = writeln!(typography, "    /// Family key for `{token}`.\n    pub const {name}_FAMILY: &str = {family:?};");
        let _ = writeln!(typography, "    pub const {name}_SIZE: f32 = {};", f32_literal(number("size")));
        let _ = writeln!(typography, "    pub const {name}_WEIGHT: f32 = {};", f32_literal(number("weight")));
        if map.get("letter_spacing_em").and_then(Value::as_f64).is_some() {
            let _ = writeln!(typography, "    pub const {name}_LETTER_SPACING: f32 = {};", f32_literal(number("letter_spacing_em")));
        }
    }
    typography.push_str("}\n\n");
    out.push_str(&typography);

    out.push_str(&space_module(tokens));
    out.push_str(&size_module(tokens));
    out.push_str(&radius_module(tokens));
    out.push_str(&motion_module(tokens));
    out
}

/// Builds the `space` module: the scale plus the padding rules.
fn space_module(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("/// Spacing tokens: the scale and the padding rules.\npub mod space {\n");
    for (token, entry) in group(tokens, "space") {
        let (value, use_text) = leaf(entry);
        if let Some(steps) = value.as_array() {
            for step in steps {
                let number = match step.as_f64() {
                    Some(number) => number,
                    None => fail(&format!("space token {token} holds a non-number step")),
                };
                out.push_str(&doc_line(&format!("{token}.{number}"), use_text, 1));
                let _ = writeln!(out, "    pub const SCALE_{}: f32 = {};", const_name("", &format!("{number}")), f32_literal(number));
            }
        } else {
            let number = match value.as_f64() {
                Some(number) => number,
                None => fail(&format!("space token {token} is not a number or list")),
            };
            out.push_str(&doc_line(token, use_text, 1));
            let _ = writeln!(out, "    pub const {}: f32 = {};", const_name("space", token), f32_literal(number));
        }
    }
    out.push_str("}\n\n");
    out
}

/// Builds the `size` module from the element sizes table.
fn size_module(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("/// Element size tokens.\npub mod size {\n");
    for (token, entry) in group(tokens, "size") {
        let (value, use_text) = leaf(entry);
        let number = match value.as_f64() {
            Some(number) => number,
            None => fail(&format!("size token {token} is not a number")),
        };
        out.push_str(&doc_line(token, use_text, 1));
        let _ = writeln!(out, "    pub const {}: f32 = {};", const_name("size", token), f32_literal(number));
    }
    out.push_str("}\n\n");
    out
}

/// Builds the `radius` module: numbers become consts, string rules only get a
/// doc comment.
fn radius_module(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("/// Corner radius tokens.\npub mod radius {\n");
    for (token, entry) in group(tokens, "radius") {
        out.push_str(&radius_entry(token, entry, 1));
    }
    out.push_str("}\n\n");
    out
}

/// Emits one radius token: a number, a nested map (clip audio/video), or a
/// string rule that only produces a doc comment.
fn radius_entry(token: &str, entry: &Value, indent: usize) -> String {
    let pad = "    ".repeat(indent);
    if let Some(sub) = entry.as_object().filter(|map| !map.contains_key("value")) {
        let mut out = String::new();
        for (part, inner) in sub {
            out.push_str(&radius_entry(&format!("{token}.{part}"), inner, indent));
        }
        return out;
    }
    let (value, use_text) = leaf(entry);
    let mut out = String::new();
    if let Some(number) = value.as_f64() {
        out.push_str(&doc_line(token, use_text, indent));
        let _ = writeln!(out, "{pad}pub const {}: f32 = {};", const_name("radius", token), f32_literal(number));
    } else {
        let clean = use_text.replace(['\n', '\r'], " ");
        let _ = writeln!(out, "{pad}// `{token}` - {clean}. Rule, not a number: no constant emitted.");
    }
    out
}

/// Builds the `motion` module: durations in ms, curve in the doc comment.
fn motion_module(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("/// Motion tokens: durations in milliseconds.\npub mod motion {\n");
    for (token, entry) in group(tokens, "motion") {
        let (value, use_text) = leaf(entry);
        let map = match value.as_object() {
            Some(map) => map,
            None => fail(&format!("motion token {token} is not an object")),
        };
        let ms = match map.get("ms").and_then(Value::as_u64) {
            Some(ms) => ms,
            None => fail(&format!("motion token {token} has no ms number")),
        };
        let curve = map.get("curve").and_then(Value::as_str).unwrap_or("");
        out.push_str(&doc_line(token, &format!("{use_text} (curve: {curve})"), 1));
        let _ = writeln!(out, "    pub const {}_MS: u32 = {ms};", const_name("motion", token));
    }
    out.push_str("}\n\n");
    out
}

/// Generates the CSS custom properties from the token groups.
fn generate_css(tokens: &Value) -> String {
    let mut out = String::new();
    out.push_str("/* Generated by crates/ui-kit/build.rs from design/tokens.json. Do not edit by hand. */\n");
    out.push_str("/* Regenerate with: $env:UPDATE_TOKENS_CSS = \"1\"; cargo test -p ui-kit */\n");
    out.push_str("/* Neutrals have the default Blue hue here. Apps tint them at runtime from their accent. */\n");
    let hue = default_hue(tokens);
    out.push_str(":root {\n");
    for group_name in ["color", "accent", "type", "space", "size", "radius", "motion"] {
        out.push_str(&format!("    /* {group_name} */\n"));
        if group_name == "color" {
            // The subgroup maps (surfaces, text, ...) organize the table only;
            // the token names inside carry no subgroup prefix.
            for entries in group(tokens, group_name).values() {
                let map = match entries.as_object() {
                    Some(map) => map,
                    None => fail("color subgroup is not an object"),
                };
                for (token, entry) in map {
                    match leaf(entry).0 {
                        Value::Object(value) => out.push_str(&css_neutral(group_name, token, neutral_lca(tokens, token, value), hue)),
                        _ => out.push_str(&css_entry(group_name, token, entry)),
                    }
                }
            }
        } else {
            for (token, entry) in group(tokens, group_name) {
                out.push_str(&css_entry(group_name, token, entry));
            }
        }
    }
    out.push_str("}\n");
    out
}

/// Emits the CSS line for one neutral: hex when opaque, `rgba()` otherwise.
fn css_neutral(group_name: &str, token: &str, lca: (f32, f32, f32), hue: f32) -> String {
    let name = css_name(group_name, token);
    let [r, g, b, _] = oklch::neutral_rgba((lca.0, lca.1, 1.0), Some(hue));
    if lca.2 < 1.0 { format!("    {name}: rgba({r},{g},{b},{});\n", lca.2) } else { format!("    {name}: #{r:02X}{g:02X}{b:02X};\n") }
}

/// Emits the CSS lines for one token entry, recursing into nested maps.
fn css_entry(group_name: &str, token: &str, entry: &Value) -> String {
    if let Some(sub) = entry.as_object().filter(|map| !map.contains_key("value")) {
        let mut out = String::new();
        for (part, inner) in sub {
            out.push_str(&css_entry(group_name, &format!("{token}.{part}"), inner));
        }
        return out;
    }
    let (value, use_text) = leaf(entry);
    let name = css_name(group_name, token);
    match value {
        Value::String(text) if text.starts_with('#') || text.starts_with("rgba(") => format!("    {name}: {text};\n"),
        Value::String(rule) => format!("    /* {name}: {rule} ({use_text}) */\n"),
        Value::Array(steps) => {
            let mut out = String::new();
            for step in steps {
                if let Some(number) = step.as_f64() {
                    let _ = writeln!(out, "    {}-{}: {}px;", name, const_name("", &format!("{number}")).to_lowercase(), number);
                }
            }
            out
        }
        Value::Object(fields) => {
            let mut out = String::new();
            for (part, field) in fields {
                match (part.as_str(), field) {
                    ("ms", Value::Number(ms)) => {
                        let _ = writeln!(out, "    {name}: {}ms;", ms);
                    }
                    ("curve", Value::String(curve)) => {
                        let _ = writeln!(out, "    {name}-curve: {curve};");
                    }
                    ("family", Value::String(family)) => {
                        let _ = writeln!(out, "    {name}-family: {family};");
                    }
                    ("size", Value::Number(size)) => {
                        let _ = writeln!(out, "    {name}-size: {}px;", size);
                    }
                    ("weight", Value::Number(weight)) => {
                        let _ = writeln!(out, "    {name}-weight: {weight};");
                    }
                    ("letter_spacing_em", Value::Number(spacing)) => {
                        let _ = writeln!(out, "    {name}-letter-spacing: {}em;", spacing);
                    }
                    _ => {}
                }
            }
            out
        }
        Value::Number(number) => format!("    {name}: {}px;\n", number),
        _ => fail(&format!("cannot render token {group_name}.{token} as CSS")),
    }
}
