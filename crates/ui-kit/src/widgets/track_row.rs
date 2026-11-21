//! The track row background (spec "Components": row "Track row").

use egui::{Color32, CornerRadius, Painter, Rect};

use crate::tokens::color;

/// The fill of one track row: `bg.track.selected` when selected, otherwise
/// `bg.track.alt` on every other row and nothing on the rest.
pub fn paint_track_row(painter: &Painter, rect: Rect, index: usize, selected: bool) {
    let fill = row_fill(index, selected);
    if fill != Color32::TRANSPARENT {
        painter.rect_filled(rect, CornerRadius::ZERO, fill);
    }
}

/// The color a track row paints with, or `Color32::TRANSPARENT` for an even,
/// unselected row (the window background shows through).
fn row_fill(index: usize, selected: bool) -> Color32 {
    if selected {
        color::BG_TRACK_SELECTED
    } else if index % 2 == 1 {
        color::BG_TRACK_ALT
    } else {
        Color32::TRANSPARENT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Odd rows shade, even rows do not, and the selected fill wins over both.
    #[test]
    fn row_fill_follows_index_and_selection() {
        assert_eq!(row_fill(0, false), Color32::TRANSPARENT);
        assert_eq!(row_fill(1, false), color::BG_TRACK_ALT);
        assert_eq!(row_fill(2, false), Color32::TRANSPARENT);
        assert_eq!(row_fill(7, false), color::BG_TRACK_ALT);
        assert_eq!(row_fill(0, true), color::BG_TRACK_SELECTED);
        assert_eq!(row_fill(1, true), color::BG_TRACK_SELECTED);
    }
}
