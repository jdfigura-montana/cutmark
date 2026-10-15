//! ui::start_screen: start screen with New, Open and Recent Projects.

use std::path::{Path, PathBuf};

use eframe::egui;

use crate::editor::{Editor, OpenOutcome, RecentEntry, recent_projects, remove_recent};
use crate::ui::dialogs::new_project::NewProjectDialog;
use crate::ui::theme::THEME;

/// Start screen: New Project, Open Project/Folder and the Recent Projects list.
#[derive(Default)]
pub struct StartScreen {
    /// The New Project dialog, when open.
    dialog: Option<NewProjectDialog>,
    /// Cached recent list; None means it must be loaded from disk.
    recent: Option<Vec<RecentEntry>>,
    /// Last open error, shown in red under the buttons.
    error: Option<String>,
    /// True when the Project > Open picker must be shown on the next frame.
    pending_open: bool,
    /// Project path awaiting an autosave-recovery decision, if any.
    recovery: Option<PathBuf>,
}

impl StartScreen {
    /// Creates the start screen with the New Project dialog already open.
    pub fn with_new_dialog() -> Self {
        StartScreen {
            dialog: Some(NewProjectDialog::new_open()),
            ..StartScreen::default()
        }
    }

    /// Creates the start screen showing the Open Project picker next frame.
    pub fn with_open_request() -> Self {
        StartScreen {
            pending_open: true,
            ..StartScreen::default()
        }
    }

    /// True when the New Project dialog is currently open.
    pub fn dialog_is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Draws the start screen; returns the Editor to edit when one opens.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Editor> {
        let mut opened = None;
        if self.pending_open {
            self.pending_open = false;
            self.open_project_file(&mut opened);
        }
        if let Some(dialog) = &mut self.dialog {
            let ctx = ui.ctx().clone();
            if let Some(editor) = dialog.show(&ctx) {
                self.recent = None;
                opened = Some(editor);
            }
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(THEME.space_xxl);
                ui.heading("Audio Editor");
                ui.add_space(THEME.space_lg);

                if ui.button("New Project").clicked() {
                    self.dialog = Some(NewProjectDialog::new_open());
                }
                if ui.button("Open Project...").clicked() {
                    self.open_project_file(&mut opened);
                }
                if ui.button("Open Folder...").clicked() {
                    self.open_project_folder(&mut opened);
                }
                if let Some(error) = &self.error {
                    ui.add_space(THEME.space_md);
                    ui.label(egui::RichText::new(error).color(THEME.error_soft));
                }

                ui.add_space(THEME.space_xl);
                ui.separator();
                ui.add_space(THEME.space_md);
                ui.strong("Recent Projects");
                ui.add_space(THEME.space_sm);
                self.show_recent_list(ui, &mut opened);
            });
        });
        self.show_recovery_modal(ui, &mut opened);
        opened
    }

    /// Draws the autosave-recovery modal when a project needs a decision.
    fn show_recovery_modal(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        let Some(path) = self.recovery.clone() else {
            return;
        };
        #[derive(PartialEq)]
        enum Choice {
            Recover,
            LastSaved,
            Cancel,
        }
        let mut choice = None;
        let mut open = true;
        egui::Window::new("Recover autosave")
            .open(&mut open)
            .default_width(THEME.dialog_w)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.label(
                    "This project has unsaved changes from a session that didn't close properly.",
                );
                ui.add_space(THEME.space_md);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        if ui.button("Recover autosave").clicked() {
                            choice = Some(Choice::Recover);
                        }
                        if ui.button("Open last saved").clicked() {
                            choice = Some(Choice::LastSaved);
                        }
                        if ui.button("Cancel").clicked() {
                            choice = Some(Choice::Cancel);
                        }
                    });
                });
            });
        self.recovery = None;
        if open && choice != Some(Choice::Cancel) {
            self.finish_open(&path, choice == Some(Choice::Recover), opened);
        }
    }

    /// Opens the project either from the autosave or from the last save.
    fn finish_open(&mut self, path: &Path, recover: bool, opened: &mut Option<Editor>) {
        self.error = None;
        let result = if recover {
            Editor::open_recovered(path)
        } else {
            Editor::open_discarding_autosave(path)
        };
        match result {
            Ok(editor) => {
                self.recent = None;
                *opened = Some(editor);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Opens a project picked with a file dialog filtered to project.json.
    fn open_project_file(&mut self, opened: &mut Option<Editor>) {
        self.error = None;
        let picked = rfd::FileDialog::new()
            .add_filter("Project file (project.json)", &["json"])
            .pick_file();
        if let Some(path) = picked {
            self.try_open(path, opened);
        }
    }

    /// Opens a project picked with a folder dialog.
    fn open_project_folder(&mut self, opened: &mut Option<Editor>) {
        self.error = None;
        let picked = rfd::FileDialog::new().pick_folder();
        if let Some(path) = picked {
            self.try_open(path, opened);
        }
    }

    /// Attempts the recovery-checked open; a newer autosave raises the modal.
    fn try_open(&mut self, path: PathBuf, opened: &mut Option<Editor>) {
        match Editor::open_with_recovery_check(&path) {
            Ok(OpenOutcome::Opened(editor)) => {
                self.recent = None;
                *opened = Some(*editor);
            }
            Ok(OpenOutcome::NeedsRecoveryChoice(_)) => self.recovery = Some(path),
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Draws the cached recent list; missing entries are greyed with a Remove button.
    fn show_recent_list(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        if self.recent.is_none() {
            self.recent = Some(recent_projects());
        }
        let entries = self.recent.clone().unwrap_or_default();
        if entries.is_empty() {
            ui.label(egui::RichText::new("(no recent projects)").weak());
            return;
        }
        for entry in entries {
            ui.horizontal(|ui| {
                if entry.exists() {
                    if ui
                        .button(format!("{}  —  {}", entry.name, entry.root.display()))
                        .clicked()
                    {
                        self.try_open(entry.root.clone(), opened);
                    }
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  —  {} (missing)",
                            entry.name,
                            entry.root.display()
                        ))
                        .weak(),
                    );
                    if ui.button("Remove").clicked() {
                        remove_recent(&entry.root);
                        self.recent = None;
                    }
                }
            });
        }
    }
}
