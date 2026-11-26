//! The scrollbar thumb (spec "Components": row "Scrollbar"), for
//! hand-painted overlay scrollbars.

use egui::{Painter, Rect};

use crate::SCROLL_ACTIVE_OPACITY;
use crate::SCROLL_DORMANT_OPACITY;
use crate::Theme;
use crate::widgets::capsule_radius;

/// The scrollbar thumb: a 6px capsule (`size.scrollbar.thumb.width` gives the
/// width, the caller sizes the rect) in `text.tertiary` at 40%, 70% on hover
/// and while dragging. Overlay, no track fill.
///
/// Usage:
///
/// ```ignore
/// ui_kit::scroll_thumb(ui.painter(), thumb_rect, response.dragged());
/// ```
pub fn scroll_thumb(painter: &Painter, rect: Rect, hovered_or_dragged: bool) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    let opacity = if hovered_or_dragged { SCROLL_ACTIVE_OPACITY } else { SCROLL_DORMANT_OPACITY };
    painter.rect_filled(rect, capsule_radius(rect.width()), neutrals.text_tertiary.gamma_multiply(opacity));
}
