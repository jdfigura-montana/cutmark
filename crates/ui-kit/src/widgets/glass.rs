//! The glass surface (spec "Elevation and glass"): the shared recipe behind
//! the transport deck, menus, popovers and toasts. Glass is always on.

use egui::{Context, CornerRadius, Frame, Painter, Rect, Stroke};

use crate::HAIRLINE;
use crate::tokens::color;

/// The shadow of the glass recipe (spec "Elevation and glass": 0, 10,
/// blur 30, `glass.shadow`).
pub(crate) fn glass_shadow() -> egui::Shadow {
    egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: color::GLASS_SHADOW }
}

/// A glass frame: the fill, rim and shadow of the glass recipe. Paint the
/// 1 px top highlight with [`paint_glass_highlight`] once the frame rect is
/// known.
///
/// Usage:
///
/// ```ignore
/// ui_kit::glass_frame(ui.ctx(), ui_kit::tokens::radius::GROUP).show(ui, |ui| {
///     ui.label("Popover");
/// });
/// ```
pub fn glass_frame(_ctx: &Context, radius: impl Into<CornerRadius>) -> Frame {
    Frame::new().corner_radius(radius).stroke(Stroke::new(HAIRLINE, color::GLASS_RIM)).fill(color::GLASS_FILL).shadow(glass_shadow())
}

/// Paints the 1 px inside top highlight of the glass recipe over a drawn
/// glass frame.
pub fn paint_glass_highlight(painter: &Painter, rect: Rect, radius: impl Into<CornerRadius>) {
    let inset = radius.into().nw as f32;
    let y = rect.top() + HAIRLINE / 2.0;
    let (from, to) = (rect.left() + inset, rect.right() - inset);
    if to > from {
        painter.line_segment([egui::pos2(from, y), egui::pos2(to, y)], Stroke::new(HAIRLINE, color::GLASS_TOP_HIGHLIGHT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::radius;

    /// The frame always uses the glass fill, rim and shadow.
    #[test]
    fn glass_frame_uses_the_glass_recipe() {
        let ctx = Context::default();
        let frame = glass_frame(&ctx, radius::GROUP);
        assert_eq!(frame.fill, color::GLASS_FILL);
        assert_eq!(frame.shadow, glass_shadow());
        assert_eq!(frame.stroke.width, HAIRLINE);
        assert_eq!(frame.stroke.color, color::GLASS_RIM);
    }
}
