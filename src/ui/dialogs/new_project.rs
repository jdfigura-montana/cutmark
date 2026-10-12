//! ui::dialogs::new_project: the New Project dialog.

use std::path::PathBuf;

use eframe::egui;

use crate::editor::Editor;
use crate::model::project::BitDepth;

/// Sample rates offered when creating a project, in kHz.
const SAMPLE_RATES_KHZ: [u32; 4] = [44, 48, 88, 96];

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
        NewProjectDialog {
            open: false,
            name: String::new(),
            parent: None,
            sample_rate_khz: 48,
            bit_depth: BitDepth::Int24,
            error: None,
        }
    }
}

impl NewProjectDialog {
    /// Shows the dialog; returns the created Editor when the user succeeds.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Editor> {
        let mut created = None;
        let mut open = self.open;
        let mut cancel = false;
        egui::Window::new("New Project")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("new_project_grid")
                    .num_columns(3)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Name:");
                        ui.text_edit_singleline(&mut self.name);
                        ui.end_row();

                        ui.label("Parent folder:");
                        let folder_text = match &self.parent {
                            Some(path) => path.display().to_string(),
                            None => String::new(),
                        };
                        ui.add(
                            egui::Label::new(egui::RichText::new(folder_text).weak()).truncate(),
                        );
                        if ui.button("Browse...").clicked() {
                            self.parent = rfd::FileDialog::new().pick_folder();
                        }
                        ui.end_row();

                        ui.label("Sample rate:");
                        egui::ComboBox::from_id_salt("sample_rate")
                            .selected_text(format!("{} kHz", self.sample_rate_khz))
                            .show_ui(ui, |ui| {
                                for khz in SAMPLE_RATES_KHZ {
                                    ui.selectable_value(
                                        &mut self.sample_rate_khz,
                                        khz,
                                        format!("{} kHz", khz),
                                    );
                                }
                            });
                        ui.end_row();

                        ui.label("Export bit depth:");
                        egui::ComboBox::from_id_salt("bit_depth")
                            .selected_text(bit_depth_text(self.bit_depth))
                            .show_ui(ui, |ui| {
                                for depth in [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32] {
                                    ui.selectable_value(
                                        &mut self.bit_depth,
                                        depth,
                                        bit_depth_text(depth),
                                    );
                                }
                            });
                        ui.end_row();
                    });

                ui.add_space(8.0);
                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(egui::Color32::LIGHT_RED));
                }
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    let can_create = !self.name.trim().is_empty() && self.parent.is_some();
                    let create = ui.add_enabled(can_create, egui::Button::new("Create"));
                    if create.clicked() {
                        self.try_create(&mut created);
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            *self = NewProjectDialog {
                open: false,
                ..NewProjectDialog::default()
            };
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
                *self = NewProjectDialog {
                    open: false,
                    ..NewProjectDialog::default()
                };
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// Human-readable text for a bit depth.
fn bit_depth_text(depth: BitDepth) -> &'static str {
    match depth {
        BitDepth::Int16 => "16-bit",
        BitDepth::Int24 => "24-bit",
        BitDepth::Float32 => "32-bit float",
    }
}
