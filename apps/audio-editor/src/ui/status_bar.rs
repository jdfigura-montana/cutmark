//! ui::status_bar: the bottom status bar (the ui-kit status bar component).

use eframe::egui;

use crate::editor::Editor;
use crate::editor::selection::Selection;
use crate::ui::dialogs::export::ExportBanner;
use crate::util::time::samples_to_timecode;

/// The status bar under the timeline, 30 tall: sentence-case items, 20 apart,
/// in the ui-kit status bar that keeps its center empty. `gap_center_x` is the
/// center of the stage panel, so the empty part sits under the Play button even
/// though the side panels differ in width. The playhead time on the left (while a take is recorded, "Rec"
/// and the elapsed recording time instead), then the selection readout, a
/// finished-export note, "Ripple on" / "Snap on" while those are on, and the
/// sample rate and bit depth. A finished export keeps its Reveal link,
/// right-aligned.
pub fn show(ui: &mut egui::Ui, editor: &Editor, export: Option<&ExportBanner>, gap_center_x: f32) {
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
    // The component centers its gap in the width it is given: make that twice
    // the distance from the bar's left edge to the wanted center, and clip to
    // the real bar.
    let area = ui.available_rect_before_wrap();
    let centered = egui::Rect::from_min_size(area.min, egui::vec2(2.0 * (gap_center_x - area.left()).max(0.0), area.height()));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(centered));
    bar_ui.set_clip_rect(area.intersect(ui.clip_rect()));
    let bar = ui_kit::status_bar_with_gap(&mut bar_ui, &items);
    // The finished export's Reveal link, right-aligned in the bar (the
    // component's items all read from the left, so the right end is free).
    if let Some(export) = export {
        let rect = egui::Rect::from_x_y_ranges(area.x_range(), bar.rect.y_range());
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

/// The title bar caption, one fact: the project format as the status bar
/// spells it, "48 kHz 24-bit".
pub fn format_caption(rate: u32, depth: crate::model::project::BitDepth) -> String {
    format!("{} kHz {}", rate / 1000, bit_depth(depth))
}

/// Start-screen status bar through the ui-kit component: the default output
/// device on the left ("Output <device>", omitted when none is known) and
/// "No project open" right-aligned.
pub fn show_start(ui: &mut egui::Ui, device: Option<&str>) {
    let neutral = crate::ui::neutrals(ui.ctx());
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
    right.label(egui::RichText::new("No project open").font(ui_kit::fonts::medium(ui_kit::tokens::typography::CAPTION_SIZE)).color(neutral.text_tertiary));
}
