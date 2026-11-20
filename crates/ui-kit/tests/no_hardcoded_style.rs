//! Style lint for opted-in apps: hard-coded colors, radii, font sizes and any
//! use of the retired `ui_kit::palette` module are build failures. Spec row
//! "Style lint" and section "Phase 5" of S1-Audio-Editor-ui-kit.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

/// Apps join here when their restyle is done (audio editor in phase 13).
const OPT_IN: &[&str] = &[];

/// Marker that lets a single line opt out of the lint.
const ALLOW_MARKER: &str = "// style-lint: allow";

/// One lint hit: workspace-relative file, 1-based line, rule and the raw line.
struct Finding {
    file: String,
    line: usize,
    rule: &'static str,
    text: String,
}

/// One skipped `// style-lint: allow <reason>` line, kept for the test output.
struct Allowed {
    file: String,
    line: usize,
    text: String,
}

/// Workspace root, two levels above this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("ui-kit sits two levels under the workspace root").to_path_buf()
}

/// Scans every `.rs` file under `dir` and returns the lint findings.
fn scan(dir: &Path) -> Vec<Finding> {
    scan_dir(dir).0
}

/// Scans every `.rs` file under `dir`, also returning the allowed lines.
fn scan_dir(dir: &Path) -> (Vec<Finding>, Vec<Allowed>) {
    let mut files = Vec::new();
    collect_rs_files(dir, &mut files);
    files.sort();
    let mut findings = Vec::new();
    let mut allowed = Vec::new();
    for path in files {
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        let display = path.strip_prefix(workspace_root()).unwrap_or(&path).display().to_string();
        for (index, line) in source.lines().enumerate() {
            let number = index + 1;
            if line.trim_start().starts_with("//") {
                continue;
            }
            if let Some(reason) = allow_reason(line) {
                allowed.push(Allowed { file: display.clone(), line: number, text: format!("{line} ({reason})") });
                continue;
            }
            if let Some(rule) = rule_for(line) {
                findings.push(Finding { file: display.clone(), line: number, rule, text: line.trim().to_string() });
            }
        }
    }
    (findings, allowed)
}

/// Collects `.rs` files under `dir` recursively; unreadable entries are skipped.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The reason text of an allow marker, present only when a reason follows it.
fn allow_reason(line: &str) -> Option<String> {
    let start = line.find(ALLOW_MARKER)?;
    let reason = line[start + ALLOW_MARKER.len()..].trim();
    if reason.is_empty() { None } else { Some(reason.to_string()) }
}

/// The first rule the line violates, checked in a fixed order: one hit per line.
fn rule_for(line: &str) -> Option<&'static str> {
    if line.contains("Color32::from_rgb")
        || line.contains("Color32::from_rgba")
        || line.contains("Color32::from_gray")
        || line.contains("Color32::from_hex")
        || contains_hex_literal(line)
    {
        return Some("color");
    }
    if line.contains("CornerRadius::same(") || line.contains("Rounding::same(") || followed_by_digit(line, ".corner_radius(") {
        return Some("radius");
    }
    if followed_by_digit(line, "FontId::new(") || followed_by_digit(line, "FontId::proportional(") || followed_by_digit(line, "FontId::monospace(") {
        return Some("font size");
    }
    if line.contains("ui_kit::palette") || line.contains("palette::") {
        return Some("palette");
    }
    None
}

/// True when the line's needle is directly followed by an ASCII digit.
fn followed_by_digit(line: &str, needle: &str) -> bool {
    match line.find(needle) {
        Some(start) => line[start + needle.len()..].chars().next().is_some_and(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// True when any string literal on the line holds `#` plus exactly 6 or 8 hex
/// digits. The quote-splitting ignores escapes, which is fine for source scans.
fn contains_hex_literal(line: &str) -> bool {
    line.split('"').enumerate().any(|(index, part)| index % 2 == 1 && holds_hex_color(part))
}

/// True when the string-literal content holds a `#` followed by a hex run of
/// exactly 6 or 8 digits.
fn holds_hex_color(part: &str) -> bool {
    let bytes = part.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'#' {
            continue;
        }
        let run = bytes[index + 1..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
        if run == 6 || run == 8 {
            return true;
        }
    }
    false
}

/// Formats one finding as `file:line rule text` for the failure message.
fn format_finding(finding: &Finding) -> String {
    format!("{}:{} {} {}", finding.file, finding.line, finding.rule, finding.text)
}

#[test]
fn hardcoded_style_in_opted_in_apps() {
    let root = workspace_root();
    for app in OPT_IN {
        let dir = root.join(app);
        assert!(dir.is_dir(), "opt-in path {app} does not exist");
        let findings = scan(&dir);
        let mut listed = String::new();
        for finding in &findings {
            listed.push_str(&format_finding(finding));
            listed.push('\n');
        }
        assert!(findings.is_empty(), "hard-coded style values found in {app}:\n{listed}");
    }
}

/// One file with every planted violation, plus a commented-out hex and an
/// allowed line; a second clean file must add nothing.
const PLANTED: &str = "\
let tint = \"#AABBCC\";
let tint = Color32::from_rgb(0xAA, 0xBB, 0xCC);
let radius = CornerRadius::same(6.0);
let font = FontId::proportional(13.0);
use ui_kit::palette::ACCENT;
// let old = \"#112233\";
let keep = Color32::from_rgb(1, 2, 3); // style-lint: allow demo color until the restyle
";

#[test]
fn scanner_catches_planted_values() {
    let dir = std::env::temp_dir().join(format!("ui-kit-style-lint-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("stale temp scan folder must be removable");
    }
    fs::create_dir_all(&dir).expect("temp scan folder must be creatable");
    fs::write(dir.join("planted.rs"), PLANTED).expect("planted file must be writable");
    fs::write(dir.join("clean.rs"), "let sum = 2 + 2;\n").expect("clean file must be writable");

    let (findings, allowed) = scan_dir(&dir);

    for entry in &allowed {
        eprintln!("skipped allowed line {}:{} {}", entry.file, entry.line, entry.text);
    }
    eprintln!("allowed lines skipped: {}", allowed.len());
    assert_eq!(allowed.len(), 1, "exactly one allow line must be skipped");

    for finding in &findings {
        eprintln!("found {}", format_finding(finding));
    }
    let rules: Vec<&str> = findings.iter().map(|f| f.rule).collect();
    assert_eq!(rules, ["color", "color", "radius", "font size", "palette"], "planted file must yield exactly these five findings");

    fs::remove_dir_all(&dir).expect("temp scan folder must be removable");
}
