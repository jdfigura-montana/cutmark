//! ui::toolbar: the toolbar with transport buttons, toggles and Add Track.

use eframe::egui;
use egui_phosphor::fill;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::transport::TransportState;
use crate::ui::theme::THEME;

/// Draws the toolbar row under the menu.
///
/// Left: phosphor-icon transport buttons (jump start, play/stop, pause, stop,
/// jump end) with shortcuts in the tooltips. Then the Add Track button
/// (Ctrl+Shift+T), the Ripple/Snap toggles (lit with the accent when on) and
/// the Wave slider, a logarithmic vertical waveform scale from 1x to 16x
/// (double-click resets to 1x).
pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    ui.add_space(THEME.space_sm);
    ui.horizontal(|ui| {
        // Transport: Home jumps to 0, the play button toggles play/stop and
        // its glyph follows the transport state, then pause, stop and End.
        if ui
            .button(fill::SKIP_BACK)
            .on_hover_text("Jump to start (Home)")
            .clicked()
        {
            editor.jump_start();
        }
        let playing = editor.transport_state() == TransportState::Playing;
        let play_resp = ui.button(if playing { fill::STOP } else { fill::PLAY });
        let play_resp = if playing {
            play_resp.on_hover_text("Stop (Space)")
        } else {
            play_resp.on_hover_text("Play (Space)")
        };
        if play_resp.clicked() {
            editor.toggle_play();
        }
        if ui.button(fill::PAUSE).on_hover_text("Pause (P)").clicked() {
            editor.toggle_pause();
        }
        if ui.button(fill::STOP).on_hover_text("Stop").clicked() {
            editor.stop();
        }
        if ui
            .button(fill::SKIP_FORWARD)
            .on_hover_text("Jump to end (End)")
            .clicked()
        {
            editor.jump_end();
        }
        ui.add_space(THEME.space_md);
        // Ripple and Snap toggles: lit with the accent while on.
        let ripple = if editor.view.ripple {
            egui::Button::new(format!("{} Ripple", fill::SCISSORS)).fill(THEME.accent)
        } else {
            egui::Button::new(format!("{} Ripple", fill::SCISSORS))
        };
        if ui.add(ripple).on_hover_text("Ripple edit (R)").clicked() {
            editor.toggle_ripple();
        }
        let snap = if editor.view.snap {
            egui::Button::new(format!("{} Snap", fill::MAGNET)).fill(THEME.accent)
        } else {
            egui::Button::new(format!("{} Snap", fill::MAGNET))
        };
        if ui.add(snap).on_hover_text("Snapping (N)").clicked() {
            editor.toggle_snap();
        }
        ui.add_space(THEME.space_md);
        if ui
            .button(format!("{} Add Track", fill::PLUS))
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
    ui.add_space(THEME.space_xs);
}
