//! ui::dialogs::preferences: the Preferences dialog, opened from Edit >
//! Preferences... (Ctrl+,). It has one section, Remote control.

use eframe::egui;
use egui::RichText;
use ui_kit::fonts;
use ui_kit::tokens::{space, typography};

use super::NARROW_W;
use crate::editor::Editor;

/// How often the open dialog repaints, so the status line follows a plugin
/// that connects or leaves while nothing else changes.
const STATUS_REFRESH: std::time::Duration = std::time::Duration::from_millis(500);

/// What the Stream Deck remote control is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RemoteStatus {
    /// The plugin is connected.
    Connected,
    /// The server runs and no plugin is connected.
    Waiting,
    /// No server runs.
    #[default]
    Off,
}

impl RemoteStatus {
    /// The status line's text.
    pub fn text(self) -> &'static str {
        match self {
            RemoteStatus::Connected => "Stream Deck connected",
            RemoteStatus::Waiting => "Waiting for Stream Deck",
            RemoteStatus::Off => "Off",
        }
    }
}

/// Modal dialog for the app-wide preferences. A switch applies as soon as it
/// is flipped, so the dialog has no Cancel.
pub struct PreferencesDialog {
    /// Whether the dialog is open.
    open: bool,
    /// The last save error, shown in red.
    error: Option<String>,
}

impl PreferencesDialog {
    /// Creates the dialog open.
    pub fn new_open() -> Self {
        PreferencesDialog { open: true, error: None }
    }

    /// True when the dialog is fully dismissed (no longer shown).
    pub fn is_closed(&self) -> bool {
        !self.open
    }

    /// Shows the dialog with the remote control's `status`. Done, Enter, Esc
    /// and the backdrop close it.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor, status: RemoteStatus) {
        let mut open = true;
        let (mut done, mut enter) = (false, false);
        super::dialog(
            ctx,
            "preferences_dialog",
            "Preferences",
            NARROW_W,
            &mut open,
            |ui| {
                ui_kit::inspector_header(ui, "Remote control", "Control CutMark Audio from an Elgato Stream Deck.");
                ui.add_space(space::SCALE_8);
                let mut on = editor.config().stream_deck;
                ui_kit::inspector_group(ui, |ui| {
                    ui_kit::inspector_row(
                        ui,
                        "Stream Deck",
                        |ui| {
                            if ui_kit::switch(ui, &mut on).changed() {
                                self.error = editor.set_stream_deck(on).err().map(|error| error.to_string());
                                // The app starts or drops the server this frame.
                                ui.ctx().request_repaint();
                            }
                        },
                        None,
                    );
                });
                ui.add_space(space::SCALE_8);
                let neutral = crate::ui::neutrals(ui.ctx());
                ui.label(RichText::new(status.text()).font(fonts::medium(typography::BODY_SIZE)).color(neutral.text_secondary));
                if let Some(error) = &self.error {
                    super::error_text(ui, error);
                }
                enter = super::enter_pressed(ui);
            },
            |ui| {
                if ui_kit::primary_button(ui, "Done").clicked() {
                    done = true;
                }
            },
        );
        ctx.request_repaint_after(STATUS_REFRESH);
        if done || enter || !open {
            self.open = false;
        }
    }
}
