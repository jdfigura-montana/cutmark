//! ui::dialogs::project_settings: the Project Settings dialog.

use std::cell::Cell;

use eframe::egui;

use super::FORM_W;
use crate::editor::Editor;
use crate::editor::action::Action;
use crate::model::project::BitDepth;

/// Modal dialog for editing the project name and default export bit depth.
pub struct ProjectSettingsDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// Project name typed by the user.
    name: String,
    /// Default export bit depth.
    bit_depth: BitDepth,
    /// Last apply error, shown in red.
    error: Option<String>,
}

impl ProjectSettingsDialog {
    /// Creates the dialog open, prefilled with the current name and bit depth.
    pub fn new_open(current_name: &str, current_depth: BitDepth) -> Self {
        ProjectSettingsDialog { open: true, name: current_name.to_string(), bit_depth: current_depth, error: None }
    }

    /// Shows the dialog; applies Action::SetProjectSettings through the Editor
    /// on OK and closes on success.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = true;
        let (mut clicked, mut enter, mut cancel) = (false, false, false);
        let can_apply = Cell::new(false);
        let ProjectSettingsDialog { name, bit_depth, error, .. } = self;
        super::dialog(
            ctx,
            "project_settings_dialog",
            "Project Settings",
            FORM_W,
            &mut open,
            |ui| {
                ui_kit::help::context(ui, crate::help::CONTEXT_DIALOG_PROJECT_SETTINGS);
                super::description(ui, "Name and default export bit depth of this project.");
                ui_kit::inspector_group(ui, |ui| {
                    ui_kit::inspector_row(
                        ui,
                        "Name",
                        |ui| {
                            ui_kit::text_field(ui, name, "Project name");
                        },
                        None,
                    );
                    ui_kit::inspector_row(
                        ui,
                        "Bit depth",
                        |ui| {
                            super::choice(
                                ui,
                                "ps_depth",
                                &[(BitDepth::Int16, "16-bit"), (BitDepth::Int24, "24-bit"), (BitDepth::Float32, "32-bit float")],
                                bit_depth,
                            )
                        },
                        None,
                    );
                });
                if let Some(error) = &*error {
                    super::error_text(ui, error);
                }
                can_apply.set(!name.trim().is_empty());
                // Enter confirms (when valid).
                enter = super::enter_pressed(ui) && can_apply.get();
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if super::primary_button(ui, "OK", can_apply.get()).clicked() {
                    clicked = true;
                }
            },
        );
        // Cancel, Esc or the backdrop closes the dialog.
        if cancel || !open {
            *self = ProjectSettingsDialog { open: false, name: String::new(), bit_depth: BitDepth::Int24, error: None };
        } else if clicked || enter {
            self.try_apply(editor);
        }
    }

    /// True when the dialog is fully dismissed (no longer shown).
    pub fn is_closed(&self) -> bool {
        !self.open
    }

    /// Applies the settings through the Editor and records the error on failure.
    fn try_apply(&mut self, editor: &mut Editor) {
        self.error = None;
        let action = Action::SetProjectSettings { name: self.name.clone(), export_bit_depth: self.bit_depth };
        match editor.apply(action) {
            Ok(()) => {
                self.open = false;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
