//! The per-app appearance file (`<Cutmark folder>/<app>/appearance.json`):
//! read, write and poll, so each Cutmark app keeps its own accent (spec:
//! Design Language "accent per app" amendment). Apps do not watch each other,
//! and the old shared `appearance.json` is neither read nor written.
//!
//! A file with no `scheme` field predates the current app colors (spec:
//! Brand-Mark.md, "App colors"). If it holds the app's old default accent,
//! loading moves it to the new default and writes the file back. Every write
//! carries `"scheme": 2`, so a file is never moved twice.
//!
//! The file stores only the raw user pick as `#RRGGBB`; the derived accent
//! tokens are recomputed with [`crate::accent::derive`] on load. Glass is
//! always on, so old files with a `glass` field still load and the field is
//! ignored. Every problem (missing file, unreadable file, bad JSON, unknown
//! version, bad hex) falls back to the app's default theme: loading never
//! panics and never returns an error.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::theme::Theme;

/// The file format version this module reads and writes.
const VERSION: u32 = 1;
/// The default-accent scheme every write carries.
const SCHEME: u32 = 2;
/// The file name inside the app's folder.
const FILE_NAME: &str = "appearance.json";
/// How far apart two polls may be (spec: at most once per second).
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// One of the four accent presets; an app's default accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// The Blue preset.
    Blue,
    /// The Violet preset.
    Violet,
    /// The Teal preset.
    Teal,
    /// The Orange preset.
    Orange,
}

impl Preset {
    /// The preset's pick color.
    pub fn pick(self) -> egui::Color32 {
        use crate::tokens::accent;
        match self {
            Preset::Blue => accent::PRESET_BLUE,
            Preset::Violet => accent::PRESET_VIOLET,
            Preset::Teal => accent::PRESET_TEAL,
            Preset::Orange => accent::PRESET_ORANGE,
        }
    }
}

/// Which app is asking: its folder name under the Cutmark config folder and
/// the accent it starts on and resets to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppAppearance {
    /// The app's folder name: `audio`, `recorder`, `gallery`, `video`, `prompter`.
    pub id: &'static str,
    /// The accent a missing or unreadable file gives, and "Reset to default" restores.
    pub default_preset: Preset,
}

impl AppAppearance {
    /// What a context uses when the app never called [`crate::apply_for`] (tests, examples).
    pub const FALLBACK: AppAppearance = AppAppearance { id: "app", default_preset: Preset::Blue };

    /// The audio editor: Orange.
    pub const AUDIO: AppAppearance = AppAppearance::new("audio", Preset::Orange);
    /// The Recorder: Teal.
    pub const RECORDER: AppAppearance = AppAppearance::new("recorder", Preset::Teal);
    /// The video editor: Violet.
    pub const VIDEO: AppAppearance = AppAppearance::new("video", Preset::Violet);
    /// The Prompter: Blue.
    pub const PROMPTER: AppAppearance = AppAppearance::new("prompter", Preset::Blue);

    /// An app's appearance identity.
    pub const fn new(id: &'static str, default_preset: Preset) -> Self {
        Self { id, default_preset }
    }

    /// The theme of a fresh install of this app: its default accent.
    pub fn default_theme(&self) -> Theme {
        Theme::new(crate::accent::derive(self.default_preset.pick()), false)
    }

    /// The default this app had before scheme 2, if that default changed.
    fn old_default_preset(&self) -> Option<Preset> {
        match self.id {
            "audio" => Some(Preset::Blue),
            "video" => Some(Preset::Orange),
            "prompter" => Some(Preset::Violet),
            _ => None,
        }
    }
}

/// The egui context data id under which the app identity is stored.
fn app_id() -> egui::Id {
    egui::Id::new("cutmark_app_appearance")
}

/// Stores the app identity in the context; [`poll`] and the accent picker read it.
pub fn set_app(ctx: &egui::Context, app: AppAppearance) {
    ctx.data_mut(|data| data.insert_temp(app_id(), app));
}

/// The app identity stored in the context, or [`AppAppearance::FALLBACK`].
pub fn app(ctx: &egui::Context) -> AppAppearance {
    ctx.data(|data| data.get_temp::<AppAppearance>(app_id())).unwrap_or(AppAppearance::FALLBACK)
}

/// The on-disk JSON: version, scheme and raw pick. A `glass` field in an old file is
/// ignored on load (serde skips unknown fields) and never written.
#[derive(serde::Serialize, serde::Deserialize)]
struct File {
    version: u32,
    /// Absent in files written before scheme 2.
    #[serde(default)]
    scheme: Option<u32>,
    accent_pick: String,
}

/// Where the app's file lives: `<folder>/<app id>/appearance.json`, where the
/// folder is `CUTMARK_APPEARANCE_DIR` if set (tests use it), else the Cutmark
/// folder in the OS config directory.
pub fn path(app: &AppAppearance) -> Option<PathBuf> {
    if let Ok(from_env) = std::env::var("CUTMARK_APPEARANCE_DIR")
        && !from_env.is_empty()
    {
        return Some(PathBuf::from(from_env).join(app.id).join(FILE_NAME));
    }
    config_folder().map(|folder| folder.join(app.id).join(FILE_NAME))
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

/// Loads the app's theme from its file, or the app's default for any problem.
/// A file that still holds the app's old default is moved to the new one and
/// written back; if that write fails, the next load tries again.
pub fn load(app: &AppAppearance) -> Theme {
    let Some(path) = path(app) else { return app.default_theme() };
    let (theme, moved) = load_from_str(app, &fs::read_to_string(path).unwrap_or_default());
    if moved && let Err(error) = save(app, &theme) {
        log::warn!("cannot save the new default accent: {error}");
    }
    theme
}

/// Parses file text into a theme; any bad input falls back to the app's
/// default. The flag is true when the old default was moved to the new one.
fn load_from_str(app: &AppAppearance, text: &str) -> (Theme, bool) {
    let fallback = (app.default_theme(), false);
    let Ok(file) = serde_json::from_str::<File>(text) else { return fallback };
    if file.version != VERSION {
        return fallback;
    }
    let Some(pick) = parse_hex(&file.accent_pick) else { return fallback };
    if file.scheme.is_none() && app.old_default_preset().is_some_and(|old| old.pick() == pick) {
        return (app.default_theme(), true);
    }
    (Theme::new(crate::accent::derive(pick), false), false)
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

/// Writes the theme to the app's file: creates the app folder on first save,
/// writes a temp file beside the target and renames it over the target.
pub fn save(app: &AppAppearance, theme: &Theme) -> Result<(), String> {
    let Some(path) = path(app) else {
        return Err("no appearance file path: CUTMARK_APPEARANCE_DIR and the OS config folder are both unset".to_string());
    };
    let folder = path.parent().ok_or_else(|| format!("{} has no parent folder", path.display()))?;
    fs::create_dir_all(folder).map_err(|error| format!("cannot create {}: {error}", folder.display()))?;
    let pick = theme.accent.pick;
    let file = File { version: VERSION, scheme: Some(SCHEME), accent_pick: format!("#{:02X}{:02X}{:02X}", pick.r(), pick.g(), pick.b()) };
    let json = serde_json::to_string(&file).map_err(|error| format!("cannot encode {}: {error}", path.display()))?;
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
    let app = app(ctx);
    if let Some(path) = path(&app) {
        let modified = fs::metadata(&path).and_then(|meta| meta.modified()).ok();
        if modified != state.last_seen_modified {
            state.last_seen_modified = modified;
            Theme::set(ctx, load(&app));
            crate::refresh(ctx);
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
        let app = AppAppearance::FALLBACK;
        assert_eq!(load_from_str(&app, "").0, Theme::default());
        assert_eq!(load_from_str(&app, "{ not json").0, Theme::default());
        assert_eq!(load_from_str(&app, r##"{"version": 99, "accent_pick": "#7C5CFF"}"##).0, Theme::default());
        assert_eq!(load_from_str(&app, r##"{"version": 1, "accent_pick": "nope"}"##).0, Theme::default());
    }

    /// Bad text gives the app's own default, not Blue.
    #[test]
    fn bad_text_loads_the_apps_own_default() {
        let teal = AppAppearance::RECORDER;
        assert_eq!(load_from_str(&teal, "").0, teal.default_theme());
        assert_ne!(teal.default_theme(), Theme::default());
    }

    /// The file lives in the app's own folder.
    #[test]
    fn the_path_ends_in_the_app_folder() {
        let found = path(&AppAppearance::new("gallery", Preset::Blue));
        assert!(found.is_some_and(|file| file.ends_with(std::path::Path::new("gallery").join("appearance.json"))));
    }

    /// A file with an old `glass` field loads; the field is ignored.
    #[test]
    fn an_old_glass_field_is_ignored() {
        let app = AppAppearance::FALLBACK;
        let with_glass = load_from_str(&app, r##"{"version": 1, "accent_pick": "#7C5CFF", "glass": false}"##).0;
        let without = load_from_str(&app, r##"{"version": 1, "accent_pick": "#7C5CFF"}"##).0;
        assert_eq!(with_glass, without);
        assert_ne!(with_glass, Theme::default());
    }
}
