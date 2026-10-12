//! ui::menu: the top Project menu bar.

use eframe::egui;

use crate::editor::Editor;
use crate::ui::dialogs::confirm::ConfirmDialog;

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

/// The Project menu plus the modal shown when Save fails.
#[derive(Default)]
pub struct ProjectMenu {
    /// Modal error dialog for a failed save, when shown.
    error_dialog: Option<ConfirmDialog>,
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
            if menu.button("Close").clicked() {
                action = ProjectMenuAction::Close;
                menu.close();
            }
        });
        if let Some(dialog) = &mut self.error_dialog {
            let ctx = ui.ctx().clone();
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
}
