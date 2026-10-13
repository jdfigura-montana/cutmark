//! ui::dialogs::project_settings: the Project Settings dialog.

use eframe::egui;

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
        ProjectSettingsDialog {
            open: true,
            name: current_name.to_string(),
            bit_depth: current_depth,
            error: None,
        }
    }

    /// Shows the dialog; applies Action::SetProjectSettings through the Editor
    /// on OK and closes on success.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = self.open;
        let mut cancel = false;
        egui::Window::new("Project Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("project_settings_grid")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.name);
                        ui.end_row();

                        ui.label("Export bit depth:");
                        egui::ComboBox::from_id_salt("settings_bit_depth")
                            .selected_text(bit_depth_text(self.bit_depth))
                            .show_ui(ui, |ui| {
                                for depth in [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32] {
                                    ui.selectable_value(
                                        &mut self.bit_depth,
                                        depth,
                                        bit_depth_text(depth),
                                    );
                                }
                            });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
                }
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    let can_apply = !self.name.trim().is_empty();
                    let apply = ui.add_enabled(can_apply, egui::Button::new("OK"));
                    if apply.clicked() {
                        self.try_apply(editor);
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            *self = ProjectSettingsDialog {
                open: false,
                name: String::new(),
                bit_depth: BitDepth::Int24,
                error: None,
            };
        }
    }

    /// True when the dialog is fully dismissed (no longer shown).
    pub fn is_closed(&self) -> bool {
        !self.open
    }

    /// Applies the settings through the Editor and records the error on failure.
    fn try_apply(&mut self, editor: &mut Editor) {
        self.error = None;
        let action = Action::SetProjectSettings {
            name: self.name.clone(),
            export_bit_depth: self.bit_depth,
        };
        match editor.apply(action) {
            Ok(()) => {
                self.open = false;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// Human-readable text for a bit depth.
fn bit_depth_text(depth: BitDepth) -> &'static str {
    match depth {
        BitDepth::Int16 => "16-bit",
        BitDepth::Int24 => "24-bit",
        BitDepth::Float32 => "32-bit float",
    }
}
