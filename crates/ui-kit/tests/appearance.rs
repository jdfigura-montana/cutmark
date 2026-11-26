//! Tests for the per-app appearance file: the path per app id, fallbacks on
//! bad input, the save/load round trip, apps not touching each other, the
//! one-second poll gate and the default theme on a fresh context.
//!
//! The appearance folder is a process-wide env var, so the tests run
//! serially: every one takes the env lock and points `CUTMARK_APPEARANCE_DIR`
//! at its own temp folder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ui_kit::{AppAppearance, Preset, Theme, accent, appearance};

/// One lock for every test: the appearance folder is a process-wide env var.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The audio editor's identity, the app most tests run as.
const AUDIO: AppAppearance = AppAppearance::new("audio", Preset::Blue);
/// The recorder's identity: Teal by default.
const RECORDER: AppAppearance = AppAppearance::new("recorder", Preset::Teal);

/// A temp folder with the env var pointing at it as the Cutmark folder.
struct TempAppearance {
    /// The temp Cutmark folder.
    folder: PathBuf,
    /// Held for the whole test so no other test touches the env var.
    _guard: MutexGuard<'static, ()>,
}

impl TempAppearance {
    /// Creates the temp folder and points the env var at it.
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let folder = std::env::temp_dir().join(format!("cutmark-ui-kit-appearance-{name}"));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("create temp folder");
        set_var(&folder);
        Self { folder, _guard: guard }
    }

    /// The appearance file of an app inside the temp folder.
    fn file(&self, app: &AppAppearance) -> PathBuf {
        self.folder.join(app.id).join("appearance.json")
    }
}

impl Drop for TempAppearance {
    /// Clears the env var and removes the temp folder.
    fn drop(&mut self) {
        remove_var();
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// Sets `CUTMARK_APPEARANCE_DIR`.
fn set_var(value: &Path) {
    // SAFETY: the caller holds ENV_LOCK, so no other thread reads the variable.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("CUTMARK_APPEARANCE_DIR", value)
    };
}

/// Clears `CUTMARK_APPEARANCE_DIR`.
fn remove_var() {
    // SAFETY: the caller holds ENV_LOCK, so no other thread reads the variable.
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("CUTMARK_APPEARANCE_DIR")
    };
}

/// The pick of a named preset from `accent::presets()`.
fn preset(name: &str) -> egui::Color32 {
    accent::presets().into_iter().find(|(preset_name, _)| *preset_name == name).expect("preset exists").1
}

/// Saves a theme for an app, failing the test on an error.
fn save_ok(app: &AppAppearance, theme: &Theme) {
    let saved = appearance::save(app, theme);
    assert!(saved.is_ok(), "save failed: {:?}", saved.err());
}

/// a. The path is `<Cutmark folder>/<app id>/appearance.json`, one per app.
#[test]
fn the_path_depends_on_the_app_id() {
    let temp = TempAppearance::new("path");
    assert_eq!(appearance::path(&AUDIO), Some(temp.file(&AUDIO)));
    assert_eq!(appearance::path(&RECORDER), Some(temp.file(&RECORDER)));
    assert_ne!(appearance::path(&AUDIO), appearance::path(&RECORDER));
}

/// b. A missing file loads the app's own default: Blue for audio, Teal for the recorder.
#[test]
fn missing_file_loads_the_apps_default() {
    let _temp = TempAppearance::new("missing");
    assert_eq!(appearance::load(&AUDIO), Theme::default());
    assert_eq!(appearance::load(&RECORDER).accent.pick, Preset::Teal.pick());
    assert_ne!(appearance::load(&RECORDER), Theme::default());
}

/// c. Corrupt JSON, an unknown version and a bad hex pick each load the app's default.
#[test]
fn bad_files_load_the_apps_default() {
    let temp = TempAppearance::new("bad");
    let texts = [r#"{ this is not json "#, r##"{"version": 99, "accent_pick": "#7C5CFF"}"##, r##"{"version": 1, "accent_pick": "nope"}"##];
    fs::create_dir_all(temp.file(&RECORDER).parent().unwrap()).expect("create app folder");
    for text in texts {
        fs::write(temp.file(&RECORDER), text).expect("write appearance file");
        assert_eq!(appearance::load(&RECORDER), RECORDER.default_theme(), "text: {text}");
    }
}

/// d. Save then load round-trips a Violet pick; the first save creates the app folder.
#[test]
fn save_then_load_round_trips() {
    let temp = TempAppearance::new("round-trip");
    let theme = Theme::new(accent::derive(preset("Violet")), false);
    assert!(!temp.file(&AUDIO).exists(), "no folder before the first save");
    save_ok(&AUDIO, &theme);
    assert!(temp.file(&AUDIO).exists(), "the first save creates the app folder and file");
    assert_eq!(appearance::load(&AUDIO), theme);
}

/// e. Saving in one app leaves every other app's file and theme alone.
#[test]
fn saving_in_one_app_does_not_change_another() {
    let temp = TempAppearance::new("isolation");
    let violet = Theme::new(accent::derive(preset("Violet")), false);
    save_ok(&AUDIO, &violet);
    assert!(!temp.file(&RECORDER).exists(), "the recorder file was not created");
    assert_eq!(appearance::load(&RECORDER), RECORDER.default_theme());

    let orange = Theme::new(accent::derive(preset("Orange")), false);
    save_ok(&RECORDER, &orange);
    assert_eq!(appearance::load(&AUDIO), violet, "audio kept its own pick");
    assert_eq!(appearance::load(&RECORDER), orange);
}

/// f. A file with an old `glass: false` loads with glass on: the field is ignored and never written.
#[test]
fn an_old_glass_field_is_ignored_and_not_written() {
    let temp = TempAppearance::new("old-glass");
    fs::create_dir_all(temp.file(&AUDIO).parent().unwrap()).expect("create app folder");
    fs::write(temp.file(&AUDIO), r##"{"version": 1, "accent_pick": "#7C5CFF", "glass": false}"##).expect("write appearance file");
    let loaded = appearance::load(&AUDIO);
    assert_eq!(loaded.accent.pick, egui::Color32::from_rgb(0x7C, 0x5C, 0xFF));

    save_ok(&AUDIO, &loaded);
    let text = fs::read_to_string(temp.file(&AUDIO)).expect("read appearance file");
    assert!(!text.contains("glass"), "the saved file has no glass field: {text}");
    assert_eq!(appearance::load(&AUDIO), loaded);
}

/// g. The old shared `<Cutmark folder>/appearance.json` is neither read nor written.
#[test]
fn the_shared_file_is_not_read_or_written() {
    let temp = TempAppearance::new("shared");
    let shared = temp.folder.join("appearance.json");
    fs::write(&shared, r##"{"version": 1, "accent_pick": "#7C5CFF", "glass": true}"##).expect("write shared file");
    assert_eq!(appearance::load(&AUDIO), AUDIO.default_theme(), "the shared file is not migrated");

    save_ok(&AUDIO, &Theme::new(accent::derive(preset("Teal")), false));
    let text = fs::read_to_string(&shared).expect("read shared file");
    assert!(text.contains("#7C5CFF"), "the shared file is left as it was");
}

/// h. A poll past the one-second gate picks up a changed file of this app only.
#[test]
fn poll_picks_up_a_changed_file() {
    let temp = TempAppearance::new("poll");
    let ctx = egui::Context::default();
    ui_kit::apply_for(&ctx, AUDIO);
    assert_eq!(Theme::get(&ctx), Theme::default(), "apply stores the loaded theme");

    let start = Instant::now();
    appearance::poll_at(&ctx, start); // baseline: the file does not exist yet
    assert_eq!(Theme::get(&ctx), Theme::default());

    // Another app's file changing does not matter to this app.
    save_ok(&RECORDER, &Theme::new(accent::derive(preset("Orange")), false));
    appearance::poll_at(&ctx, start + Duration::from_secs(2));
    assert_eq!(Theme::get(&ctx), Theme::default(), "the recorder's file is not watched");

    let theme = Theme::new(accent::derive(preset("Teal")), false);
    save_ok(&AUDIO, &theme);
    bump(&temp.file(&AUDIO));

    // Inside the gate: nothing changes.
    appearance::poll_at(&ctx, start + Duration::from_millis(2500));
    assert_eq!(Theme::get(&ctx), Theme::default());

    // Past the gate: the changed file is loaded and stored.
    appearance::poll_at(&ctx, start + Duration::from_secs(4));
    assert_eq!(Theme::get(&ctx), theme);
}

/// Rewrites the file until its modified time moves, for coarse file systems.
fn bump(path: &Path) {
    let content = fs::read(path).expect("read appearance file");
    let before = fs::metadata(path).and_then(|meta| meta.modified()).ok();
    for _ in 0..100 {
        fs::write(path, &content).expect("rewrite appearance file");
        let now = fs::metadata(path).and_then(|meta| meta.modified()).ok();
        if now.is_some() && now != before {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // On a one-second-resolution file system the stamp may never move; the
    // poll still detects the change because no file existed at the baseline.
}

/// e. A fresh context with no stored theme reads back the default.
#[test]
fn fresh_context_returns_the_default() {
    let ctx = egui::Context::default();
    assert_eq!(Theme::get(&ctx), Theme::default());
}
