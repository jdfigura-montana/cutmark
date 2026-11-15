//! ui::dialogs::audio_settings: the Audio Settings dialog (input device,
//! record channels, latency offset), opened from Project > Audio Settings...

use eframe::egui;

use crate::editor::Editor;
use crate::io::config::{AppConfig, MAX_LATENCY_OFFSET_MS, MIN_LATENCY_OFFSET_MS, RecordChannels};
use crate::ui::theme::THEME;

/// Modal dialog for the app-wide audio input settings.
pub struct AudioSettingsDialog {
    /// Whether the dialog is open.
    open: bool,
    /// Chosen input device name; None is the Windows default input.
    device: Option<String>,
    /// Chosen record channels.
    channels: RecordChannels,
    /// Chosen latency offset in milliseconds.
    latency_ms: i32,
    /// Name of the Windows default input, for the first list entry.
    default_name: Option<String>,
    /// Every input device name, listed once when the dialog opened.
    devices: Vec<String>,
    /// Channel count of the chosen device, when it can be read.
    device_channels: Option<u16>,
    /// Last save or reopen error, shown in red.
    error: Option<String>,
}

/// True when a device with `device_channels` input channels has an input 2
/// (an unknown channel count does not restrict anything).
fn has_second_channel(device_channels: Option<u16>) -> bool {
    device_channels.is_none_or(|channels| channels >= 2)
}

/// `choice`, or Mono (input 1) when the device cannot serve it.
fn fit_channels(choice: RecordChannels, device_channels: Option<u16>) -> RecordChannels {
    if has_second_channel(device_channels) { choice } else { RecordChannels::Mono1 }
}

/// The first entry of the device list: "Windows default (<name>)".
fn default_label(default_name: Option<&str>) -> String {
    match default_name {
        Some(name) => format!("Windows default ({name})"),
        None => "Windows default".to_string(),
    }
}

impl AudioSettingsDialog {
    /// Creates the dialog open, prefilled with the current settings. Lists the
    /// input devices (slow), so it is built when the menu item is clicked,
    /// never per frame.
    pub fn new_open(editor: &Editor) -> Self {
        let config = editor.config();
        let device = config.input_device.clone();
        let device_channels = editor.input_device_channels(device.as_deref());
        AudioSettingsDialog {
            open: true,
            channels: fit_channels(config.record_channels, device_channels),
            latency_ms: config.latency_offset_ms,
            default_name: editor.default_input_name(),
            devices: editor.input_device_names(),
            device,
            device_channels,
            error: None,
        }
    }

    /// True when the dialog is fully dismissed (no longer shown).
    pub fn is_closed(&self) -> bool {
        !self.open
    }

    /// Shows the dialog. OK (or Enter) saves through the Editor and closes on
    /// success; Cancel, Esc and the backdrop discard the changes.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut cancel = false;
        let mut apply = false;
        let response = super::modal("audio_settings_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Audio Settings");
            ui.add_space(THEME.space_xs);
            super::description(ui, "Input device and recording options for this computer.");
            super::before_form(ui);
            super::form_row(ui, "Input device", |ui| self.device_combo(ui, editor));
            super::form_row(ui, "Record channels", |ui| {
                let second = has_second_channel(self.device_channels);
                super::segmented_enabled(
                    ui,
                    "as_channels",
                    &[
                        (RecordChannels::Mono1, "Mono (input 1)", true),
                        (RecordChannels::Mono2, "Mono (input 2)", second),
                        (RecordChannels::Stereo, "Stereo", second),
                    ],
                    &mut self.channels,
                );
            });
            super::form_row(ui, "Latency offset", |ui| {
                super::ms_control(ui, "as_latency", &mut self.latency_ms, (MIN_LATENCY_OFFSET_MS, MAX_LATENCY_OFFSET_MS));
            });
            if let Some(error) = &self.error {
                ui.label(egui::RichText::new(error).color(THEME.error));
            }
            super::footer(ui, |ui| {
                if super::footer_button(ui, "OK", super::FooterButton::Primary, true).clicked() {
                    apply = true;
                }
                if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                    cancel = true;
                }
            });
            // Enter confirms.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                apply = true;
            }
        });
        // Esc or the backdrop cancels.
        if response.should_close() {
            cancel = true;
        }
        if apply && !cancel {
            self.try_apply(editor);
        }
        if cancel {
            self.open = false;
        }
    }

    /// The input device list. Picking a device re-reads its channel count; a
    /// device with one channel drops the choice to Mono (input 1).
    fn device_combo(&mut self, ui: &mut egui::Ui, editor: &Editor) {
        let width = ui.available_width();
        let shown = match &self.device {
            None => default_label(self.default_name.as_deref()),
            Some(name) if self.devices.contains(name) => name.clone(),
            Some(name) => format!("{name} (not found)"),
        };
        let before = self.device.clone();
        ui.scope(|ui| {
            // The combo button is as tall as a text field.
            ui.spacing_mut().interact_size.y = THEME.field_h;
            egui::ComboBox::from_id_salt("as_device")
                .width(width)
                .truncate()
                .selected_text(shown)
                .popup_style(crate::ui::theme::menu_item_style.into())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.device, None, default_label(self.default_name.as_deref()));
                    for name in &self.devices {
                        ui.selectable_value(&mut self.device, Some(name.clone()), name);
                    }
                });
        });
        if self.device != before {
            self.device_channels = editor.input_device_channels(self.device.as_deref());
            self.channels = fit_channels(self.channels, self.device_channels);
        }
    }

    /// Saves the settings through the Editor; the error stays in the dialog.
    fn try_apply(&mut self, editor: &mut Editor) {
        self.error = None;
        let config = AppConfig { input_device: self.device.clone(), record_channels: self.channels, latency_offset_ms: self.latency_ms };
        match editor.set_config(config) {
            Ok(()) => self.open = false,
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::editor::record::FakeInput;
    use crate::io::config;
    use crate::io::paths::ProjectPaths;
    use crate::model::project::{BitDepth, Project, ViewState};
    use crate::ui::test_util::{Headless, Out};

    /// Slack for float rounding when comparing painted rectangles.
    const TOL: f32 = 0.6;

    fn editor() -> Editor {
        Editor::with_project(Project::new("Test".to_string(), 48_000, BitDepth::Int24), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") })
    }

    /// An editor listing a 2-channel and a 1-channel input, with its config
    /// saved to `config_path`.
    fn editor_with_devices(config_path: PathBuf) -> Editor {
        let mut editor = editor();
        editor.set_config_path(config_path);
        editor.set_fake_input(FakeInput {
            default: Some("Mic".to_string()),
            devices: vec![("Mic".to_string(), 2), ("Line".to_string(), 1)],
            ..FakeInput::default()
        });
        editor
    }

    /// The painted dialog card: the largest filled and outlined rectangle.
    fn card(out: &Out) -> egui::Rect {
        out.rects()
            .into_iter()
            .filter(|r| r.stroke.width > 0.0 && r.fill.a() > 0)
            .map(|r| r.rect)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the dialog card is painted")
    }

    fn button(out: &Out, label: &str) -> egui::Rect {
        let center = out.text_rect(label).expect("button text").center();
        out.filled_rect_at(center).expect("button background")
    }

    #[test]
    fn a_device_without_a_second_channel_only_allows_mono_1() {
        assert!(has_second_channel(Some(2)));
        assert!(has_second_channel(Some(8)));
        assert!(has_second_channel(None), "unknown restricts nothing");
        assert!(!has_second_channel(Some(1)));
        assert!(!has_second_channel(Some(0)));
        for choice in [RecordChannels::Mono1, RecordChannels::Mono2, RecordChannels::Stereo] {
            assert_eq!(fit_channels(choice, Some(2)), choice);
            assert_eq!(fit_channels(choice, None), choice);
            assert_eq!(fit_channels(choice, Some(1)), RecordChannels::Mono1);
        }
    }

    #[test]
    fn the_default_entry_names_the_windows_default_device() {
        assert_eq!(default_label(Some("Mic (USB)")), "Windows default (Mic (USB))");
        assert_eq!(default_label(None), "Windows default");
    }

    #[test]
    fn the_dialog_opens_with_the_current_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        editor.set_config(AppConfig { input_device: Some("Mic".to_string()), record_channels: RecordChannels::Stereo, latency_offset_ms: -12 }).unwrap();
        let dialog = AudioSettingsDialog::new_open(&editor);
        assert_eq!(dialog.device.as_deref(), Some("Mic"));
        assert_eq!(dialog.channels, RecordChannels::Stereo);
        assert_eq!(dialog.latency_ms, -12);
        assert_eq!(dialog.devices, vec!["Mic", "Line"]);
        assert_eq!(dialog.default_name.as_deref(), Some("Mic"));
        assert_eq!(dialog.device_channels, Some(2));
        assert!(!dialog.is_closed());
    }

    #[test]
    fn a_saved_stereo_choice_drops_to_mono_1_on_a_one_channel_device() {
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        editor.set_config(AppConfig { input_device: Some("Line".to_string()), record_channels: RecordChannels::Stereo, latency_offset_ms: 0 }).unwrap();
        let dialog = AudioSettingsDialog::new_open(&editor);
        assert_eq!(dialog.channels, RecordChannels::Mono1);
    }

    #[test]
    fn the_card_is_420_wide_with_ok_then_cancel_flush_right_8px_apart() {
        let mut h = Headless::new(1280.0, 800.0);
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));

        let card = card(&out);
        assert!((card.width() - THEME.dialog_w).abs() < TOL, "card is {} wide", card.width());
        assert!(card.top() > 0.0 && card.bottom() < 800.0, "card on screen");

        // Same footer as the other dialogs: "OK  Cancel", 8 px apart, the
        // last button flush with the content edge, equal card padding.
        let line = out.footer_line().expect("footer divider");
        let (pad_left, pad_right) = (line.left() - card.left(), card.right() - line.right());
        assert!((pad_left - pad_right).abs() < TOL, "{pad_left} vs {pad_right}");
        let (ok, cancel) = (button(&out, "OK"), button(&out, "Cancel"));
        assert!(ok.center().x < cancel.center().x, "OK then Cancel");
        assert!((cancel.left() - ok.right() - THEME.space_md).abs() < TOL, "gap is {}", cancel.left() - ok.right());
        assert!((cancel.right() - line.right()).abs() < TOL);

        // Everything sits inside the card, top to bottom.
        let rows: Vec<egui::Rect> = ["Audio Settings", "Input device", "Record channels", "Latency offset"]
            .iter()
            .map(|label| out.text_rect(label).unwrap_or_else(|| panic!("{label}")))
            .collect();
        for rect in &rows {
            assert!(card.contains_rect(*rect), "{rect:?} inside {card:?}");
        }
        assert!(rows.windows(2).all(|pair| pair[0].center().y + 10.0 < pair[1].center().y), "rows run top to bottom: {rows:?}");
        // The controls never push past the card's content edge.
        // (The typed value paints its number and its " ms" suffix as two runs.)
        for label in ["Mono (input 1)", "Mono (input 2)", "Stereo", "+0", " ms"] {
            let rect = out.text_rect(label).unwrap_or_else(|| panic!("{label}"));
            assert!(rect.right() <= line.right() + TOL, "{label} {rect:?}");
        }
        let (value, suffix) = (out.text_rect("+0").unwrap(), out.text_rect(" ms").unwrap());
        assert!(value.right() <= suffix.left() + TOL, "+0 then ms, left to right");
        assert!(out.text_rect("Windows default (Mic)").is_some(), "the combo shows the default entry");
    }

    #[test]
    fn the_second_channel_options_grey_out_for_a_one_channel_device() {
        let mut h = Headless::new(1280.0, 800.0);
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        editor.set_config(AppConfig { input_device: Some("Line".to_string()), ..AppConfig::default() }).unwrap();
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let rect_of = |label: &str| out.texts().into_iter().find(|t| t.text == label).map(|t| t.rect).unwrap_or_else(|| panic!("{label}"));
        // A click on a disabled segment changes nothing.
        let stereo = rect_of("Stereo").center();
        let _ = h.click(stereo, |ui| dialog.show(ui.ctx(), &mut editor));
        let mono2 = rect_of("Mono (input 2)").center();
        let _ = h.click(mono2, |ui| dialog.show(ui.ctx(), &mut editor));
        assert_eq!(dialog.channels, RecordChannels::Mono1);
    }

    #[test]
    fn a_two_channel_device_lets_stereo_be_chosen() {
        let mut h = Headless::new(1280.0, 800.0);
        let dir = tempfile::tempdir().unwrap();
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let stereo = out.text_rect("Stereo").expect("Stereo").center();
        let _ = h.click(stereo, |ui| dialog.show(ui.ctx(), &mut editor));
        assert_eq!(dialog.channels, RecordChannels::Stereo);
    }

    #[test]
    fn cancel_leaves_the_config_unchanged_and_ok_saves_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor_with_devices(path.clone());
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));

        // Change everything, then Cancel.
        let stereo = out.text_rect("Stereo").expect("Stereo").center();
        let _ = h.click(stereo, |ui| dialog.show(ui.ctx(), &mut editor));
        dialog.latency_ms = 40;
        dialog.device = Some("Line".to_string());
        let cancel = button(&out, "Cancel").center();
        let _ = h.click(cancel, |ui| dialog.show(ui.ctx(), &mut editor));
        assert!(dialog.is_closed());
        assert_eq!(editor.config(), &AppConfig::default());
        assert!(!path.exists(), "Cancel writes nothing");

        // Open again, change, OK.
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let stereo = out.text_rect("Stereo").expect("Stereo").center();
        let _ = h.click(stereo, |ui| dialog.show(ui.ctx(), &mut editor));
        dialog.latency_ms = 40;
        let ok = button(&out, "OK").center();
        let _ = h.click(ok, |ui| dialog.show(ui.ctx(), &mut editor));
        assert!(dialog.is_closed());
        let want = AppConfig { input_device: None, record_channels: RecordChannels::Stereo, latency_offset_ms: 40 };
        assert_eq!(editor.config(), &want);
        assert_eq!(config::load_from(&path), want);
    }

    #[test]
    fn a_failed_save_keeps_the_dialog_open_with_the_error() {
        let mut h = Headless::new(1280.0, 800.0);
        // No config path: saving fails.
        let mut editor = editor();
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let ok = button(&out, "OK").center();
        let _ = h.click(ok, |ui| dialog.show(ui.ctx(), &mut editor));
        assert!(!dialog.is_closed(), "stays open");
        let error = dialog.error.clone().expect("the save error");
        // The error is set while handling the click, so it paints one frame
        // later: in red, inside the card, and the card stays 420 wide.
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let shown = out.texts().into_iter().find(|t| t.text == error).expect("the error is painted in the dialog");
        assert_eq!(shown.color, THEME.error);
        let card = card(&out);
        assert!(card.contains_rect(shown.rect), "{:?} inside {card:?}", shown.rect);
        assert!((card.width() - THEME.dialog_w).abs() < TOL);
    }

    #[test]
    fn enter_confirms_and_escape_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let key = |key| egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };

        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor_with_devices(dir.path().join("config.json"));
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        dialog.latency_ms = 7;
        let _ = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let _ = h.frame(vec![key(egui::Key::Enter)], |ui| dialog.show(ui.ctx(), &mut editor));
        assert!(dialog.is_closed());
        assert_eq!(editor.config().latency_offset_ms, 7);

        let mut h = Headless::new(1280.0, 800.0);
        let mut dialog = AudioSettingsDialog::new_open(&editor);
        dialog.latency_ms = 50;
        let _ = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let _ = h.frame(vec![key(egui::Key::Escape)], |ui| dialog.show(ui.ctx(), &mut editor));
        assert!(dialog.is_closed());
        assert_eq!(editor.config().latency_offset_ms, 7, "Esc discards");
    }
}
