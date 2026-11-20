//! The suite-wide appearance file (`appearance.json`): read, write and poll,
//! so every running Cutmark app follows one accent and glass setting
//! (spec: Design Language "Accent picker" rules, S1 row "Appearance file").
//!
//! The file stores only the raw user inputs (the pick as `#RRGGBB` and the
//! glass switch); the derived accent tokens are recomputed with
//! [`crate::accent::derive`] on load. Every problem (missing file, unreadable
//! file, bad JSON, unknown version, bad hex) falls back to the default theme:
//! loading never panics and never returns an error.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::theme::Theme;

/// The file format version this module reads and writes.
const VERSION: u32 = 1;
/// The file name inside the Cutmark config folder.
const FILE_NAME: &str = "appearance.json";
/// How far apart two polls may be (spec: at most once per second).
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The on-disk JSON: version, raw pick, glass switch.
#[derive(serde::Serialize, serde::Deserialize)]
struct File {
    version: u32,
    accent_pick: String,
    glass: bool,
}

/// Where the file lives: `CUTMARK_APPEARANCE_PATH` if set (tests use it),
/// else the Cutmark folder in the OS config directory.
pub fn path() -> Option<PathBuf> {
    if let Ok(from_env) = std::env::var("CUTMARK_APPEARANCE_PATH")
        && !from_env.is_empty()
    {
        return Some(PathBuf::from(from_env));
    }
    config_folder().map(|folder| folder.join(FILE_NAME))
}

/// The Cutmark config folder for this OS.
fn config_folder() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("Cutmark"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support").join("Cutmark"))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if let Some(base) = std::env::var_os("XDG_CONFIG_HOME") {
            let base = PathBuf::from(base);
            if base.is_absolute() {
                return Some(base.join("cutmark"));
            }
        }
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").join("cutmark"))
    }
}

/// Loads the theme from the file, or the default for any problem.
pub fn load() -> Theme {
    let Some(path) = path() else { return Theme::default() };
    load_from_str(&fs::read_to_string(path).unwrap_or_default())
}

/// Parses file text into a theme; any bad input falls back to the default.
fn load_from_str(text: &str) -> Theme {
    let Ok(file) = serde_json::from_str::<File>(text) else { return Theme::default() };
    if file.version != VERSION {
        return Theme::default();
    }
    let Some(pick) = parse_hex(&file.accent_pick) else { return Theme::default() };
    Theme { accent: crate::accent::derive(pick), glass: file.glass }
}

/// Parses a `#RRGGBB` pick (the `#` optional) into an sRGB color.
fn parse_hex(text: &str) -> Option<egui::Color32> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some(egui::Color32::from_rgb(byte(0..2)?, byte(2..4)?, byte(4..6)?))
}

/// Writes the theme to the file: creates the folder, writes a temp file beside
/// the target and renames it over the target.
pub fn save(theme: &Theme) -> Result<(), String> {
    let Some(path) = path() else {
        return Err("no appearance file path: CUTMARK_APPEARANCE_PATH and the OS config folder are both unset".to_string());
    };
    let folder = path.parent().ok_or_else(|| format!("{} has no parent folder", path.display()))?;
    fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    let pick = theme.accent.pick;
    let json = serde_json::to_string(&File { version: VERSION, accent_pick: format!("#{:02X}{:02X}{:02X}", pick.r(), pick.g(), pick.b()), glass: theme.glass })
        .map_err(|error| format!("cannot encode {}: {error}", path.display()))?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(|error| format!("cannot write {}: {error}", temp.display()))?;
    fs::rename(&temp, &path).map_err(|error| format!("cannot move {} over {}: {error}", temp.display(), path.display()))
}
/// The poll state kept in the egui context data.
#[derive(Clone)]
struct PollState {
    /// When the file was last checked.
    last_check: Option<Instant>,
    /// The file modified time last seen, so only real changes trigger a reload.
    last_seen_modified: Option<SystemTime>,
}

/// Polls the appearance file at most once per second; a changed file reloads
/// the theme. Always schedules a repaint one second out, so an idle app still
/// notices changes.
pub fn poll(ctx: &egui::Context) {
    poll_at(ctx, Instant::now());
}

/// `poll` with an injectable clock, so tests do not have to sleep.
pub fn poll_at(ctx: &egui::Context, now: Instant) {
    let id = egui::Id::new("cutmark_appearance_poll");
    let mut state = ctx.data(|data| data.get_temp::<PollState>(id)).unwrap_or(PollState { last_check: None, last_seen_modified: None });
    if state.last_check.is_some_and(|last| now.saturating_duration_since(last) < POLL_INTERVAL) {
        ctx.request_repaint_after(POLL_INTERVAL);
        return;
    }
    state.last_check = Some(now);
    if let Some(path) = path() {
        let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        if modified != state.last_seen_modified {
            state.last_seen_modified = modified;
            Theme::set(ctx, load());
        }
    }
    ctx.data_mut(|data| data.insert_temp(id, state));
    ctx.request_repaint_after(POLL_INTERVAL);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `#RRGGBB` pick parses; the `#` is optional.
    #[test]
    fn hex_picks_parse() {
        assert_eq!(parse_hex("#4C9BE8"), Some(egui::Color32::from_rgb(0x4C, 0x9B, 0xE8)));
        assert_eq!(parse_hex("4C9BE8"), Some(egui::Color32::from_rgb(0x4C, 0x9B, 0xE8)));
        assert_eq!(parse_hex("nope"), None);
        assert_eq!(parse_hex("#4C9BE"), None);
        assert_eq!(parse_hex("#4C9BE8FF"), None);
    }

    /// Bad file text falls back to the default theme.
    #[test]
    fn bad_text_loads_the_default() {
        assert_eq!(load_from_str(""), Theme::default());
        assert_eq!(load_from_str("{ not json"), Theme::default());
        assert_eq!(load_from_str(r##"{"version": 99, "accent_pick": "#7C5CFF", "glass": false}"##), Theme::default());
        assert_eq!(load_from_str(r##"{"version": 1, "accent_pick": "nope", "glass": false}"##), Theme::default());
    }
}
