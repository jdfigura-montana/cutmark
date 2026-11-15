//! ui::dialogs::new_project: the New Project dialog.

use std::path::PathBuf;

use eframe::egui;

use crate::editor::Editor;
use crate::model::project::BitDepth;
use crate::ui::theme::THEME;

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
        let mut cancel = false;
        if self.open {
            let response = super::modal("new_project_dialog").show(ctx, |ui| {
                super::card_width(ui, THEME.dialog_w);
                super::title(ui, "New Project");
                ui.add_space(THEME.space_xs);
                super::description(ui, "Create a project folder with a timeline and media bin.");
                super::before_form(ui);
                super::form_row(ui, "Name", |ui| {
                    super::text_field(ui, &mut self.name, "np_name");
                });
                super::form_row(ui, "Parent folder", |ui| {
                    let folder_text = match &self.parent {
                        Some(path) => path.display().to_string(),
                        None => String::new(),
                    };
                    let browse = super::with_browse(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(folder_text).color(THEME.text_normal).size(THEME.dialog_label_size)).truncate());
                    });
                    if browse {
                        self.parent = rfd::FileDialog::new().pick_folder();
                    }
                });
                super::form_row(ui, "Sample rate", |ui| {
                    super::segmented(ui, "np_rate", &[(44, "44 kHz"), (48, "48 kHz"), (88, "88 kHz"), (96, "96 kHz")], &mut self.sample_rate_khz);
                });
                super::form_row(ui, "Bit depth", |ui| {
                    super::segmented(
                        ui,
                        "np_depth",
                        &[(BitDepth::Int16, "16-bit"), (BitDepth::Int24, "24-bit"), (BitDepth::Float32, "32-bit float")],
                        &mut self.bit_depth,
                    );
                });
                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(THEME.error_soft));
                }
                let can_create = !self.name.trim().is_empty() && self.parent.is_some();
                super::footer(ui, |ui| {
                    if super::footer_button(ui, "Create", super::FooterButton::Primary, can_create).clicked() {
                        self.try_create(&mut created);
                    }
                    if super::footer_button(ui, "Cancel", super::FooterButton::Cancel, true).clicked() {
                        cancel = true;
                    }
                });
                // Enter confirms (when valid).
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && can_create {
                    self.try_create(&mut created);
                }
            });
            // Esc or the backdrop cancels.
            if response.should_close() {
                cancel = true;
            }
        }
        if cancel {
            *self = NewProjectDialog { open: false, ..NewProjectDialog::default() };
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
