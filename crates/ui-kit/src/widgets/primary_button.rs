//! The primary button (spec "Components": row "Primary button").

use egui::{Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{size, space, typography};
use crate::widgets::{capsule_radius, focus};

/// The primary button: an accent capsule with `accent.on` text in
/// `type.body` SemiBold, `accent.hover` on hover and `accent.pressed`
/// pressed. One per screen (Export, Record, Go Live).
///
/// Usage:
///
/// ```ignore
/// if ui_kit::primary_button(ui, "Export").clicked() {
///     start_export();
/// }
/// ```
pub fn primary_button(ui: &mut Ui, label: &str) -> Response {
    let theme = Theme::get(ui.ctx());
    let font = fonts::semibold(typography::BODY_SIZE);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, theme.accent.on);
    let height = size::PANEL_BUTTON_HEIGHT;
    let width = galley.size().x + 2.0 * space::SCALE_4;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    if ui.is_rect_visible(rect) {
        let fill = if response.is_pointer_button_down_on() {
            theme.accent.pressed
        } else if response.hovered() {
            theme.accent.hover
        } else {
            theme.accent.accent
        };
        ui.painter().rect_filled(rect, capsule_radius(height), fill);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, theme.accent.on);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(height));
    }
    response
}
