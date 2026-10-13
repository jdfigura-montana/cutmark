//! ui::status_bar: the bottom status bar with readouts and project info.

use eframe::egui;

use crate::editor::Editor;
use crate::util::time::samples_to_timecode;

/// Draws the status bar: the playhead time on the left, the project sample
/// rate on the right. Selection readouts come later.
pub fn show(ui: &mut egui::Ui, editor: &Editor) {
    let timecode = samples_to_timecode(editor.view.playhead, editor.project.sample_rate);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(timecode).monospace());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(editor.audio_status());
        });
    });
}
