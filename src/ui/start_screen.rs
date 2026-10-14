//! ui::start_screen: start screen with New, Open and Recent Projects.

use std::path::PathBuf;

use eframe::egui;

use crate::editor::{Editor, RecentEntry, recent_projects, remove_recent};
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
        opened
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

    /// Attempts Editor::open and records the error message on failure.
    fn try_open(&mut self, path: PathBuf, opened: &mut Option<Editor>) {
        match Editor::open(&path) {
            Ok(editor) => {
                self.recent = None;
                *opened = Some(editor);
            }
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
