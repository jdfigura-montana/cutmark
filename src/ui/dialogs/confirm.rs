//! ui::dialogs::confirm: modal dialog with a message and a single OK button.

use eframe::egui;

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
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(&self.message);
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    if ui.button("OK").clicked() {
                        ok = true;
                    }
                });
            });
        if ok {
            open = false;
        }
        self.open = open;
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
}
