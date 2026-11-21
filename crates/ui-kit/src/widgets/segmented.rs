//! The segmented control (spec "Components": row "Segmented control").

use egui::{Color32, Id, Key, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::tokens::{color, radius, size, space, typography};
use crate::widgets::focus_ring;

/// Track padding around the segments (spec "Segmented control": padding 3;
/// not on the 2-based spacing scale, so a named constant of its own).
const TRACK_PADDING: f32 = 3.0;
/// Horizontal padding inside each segment.
const SEGMENT_PADDING: f32 = space::SCALE_4;
/// The 1px shadow under the selected segment (spec "Segmented control").
const SEGMENT_SHADOW_OFFSET: f32 = 1.0;

/// The segmented control: a `bg.control` track with `radius.control`,
/// padding 3, the selected segment `bg.selected` with a 1px shadow and
/// `radius.segment`, 28 tall. Arrow keys move the selection when focused.
///
/// `id_salt` must be unique per control on the page: the segments' ids are
/// derived from it, and several controls with the same labels must never
/// share ids (egui reports reused ids as red "First use / Second use of
/// widget ID" overlays).
///
/// Usage:
///
/// ```ignore
/// let mut mode = 0;
/// ui_kit::segmented(ui, "modes", &mut mode, &["Edit", "Mix"]);
/// ```
pub fn segmented(ui: &mut Ui, id_salt: impl Into<Id>, selected: &mut usize, labels: &[&str]) -> Response {
    let id_salt = id_salt.into();
    let font = fonts::medium(typography::BODY_SIZE);
    let galleys: Vec<_> = labels.iter().map(|label| ui.painter().layout_no_wrap((*label).to_owned(), font.clone(), Color32::PLACEHOLDER)).collect();
    let segment_widths: Vec<f32> = galleys.iter().map(|galley| galley.size().x + 2.0 * SEGMENT_PADDING).collect();
    let height = size::SEGMENT_HEIGHT;
    let total_width: f32 = segment_widths.iter().sum::<f32>() + 2.0 * TRACK_PADDING;
    let (rect, track) = ui.allocate_exact_size(vec2(total_width, height + 2.0 * TRACK_PADDING), Sense::hover());
    track.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Segmented control"));

    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, radius::CONTROL, color::BG_CONTROL);
    }

    let mut x = rect.left() + TRACK_PADDING;
    let mut focused: Option<Response> = None;
    for (index, galley) in galleys.iter().enumerate() {
        let segment_rect = Rect::from_min_size(Pos2::new(x, rect.top() + TRACK_PADDING), vec2(segment_widths[index], height));
        x += segment_widths[index];
        let is_selected = index == *selected;
        let segment = ui.interact(segment_rect, ui.id().with(id_salt).with("segmented_segment").with(index), Sense::click());
        segment.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, is_selected, labels[index]));
        if segment.clicked() {
            *selected = index;
        }
        if segment.has_focus() {
            let (right, left) = ui.input(|input| (input.key_pressed(Key::ArrowRight), input.key_pressed(Key::ArrowLeft)));
            if right {
                *selected = (*selected + 1).min(labels.len().saturating_sub(1));
            }
            if left {
                *selected = selected.saturating_sub(1);
            }
            focused = Some(segment);
        }
        if ui.is_rect_visible(segment_rect) {
            if is_selected {
                // The 1px shadow: the glass shadow color, offset 1px down
                // under the selected segment (spec "Segmented control").
                ui.painter().rect_filled(segment_rect.translate(vec2(0.0, SEGMENT_SHADOW_OFFSET)), radius::SEGMENT, color::GLASS_SHADOW);
                ui.painter().rect_filled(segment_rect, radius::SEGMENT, color::BG_SELECTED);
            }
            let text = if is_selected { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
            ui.painter().galley(segment_rect.center() - galley.size() / 2.0, galley.clone(), text);
        }
    }
    if let Some(segment) = focused {
        focus_ring(ui, segment.rect, egui::CornerRadius::from(radius::SEGMENT));
    }
    track
}
