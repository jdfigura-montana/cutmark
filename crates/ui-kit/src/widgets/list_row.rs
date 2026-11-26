//! The list row (spec "Components": row "List row").

use egui::{Color32, CornerRadius, Pos2, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{radius, size, space, typography};
use crate::widgets::focus;

/// Diameter of the leading category dot (spec "List row": leading 8px dot).
const DOT_DIAMETER: f32 = 8.0;

/// The list row: 38 tall, radius 9 (the spec "List row" radius, the same
/// value as `radius.segment`), hover `bg.group`, selected `bg.row.selected`,
/// a leading 8px category dot and a trailing value in `type.value`
/// `text.tertiary`.
///
/// Usage:
///
/// ```ignore
/// if ui_kit::list_row(ui, false, None, "Take 3", Some("02:14")).clicked() {
///     open_take(3);
/// }
/// ```
pub fn list_row(ui: &mut Ui, selected: bool, dot: Option<Color32>, label: &str, trailing: Option<&str>) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::LIST_ROW_HEIGHT), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            Some(neutrals.bg_row_selected)
        } else if response.hovered() {
            Some(neutrals.bg_group)
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect, radius::SEGMENT, fill);
        }
        let inset = space::SCALE_4;
        let mut x = rect.left() + inset;
        if let Some(dot) = dot {
            painter.circle_filled(Pos2::new(x + DOT_DIAMETER / 2.0, rect.center().y), DOT_DIAMETER / 2.0, dot);
            x += DOT_DIAMETER + inset;
        }
        let text_color = if selected { neutrals.text_primary } else { neutrals.text_secondary };
        let label_galley = painter.layout_no_wrap(label.to_owned(), fonts::medium(typography::BODY_SIZE), text_color);
        painter.galley(Pos2::new(x, rect.center().y - label_galley.size().y / 2.0), label_galley, text_color);
        if let Some(trailing) = trailing {
            let trailing_galley = painter.layout_no_wrap(trailing.to_owned(), fonts::mono(typography::VALUE_SIZE), neutrals.text_tertiary);
            let pos = Pos2::new(rect.right() - inset - trailing_galley.size().x, rect.center().y - trailing_galley.size().y / 2.0);
            painter.galley(pos, trailing_galley, neutrals.text_tertiary);
        }
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, CornerRadius::from(radius::SEGMENT));
    }
    response
}
