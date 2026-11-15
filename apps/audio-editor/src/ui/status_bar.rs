//! ui::status_bar: the bottom status bar with readouts and project info.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::selection::Selection;
use crate::ui::dialogs::export::ExportBanner;
use crate::ui::theme::{THEME, Theme};
use crate::util::time::samples_to_timecode;

/// Font size of every status bar readout.
const SIZE: f32 = 11.0;

/// Draws the status bar inside its panel: the playhead time on the left (while
/// a take is recorded, "REC" in red and the elapsed recording time instead),
/// the selection readout next to it, a finished-export banner with a Reveal
/// link, and the sample rate, bit depth and snap/ripple state on the right.
pub fn show(ui: &mut egui::Ui, editor: &Editor, export: Option<&ExportBanner>) {
    let rate = editor.project.sample_rate;
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
        match editor.recording_status().filter(|_| editor.take_is_live()) {
            Some(take) => {
                ui.label(egui::RichText::new("REC").font(Theme::mono(SIZE)).color(THEME.record_red));
                ui.label(egui::RichText::new(samples_to_timecode(take.frames, rate)).font(Theme::mono(SIZE)).color(THEME.text_primary));
            }
            None => {
                ui.label(egui::RichText::new("PLAYHEAD").font(Theme::mono(SIZE)).color(THEME.text_muted));
                ui.label(egui::RichText::new(samples_to_timecode(editor.view.playhead, rate)).font(Theme::mono(SIZE)).color(THEME.text_primary));
            }
        }
        if let Selection::Range { start, end, .. } = editor.selection() {
            let t = |samples: u64| samples_to_timecode(samples, rate);
            ui.label(egui::RichText::new("SEL").font(Theme::mono(SIZE)).color(THEME.text_muted));
            ui.label(egui::RichText::new(t(*start)).font(Theme::mono(SIZE)).color(THEME.text_primary));
            ui.label(egui::RichText::new("to").font(Theme::mono(SIZE)).color(THEME.text_muted));
            ui.label(egui::RichText::new(t(*end)).font(Theme::mono(SIZE)).color(THEME.text_primary));
            ui.label(egui::RichText::new(t(end.saturating_sub(*start))).font(Theme::mono(SIZE)).color(THEME.accent));
        }
        if let Some(export) = export {
            ui.label(egui::RichText::new(format!("Exported {}", export.name)).font(Theme::mono(SIZE)).color(THEME.text_muted));
            if ui.link(egui::RichText::new("Reveal").font(Theme::mono(SIZE))).clicked() {
                editor.reveal_file(&export.path);
            }
        }
        // Right-aligned readouts, added in reverse so the reading order is
        // "48 kHz 24-bit SNAP ON RIPPLE OFF". Each label and its value are
        // one unit (4 px apart); 22 px between units, 12 px off the right
        // window edge.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            ui.spacing_mut().item_spacing.x = 22.0;
            toggle_readout(ui, "RIPPLE", on_off(editor.view.ripple), editor.view.ripple);
            toggle_readout(ui, "SNAP", on_off(editor.view.snap), editor.view.snap);
            ui.label(egui::RichText::new(bit_depth(editor.project.export_bit_depth)).font(Theme::mono(SIZE)).color(THEME.text_primary));
            ui.label(egui::RichText::new(format!("{} kHz", rate / 1000)).font(Theme::mono(SIZE)).color(THEME.text_primary));
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

/// Start-screen status bar: the default output device on the left (omitted
/// when none is known) and "NO PROJECT OPEN" on the right.
pub fn show_start(ui: &mut egui::Ui, device: Option<&str>) {
    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 0.0);
        if let Some(device) = device {
            ui.label(egui::RichText::new("OUTPUT").font(Theme::mono(SIZE)).color(THEME.text_muted));
            ui.label(egui::RichText::new(device).font(Theme::mono(SIZE)).color(THEME.text_primary));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);
            ui.label(egui::RichText::new("NO PROJECT OPEN").font(Theme::mono(SIZE)).color(THEME.text_muted));
        });
    });
}

/// One toggle readout: label and value laid out as a single job, so the pair
/// always reads left to right: the label muted, a space, then the value
/// accent while on and plain primary while off.
fn toggle_readout(ui: &mut egui::Ui, label: &str, value: &str, on: bool) {
    let mut job = egui::text::LayoutJob::default();
    let format = |color| egui::TextFormat::simple(Theme::mono(SIZE), color);
    job.append(label, 0.0, format(THEME.text_muted));
    job.append(" ", 0.0, format(THEME.text_muted));
    job.append(value, 0.0, format(if on { THEME.accent } else { THEME.text_primary }));
    let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
    let (rect, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    ui.painter().galley(rect.min, galley, THEME.text_primary);
}
