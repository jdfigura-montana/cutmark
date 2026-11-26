//! ui::timeline::lane: a track lane, where clips live.

use eframe::egui;

use crate::ui::timeline::metrics;

/// Draws one track lane over `rect`.
///
/// The lanes surface (`bg.stage.inner`) is already painted under every row, so
/// a lane adds only the faint `bg.lane.selected` tint when `selected`, then
/// the `bg.ground` line along its bottom edge.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, selected: bool) {
    if selected {
        let neutral = crate::ui::neutrals(painter.ctx());
        painter.rect_filled(rect, 0.0, neutral.bg_lane_selected);
    }
    paint_row_line(painter, rect);
}

/// The 1 px `bg.ground` line between tracks, along the bottom edge of `rect`.
/// Headers and lanes both end in it, so each track reads as one row.
pub(crate) fn paint_row_line(painter: &egui::Painter, rect: egui::Rect) {
    let neutral = crate::ui::neutrals(painter.ctx());
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - metrics::HAIRLINE), rect.right_bottom());
    painter.rect_filled(line, 0.0, neutral.bg_ground);
}

#[cfg(test)]
mod tests {
    use ui_kit::tokens::color;

    use super::*;
    use crate::ui::test_util::Headless;

    #[test]
    fn only_the_selected_lane_is_tinted_and_every_lane_ends_in_a_line() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 50.0));
        let mut h = Headless::new(400.0, 200.0);
        let out = h.frame(Vec::new(), |ui| {
            draw(ui.painter(), rect, false);
            draw(ui.painter(), rect.translate(egui::vec2(0.0, 60.0)), true);
        });
        let rects = out.rects();
        let tinted: Vec<_> = rects.iter().filter(|r| r.fill == color::BG_LANE_SELECTED).collect();
        assert_eq!(tinted.len(), 1, "one tint, on the selected lane");
        assert_eq!(tinted[0].rect, rect.translate(egui::vec2(0.0, 60.0)));
        let lines = rects.iter().filter(|r| r.fill == color::BG_GROUND && r.rect.height() == metrics::HAIRLINE).count();
        assert_eq!(lines, 2, "one line per lane");
        assert!(rects.iter().all(|r| r.stroke.width == 0.0), "no outline");
    }
}
