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
/// and the audio status, snap/ripple state, bit depth and sample rate on the
/// right.
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(editor.audio_status())
                    .font(Theme::mono(SIZE))
                    .color(THEME.text_muted),
            );
            toggle_readout(ui, "SNAP", editor.view.snap);
            toggle_readout(ui, "RIPPLE", editor.view.ripple);
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

/// One toggle readout: the name in muted text and ON/OFF in primary text.
fn toggle_readout(ui: &mut egui::Ui, name: &str, on: bool) {
    ui.label(
        egui::RichText::new(name)
            .font(Theme::mono(SIZE))
            .color(THEME.text_muted),
    );
    ui.label(
        egui::RichText::new(if on { "ON" } else { "OFF" })
            .font(Theme::mono(SIZE))
            .color(THEME.text_primary),
    );
}
