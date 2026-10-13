//! ui::status_bar: the bottom status bar with readouts and project info.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::selection::Selection;
use crate::util::time::samples_to_timecode;

/// Draws the status bar: the playhead time on the left, the selection readout
/// next to it, the project sample rate and audio status on the right.
pub fn show(ui: &mut egui::Ui, editor: &Editor) {
    let rate = editor.project.sample_rate;
    let timecode = samples_to_timecode(editor.view.playhead, rate);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(timecode).monospace());
        if let Selection::Range { start, end, .. } = editor.selection() {
            let t = |samples: u64| samples_to_timecode(samples, rate);
            ui.label(
                egui::RichText::new(format!(
                    "Sel {} to {} ({})",
                    t(*start),
                    t(*end),
                    t(end.saturating_sub(*start))
                ))
                .monospace(),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(editor.audio_status());
        });
    });
}
