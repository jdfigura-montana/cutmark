//! ui::toolbar: the toolbar with transport buttons, toggles and Add Track.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::action::Action;

/// Draws the toolbar row under the menu.
///
/// The left part is reserved for the transport buttons (phase 5); for now it
/// holds an Add Track button (Ctrl+Shift+T) and the Wave slider, a logarithmic
/// vertical waveform scale from 1x to 16x (double-click resets to 1x).
pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // Reserved space for the transport buttons (phase 5).
        ui.add_space(140.0);
        if ui
            .button("Add Track")
            .on_hover_text("Add a new track (Ctrl+Shift+T)")
            .clicked()
        {
            let _ = editor.apply(Action::AddTrack {
                index: None,
                name: None,
            });
        }

        // Wave: vertical waveform scale, edited as log2 so 1x..=16x becomes
        // a linear 0..=4 slider. Display only; never changes the audio.
        let mut log_scale = editor.wave_scale().log2();
        let slider = egui::Slider::new(&mut log_scale, 0.0..=4.0)
            .text("Wave")
            .custom_formatter(|value, _| format!("{:.1}x", 2.0_f32.powf(value as f32)));
        let resp = ui.add(slider);
        editor.set_wave_scale(2.0_f32.powf(log_scale));
        if resp.double_clicked() {
            editor.set_wave_scale(1.0);
        }
    });
    ui.add_space(2.0);
}
