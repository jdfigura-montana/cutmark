//! ui::timeline::lane: a track lane, where clips live.

use eframe::egui;

/// Draws one track lane background over `rect`.
///
/// The fill is the ui-kit track row: `bg.track.alt` on every other row (odd
/// `index`), `bg.track.selected` when `selected`, nothing on the rest so the
/// panel shows through. Each lane also gets a 1 px separator line under its
/// bottom edge.
pub fn draw(painter: &egui::Painter, rect: egui::Rect, index: usize, selected: bool, separator: egui::Stroke) {
    ui_kit::paint_track_row(painter, rect, index, selected);
    painter.line_segment([egui::pos2(rect.left(), rect.bottom()), egui::pos2(rect.right(), rect.bottom())], separator);
}

#[cfg(test)]
mod tests {
    use ui_kit::tokens::color;

    use super::*;
    use crate::ui::test_util::Headless;

    #[test]
    fn lanes_shade_every_other_row_and_the_selected_one() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 50.0));
        let mut h = Headless::new(400.0, 200.0);
        let out = h.frame(Vec::new(), |ui| {
            draw(ui.painter(), rect, 0, false, egui::Stroke::NONE);
            draw(ui.painter(), rect.translate(egui::vec2(0.0, 60.0)), 1, false, egui::Stroke::NONE);
            draw(ui.painter(), rect.translate(egui::vec2(0.0, 120.0)), 2, true, egui::Stroke::NONE);
        });
        let fills: Vec<_> = out.rects().into_iter().map(|r| r.fill).collect();
        assert_eq!(fills.iter().filter(|f| **f == color::BG_TRACK_ALT).count(), 1, "the odd row is shaded");
        assert_eq!(fills.iter().filter(|f| **f == color::BG_TRACK_SELECTED).count(), 1, "the selected row is highlighted");
    }
}
