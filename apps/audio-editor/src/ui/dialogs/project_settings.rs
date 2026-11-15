//! ui::dialogs::project_settings: the Project Settings dialog.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::model::project::BitDepth;
use crate::ui::theme::THEME;

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
        let mut cancel = false;
        let response = super::modal("project_settings_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Project Settings");
            ui.add_space(THEME.space_xs);
            super::description(ui, "Name and default export bit depth of this project.");
            super::before_form(ui);
            super::form_row(ui, "Name", |ui| {
                super::text_field(ui, &mut self.name, "ps_name");
            });
            super::form_row(ui, "Bit depth", |ui| {
                super::segmented(
                    ui,
                    "ps_depth",
                    &[(BitDepth::Int16, "16-bit"), (BitDepth::Int24, "24-bit"), (BitDepth::Float32, "32-bit float")],
                    &mut self.bit_depth,
                );
            });
            if let Some(error) = &self.error {
                ui.label(egui::RichText::new(error).color(THEME.error_soft));
            }
            let can_apply = !self.name.trim().is_empty();
            super::footer(ui, |ui| {
                if super::footer_button(ui, "OK", super::FooterButton::Primary, can_apply).clicked() {
                    self.try_apply(editor);
                }
                if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                    cancel = true;
                }
            });
            // Enter confirms (when valid).
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_apply {
                self.try_apply(editor);
            }
        });
        // Esc or the backdrop cancels.
        if response.should_close() {
            cancel = true;
        }
        if cancel {
            *self = ProjectSettingsDialog { open: false, name: String::new(), bit_depth: BitDepth::Int24, error: None };
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
