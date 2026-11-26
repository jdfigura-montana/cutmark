//! The text field and the search field (spec "Components": rows "Search
//! field", which shares its recipe with plain text fields).

use egui::{Margin, Response, Ui, WidgetInfo, WidgetType};

use crate::Theme;
use crate::fonts;
use crate::tokens::{radius, space, typography};

/// The shared field body: a `bg.group` frame with `radius.group`, the hint in
/// `text.tertiary`, and `icon` drawn ahead of the edit when present.
pub(crate) fn field(ui: &mut Ui, text: &mut String, hint: &str, icon: Option<&str>) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let response = egui::Frame::new()
        .fill(neutrals.bg_group)
        .corner_radius(radius::GROUP)
        .inner_margin(Margin::symmetric(space::SCALE_4 as i8, space::SCALE_2 as i8))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(icon) = icon {
                    ui.label(egui::RichText::new(icon).font(fonts::icon_regular(typography::BODY_SIZE)).color(neutrals.text_tertiary));
                }
                ui.scope(|ui| {
                    // egui takes the hint color from `weak_text_color`
                    // (spec "Search field": placeholder in `text.tertiary`).
                    ui.style_mut().visuals.weak_text_color = Some(neutrals.text_tertiary);
                    egui::TextEdit::singleline(text).hint_text(hint).desired_width(ui.available_width()).frame(egui::Frame::NONE).show(ui).response.response
                })
                .inner
            })
            .inner
        })
        .inner;
    // The accessible name is the hint; egui itself leaves the TextEdit nameless.
    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, hint));
    response
}

/// The text field: `bg.group`, `radius.group`, the placeholder in
/// `text.tertiary`.
///
/// Usage:
///
/// ```ignore
/// let mut name = String::new();
/// ui_kit::text_field(ui, &mut name, "Project name");
/// ```
pub fn text_field(ui: &mut Ui, text: &mut String, hint: &str) -> Response {
    field(ui, text, hint, None)
}
