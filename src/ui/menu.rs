//! ui::menu: the top Project menu bar.

use eframe::egui;

use crate::editor::Editor;
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::dialogs::project_settings::ProjectSettingsDialog;
use crate::ui::dialogs::save_as::SaveAsDialog;

/// What the user chose in the Project menu this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectMenuAction {
    /// Nothing chosen.
    None,
    /// Close the project and open the New Project dialog on the start screen.
    New,
    /// Close the project and open the project picker on the start screen.
    Open,
    /// Close the project and return to the start screen.
    Close,
}

/// The Project menu plus its modals (save error, Save As).
#[derive(Default)]
pub struct ProjectMenu {
    /// Modal error dialog for a failed save, when shown.
    error_dialog: Option<ConfirmDialog>,
    /// Save As dialog, when open.
    save_as_dialog: Option<SaveAsDialog>,
    /// Project Settings dialog, when open.
    settings_dialog: Option<ProjectSettingsDialog>,
}

impl ProjectMenu {
    /// Shows the Project menu; returns the action the user chose.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> ProjectMenuAction {
        let mut action = ProjectMenuAction::None;
        ui.menu_button("Project", |menu| {
            if menu.button("New (Ctrl+N)").clicked() {
                action = ProjectMenuAction::New;
                menu.close();
            }
            if menu.button("Open... (Ctrl+O)").clicked() {
                action = ProjectMenuAction::Open;
                menu.close();
            }
            if menu.button("Save (Ctrl+S)").clicked() {
                self.save(editor);
                menu.close();
            }
            if menu.button("Save As... (Ctrl+Shift+S)").clicked() {
                self.open_save_as(editor);
                menu.close();
            }
            if menu.button("Close").clicked() {
                action = ProjectMenuAction::Close;
                menu.close();
            }
            if menu.button("Project Settings...").clicked() {
                self.open_settings(editor);
                menu.close();
            }
        });
        let ctx = ui.ctx().clone();
        if let Some(dialog) = &mut self.save_as_dialog {
            dialog.show(&ctx, editor);
            if dialog.is_closed() {
                self.save_as_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.settings_dialog {
            dialog.show(&ctx, editor);
            if dialog.is_closed() {
                self.settings_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.error_dialog {
            dialog.show(&ctx);
        }
        action
    }

    /// Saves the project; on failure shows the error modal.
    pub fn save(&mut self, editor: &mut Editor) {
        if let Err(error) = editor.save() {
            self.error_dialog = Some(ConfirmDialog::new("Save Error", error.to_string()));
        }
    }

    /// Opens the Save As dialog prefilled with the current name and folder.
    pub fn open_save_as(&mut self, editor: &mut Editor) {
        self.save_as_dialog = Some(SaveAsDialog::new_open(
            &editor.project.name,
            &editor.paths.root,
        ));
    }

    /// Opens the Project Settings dialog prefilled with the current settings.
    pub fn open_settings(&mut self, editor: &mut Editor) {
        self.settings_dialog = Some(ProjectSettingsDialog::new_open(
            &editor.project.name,
            editor.project.export_bit_depth,
        ));
    }
}
