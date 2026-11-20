//! The chip (spec "Components": row "Chip").

use egui::{Color32, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::tokens::{color, size, space, typography};
use crate::widgets::{capsule_radius, focus};

/// The chip: a 26 tall capsule, `bg.control` when off (`bg.hover` on hover),
/// a `text.primary` fill with `bg.ground` text when on. Used for filters and
/// editing toggles (Snap, Ripple, Linked).
///
/// Usage:
///
/// ```ignore
/// let mut snap = true;
/// ui_kit::chip(ui, &mut snap, "Snap");
/// ```
pub fn chip(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let height = size::CHIP_HEIGHT;
    let font = fonts::medium(typography::BODY_SIZE);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let width = galley.size().x + 2.0 * space::SCALE_4;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, *on, label));
    if response.clicked() {
        *on = !*on;
    }
    if ui.is_rect_visible(rect) {
        let fill = if *on {
            color::TEXT_PRIMARY
        } else if response.hovered() {
            color::BG_HOVER
        } else {
            color::BG_CONTROL
        };
        let text = if *on { color::BG_GROUND } else { color::TEXT_SECONDARY };
        ui.painter().rect_filled(rect, capsule_radius(height), fill);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, text);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(height));
    }
    response
}
