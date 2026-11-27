//! The track area backgrounds (spec: Design Language rev 3 "Track area"): the
//! track row, the header column strip and the empty area below the last track.

use egui::{Color32, CornerRadius, Painter, Rect, pos2};

use crate::HAIRLINE;
use crate::{Neutrals, Theme};

/// The fill of one track row: `bg.track.selected` when selected, otherwise
/// `bg.track.alt` on every other row and nothing on the rest. Every row ends
/// in a 1px `stroke.track` line along its bottom edge.
pub fn paint_track_row(painter: &Painter, rect: Rect, index: usize, selected: bool) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    let fill = row_fill(index, selected, &neutrals);
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
    }
    paint_row_separator(painter, rect);
}

/// The 1px `stroke.track` line along the bottom edge of `rect`. Track rows
/// and track headers end in it.
pub(crate) fn paint_row_separator(painter: &Painter, rect: Rect) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - HAIRLINE), rect.right_bottom());
    painter.rect_filled(line, CornerRadius::ZERO, neutrals.stroke_track);
}

/// The track header column: one continuous `bg.track.header` strip over
/// `rect` (from the ruler row's corner cell down to the Add track row), with
/// the 1px `stroke.track.header` divider along its right edge, between the
/// header column and the lanes.
pub fn paint_track_header_column(painter: &Painter, rect: Rect) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    painter.rect_filled(rect, CornerRadius::ZERO, neutrals.bg_track_header);
    let divider = Rect::from_min_max(rect.left_top() + egui::vec2(rect.width() - HAIRLINE, 0.0), rect.right_bottom());
    painter.rect_filled(divider, CornerRadius::ZERO, neutrals.stroke_track_header);
}

/// The area below the last track (below the Add track row): `bg.track.empty`,
/// so the end of the track list is obvious.
pub fn paint_track_area_empty(painter: &Painter, rect: Rect) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    painter.rect_filled(rect, CornerRadius::ZERO, neutrals.bg_track_empty);
}

/// The color a track row paints with, or `Color32::TRANSPARENT` for an even,
/// unselected row (the window background shows through).
fn row_fill(index: usize, selected: bool, neutrals: &Neutrals) -> Color32 {
    if selected {
        neutrals.bg_track_selected
    } else if index % 2 == 1 {
        neutrals.bg_track_alt
    } else {
        Color32::TRANSPARENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokens::color;

    /// Odd rows shade, even rows do not, and the selected fill wins over both.
    #[test]
    fn row_fill_follows_index_and_selection() {
        let neutrals = Theme::default().neutrals;
        let row_fill = |index, selected| row_fill(index, selected, &neutrals);
        assert_eq!(row_fill(0, false), Color32::TRANSPARENT);
        assert_eq!(row_fill(1, false), color::BG_TRACK_ALT);
        assert_eq!(row_fill(2, false), Color32::TRANSPARENT);
        assert_eq!(row_fill(7, false), color::BG_TRACK_ALT);
        assert_eq!(row_fill(0, true), color::BG_TRACK_SELECTED);
        assert_eq!(row_fill(1, true), color::BG_TRACK_SELECTED);
    }
}
