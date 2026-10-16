//! ui::dialogs::save_as: the Save As dialog (Project > Save As, Ctrl+Shift+S).

use std::path::PathBuf;

use eframe::egui;

use crate::editor::Editor;
use crate::ui::theme::THEME;

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
        let mut cancel = false;
        let response = super::modal("save_as_dialog").show(ctx, |ui| {
            super::card_width(ui, THEME.dialog_w);
            super::title(ui, "Save As");
            ui.add_space(THEME.space_xs);
            super::description(ui, "Copy the project to a new folder and name.");
            super::before_form(ui);
            super::form_row(ui, "Name", |ui| {
                super::text_field(ui, &mut self.name, "sa_name");
            });
            super::form_row(ui, "Parent folder", |ui| {
                let folder_text = match &self.parent {
                    Some(path) => path.display().to_string(),
                    None => String::new(),
                };
                let browse = super::with_browse(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(folder_text)
                                .color(THEME.text_normal)
                                .size(THEME.dialog_label_size),
                        )
                        .truncate(),
                    );
                });
                if browse {
                    self.parent = rfd::FileDialog::new().pick_folder();
                }
            });
            if let Some(error) = &self.error {
                ui.label(egui::RichText::new(error).color(THEME.error_soft));
            }
            let can_save = !self.name.trim().is_empty() && self.parent.is_some();
            super::footer(ui, |ui| {
                if super::footer_button(ui, "Save", super::FooterButton::Primary, can_save)
                    .clicked()
                {
                    self.try_save_as(editor);
                }
                if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                    cancel = true;
                }
            });
            // Enter confirms (when valid).
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_save {
                self.try_save_as(editor);
            }
        });
        // Esc or the backdrop cancels.
        if response.should_close() {
            cancel = true;
        }
        if cancel {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::project::{BitDepth, Project, ViewState};
    use crate::ui::test_util::Headless;

    #[test]
    fn browse_stays_flush_right_even_with_a_very_long_parent_path() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = Editor::with_project(
            Project::new("Test 2".to_string(), 48_000, BitDepth::Int24),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let long = "C:/Users/someone/Documents/Programming_Projects/Audio_Editor/projects/a/very/deep/folder/tree/Test 2";
        let mut dialog = SaveAsDialog::new_open("Test 2", std::path::Path::new(long));
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let line = out.footer_line().expect("footer divider");
        let text = out.text_rect("Browse...").expect("Browse button text");
        let browse = out.filled_rect_at(text.center()).expect("Browse button");
        assert!(
            (browse.right() - line.right()).abs() < 0.6,
            "Browse ends {} short of the content edge",
            line.right() - browse.right()
        );
        assert!(
            out.texts()
                .iter()
                .all(|t| t.rect.right() <= line.right() + 0.6),
            "no text runs past the card content"
        );
    }
}
