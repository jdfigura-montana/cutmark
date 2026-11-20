//! Tests for the appearance file: fallbacks on bad input, the save/load round
//! trip, the one-second poll gate and the default theme on a fresh context.
//!
//! The appearance path is a process-wide env var, so the tests run serially:
//! every one takes the env lock and points `CUTMARK_APPEARANCE_PATH` at its
//! own temp folder.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ui_kit::{Theme, accent, appearance};

/// One lock for every test: the appearance path is a process-wide env var.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// A temp folder with the env var pointing at its `appearance.json`.
struct TempAppearance {
    /// The appearance file inside the temp folder.
    file: PathBuf,
    /// Held for the whole test so no other test touches the env var.
    _guard: MutexGuard<'static, ()>,
}

impl TempAppearance {
    /// Creates the temp folder and points the env var at its appearance file.
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let folder = std::env::temp_dir().join(format!("cutmark-ui-kit-appearance-{name}"));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("create temp folder");
        let file = folder.join("appearance.json");
        set_var(&file);
        Self { file, _guard: guard }
    }
}

impl Drop for TempAppearance {
    /// Clears the env var and removes the temp folder.
    fn drop(&mut self) {
        remove_var();
        let _ = fs::remove_dir_all(self.file.parent().expect("temp folder"));
    }
}

/// Sets `CUTMARK_APPEARANCE_PATH`.
fn set_var(value: &Path) {
    // SAFETY: the caller holds ENV_LOCK, so no other thread reads the variable.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("CUTMARK_APPEARANCE_PATH", value)
    };
}

/// Clears `CUTMARK_APPEARANCE_PATH`.
fn remove_var() {
    // SAFETY: the caller holds ENV_LOCK, so no other thread reads the variable.
    #[allow(unsafe_code)]
    unsafe {
        std::env::remove_var("CUTMARK_APPEARANCE_PATH")
    };
}

/// The pick of a named preset from `accent::presets()`.
fn preset(name: &str) -> egui::Color32 {
    accent::presets().into_iter().find(|(preset_name, _)| *preset_name == name).expect("preset exists").1
}

/// Saves a theme, failing the test on an error.
fn save_ok(theme: &Theme) {
    let saved = appearance::save(theme);
    assert!(saved.is_ok(), "save failed: {:?}", saved.err());
}

/// a. A missing file loads the default theme (Blue, glass on).
#[test]
fn missing_file_loads_the_default() {
    let _temp = TempAppearance::new("missing");
    assert_eq!(appearance::load(), Theme::default());
    assert!(Theme::default().glass);
}

/// b. Corrupt JSON, an unknown version and a bad hex pick each load the default.
#[test]
fn bad_files_load_the_default() {
    let _temp = TempAppearance::new("bad");
    let texts = [
        r#"{ this is not json "#,
        r##"{"version": 99, "accent_pick": "#7C5CFF", "glass": false}"##,
        r##"{"version": 1, "accent_pick": "nope", "glass": false}"##,
    ];
    for text in texts {
        fs::write(&_temp.file, text).expect("write appearance file");
        assert_eq!(appearance::load(), Theme::default(), "text: {text}");
    }
}

/// c. Save then load round-trips a Violet pick and glass off.
#[test]
fn save_then_load_round_trips() {
    let _temp = TempAppearance::new("round-trip");
    let theme = Theme { accent: accent::derive(preset("Violet")), glass: false };
    save_ok(&theme);
    assert_eq!(appearance::load(), theme);
}

/// d. A poll past the one-second gate picks up a changed file.
#[test]
fn poll_picks_up_a_changed_file() {
    let _temp = TempAppearance::new("poll");
    let ctx = egui::Context::default();
    ui_kit::apply(&ctx);
    assert_eq!(Theme::get(&ctx), Theme::default(), "apply stores the loaded theme");

    let start = Instant::now();
    appearance::poll_at(&ctx, start); // baseline: the file does not exist yet
    assert_eq!(Theme::get(&ctx), Theme::default());

    let theme = Theme { accent: accent::derive(preset("Teal")), glass: false };
    save_ok(&theme);
    bump(&_temp.file);

    // Inside the gate: nothing changes.
    appearance::poll_at(&ctx, start + Duration::from_millis(500));
    assert_eq!(Theme::get(&ctx), Theme::default());

    // Past the gate: the changed file is loaded and stored.
    appearance::poll_at(&ctx, start + Duration::from_secs(2));
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
