//! ui::dialogs::confirm: modal dialogs (OK message, unsaved-changes and
//! export-in-progress close prompts).

use eframe::egui;

use crate::ui::theme::THEME;

/// Modal dialog showing a message with an OK button, e.g. for save errors.
pub struct ConfirmDialog {
    /// Whether the dialog is shown.
    pub open: bool,
    /// Window title.
    title: String,
    /// Message shown in the dialog body.
    message: String,
}

impl ConfirmDialog {
    /// Creates an open dialog with the given title and message.
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        ConfirmDialog {
            open: true,
            title: title.into(),
            message: message.into(),
        }
    }

    /// Shows the dialog; OK or the close box dismisses it.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut open = self.open;
        let mut ok = false;
        egui::Window::new(self.title.as_str())
            .open(&mut open)
            .default_width(THEME.dialog_w)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&self.message);
                ui.add_space(THEME.space_md);
                ui.vertical_centered(|ui| {
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
                // Enter confirms, Esc cancels.
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ok = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ok = true;
                }
            });
        self.open = open;
    }
}

/// What the user chose in the unsaved-changes close prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseChoice {
    /// Save the project, then continue closing.
    Save,
    /// Continue closing without saving; the autosave is left alone.
    DontSave,
    /// Abort the close.
    Cancel,
}

/// Reusable unsaved-changes modal: "Save changes to <name>?" with
/// Save, Don't Save and Cancel.
pub struct CloseConfirmDialog {
    /// Message shown in the dialog body.
    message: String,
}

impl CloseConfirmDialog {
    /// Creates the prompt for the given project name.
    pub fn new(project_name: &str) -> Self {
        CloseConfirmDialog {
            message: format!("Save changes to {project_name}?"),
        }
    }

    /// Shows the modal; Save / Don't Save / Cancel, the close box and Esc
    /// cancel, Enter saves.
    pub fn show(&self, ctx: &egui::Context) -> Option<CloseChoice> {
        let mut choice = None;
        egui::Window::new("Unsaved Changes")
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .default_width(THEME.dialog_w)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&self.message);
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(CloseChoice::Save);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(CloseChoice::DontSave);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(CloseChoice::Cancel);
                    }
                });
                // Enter saves, Esc and the close box cancel the close.
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    choice = Some(CloseChoice::Save);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        choice
    }
}

/// What the user chose in the export-in-progress close prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportCloseChoice {
    /// Cancel the running export, then continue closing.
    CancelExport,
    /// Keep the export running; the close is aborted.
    KeepExporting,
}

/// Modal shown when quitting or closing while an export runs.
pub struct ExportCloseDialog;

impl ExportCloseDialog {
    /// Shows the modal; "Cancel Export and Close" or "Keep Exporting";
    /// Esc keeps exporting.
    pub fn show(&self, ctx: &egui::Context) -> Option<ExportCloseChoice> {
        let mut choice = None;
        egui::Window::new("Exporting")
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .default_width(THEME.dialog_w)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("An export is in progress. Cancel it and close?");
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui.button("Cancel Export and Close").clicked() {
                        choice = Some(ExportCloseChoice::CancelExport);
                    }
                    if ui.button("Keep Exporting").clicked() {
                        choice = Some(ExportCloseChoice::KeepExporting);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(ExportCloseChoice::KeepExporting);
                }
            });
        choice
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_dialog_is_open_and_keeps_message() {
        let dialog = ConfirmDialog::new("Save Error", "disk full");
        assert!(dialog.open);
        assert_eq!(dialog.title, "Save Error");
        assert_eq!(dialog.message, "disk full");
    }

    #[test]
    fn close_prompt_names_the_project() {
        let dialog = CloseConfirmDialog::new("Song");
        assert_eq!(dialog.message, "Save changes to Song?");
    }
}
