//! ui::dialogs::save_as: the Save As dialog (Project > Save As, Ctrl+Shift+S).

use std::cell::Cell;
use std::path::PathBuf;

use eframe::egui;

use super::FORM_W;
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
        SaveAsDialog { open: true, name: current_name.to_string(), parent: current_root.parent().map(|p| p.to_path_buf()), error: None }
    }

    /// Shows the dialog; runs Editor::save_as and closes on success.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        let mut open = true;
        let (mut clicked, mut enter, mut cancel) = (false, false, false);
        let can_save = Cell::new(false);
        let SaveAsDialog { name, parent, error, .. } = self;
        super::dialog(
            ctx,
            "save_as_dialog",
            "Save As",
            FORM_W,
            &mut open,
            |ui| {
                ui_kit::help::context(ui, crate::help::CONTEXT_DIALOG_SAVE_AS);
                super::description(ui, "Copy the project to a new folder and name.");
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
                        "Parent folder",
                        |ui| {
                            let folder_text = match &*parent {
                                Some(path) => path.display().to_string(),
                                None => String::new(),
                            };
                            let browse = super::with_browse(ui, |ui| {
                                ui.add(egui::Label::new(folder_text).truncate());
                            });
                            if browse {
                                *parent = rfd::FileDialog::new().pick_folder();
                            }
                        },
                        None,
                    );
                });
                if let Some(error) = &*error {
                    super::error_text(ui, error);
                }
                can_save.set(!name.trim().is_empty() && parent.is_some());
                // Enter confirms (when valid).
                enter = super::enter_pressed(ui) && can_save.get();
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if super::primary_button(ui, "Save", can_save.get()).clicked() {
                    clicked = true;
                }
            },
        );
        // Cancel, Esc or the backdrop closes the dialog.
        if cancel || !open {
            *self = SaveAsDialog { open: false, name: String::new(), parent: None, error: None };
        } else if clicked || enter {
            self.try_save_as(editor);
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
            ProjectPaths { root: PathBuf::from("unused") },
        );
        let long = "C:/Users/someone/Documents/Programming_Projects/Audio_Editor/projects/a/very/deep/folder/tree/Test 2";
        let mut dialog = SaveAsDialog::new_open("Test 2", std::path::Path::new(long));
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));
        let content = out.dialog_content().expect("dialog card");
        let text = out.text_rect("Browse...").expect("Browse button text");
        let browse = out.filled_rect_at(text.center()).expect("Browse button");
        // The inspector row keeps the group margin, its inset and one gap
        // (4 + 4 + 8) between a control and the card's content edge.
        let edge = content.right() - ui_kit::tokens::space::SCALE_16;
        assert!((browse.right() - edge).abs() < 0.6, "Browse ends {} off the row's control edge", edge - browse.right());
        assert!(out.texts().iter().all(|t| t.rect.right() <= content.right() + 0.6), "no text runs past the card content");
    }
}
