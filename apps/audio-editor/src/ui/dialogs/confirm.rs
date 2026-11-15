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
        ConfirmDialog { open: true, title: title.into(), message: message.into() }
    }

    /// Shows the dialog; OK, Esc or clicking the backdrop dismisses it.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let mut ok = false;
        let response = super::modal("confirm_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, &self.title);
            ui.add_space(THEME.space_xs);
            super::description(ui, &self.message);
            super::before_form(ui);
            super::footer(ui, |ui| {
                if super::footer_button(ui, "OK", super::FooterButton::Primary, true).clicked() {
                    ok = true;
                }
            });
            // Enter confirms.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                ok = true;
            }
        });
        // Esc or the backdrop dismisses the dialog.
        if ok || response.should_close() {
            self.open = false;
        }
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
        CloseConfirmDialog { message: format!("Save changes to {project_name}?") }
    }

    /// Shows the modal; Save / Don't Save / Cancel. Enter saves, Esc, the
    /// backdrop and Cancel abort the close.
    pub fn show(&self, ctx: &egui::Context) -> Option<CloseChoice> {
        let mut choice = None;
        let response = super::modal("close_confirm_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Unsaved Changes");
            ui.add_space(THEME.space_xs);
            super::description(ui, &self.message);
            super::before_form(ui);
            super::footer(ui, |ui| {
                if super::footer_button(ui, "Save", super::FooterButton::Primary, true).clicked() {
                    choice = Some(CloseChoice::Save);
                }
                if super::footer_button(ui, "Don't Save", super::FooterButton::Destructive, true).clicked() {
                    choice = Some(CloseChoice::DontSave);
                }
                if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
            // Enter saves.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                choice = Some(CloseChoice::Save);
            }
        });
        // Esc and the backdrop cancel the close.
        if choice.is_none() && response.should_close() {
            choice = Some(CloseChoice::Cancel);
        }
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
        let response = super::modal("export_close_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Exporting");
            ui.add_space(THEME.space_xs);
            super::description(ui, "An export is in progress. Cancel it and close?");
            super::before_form(ui);
            super::footer(ui, |ui| {
                if super::footer_button(ui, "Keep Exporting", super::FooterButton::Primary, true).clicked() {
                    choice = Some(ExportCloseChoice::KeepExporting);
                }
                if super::footer_button(ui, "Cancel Export and Close", super::FooterButton::Cancel, true).clicked() {
                    choice = Some(ExportCloseChoice::CancelExport);
                }
            });
        });
        // Esc and the backdrop keep the export running.
        if choice.is_none() && response.should_close() {
            choice = Some(ExportCloseChoice::KeepExporting);
        }
        choice
    }
}

/// What the user chose in the recording-in-progress close prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordCloseChoice {
    /// End the take and keep it, then continue closing.
    KeepTake,
    /// End the take and throw it away, then continue closing.
    Discard,
    /// Keep recording; the close is aborted.
    Cancel,
}

/// Modal shown when quitting, opening or closing while a take is recording:
/// "A recording is in progress." with Keep Take, Discard and Cancel.
pub struct RecordCloseDialog;

impl RecordCloseDialog {
    /// Shows the modal. Enter keeps the take; Esc, the backdrop and Cancel
    /// leave the recording running.
    pub fn show(&self, ctx: &egui::Context) -> Option<RecordCloseChoice> {
        let mut choice = None;
        let response = super::modal("record_close_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Recording");
            ui.add_space(THEME.space_xs);
            super::description(ui, "A recording is in progress.");
            super::before_form(ui);
            super::footer(ui, |ui| {
                if super::footer_button(ui, "Keep Take", super::FooterButton::Primary, true).clicked() {
                    choice = Some(RecordCloseChoice::KeepTake);
                }
                if super::footer_button(ui, "Discard", super::FooterButton::Destructive, true).clicked() {
                    choice = Some(RecordCloseChoice::Discard);
                }
                if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                    choice = Some(RecordCloseChoice::Cancel);
                }
            });
            // Enter keeps the take (nothing is lost).
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                choice = Some(RecordCloseChoice::KeepTake);
            }
        });
        // Esc and the backdrop cancel the close.
        if choice.is_none() && response.should_close() {
            choice = Some(RecordCloseChoice::Cancel);
        }
        choice
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_util::Headless;

    /// Frames the unsaved-changes prompt in `h`, remembering the first choice.
    fn show_prompt(h: &mut Headless, events: Vec<egui::Event>, got: &mut Option<CloseChoice>) -> crate::ui::test_util::Out {
        let dialog = CloseConfirmDialog::new("Test 2");
        h.frame(events, |ui| {
            if let Some(choice) = dialog.show(ui.ctx()) {
                got.get_or_insert(choice);
            }
        })
    }

    #[test]
    fn unsaved_footer_reads_save_dont_save_cancel_left_to_right() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut got = None;
        for _ in 0..2 {
            let _ = show_prompt(&mut h, Vec::new(), &mut got);
        }
        let out = show_prompt(&mut h, Vec::new(), &mut got);
        let save = out.text_rect("Save").expect("Save button").center().x;
        let dont = out.text_rect("Don't Save").expect("Don't Save").center().x;
        let cancel = out.text_rect("Cancel").expect("Cancel button").center().x;
        assert!(save < dont && dont < cancel, "{save} {dont} {cancel}");
        assert_eq!(got, None, "nothing is chosen without a click");
    }

    #[test]
    fn unsaved_footer_buttons_are_8px_apart_and_flush_right() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut got = None;
        let dialog = CloseConfirmDialog::new("Test 2");
        let out = h.settle(|ui| {
            if let Some(choice) = dialog.show(ui.ctx()) {
                got.get_or_insert(choice);
            }
        });
        let button = |label: &str| {
            let center = out.text_rect(label).expect("button text").center();
            out.filled_rect_at(center).expect("button background")
        };
        let (save, dont, cancel) = (button("Save"), button("Don't Save"), button("Cancel"));
        let tol = 0.6;
        assert!((dont.left() - save.right() - THEME.space_md).abs() < tol);
        assert!((cancel.left() - dont.right() - THEME.space_md).abs() < tol);
        let line = out.footer_line().expect("footer divider");
        assert!((cancel.right() - line.right()).abs() < tol, "Cancel ends {} short of the content edge", line.right() - cancel.right());
    }

    #[test]
    fn clicking_each_footer_button_returns_its_choice() {
        for (label, expected) in [("Save", CloseChoice::Save), ("Don't Save", CloseChoice::DontSave), ("Cancel", CloseChoice::Cancel)] {
            let mut h = Headless::new(1280.0, 800.0);
            let mut got = None;
            for _ in 0..2 {
                let _ = show_prompt(&mut h, Vec::new(), &mut got);
            }
            let out = show_prompt(&mut h, Vec::new(), &mut got);
            let target = out.text_rect(label).expect("button text").center();
            let dialog = CloseConfirmDialog::new("Test 2");
            let _ = h.click(target, |ui| {
                if let Some(choice) = dialog.show(ui.ctx()) {
                    got.get_or_insert(choice);
                }
            });
            assert_eq!(got, Some(expected), "clicking {label}");
        }
    }

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
