//! ui::timeline::lane: a track lane, where clips live.

use eframe::egui;

/// Draws one track lane background over `rect`.
///
/// Track lanes alternate between two subtle background shades (`even_bg` for
/// even tracks, `odd_bg` for odd ones) and each lane gets a 1 px separator
/// line under its bottom edge.
pub fn draw(
    painter: &egui::Painter,
    rect: egui::Rect,
    is_even: bool,
    even_bg: egui::Color32,
    odd_bg: egui::Color32,
    separator: egui::Stroke,
) {
    painter.rect_filled(rect, 0.0, if is_even { even_bg } else { odd_bg });
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        separator,
    );
}
