//! ui::status_bar: the bottom status bar with readouts and project info.

use eframe::egui;

use crate::model::project::Project;

/// Draws the status bar: playhead and selection readouts go on the left later;
/// the project sample rate sits on the right.
pub fn show(ui: &mut egui::Ui, project: &Project) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(format!("{} Hz", project.sample_rate));
    });
}
