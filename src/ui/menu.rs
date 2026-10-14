//! ui::menu: the top Project, Edit, Process and View menus.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::dialogs::export::ExportDialog;
use crate::ui::dialogs::project_settings::ProjectSettingsDialog;
use crate::ui::dialogs::save_as::SaveAsDialog;
use crate::ui::timeline::ViewMenuAction;

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
    /// Open the media import picker.
    Import,
}

/// The Project, Edit and View menus plus their modals (save error, Save As).
#[derive(Default)]
pub struct ProjectMenu {
    /// Modal error dialog for a failed save, when shown.
    error_dialog: Option<ConfirmDialog>,
    /// Save As dialog, when open.
    save_as_dialog: Option<SaveAsDialog>,
    /// Project Settings dialog, when open.
    settings_dialog: Option<ProjectSettingsDialog>,
    /// Export dialog; kept for the whole session so the last choices are
    /// remembered between opens.
    export_dialog: Option<ExportDialog>,
}

impl ProjectMenu {
    /// Shows the menus; returns the Project and View actions the user chose.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
    ) -> (ProjectMenuAction, ViewMenuAction) {
        let mut action = ProjectMenuAction::None;
        let mut view_action = ViewMenuAction::None;
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
            if menu.button("Import Media... (Ctrl+I)").clicked() {
                action = ProjectMenuAction::Import;
                menu.close();
            }
            if !editor.is_locked() {
                if menu.button("Export... (Ctrl+E)").clicked() {
                    self.open_export(editor);
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Export... (Ctrl+E)"));
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
        ui.menu_button("Edit", |menu| {
            match editor.undo_label() {
                Some(label) => {
                    if menu.button(format!("Undo {label} (Ctrl+Z)")).clicked() {
                        let _ = editor.undo();
                        menu.close();
                    }
                }
                None => {
                    menu.add_enabled(false, egui::Button::new("Undo (Ctrl+Z)"));
                }
            }
            match editor.redo_label() {
                Some(label) => {
                    if menu.button(format!("Redo {label} (Ctrl+Y)")).clicked() {
                        let _ = editor.redo();
                        menu.close();
                    }
                }
                None => {
                    menu.add_enabled(false, egui::Button::new("Redo (Ctrl+Y)"));
                }
            }
            menu.separator();
            let selection = editor.selection().clone();
            let has_selection = !matches!(selection, crate::editor::selection::Selection::None);
            let can_paste = editor
                .clipboard()
                .is_some_and(|c| c.tracks.iter().any(|t| !t.is_empty()));
            let can_split = {
                let pos = editor.view.playhead;
                let range_tracks: Option<Vec<Uuid>> = match &selection {
                    crate::editor::selection::Selection::Range { tracks, .. } => {
                        Some(tracks.clone())
                    }
                    _ => None,
                };
                editor.project.tracks.iter().any(|track| {
                    range_tracks
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&track.id))
                        && track
                            .clips
                            .iter()
                            .any(|clip| clip.start < pos && pos < clip.end())
                })
            };
            let can_trim = matches!(selection, crate::editor::selection::Selection::Range { .. });
            let can_select_all = !editor.project.tracks.is_empty() && editor.project_length() > 0;

            if has_selection {
                if menu.button("Cut (Ctrl+X)").clicked() {
                    let _ = editor.cut_selection();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Cut (Ctrl+X)"));
            }
            if has_selection {
                if menu.button("Copy (Ctrl+C)").clicked() {
                    editor.copy_selection();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Copy (Ctrl+C)"));
            }
            if can_paste {
                if menu.button("Paste (Ctrl+V)").clicked() {
                    let _ = editor.paste();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Paste (Ctrl+V)"));
            }
            if has_selection {
                if menu.button("Delete (Del)").clicked() {
                    match &selection {
                        crate::editor::selection::Selection::Clips(ids) => {
                            let _ = editor.apply(crate::editor::action::Action::DeleteClips {
                                clip_ids: ids.clone(),
                                ripple: editor.view.ripple,
                            });
                        }
                        crate::editor::selection::Selection::Range { start, end, tracks } => {
                            let _ = editor.apply(crate::editor::action::Action::DeleteRange {
                                s: *start,
                                e: *end,
                                track_ids: tracks.clone(),
                                ripple: editor.view.ripple,
                            });
                            editor.clear_selection();
                            editor.set_playhead(*start);
                        }
                        crate::editor::selection::Selection::None => {}
                    }
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Delete (Del)"));
            }
            menu.separator();
            if can_split {
                if menu.button("Split (S)").clicked() {
                    let _ = editor.split();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Split (S)"));
            }
            if can_trim {
                if menu.button("Trim to Selection (Ctrl+T)").clicked() {
                    let _ = editor.trim_to_selection();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Trim to Selection (Ctrl+T)"));
            }
            menu.separator();
            if can_select_all {
                if menu.button("Select All (Ctrl+A)").clicked() {
                    editor.select_all();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Select All (Ctrl+A)"));
            }
            if has_selection {
                if menu.button("Clear Selection (Esc)").clicked() {
                    editor.clear_selection();
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Clear Selection (Esc)"));
            }
            menu.separator();
            let mut ripple = editor.view.ripple;
            if menu
                .add(egui::Checkbox::new(&mut ripple, "Ripple (R)"))
                .changed()
            {
                editor.toggle_ripple();
            }
            let mut snap = editor.view.snap;
            if menu
                .add(egui::Checkbox::new(&mut snap, "Snap (N)"))
                .changed()
            {
                editor.toggle_snap();
            }
        });
        ui.menu_button("Markers", |menu| {
            if menu.button("Add Marker (M)").clicked() {
                let _ = editor.add_marker_at_playhead();
                menu.close();
            }
            if menu.button("Previous Marker (,)").clicked() {
                editor.jump_prev_marker();
                menu.close();
            }
            if menu.button("Next Marker (.)").clicked() {
                editor.jump_next_marker();
                menu.close();
            }
            if let Some(id) = editor.selected_marker() {
                if menu.button("Delete Marker (Del)").clicked() {
                    let _ = editor
                        .apply(crate::editor::action::Action::DeleteMarkers { ids: vec![id] });
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Delete Marker (Del)"));
            }
        });
        ui.menu_button("Process", |menu| {
            let has_target = editor.process_target().is_some();
            let has_range = matches!(
                editor.selection(),
                crate::editor::selection::Selection::Range { .. }
            );
            if has_target {
                if menu.button("Gain... (Ctrl+G)").clicked() {
                    crate::ui::dialogs::gain::request_process(menu.ctx());
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Gain... (Ctrl+G)"));
            }
            if has_target {
                if menu.button("Normalize... (Ctrl+Shift+N)").clicked() {
                    crate::ui::dialogs::normalize::request(menu.ctx());
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Normalize... (Ctrl+Shift+N)"));
            }
            if has_target {
                if menu.button("Fade In (Ctrl+Shift+I)").clicked() {
                    if let Err(error) = editor.fade_in() {
                        self.show_process_error(error);
                    }
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Fade In (Ctrl+Shift+I)"));
            }
            if has_target {
                if menu.button("Fade Out (Ctrl+Shift+O)").clicked() {
                    if let Err(error) = editor.fade_out() {
                        self.show_process_error(error);
                    }
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Fade Out (Ctrl+Shift+O)"));
            }
            if has_range {
                if menu.button("Silence (Ctrl+L)").clicked() {
                    if let Err(error) = editor.silence() {
                        self.show_process_error(error);
                    }
                    menu.close();
                }
            } else {
                menu.add_enabled(false, egui::Button::new("Silence (Ctrl+L)"));
            }
        });
        ui.menu_button("View", |menu| {
            if menu.button("Zoom In (+)").clicked() {
                view_action = ViewMenuAction::ZoomIn;
                menu.close();
            }
            if menu.button("Zoom Out (-)").clicked() {
                view_action = ViewMenuAction::ZoomOut;
                menu.close();
            }
            if !matches!(
                editor.selection(),
                crate::editor::selection::Selection::Range { .. }
            ) {
                menu.add_enabled(false, egui::Button::new("Zoom to Selection (Ctrl+\\)"));
            } else if menu.button("Zoom to Selection (Ctrl+\\)").clicked() {
                view_action = ViewMenuAction::ZoomToSelection;
                menu.close();
            }
            if menu.button("Zoom to Fit (\\)").clicked() {
                view_action = ViewMenuAction::ZoomToFit;
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
        if let Some(dialog) = &mut self.export_dialog {
            dialog.show(&ctx, editor);
        }
        if let Some(dialog) = &mut self.error_dialog {
            dialog.show(&ctx);
        }
        (action, view_action)
    }

    /// Shows a failed Process menu operation in the error modal, with the
    /// ops prefix stripped so the SPEC's message shows verbatim.
    fn show_process_error(&mut self, error: crate::editor::editor::EditorError) {
        let text = error.to_string();
        self.error_dialog = Some(ConfirmDialog::new(
            "Process",
            text.strip_prefix("invalid operation: ")
                .unwrap_or(&text)
                .to_string(),
        ));
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

    /// Opens the Export dialog; created once per session, afterwards it
    /// reopens with the last choices.
    pub fn open_export(&mut self, editor: &Editor) {
        if self.export_dialog.is_none() {
            self.export_dialog = Some(ExportDialog::new_open(editor));
        }
        if let Some(dialog) = &mut self.export_dialog {
            dialog.reopen();
        }
    }
}
