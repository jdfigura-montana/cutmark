//! The status bar (spec "Components": row "Status bar").

use egui::{Pos2, Response, Sense, Ui, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{size, space, typography};

/// Height of the status bar under a stage panel (spec: E3 amendment, "1. The
/// notch").
pub const STATUS_BAR_GAP_HEIGHT: f32 = 30.0;
/// Width of the empty center of [`status_bar_with_gap`], kept for the stage's
/// primary button.
pub const STATUS_BAR_CENTER_GAP: f32 = 120.0;
/// Space between items in [`status_bar_with_gap`] (spec: E3 amendment, "4. No
/// generic tells").
const GAP_ITEM_SPACING: f32 = 20.0;

/// The status bar: the items in `type.caption` `text.tertiary`, no
/// background, 26 tall.
///
/// Usage:
///
/// ```ignore
/// ui_kit::status_bar(ui, &["Ready", "48 kHz", "24 bit"]);
/// ```
pub fn status_bar(ui: &mut Ui, items: &[&str]) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::STATUS_BAR_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let font = fonts::medium(typography::CAPTION_SIZE);
        let mut x = rect.left() + space::SCALE_4;
        for item in items {
            let galley = painter.layout_no_wrap((*item).to_owned(), font.clone(), neutrals.text_tertiary);
            let pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
            let width = galley.size().x;
            painter.galley(pos, galley, neutrals.text_tertiary);
            x = pos.x + width + space::SCALE_6;
        }
    }
    response
}

/// The status bar under a stage panel: 30 tall, with the center
/// [`STATUS_BAR_CENTER_GAP`] left empty so no text sits under the notch button.
/// Items run left to right, 20 apart. The first one that would reach into the
/// gap continues after it, and so do the rest. Items past the right edge are
/// clipped.
///
/// Usage:
///
/// ```ignore
/// ui_kit::status_bar_with_gap(ui, &["Ready", "48 kHz", "24 bit"]);
/// ```
pub fn status_bar_with_gap(ui: &mut Ui, items: &[&str]) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), STATUS_BAR_GAP_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let font = fonts::medium(typography::CAPTION_SIZE);
        let gap_left = rect.center().x - STATUS_BAR_CENTER_GAP / 2.0;
        let gap_right = rect.center().x + STATUS_BAR_CENTER_GAP / 2.0;
        let mut x = rect.left() + space::SCALE_4;
        for item in items {
            let galley = painter.layout_no_wrap((*item).to_owned(), font.clone(), neutrals.text_tertiary);
            let width = galley.size().x;
            if x < gap_right && x + width > gap_left {
                x = gap_right;
            }
            let pos = Pos2::new(x, rect.center().y - galley.size().y / 2.0);
            painter.galley(pos, galley, neutrals.text_tertiary);
            x += width + GAP_ITEM_SPACING;
        }
    }
    response
}
