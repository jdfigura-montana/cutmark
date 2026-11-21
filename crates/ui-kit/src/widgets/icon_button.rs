//! The icon button (spec "Components": row "Icon button").
//!
//! The icon is a line icon from [`crate::icons::regular`], laid out in the
//! phosphor regular family (`fonts::icon_regular`).

use egui::{Color32, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::fonts;
use crate::tokens::{color, size, typography};
use crate::widgets::{capsule_radius, focus};

/// The icon button: a 32px transparent circle, `bg.hover` on hover, the icon
/// in `text.secondary` (`text.primary` hovered). The tooltip doubles as the
/// accessible label and is required (spec "Icon button").
///
/// The icon string is a phosphor regular (line) glyph, for example
/// `ui_kit::icons::regular::GEAR_SIX`.
///
/// Usage:
///
/// ```ignore
/// if ui_kit::icon_button(ui, ui_kit::icons::regular::GEAR_SIX, "Settings").clicked() {
///     open_settings();
/// }
/// ```
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let side = size::ICON_BUTTON;
    let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::click());
    let response = response.on_hover_text(tooltip);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, tooltip));
    let hovered = response.hovered();
    if ui.is_rect_visible(rect) {
        if hovered {
            ui.painter().circle_filled(rect.center(), side / 2.0, color::BG_HOVER);
        }
        // No icon-size token exists; the headline size is the closest match
        // for an icon inside the 32px circle (spec "Icons").
        let galley = ui.painter().layout_no_wrap(
            icon.to_owned(),
            fonts::icon_regular(typography::HEADLINE_SIZE),
            if hovered { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY },
        );
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(side));
    }
    response
}
