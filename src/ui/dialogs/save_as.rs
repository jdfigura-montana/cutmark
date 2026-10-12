//! ui::dialogs::save_as: the Save As dialog (Project > Save As, Ctrl+Shift+S).

use std::path::PathBuf;

use eframe::egui;

use crate::editor::Editor;

/// Modal dialog for copying the open project to a new location and name.
pub struct SaveAsDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// New project name typed by the user.
    name: String,
    /// Parent folder chosen by the user.
    parent: Option<PathBuf>,
    /// Last save-as error, shown in red.
    error: Option<String>,
}

impl SaveAsDialog {
    /// Creates the dialog open, prefilled with the current project name and
    /// the parent of the current project folder.
    pub fn new_open(current_name: &str, current_root: &std::path::Path) -> Self {
        SaveAsDialog {
            open: true,
            name: current_name.to_string(),
            parent: current_root.parent().map(|p| p.to_path_buf()),
            error: None,
        }
    }

    /// Shows the dialog; runs Editor::save_as and closes on success.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = self.open;
        let mut cancel = false;
        egui::Window::new("Save As")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("save_as_grid")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.name);
                        ui.end_row();

                        ui.label("Parent folder:");
                        let folder_text = match &self.parent {
                            Some(path) => path.display().to_string(),
                            None => String::new(),
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(folder_text).weak()).truncate(),
                        );
                        if ui.button("Browse...").clicked() {
                            self.parent = rfd::FileDialog::new().pick_folder();
                        }
                        ui.end_row();
                    });

                ui.add_space(8.0);
                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
                }
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    let can_save = !self.name.trim().is_empty() && self.parent.is_some();
                    let save = ui.add_enabled(can_save, egui::Button::new("Save"));
                    if save.clicked() {
                        self.try_save_as(editor);
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            *self = SaveAsDialog {
                open: false,
                name: String::new(),
                parent: None,
                error: None,
            };
        }
    }

    /// True when the dialog is fully dismissed (no longer shown).
    pub fn is_closed(&self) -> bool {
        !self.open
    }

    /// Runs Editor::save_as and records the error message on failure.
    fn try_save_as(&mut self, editor: &mut Editor) {
        self.error = None;
        let parent = match &self.parent {
            Some(parent) => parent.clone(),
            None => return,
        };
        match editor.save_as(&parent, &self.name) {
            Ok(()) => {
                self.open = false;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
