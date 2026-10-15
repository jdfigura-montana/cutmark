//! ui::toolbar: the toolbar with transport buttons, timecode, toggles, Add
//! Track and the wave-scale slider.

use eframe::egui;
use egui_phosphor::fill;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::transport::TransportState;
use crate::ui::theme::{THEME, Theme};
use crate::util::time::samples_to_timecode;

/// Horizontal gap between the toolbar groups.
const GROUP_GAP: f32 = 12.0;
/// Size of the transport icon buttons.
const TRANSPORT_BTN: egui::Vec2 = egui::Vec2::new(34.0, 30.0);
/// Corner radius of the recessed transport and timecode boxes.
const BOX_RADIUS: f32 = 7.0;
/// Font size of the toolbar timecode.
const TIMECODE_SIZE: f32 = 17.0;

/// Draws the toolbar row inside its panel.
///
/// Left to right: the recessed transport group (Home, Play, Pause, Stop, End),
/// the recessed timecode box, then the Ripple and Snap toggles and the Add
/// Track button. Right aligned: the "WAVE SCALE" caption, its slider and its
/// value, a logarithmic vertical waveform scale from 1x to 16x (double-click
/// resets to 1x, display only, never changes the audio).
pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        transport_group(ui, editor);
        ui.add_space(GROUP_GAP);
        timecode_box(ui, editor);
        ui.add_space(GROUP_GAP);
        divider(ui);
        toggle_button(
            ui,
            "Ripple",
            fill::SCISSORS,
            editor.view.ripple,
            "Ripple edit (R)",
            || editor.toggle_ripple(),
        );
        divider(ui);
        toggle_button(
            ui,
            "Snap",
            fill::MAGNET,
            editor.view.snap,
            "Snapping (N)",
            || editor.toggle_snap(),
        );
        ui.add_space(GROUP_GAP);
        if ui
            .add(egui::Button::new(format!("{} Add Track", fill::PLUS)))
            .on_hover_text("Add a new track (Ctrl+Shift+T)")
            .clicked()
        {
            let _ = editor.apply(Action::AddTrack {
                index: None,
                name: None,
            });
        }

        // Wave: vertical waveform scale on the right, edited as log2 so
        // 1x..=16x becomes a linear 0..=4 slider. Display only. Added in
        // reverse order so the reading order is label, slider, value; the
        // 12 px space keeps the group off the right window edge.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            let scale = editor.wave_scale();
            ui.add_sized(
                egui::vec2(40.0, 18.0),
                egui::Label::new(
                    egui::RichText::new(format!("{scale:.1}x"))
                        .font(Theme::mono(13.0))
                        .color(THEME.text_primary),
                ),
            );
            let mut log_scale = scale.log2();
            let slider = egui::Slider::new(&mut log_scale, 0.0..=4.0)
                .custom_formatter(|v, _| format!("{:.1}x", 2.0_f32.powf(v as f32)))
                .show_value(false);
            {
                let style = ui.style_mut();
                style.visuals.slider_trailing_fill = true;
                style.visuals.widgets.inactive.bg_fill = THEME.slider_track;
                style.visuals.selection.bg_fill = THEME.slider_fill;
                style.visuals.widgets.hovered.bg_fill = THEME.slider_thumb;
            }
            let resp = ui.add_sized(egui::vec2(110.0, 18.0), slider);
            editor.set_wave_scale(2.0_f32.powf(log_scale));
            if resp.double_clicked() {
                editor.set_wave_scale(1.0);
            }
            ui.label(
                egui::RichText::new("WAVE SCALE")
                    .color(THEME.text_faint)
                    .size(10.0),
            );
        });
    });
}

/// The recessed transport group: jump start, play/stop, pause, stop and end.
fn transport_group(ui: &mut egui::Ui, editor: &mut Editor) {
    egui::Frame::new()
        .fill(THEME.bar_bg)
        .stroke(egui::Stroke::new(THEME.hairline, THEME.control_border))
        .corner_radius(BOX_RADIUS)
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                if icon_button(ui, fill::SKIP_BACK, THEME.text_normal)
                    .on_hover_text("Jump to start (Home)")
                    .clicked()
                {
                    editor.jump_start();
                }
                let playing = editor.transport_state() == TransportState::Playing;
                let (glyph, color, tooltip) = if playing {
                    (fill::STOP, THEME.text_normal, "Stop (Space)")
                } else {
                    (fill::PLAY, THEME.play_green, "Play (Space)")
                };
                if icon_button(ui, glyph, color)
                    .on_hover_text(tooltip)
                    .clicked()
                {
                    editor.toggle_play();
                }
                if icon_button(ui, fill::PAUSE, THEME.text_normal)
                    .on_hover_text("Pause (P)")
                    .clicked()
                {
                    editor.toggle_pause();
                }
                if icon_button(ui, fill::STOP, THEME.text_normal)
                    .on_hover_text("Stop")
                    .clicked()
                {
                    editor.stop();
                }
                if icon_button(ui, fill::SKIP_FORWARD, THEME.text_normal)
                    .on_hover_text("Jump to end (End)")
                    .clicked()
                {
                    editor.jump_end();
                }
            });
        });
}

/// One 34x30 icon button.
fn icon_button(ui: &mut egui::Ui, icon: &str, color: egui::Color32) -> egui::Response {
    let button = egui::Button::new(egui::RichText::new(icon).color(color).size(14.0))
        .min_size(TRANSPORT_BTN)
        .frame(false);
    ui.add(button)
}

/// The recessed timecode box showing the playhead time.
fn timecode_box(ui: &mut egui::Ui, editor: &Editor) {
    let timecode = samples_to_timecode(editor.view.playhead, editor.project.sample_rate);
    egui::Frame::new()
        .fill(THEME.recessed_a)
        .stroke(egui::Stroke::new(THEME.hairline, THEME.control_border))
        .corner_radius(BOX_RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(timecode)
                    .font(Theme::mono(TIMECODE_SIZE))
                    .color(THEME.text_primary),
            );
        });
}

/// A 1 px vertical divider between toolbar groups.
fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 24.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, THEME.control_border);
}

/// An icon + text toggle button: lit with accent_soft fill, an accent border
/// and accent icon while on. Calls `toggle` when clicked.
fn toggle_button(
    ui: &mut egui::Ui,
    name: &str,
    icon: &str,
    on: bool,
    tooltip: &str,
    toggle: impl FnOnce(),
) {
    let icon_color = if on { THEME.accent } else { THEME.text_normal };
    let mut button = egui::Button::new((
        egui::RichText::new(icon).color(icon_color),
        egui::RichText::new(format!(" {name}")),
    ));
    if on {
        button = button
            .fill(THEME.accent_soft)
            .stroke(egui::Stroke::new(THEME.hairline, THEME.accent));
    }
    if ui.add(button).on_hover_text(tooltip).clicked() {
        toggle();
    }
}
