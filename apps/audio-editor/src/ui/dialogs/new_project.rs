//! ui::dialogs::new_project: the New Project dialog.

use std::cell::Cell;
use std::path::PathBuf;

use eframe::egui;

use super::FORM_W;
use crate::editor::Editor;
use crate::model::project::BitDepth;

/// Modal dialog for creating a new project.
pub struct NewProjectDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// Project name typed by the user.
    name: String,
    /// Parent folder chosen by the user.
    parent: Option<PathBuf>,
    /// Sample rate in kHz (44, 48, 88, 96).
    sample_rate_khz: u32,
    /// Export bit depth.
    bit_depth: BitDepth,
    /// Last creation error, shown in red.
    error: Option<String>,
}

impl Default for NewProjectDialog {
    fn default() -> Self {
        NewProjectDialog { open: false, name: String::new(), parent: None, sample_rate_khz: 48, bit_depth: BitDepth::Int24, error: None }
    }
}

impl NewProjectDialog {
    /// Creates the dialog already open, e.g. from the start screen or Project > New.
    pub fn new_open() -> Self {
        NewProjectDialog { open: true, ..NewProjectDialog::default() }
    }
    /// Shows the dialog; returns the created Editor when the user succeeds.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Editor> {
        let mut created = None;
        if self.open {
            let mut open = true;
            let (mut clicked, mut enter, mut cancel) = (false, false, false);
            let can_create = Cell::new(false);
            let NewProjectDialog { name, parent, sample_rate_khz, bit_depth, error, .. } = self;
            super::dialog(
                ctx,
                "new_project_dialog",
                "New Project",
                FORM_W,
                &mut open,
                |ui| {
                    ui_kit::help::context(ui, crate::help::CONTEXT_DIALOG_NEW_PROJECT);
                    super::description(ui, "Create a project folder with a timeline and media bin.");
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
                        ui_kit::inspector_row(
                            ui,
                            "Sample rate",
                            |ui| super::choice(ui, "np_rate", &[(44, "44 kHz"), (48, "48 kHz"), (88, "88 kHz"), (96, "96 kHz")], sample_rate_khz),
                            None,
                        );
                        ui_kit::inspector_row(
                            ui,
                            "Bit depth",
                            |ui| {
                                super::choice(
                                    ui,
                                    "np_depth",
                                    &[(BitDepth::Int16, "16-bit"), (BitDepth::Int24, "24-bit"), (BitDepth::Float32, "32-bit float")],
                                    bit_depth,
                                )
                            },
                            None,
                        );
                    });
                    if let Some(error) = &*error {
                        super::error_text(ui, error);
                    }
                    can_create.set(!name.trim().is_empty() && parent.is_some());
                    // Enter confirms (when valid).
                    enter = super::enter_pressed(ui) && can_create.get();
                },
                |ui| {
                    if super::secondary_button(ui, "Cancel").clicked() {
                        cancel = true;
                    }
                    if super::primary_button(ui, "Create", can_create.get()).clicked() {
                        clicked = true;
                    }
                },
            );
            // Cancel, Esc or the backdrop closes the dialog.
            if cancel || !open {
                *self = NewProjectDialog { open: false, ..NewProjectDialog::default() };
            } else if clicked || enter {
                self.try_create(&mut created);
            }
        }
        created
    }

    /// Runs Editor::create and records the error message on failure.
    fn try_create(&mut self, created: &mut Option<Editor>) {
        self.error = None;
        let parent = match &self.parent {
            Some(parent) => parent.clone(),
            None => return,
        };
        let sample_rate = self.sample_rate_khz * 1000;
        match Editor::create(&self.name, &parent, sample_rate, self.bit_depth) {
            Ok(editor) => {
                *created = Some(editor);
                *self = NewProjectDialog { open: false, ..NewProjectDialog::default() };
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
