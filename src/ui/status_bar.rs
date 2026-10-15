//! ui::status_bar: the bottom status bar with readouts and project info.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::selection::Selection;
use crate::ui::dialogs::export::ExportBanner;
use crate::ui::theme::{THEME, Theme};
use crate::util::time::samples_to_timecode;

/// Font size of every status bar readout.
const SIZE: f32 = 11.0;

/// Draws the status bar inside its panel: the playhead time on the left, the
/// selection readout next to it, a finished-export banner with a Reveal link,
/// and the sample rate, bit depth and snap/ripple state on the right.
pub fn show(ui: &mut egui::Ui, editor: &Editor, export: Option<&ExportBanner>) {
    let rate = editor.project.sample_rate;
    let timecode = samples_to_timecode(editor.view.playhead, rate);
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
        ui.label(
            egui::RichText::new("PLAYHEAD")
                .font(Theme::mono(SIZE))
                .color(THEME.text_muted),
        );
        ui.label(
            egui::RichText::new(timecode)
                .font(Theme::mono(SIZE))
                .color(THEME.text_primary),
        );
        if let Selection::Range { start, end, .. } = editor.selection() {
            let t = |samples: u64| samples_to_timecode(samples, rate);
            ui.label(
                egui::RichText::new("SEL")
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            ui.label(
                egui::RichText::new(t(*start))
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_primary),
            );
            ui.label(
                egui::RichText::new("to")
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            ui.label(
                egui::RichText::new(t(*end))
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_primary),
            );
            ui.label(
                egui::RichText::new(t(end.saturating_sub(*start)))
                    .font(Theme::mono(SIZE))
                    .color(THEME.accent),
            );
        }
        if let Some(export) = export {
            ui.label(
                egui::RichText::new(format!("Exported {}", export.name))
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            if ui
                .link(egui::RichText::new("Reveal").font(Theme::mono(SIZE)))
                .clicked()
            {
                editor.reveal_file(&export.path);
            }
        }
        // Right-aligned readouts, added in reverse so the reading order is
        // "48 kHz 24-bit SNAP ON RIPPLE OFF". 22 px between groups, 6 px
        // between a label and its value, 12 px off the right window edge.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
            readout_value(ui, on_off(editor.view.ripple), editor.view.ripple);
            ui.label(
                egui::RichText::new("RIPPLE")
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            ui.spacing_mut().item_spacing = egui::vec2(22.0, 0.0);
            readout_value(ui, on_off(editor.view.snap), editor.view.snap);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
            ui.label(
                egui::RichText::new("SNAP")
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            ui.spacing_mut().item_spacing = egui::vec2(22.0, 0.0);
            ui.label(
                egui::RichText::new(bit_depth(editor.project.export_bit_depth))
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_primary),
            );
            ui.label(
                egui::RichText::new(format!("{} kHz", rate / 1000))
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_primary),
            );
        });
    });
}

/// Label for a project's export bit depth, as shown in the status bar.
fn bit_depth(depth: crate::model::project::BitDepth) -> &'static str {
    match depth {
        crate::model::project::BitDepth::Int16 => "16-bit",
        crate::model::project::BitDepth::Int24 => "24-bit",
        crate::model::project::BitDepth::Float32 => "32-bit",
    }
}

/// Text of a toggle readout's value: ON or OFF.
fn on_off(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
}

/// One toggle readout value: accent while on, plain primary text while off.
fn readout_value(ui: &mut egui::Ui, text: &str, on: bool) {
    ui.label(
        egui::RichText::new(text)
            .font(Theme::mono(SIZE))
            .color(if on { THEME.accent } else { THEME.text_primary }),
    );
}
