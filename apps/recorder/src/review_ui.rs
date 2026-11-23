//! The Review mode panels (spec "Review mode"): the takes list on the left,
//! the take's picture in the center and the take info on the right.
//!
//! Playback arrives with R9, so the center shows a placeholder with the take's
//! name: media-io can decode live frames but offers no cheap one-frame decode
//! of a file, so there is no thumbnail yet. Clicking a mark does nothing for
//! the same reason; the seek comes with the playback.

use egui::{Align, Color32, Layout, Rect, RichText, Ui, vec2};
use marks::MarkType;
use ui_kit::tokens::{color, radius, size, space, typography};
use ui_kit::{MarkerKind, inspector_group, inspector_row, marker_color};

use crate::review::{self, Day, Take};

/// Left padding of a panel header's title, the same inset the app's other
/// headers use.
const HEADER_PAD: f32 = 6.0;

/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// What the takes panel's header buttons did this frame.
#[derive(Debug, PartialEq, Eq)]
pub enum TakesAction {
    /// Nothing was clicked.
    None,
    /// Open the project folder.
    OpenFolder,
    /// Rescan the takes.
    Refresh,
}

/// The left Takes panel (spec "Review mode"): the header with the folder and
/// refresh buttons, then the days with their takes, newest first. Returns
/// what the header buttons did.
pub fn takes_panel(ui: &mut Ui, days: &[Day], selected: &mut Option<String>) -> TakesAction {
    let action = takes_header(ui);
    ui.add_space(space::SCALE_8);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        let mut picked = None;
        for day in days {
            group_caption(ui, &day.date.format("%Y-%m-%d").to_string());
            for take in &day.takes {
                let trailing = take_row_trailing(take);
                let is_selected = selected.as_deref() == Some(take.name.as_str());
                if ui_kit::list_row(ui, is_selected, None, &take.name, Some(&trailing)).clicked() {
                    picked = Some(take.name.clone());
                }
            }
            ui.add_space(space::SCALE_8);
        }
        if let Some(name) = picked {
            *selected = Some(name);
        }
    });
    action
}

/// The takes panel's header row (44 tall): "Takes" and the open-folder and
/// refresh icon buttons on the right, with the hairline along the bottom of
/// the row.
fn takes_header(ui: &mut Ui) -> TakesAction {
    let mut action = TakesAction::None;
    let row = vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.add(egui::Label::new(RichText::new("Takes").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY)).selectable(false));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Right to left, so the pair still reads "open folder, refresh" left to right.
            if ui_kit::icon_button(ui, ui_kit::icons::regular::ARROWS_CLOCKWISE, "Refresh takes").clicked() {
                action = TakesAction::Refresh;
            }
            if ui_kit::icon_button(ui, ui_kit::icons::regular::FOLDER_OPEN, "Open project folder").clicked() {
                action = TakesAction::OpenFolder;
            }
        });
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    action
}

/// The trailing caption of a take row: the start time, the mark count and the
/// duration when the sidecar knows it.
fn take_row_trailing(take: &Take) -> String {
    let mut trailing = format!("{} \u{b7} {} marks", take.start.format("%H:%M"), take.marks.len());
    if let Some(duration) = take.duration_ms {
        trailing.push_str(&format!(" \u{b7} {}", review::format_duration(duration)));
    }
    trailing
}

/// A group caption ("2026-11-21", "File", "Marks") in the caption style.
fn group_caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false));
}

/// The Review center (spec "Review mode"): the take's first frame letterboxed
/// in 16:9. Until R9 the placeholder shows the take's name, because media-io
/// offers no cheap one-frame decode of a file.
pub fn center(ui: &mut Ui, days: &[Day], selected: Option<&str>) {
    let available = ui.available_rect_before_wrap();
    let Some(take) = review::find(days, selected) else {
        placeholder_caption(ui, available.center(), "No take selected");
        return;
    };
    let box_rect = fit_16_9(available);
    let painter = ui.painter();
    painter.rect_filled(box_rect, radius::GROUP, color::BG_GROUND);
    // The name wraps inside the box instead of spilling over its edges.
    let galley =
        painter.layout(take.name.clone(), ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_TERTIARY, box_rect.width() - 2.0 * space::SCALE_16);
    painter.galley(box_rect.center() - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
}

/// A centered tertiary caption where the picture would be.
fn placeholder_caption(ui: &Ui, at: egui::Pos2, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), color::TEXT_TERTIARY);
    ui.painter().galley(at - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
}

/// The largest 16:9 box that fits in `available`, centered in it.
fn fit_16_9(available: Rect) -> Rect {
    const ASPECT: f32 = 16.0 / 9.0;
    let width = available.width().min(available.height() * ASPECT).max(0.0);
    Rect::from_center_size(available.center(), vec2(width, width / ASPECT))
}

/// The right take info panel (spec "Review mode"): the header with the take's
/// name and date, then the File group and the sidecar's marks.
pub fn take_panel(ui: &mut Ui, take: Option<&Take>) {
    take_header(ui, take);
    ui.add_space(space::SCALE_8);
    let Some(take) = take else { return };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        group_caption(ui, "File");
        inspector_group(ui, |ui| {
            let duration = take.duration_ms.map(review::format_duration).unwrap_or_else(|| "\u{2014}".to_owned());
            row(ui, "Duration", &duration);
            // The hover names every file of the take with its own size.
            let file_list = take
                .files
                .iter()
                .map(|file| {
                    let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    format!("{name} ({})", review::format_size(file.size))
                })
                .collect::<Vec<_>>()
                .join("\n");
            row(ui, "Size", &review::format_size(take.total_size)).on_hover_text(file_list);
            row(ui, "Video format", &format_label(&take.video));
            if let Some(iso) = &take.iso {
                row(ui, "Camera ISO format", &format_label(iso));
            }
        });
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Marks");
        if take.marks.is_empty() {
            placeholder_caption(ui, ui.cursor().left_top() + egui::vec2(ui.available_width() / 2.0, 10.0), "No marks");
            ui.add_space(20.0);
        }
        // Clicking a mark seeks to it only with R9's playback, so the row
        // carries no action yet.
        for mark in &take.marks {
            let label = mark.label.clone().unwrap_or_else(|| type_name(mark.mark_type, mark.original_type.as_deref()));
            let trailing = marks::format_tc(mark.t_ms);
            let _ = ui_kit::list_row(ui, false, Some(mark_dot(mark.mark_type)), &label, Some(&trailing));
        }
        ui.add_space(space::SCALE_8);
    });
}

/// The take info header row (44 tall): the take's name over its start date,
/// with the hairline along the bottom of the row.
fn take_header(ui: &mut Ui, take: Option<&Take>) {
    let row = vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.vertical(|ui| {
            let name = take.map_or_else(|| "No take selected".to_owned(), |take| take.name.clone());
            ui.add(
                egui::Label::new(RichText::new(name).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                    .selectable(false)
                    .truncate(),
            );
            if let Some(take) = take {
                ui.add(
                    egui::Label::new(
                        RichText::new(take.start.format("%b %d, %Y").to_string())
                            .font(ui_kit::fonts::medium(typography::CAPTION_SIZE))
                            .color(color::TEXT_TERTIARY),
                    )
                    .selectable(false),
                );
            }
        });
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// One read-only inspector row with its value.
fn row(ui: &mut Ui, label: &str, value: &str) -> egui::Response {
    inspector_row(ui, label, |_ui| {}, Some(value))
}

/// The format of a take file: its extension uppercased, or a dash when it has none.
fn format_label(path: &std::path::Path) -> String {
    path.extension().map(|extension| extension.to_string_lossy().to_uppercase()).unwrap_or_else(|| "\u{2014}".to_owned())
}

/// The dot color of a sidecar mark: the marker palette has no Highlight or
/// Chapter tokens, so they take the Retake and Note colors (the same mapping
/// the live marks list uses).
fn mark_dot(mark_type: MarkType) -> Color32 {
    let kind = match mark_type {
        MarkType::Cut => MarkerKind::Cut,
        MarkType::Retake | MarkType::Highlight => MarkerKind::Retake,
        MarkType::Note | MarkType::Chapter => MarkerKind::Note,
        MarkType::BRoll => MarkerKind::BRoll,
        MarkType::System => MarkerKind::System,
    };
    marker_color(kind)
}

/// The display name of a mark type; an unknown type keeps the original string
/// the reader saved.
fn type_name(mark_type: MarkType, original: Option<&str>) -> String {
    match original {
        Some(original) => original.to_owned(),
        None => match mark_type {
            MarkType::Highlight => "Highlight".to_owned(),
            MarkType::Cut => "Cut".to_owned(),
            MarkType::Retake => "Retake".to_owned(),
            MarkType::Chapter => "Chapter".to_owned(),
            MarkType::Note => "Note".to_owned(),
            MarkType::BRoll => "B-roll".to_owned(),
            MarkType::System => "System".to_owned(),
        },
    }
}
