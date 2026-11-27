//! The window caption buttons: minimize, maximize or restore, close (spec
//! "Window chrome", Windows 11 list; sizes row: 46 x 52 each).

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, pos2, vec2};

use crate::Theme;
use crate::chrome;
use crate::tokens::{color, size};

/// The stroke width of the Segoe Fluent style glyphs: 1px lines.
const GLYPH_STROKE: f32 = 1.0;
/// Half the glyph extent: the dash half length, the squares half side and the
/// x half diagonal.
const GLYPH_HALF: f32 = 4.5;
/// The offset between the two squares of the restore glyph.
const RESTORE_OFFSET: f32 = 1.5;

/// The rectangles of the three caption buttons, in egui points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptionRects {
    /// The minimize button rectangle.
    pub minimize: Rect,
    /// The maximize or restore button rectangle.
    pub maximize: Rect,
    /// The close button rectangle.
    pub close: Rect,
}

/// Draws the three caption buttons at the current cursor and returns their
/// rectangles, or `None` in full screen (spec "Window chrome": full screen
/// hides the caption buttons and keeps the toolbar).
///
/// In a right-to-left layout the buttons land left to right as minimize,
/// maximize, close on the far right. The actions go through egui
/// `ViewportCommand`s (Minimized, Maximized toggle, Close); on Windows the
/// maximize click never reaches egui, because the chrome answers the hit test
/// with `HTMAXBUTTON` and the OS maximizes itself. The maximize button's
/// hover look comes from [`chrome::is_maximized_button_hovered`], not egui
/// hover, for the same reason. [`crate::widgets::main_toolbar`] only draws
/// these buttons when [`chrome::is_custom`] is true.
pub fn caption_buttons(ui: &mut Ui) -> Option<CaptionRects> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    if ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return None;
    }
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    let side = vec2(size::CAPTION_BUTTON_WIDTH, size::CAPTION_BUTTON_HEIGHT);

    // Close (rightmost in a right-to-left layout).
    let (close, response) = ui.allocate_exact_size(side, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Close"));
    let close_hovered = response.hovered();
    paint_fill(ui, close, close_hovered, color::CAPTION_CLOSE_HOVER);
    let c = close.center();
    let stroke = Stroke::new(GLYPH_STROKE, glyph_color(close_hovered, neutrals.text_secondary, color::CAPTION_CLOSE_GLYPH_HOVER));
    ui.painter().line_segment([pos2(c.x - GLYPH_HALF, c.y - GLYPH_HALF), pos2(c.x + GLYPH_HALF, c.y + GLYPH_HALF)], stroke);
    ui.painter().line_segment([pos2(c.x + GLYPH_HALF, c.y - GLYPH_HALF), pos2(c.x - GLYPH_HALF, c.y + GLYPH_HALF)], stroke);
    if response.clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // Maximize or restore.
    let (maximize, response) = ui.allocate_exact_size(side, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, if maximized { "Restore" } else { "Maximize" }));
    // The hover comes from the chrome, which tracks the non-client pointer
    // moves egui never sees (spec "Window chrome": Snap Layouts flyout).
    let max_hovered = chrome::is_maximized_button_hovered(ui.ctx());
    paint_fill(ui, maximize, max_hovered, neutrals.bg_hover);
    let stroke = Stroke::new(GLYPH_STROKE, glyph_color(max_hovered, neutrals.text_secondary, neutrals.text_secondary));
    let c = maximize.center();
    if maximized {
        // Restore: two overlapping squares, the back one offset up-right, its
        // top and right edges the only parts clear of the front square.
        let square = egui::Vec2::splat(2.0 * GLYPH_HALF - 1.0);
        let front = Rect::from_center_size(c + vec2(-RESTORE_OFFSET, RESTORE_OFFSET), square);
        let back = Rect::from_center_size(c + vec2(RESTORE_OFFSET, -RESTORE_OFFSET), square);
        ui.painter().rect_stroke(front, 0.0, stroke, StrokeKind::Inside);
        ui.painter().line_segment([back.left_top(), back.right_top()], stroke);
        ui.painter().line_segment([back.right_top(), back.right_bottom()], stroke);
    } else {
        // Maximize: one square outline.
        ui.painter().rect_stroke(Rect::from_center_size(c, egui::Vec2::splat(2.0 * GLYPH_HALF)), 0.0, stroke, StrokeKind::Inside);
    }
    if response.clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
    }

    // Minimize (leftmost).
    let (minimize, response) = ui.allocate_exact_size(side, Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, "Minimize"));
    let min_hovered = response.hovered();
    paint_fill(ui, minimize, min_hovered, neutrals.bg_hover);
    let c = minimize.center();
    ui.painter().line_segment(
        [pos2(c.x - GLYPH_HALF, c.y), pos2(c.x + GLYPH_HALF, c.y)],
        Stroke::new(GLYPH_STROKE, glyph_color(min_hovered, neutrals.text_secondary, neutrals.text_secondary)),
    );
    if response.clicked() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    Some(CaptionRects { minimize, maximize, close })
}

/// Paints a caption button hover fill; nothing at rest.
fn paint_fill(ui: &mut Ui, rect: Rect, hovered: bool, hover_fill: Color32) {
    if hovered {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, hover_fill);
    }
}

/// The glyph color: `text.secondary` at rest, `hover_color` on hover.
fn glyph_color(hovered: bool, rest_color: Color32, hover_color: Color32) -> Color32 {
    if hovered { hover_color } else { rest_color }
}
