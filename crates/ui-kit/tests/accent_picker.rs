//! kittest tests for the accent picker (spec "Accent picker"): the Teal
//! swatch applies and saves, the hex field shows the readability and
//! near-record notes, Reset to default returns to the app's default accent,
//! there is no glass switch, and a wheel drag saves only on release.
//!
//! The appearance folder is a process-wide env var, so the tests run serially:
//! every one takes the env lock and points `CUTMARK_APPEARANCE_DIR` at its
//! own temp folder, as in `tests/appearance.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use egui::{Event, PointerButton, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use ui_kit::{AppAppearance, Preset, Theme, accent};

/// One lock for every test: the appearance folder is a process-wide env var.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The app the tests run as unless one says otherwise (the ui-kit fallback).
const TEST_APP: AppAppearance = AppAppearance::FALLBACK;

/// A temp folder with the env var pointing at it as the Cutmark folder.
struct TempAppearance {
    /// The appearance file of [`TEST_APP`] inside the temp folder.
    file: PathBuf,
    /// The temp Cutmark folder.
    folder: PathBuf,
    /// Held for the whole test so no other test touches the env var.
    _guard: MutexGuard<'static, ()>,
}

impl TempAppearance {
    /// Creates the temp folder and points the env var at it.
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let folder = std::env::temp_dir().join(format!("cutmark-ui-kit-accent-picker-{name}"));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("create temp folder");
        // SAFETY: the caller holds ENV_LOCK, so no other thread reads the variable.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("CUTMARK_APPEARANCE_DIR", &folder)
        };
        Self { file: folder.join(TEST_APP.id).join("appearance.json"), folder, _guard: guard }
    }
}

impl Drop for TempAppearance {
    /// Clears the env var and removes the temp folder.
    fn drop(&mut self) {
        // SAFETY: the guard in self is still held, so no other thread reads the variable.
        #[allow(unsafe_code)]
        unsafe {
            std::env::remove_var("CUTMARK_APPEARANCE_DIR")
        };
        let _ = fs::remove_dir_all(&self.folder);
    }
}

/// A harness for one app with the ui-kit font set and the theme from the
/// app's appearance file: its id picks the file, its default the reset. The
/// first pass only installs fonts and styles (egui applies a font change at
/// the start of the next pass).
fn new_harness_for(app: AppAppearance, mut draw: impl FnMut(&mut egui::Ui) + 'static) -> Harness<'static> {
    let first_pass = Rc::new(Cell::new(true));
    let mut harness = Harness::builder().with_size(vec2(640.0, 820.0)).build_ui(move |ui| {
        if first_pass.get() {
            first_pass.set(false);
            ui_kit::apply_for(ui.ctx(), app);
            return;
        }
        draw(ui);
    });
    harness.run();
    harness
}

/// A harness showing the open Appearance dialog; returns it with the open flag.
fn dialog_harness() -> (Harness<'static>, Rc<Cell<bool>>) {
    dialog_harness_for(TEST_APP)
}

/// [`dialog_harness`] for one app.
fn dialog_harness_for(app: AppAppearance) -> (Harness<'static>, Rc<Cell<bool>>) {
    let open = Rc::new(Cell::new(true));
    let open_cell = open.clone();
    let harness = new_harness_for(app, move |ui| {
        let mut open = open_cell.get();
        ui_kit::appearance_dialog(ui.ctx(), &mut open);
        open_cell.set(open);
    });
    (harness, open)
}

/// The `accent_pick` saved in the appearance file (panics when absent).
fn saved_pick(path: &std::path::Path) -> String {
    let text = fs::read_to_string(path).expect("appearance file saved");
    let json: serde_json::Value = serde_json::from_str(&text).expect("saved JSON parses");
    json["accent_pick"].as_str().expect("accent_pick").to_owned()
}

/// The `#RRGGBB` text of a color, as the picker writes it.
fn hex(color: egui::Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r(), color.g(), color.b())
}

/// Clicking the Teal swatch sets the theme to Teal and saves it.
#[test]
fn clicking_the_teal_swatch_applies_and_saves_teal() {
    let temp = TempAppearance::new("teal");
    let (mut harness, _open) = dialog_harness();

    harness.get_by_label("Teal").click();
    harness.run();

    let teal = accent::presets().into_iter().find(|(name, _)| *name == "Teal").expect("Teal preset").1;
    assert_eq!(Theme::get(&harness.ctx).accent, accent::derive(teal), "the theme must hold the Teal accent");
    assert_eq!(saved_pick(&temp.file), hex(teal), "the file must hold the Teal pick");

    // More frames do not rewrite the file with anything else.
    harness.run();
    harness.run();
    assert_eq!(saved_pick(&temp.file), hex(teal), "further frames must not change the saved pick");
}

/// Typing `#000000` in the hex field applies black and shows the adjusted note.
#[test]
fn typing_black_in_the_hex_field_shows_the_adjusted_note() {
    let _temp = TempAppearance::new("adjusted");
    let (mut harness, _open) = dialog_harness();

    harness.get_by_label("Hex value").click();
    harness.run();
    harness.get_by_label("Hex value").type_text("#000000");
    harness.run();

    assert_eq!(Theme::get(&harness.ctx).accent.pick, egui::Color32::BLACK, "the pick must be black");
    assert!(Theme::get(&harness.ctx).accent.adjusted, "black must report adjusted");
    harness.get_by_label("Adjusted for readability");
}

/// Typing `#FF3B30` in the hex field shows the near-record note.
#[test]
fn typing_a_near_record_hex_shows_the_near_record_note() {
    let _temp = TempAppearance::new("near-record");
    let (mut harness, _open) = dialog_harness();

    harness.get_by_label("Hex value").click();
    harness.run();
    harness.get_by_label("Hex value").type_text("#FF3B30");
    harness.run();

    let pick = egui::Color32::from_rgb(0xFF, 0x3B, 0x30);
    assert_eq!(Theme::get(&harness.ctx).accent.pick, pick, "the pick must be #FF3B30");
    assert!(Theme::get(&harness.ctx).accent.near_record, "#FF3B30 must report near_record");
    harness.get_by_label("Close to the record color. Record buttons will still show their R and dot.");
}

/// "Reset to default" returns the theme and the saved file to the Blue default.
#[test]
fn reset_returns_to_the_blue_default() {
    let temp = TempAppearance::new("reset");
    let (mut harness, _open) = dialog_harness();

    harness.get_by_label("Teal").click();
    harness.run();
    assert_ne!(Theme::get(&harness.ctx).accent, accent::default_accent(), "Teal must be applied first");

    harness.get_by_label("Reset to default").click();
    harness.run();

    assert_eq!(Theme::get(&harness.ctx).accent, accent::default_accent(), "the accent must be Blue again");
    assert_eq!(saved_pick(&temp.file), hex(accent::presets()[0].1), "the file must hold the Blue pick");
}

/// "Reset to default" goes to the app's own default, and saves it in that app's file.
#[test]
fn reset_returns_to_each_apps_own_default() {
    let temp = TempAppearance::new("reset-per-app");
    for app in [AppAppearance::AUDIO, AppAppearance::RECORDER, AppAppearance::VIDEO, AppAppearance::PROMPTER] {
        let (id, preset) = (app.id, app.default_preset);
        let (mut harness, _open) = dialog_harness_for(app);
        assert_eq!(Theme::get(&harness.ctx), app.default_theme(), "{id} starts on its default with no file");

        let other = if preset == Preset::Blue { "Teal" } else { "Blue" };
        harness.get_by_label(other).click();
        harness.run();
        harness.get_by_label("Reset to default").click();
        harness.run();

        let default_pick = preset.pick();
        assert_eq!(Theme::get(&harness.ctx).accent.pick, default_pick, "{id} resets to its own default");
        assert_eq!(saved_pick(&temp.folder.join(id).join("appearance.json")), hex(default_pick), "{id} saved its default in its own file");
    }
}

/// Saving in one app leaves another app's file alone.
#[test]
fn saving_in_one_app_does_not_touch_another_apps_file() {
    let temp = TempAppearance::new("two-apps");
    let (mut audio, _open) = dialog_harness_for(AppAppearance::AUDIO);
    audio.get_by_label("Violet").click();
    audio.run();
    let (recorder, _open) = dialog_harness_for(AppAppearance::RECORDER);

    assert_eq!(saved_pick(&temp.folder.join("audio").join("appearance.json")), hex(accent::presets()[1].1), "audio saved Violet");
    assert!(!temp.folder.join("recorder").join("appearance.json").exists(), "the recorder file was not written");
    assert_eq!(Theme::get(&recorder.ctx).accent.pick, Preset::Teal.pick(), "the recorder still starts on Teal");
}

/// The picker has no glass switch any more.
#[test]
fn there_is_no_glass_switch() {
    let _temp = TempAppearance::new("no-glass");
    let (harness, _open) = dialog_harness();
    assert!(harness.query_by_label("Glass switch").is_none());
    assert!(harness.query_by_label("Glass effects").is_none());
}

/// Dragging on the wheel previews live but saves the file only on release.
#[test]
fn dragging_the_wheel_saves_only_on_release() {
    let temp = TempAppearance::new("wheel-drag");
    let (mut harness, _open) = dialog_harness();
    let blue_pick = accent::presets()[0].1;

    let wheel = harness.get_by_label("Accent wheel").rect();
    let start = wheel.center();
    harness.input_mut().events.push(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    harness.run();

    let target = pos2(wheel.center().x + wheel.width() * 0.3, wheel.center().y + wheel.height() * 0.2);
    harness.input_mut().events.push(egui::Event::PointerMoved(target));
    harness.run();
    harness.run();

    assert_ne!(Theme::get(&harness.ctx).accent.pick, blue_pick, "the drag must preview a new pick live");
    assert!(!temp.file.exists(), "a drag must not save until release");

    harness.input_mut().events.push(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    harness.run();

    assert!(temp.file.exists(), "the release must save");
    assert_eq!(saved_pick(&temp.file), hex(Theme::get(&harness.ctx).accent.pick), "the saved pick must match the theme");
}
