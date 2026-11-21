//! The glass surface (spec "Elevation and glass"): the shared recipe behind
//! the transport deck, menus, popovers and toasts. When [`Theme::glass`] is
//! off, every glass surface falls back to solid `bg.group` with the same rim
//! and no highlight or shadow.

use egui::{Context, CornerRadius, Frame, Painter, Rect, Stroke};

use crate::HAIRLINE;
use crate::Theme;
use crate::tokens::color;

/// The shadow of the glass recipe (spec "Elevation and glass": 0, 10,
/// blur 30, `glass.shadow`).
pub(crate) fn glass_shadow() -> egui::Shadow {
    egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: color::GLASS_SHADOW }
}

/// A glass frame: the fill, rim and shadow of the glass recipe, or the solid
/// `bg.group` fallback with the same rim when [`Theme::glass`] is off. Paint
/// the 1 px top highlight with [`paint_glass_highlight`] once the frame rect
/// is known.
///
/// Usage:
///
/// ```ignore
/// ui_kit::glass_frame(ui.ctx(), ui_kit::tokens::radius::GROUP).show(ui, |ui| {
///     ui.label("Popover");
/// });
/// ```
pub fn glass_frame(ctx: &Context, radius: impl Into<CornerRadius>) -> Frame {
    let mut frame = Frame::new().corner_radius(radius).stroke(Stroke::new(HAIRLINE, color::GLASS_RIM));
    if Theme::get(ctx).glass {
        frame = frame.fill(color::GLASS_FILL).shadow(glass_shadow());
    } else {
        frame = frame.fill(color::BG_GROUP);
    }
    frame
}

/// Paints the 1 px inside top highlight of the glass recipe over a drawn
/// glass frame. Draws nothing when [`Theme::glass`] is off.
pub fn paint_glass_highlight(painter: &Painter, rect: Rect, radius: impl Into<CornerRadius>) {
    if !Theme::get(painter.ctx()).glass {
        return;
    }
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

    /// With glass on, the frame uses the glass fill and casts the glass shadow.
    #[test]
    fn glass_frame_uses_the_glass_recipe_when_glass_is_on() {
        let ctx = Context::default();
        let frame = glass_frame(&ctx, radius::GROUP);
        assert_eq!(frame.fill, color::GLASS_FILL);
        assert_eq!(frame.shadow, glass_shadow());
    }

    /// With glass off, the frame falls back to `bg.group`, keeps the rim and
    /// drops the shadow.
    #[test]
    fn glass_frame_falls_back_to_bg_group_when_glass_is_off() {
        let ctx = Context::default();
        Theme::set(&ctx, Theme { glass: false, ..Theme::default() });
        let frame = glass_frame(&ctx, radius::GROUP);
        assert_eq!(frame.fill, color::BG_GROUP);
        assert_eq!(frame.shadow, egui::Shadow::NONE);
        assert_eq!(frame.stroke.width, HAIRLINE);
        assert_eq!(frame.stroke.color, color::GLASS_RIM);
    }
}
