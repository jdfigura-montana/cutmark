//! The status bar (spec "Components": row "Status bar").

use egui::{Pos2, Response, Sense, Ui, vec2};

use crate::fonts;
use crate::tokens::{color, size, space, typography};

/// The status bar: the items in `type.caption` `text.tertiary`, no
/// background, 26 tall.
///
/// Usage:
///
/// ```ignore
/// ui_kit::status_bar(ui, &["Ready", "48 kHz", "24 bit"]);
/// ```
pub fn status_bar(ui: &mut Ui, items: &[&str]) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::STATUS_BAR_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let font = fonts::medium(typography::CAPTION_SIZE);
        let mut x = rect.left() + space::SCALE_4;
        for item in items {
            let galley = painter.layout_no_wrap((*item).to_owned(), font.clone(), color::TEXT_TERTIARY);
            let pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
            let width = galley.size().x;
            painter.galley(pos, galley, color::TEXT_TERTIARY);
            x = pos.x + width + space::SCALE_6;
        }
    }
    response
}
