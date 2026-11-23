//! The Settings dialog (spec "Settings dialog"): a ui-kit dialog with the sections in a list on
//! the left and the open section on the right. File > Settings (Ctrl+,) and View > Appearance
//! open it; Esc closes it. The app owns the open state and everything the sections edit.

use std::collections::VecDeque;

use egui::{Context, Id};

use ui_kit::fonts;
use ui_kit::tokens::{color, space, typography};

use crate::api::ApiServer;
use crate::config::Settings;
use crate::engine::bus::Command;
use crate::hotkeys::Hotkeys;

/// The sections of the Settings dialog, in the order the left list shows them (spec "Settings dialog").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    /// The Hide Recorder from screen captures switch.
    #[default]
    General,
    /// The naming template and the countdown.
    Recording,
    /// The read-only hotkey status table.
    Hotkeys,
    /// The local control API.
    ControlApi,
    /// The recent commands the engine received.
    Diagnostics,
    /// The accent picker and the glass switch.
    Appearance,
}

impl Section {
    /// Every section in list order (spec "Settings dialog").
    const ALL: [Section; 6] = [Section::General, Section::Recording, Section::Hotkeys, Section::ControlApi, Section::Diagnostics, Section::Appearance];

    /// The label the section list shows.
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Recording => "Recording",
            Self::Hotkeys => "Hotkeys",
            Self::ControlApi => "Control API",
            Self::Diagnostics => "Diagnostics",
            Self::Appearance => "Appearance",
        }
    }
}

/// The open state of the Settings dialog and the section its right side shows.
#[derive(Debug, Clone, Copy, Default)]
pub struct SettingsDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The section the right side shows.
    pub section: Section,
}

/// The next take the Recording section shows; the app computes it from the profile and settings.
#[derive(Debug, Clone, Default)]
pub struct NextTake {
    /// The full video file path of the next take.
    pub path: String,
}

/// The Control API state the dialog reads and edits; the app keeps it between frames.
pub struct ApiUi<'a> {
    /// The running API server, or None while it is off or could not start.
    pub server: Option<&'a ApiServer>,
    /// The token clients must send, or None when Credential Manager could not give one.
    pub token: Option<&'a str>,
    /// Whether the token is shown in full.
    pub show_token: &'a mut bool,
    /// The port as typed, applied to the settings when the field is left.
    pub port_edit: &'a mut u16,
}

/// What the dialog's rows changed this frame, for the app to act on.
#[derive(Debug, Clone, Copy, Default)]
pub struct Outcome {
    /// A settings field changed and the app must send and save the settings.
    pub settings_changed: bool,
    /// The Hide-from-screen-captures switch flipped and the app must re-apply the exclusion.
    pub hide_from_capture_changed: bool,
    /// The token Regenerate button was pressed.
    pub regenerate_token: bool,
}

/// The width the Settings body wants, clamped to the screen.
const BODY_WIDTH: f32 = 760.0;

/// The height the Settings body wants, clamped to the screen.
const BODY_HEIGHT: f32 = 520.0;

/// Width of the section list on the left.
const SECTION_LIST_WIDTH: f32 = 200.0;

/// The margin the dialog keeps to the screen edges on every side.
const SCREEN_MARGIN: f32 = 48.0;

/// Width of the naming template field.
const NAMING_FIELD_WIDTH: f32 = 320.0;

/// The countdown lengths the settings offer, in seconds; 0 is off.
const COUNTDOWN_CHOICES: [u8; 3] = [0, 3, 5];

/// Draws the Settings dialog while it is open and reports what its rows changed this frame.
pub fn show(
    ctx: &Context,
    dialog: &mut SettingsDialog,
    settings: &mut Settings,
    next_take: &NextTake,
    hotkeys: &Hotkeys,
    recent: &VecDeque<Command>,
    api: ApiUi<'_>,
) -> Outcome {
    let mut outcome = Outcome::default();
    let mut open = dialog.open;
    // Fixed body size, clamped to the screen: an egui::Modal has no width
    // limit of its own, so the body must never grow from its available_width().
    let screen = ctx.input(|input| input.content_rect());
    let body_width = BODY_WIDTH.min((screen.width() - 2.0 * SCREEN_MARGIN).max(0.0));
    let body_height = BODY_HEIGHT.min((screen.height() - 2.0 * SCREEN_MARGIN).max(0.0));
    ui_kit::dialog(
        ctx,
        Id::new("recorder-settings-dialog"),
        "Settings",
        &mut open,
        |ui| {
            ui.set_width(body_width);
            ui.set_height(body_height);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(SECTION_LIST_WIDTH);
                    for candidate in Section::ALL {
                        if ui.selectable_label(dialog.section == candidate, candidate.label()).clicked() {
                            dialog.section = candidate;
                        }
                    }
                });
                ui.separator();
                let content_width = (body_width - SECTION_LIST_WIDTH - 2.0 * ui.spacing().item_spacing.x - 1.0).max(0.0);
                egui::ScrollArea::vertical().auto_shrink(false).min_scrolled_width(content_width).max_width(content_width).max_height(body_height).show(
                    ui,
                    |ui| match dialog.section {
                        Section::General => general_section(ui, settings, &mut outcome),
                        Section::Recording => recording_section(ui, settings, next_take, &mut outcome),
                        Section::Hotkeys => hotkey_table(ui, hotkeys),
                        Section::ControlApi => api_section(ui, settings, api, &mut outcome),
                        Section::Diagnostics => diagnostics_section(ui, recent),
                        Section::Appearance => {
                            ui_kit::accent_picker(ui);
                        }
                    },
                );
            });
        },
        |_ui| {},
    );
    dialog.open = open;
    outcome
}

/// The General section: the Hide Recorder from screen captures switch.
fn general_section(ui: &mut egui::Ui, settings: &mut Settings, outcome: &mut Outcome) {
    let response = ui.checkbox(&mut settings.hide_from_capture, "Hide Recorder from screen captures");
    if response.changed() {
        outcome.settings_changed = true;
        outcome.hide_from_capture_changed = true;
    }
}

/// The Recording section: the naming template with its tokens hint and the next-take path,
/// the countdown length and the prompter switch (spec "Settings dialog").
fn recording_section(ui: &mut egui::Ui, settings: &mut Settings, next_take: &NextTake, outcome: &mut Outcome) {
    ui.horizontal(|ui| {
        ui.label("Naming template");
        if ui.add(egui::TextEdit::singleline(&mut settings.naming_template).desired_width(NAMING_FIELD_WIDTH)).changed() {
            outcome.settings_changed = true;
        }
    });
    ui.add(
        egui::Label::new(
            egui::RichText::new("Tokens: {project} {date} {time} {take} {profile}.").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY),
        )
        .selectable(false),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(format!("Next take: {}", next_take.path)).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY),
        )
        .selectable(false)
        .truncate(),
    )
    .on_hover_text(&next_take.path);
    ui.add_space(space::SCALE_8);
    let before = settings.countdown_secs;
    egui::ComboBox::from_label("Countdown before recording").selected_text(countdown_label(settings.countdown_secs)).show_ui(ui, |ui| {
        for secs in COUNTDOWN_CHOICES {
            ui.selectable_value(&mut settings.countdown_secs, secs, countdown_label(secs));
        }
    });
    if settings.countdown_secs != before {
        outcome.settings_changed = true;
    }
    if ui.checkbox(&mut settings.hide_countdown_when_linked, "Hide my countdown when a prompter is linked").changed() {
        outcome.settings_changed = true;
    }
}

/// The Diagnostics section: the recent commands the engine received, newest first.
fn diagnostics_section(ui: &mut egui::Ui, recent: &VecDeque<Command>) {
    if recent.is_empty() {
        ui.label("None yet. Press a hotkey.");
        return;
    }
    for command in recent.iter().rev() {
        ui.label(format!("{:?}", command.kind));
    }
}

/// The Control API section: the switch, the port, the listening and clients lines and the
/// token with Show or Hide, Copy and Regenerate (spec "Settings dialog").
fn api_section(ui: &mut egui::Ui, settings: &mut Settings, api: ApiUi<'_>, outcome: &mut Outcome) {
    if ui.checkbox(&mut settings.api.enabled, "On").changed() {
        outcome.settings_changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("Port");
        let response = ui.add(egui::DragValue::new(api.port_edit).range(1024..=65535).speed(0.0));
        if response.lost_focus() || response.drag_stopped() {
            settings.api.port = *api.port_edit;
            outcome.settings_changed = true;
        }
    });
    match api.server {
        Some(server) => {
            ui.label(format!("Listening on 127.0.0.1:{}", server.port()));
            ui.label(format!("Connected clients: {}", server.client_count()));
        }
        None => {
            ui.label("Not running");
        }
    }
    let Some(api_token) = api.token else { return };
    ui.horizontal_wrapped(|ui| {
        ui.label("Token");
        let shown = if *api.show_token { api_token.to_owned() } else { "*".repeat(api_token.chars().count().min(24)) };
        ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).wrap());
    });
    ui.horizontal(|ui| {
        if ui.button(if *api.show_token { "Hide" } else { "Show" }).clicked() {
            *api.show_token = !*api.show_token;
        }
        if ui.button("Copy").clicked() {
            ui.ctx().copy_text(api_token.to_owned());
        }
        if ui.button("Regenerate").on_hover_text("Makes a new token and disconnects every client").clicked() {
            outcome.regenerate_token = true;
        }
    });
}

/// The read-only hotkey status table (chord, action, status), as today (spec "Settings dialog").
fn hotkey_table(ui: &mut egui::Ui, hotkeys: &Hotkeys) {
    let warn = ui.visuals().warn_fg_color;
    let error = ui.visuals().error_fg_color;
    egui::Grid::new("settings-hotkeys").num_columns(3).striped(true).show(ui, |ui| {
        ui.strong("Chord");
        ui.strong("Action");
        ui.strong("Status");
        ui.end_row();
        for hotkey in &hotkeys.registered {
            ui.monospace(&hotkey.chord);
            ui.label(&hotkey.action);
            match hotkeys.altgr.iter().find(|conflict| conflict.action == hotkey.action) {
                Some(conflict) => {
                    ui.colored_label(warn, format!("AltGr types '{}'", conflict.typed));
                }
                None => {
                    ui.label("OK");
                }
            }
            ui.end_row();
        }
        for hotkey in &hotkeys.refused {
            ui.monospace(&hotkey.chord);
            ui.label(&hotkey.action);
            ui.colored_label(error, "Taken by another program").on_hover_text(&hotkey.reason);
            ui.end_row();
        }
    });
}

/// The text for a countdown length in the settings.
fn countdown_label(secs: u8) -> String {
    if secs == 0 { "Off".to_owned() } else { format!("{secs} seconds") }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The countdown lengths get their labels: Off for 0, "N seconds" otherwise.
    #[test]
    fn countdown_lengths_get_their_labels() {
        assert_eq!(countdown_label(0), "Off");
        assert_eq!(countdown_label(3), "3 seconds");
        assert_eq!(countdown_label(5), "5 seconds");
    }
}
