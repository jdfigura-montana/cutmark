//! ui::status_bar: the bottom status bar (the ui-kit status bar component).

use eframe::egui;

use crate::editor::Editor;
use crate::editor::selection::Selection;
use crate::ui::dialogs::export::ExportBanner;
use crate::util::time::samples_to_timecode;

/// The status bar, 26 tall: sentence-case items in the ui-kit status bar
/// component. The playhead time on the left (while a take is recorded, "Rec"
/// and the elapsed recording time instead), then the selection readout, a
/// finished-export note, "Ripple on" / "Snap on" while those are on, and the
/// sample rate and bit depth. A finished export keeps its Reveal link,
/// right-aligned.
pub fn show(ui: &mut egui::Ui, editor: &Editor, export: Option<&ExportBanner>) {
    let rate = editor.project.sample_rate;
    let time = |samples: u64| samples_to_timecode(samples, rate);
    let mut items: Vec<String> = Vec::new();
    match editor.recording_status().filter(|_| editor.take_is_live()) {
        Some(take) => items.push(format!("Rec {}", time(take.frames))),
        None => items.push(format!("Playhead {}", time(editor.view.playhead))),
    }
    if let Selection::Range { start, end, .. } = editor.selection() {
        items.push(format!("Sel {} to {} ({})", time(*start), time(*end), time(end.saturating_sub(*start))));
    }
    if let Some(export) = export {
        items.push(format!("Exported {}", export.name));
    }
    if editor.view.ripple {
        items.push("Ripple on".to_string());
    }
    if editor.view.snap {
        items.push("Snap on".to_string());
    }
    items.push(format!("{} kHz", rate / 1000));
    items.push(bit_depth(editor.project.export_bit_depth).to_string());
    let items: Vec<&str> = items.iter().map(String::as_str).collect();
    let bar = ui_kit::status_bar(ui, &items);
    // The finished export's Reveal link, right-aligned in the bar (the
    // component's items all read from the left, so the right end is free).
    if let Some(export) = export {
        let rect = bar.rect;
        let pad = ui_kit::tokens::space::SCALE_4;
        let max_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - 200.0, rect.top()), egui::pos2(rect.right() - pad, rect.bottom()));
        let mut link = ui.new_child(egui::UiBuilder::new().max_rect(max_rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if link.link("Reveal").clicked() {
            editor.reveal_file(&export.path);
        }
    }
}

/// Label for a project's export bit depth, as shown in the status bar.
fn bit_depth(depth: crate::model::project::BitDepth) -> &'static str {
    match depth {
        crate::model::project::BitDepth::Int16 => "16-bit",
        crate::model::project::BitDepth::Int24 => "24-bit",
        crate::model::project::BitDepth::Float32 => "32-bit",
    }
}

/// The title bar caption: the project format as the status bar spells it plus
/// the track count, "48 kHz · 24-bit · 4 tracks".
pub fn format_caption(rate: u32, depth: crate::model::project::BitDepth, tracks: usize) -> String {
    let noun = if tracks == 1 { "track" } else { "tracks" };
    format!("{} kHz \u{b7} {} \u{b7} {tracks} {noun}", rate / 1000, bit_depth(depth))
}

/// Start-screen status bar through the ui-kit component: the default output
/// device on the left ("Output <device>", omitted when none is known) and
/// "No project open" right-aligned.
pub fn show_start(ui: &mut egui::Ui, device: Option<&str>) {
    let mut items: Vec<String> = Vec::new();
    if let Some(device) = device {
        items.push(format!("Output {device}"));
    }
    let items: Vec<&str> = items.iter().map(String::as_str).collect();
    let bar = ui_kit::status_bar(ui, &items);
    // "No project open", right-aligned in the bar (the component's items all
    // read from the left, so the right end is free).
    let rect = bar.rect;
    let pad = ui_kit::tokens::space::SCALE_4;
    let max_rect = egui::Rect::from_min_max(egui::pos2(rect.right() - 200.0, rect.top()), egui::pos2(rect.right() - pad, rect.bottom()));
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(max_rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    right.label(
        egui::RichText::new("No project open")
            .font(ui_kit::fonts::medium(ui_kit::tokens::typography::CAPTION_SIZE))
            .color(ui_kit::tokens::color::TEXT_TERTIARY),
    );
}
