//! A horizontal level meter with a clip light, drawn only from the suite palette.

use egui::{CornerRadius, Pos2, Rect, Response, Sense, Ui, vec2};

use crate::palette::{CONTROL_BORDER, METER_GREEN, METER_RED, METER_YELLOW, RECESSED};

/// The level at the left end of the bar, in dBFS.
const FLOOR_DBFS: f32 = -60.0;
/// The level where the bar turns from green to yellow, in dBFS.
const YELLOW_FROM_DBFS: f32 = -12.0;
/// The level where the bar turns from yellow to red, in dBFS.
const RED_FROM_DBFS: f32 = -3.0;
/// The level at the right end of the bar, in dBFS.
const CEILING_DBFS: f32 = 0.0;
/// Height of the bar and side of the clip light.
const HEIGHT: f32 = 10.0;
/// Gap between the bar and the clip light.
const GAP: f32 = 4.0;
/// Corner radius of the bar and the clip light.
const ROUNDING: f32 = 2.0;

/// Where a level sits along the bar, from 0.0 at -60 dBFS to 1.0 at 0 dBFS.
fn fraction(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs - FLOOR_DBFS) / (CEILING_DBFS - FLOOR_DBFS)).clamp(0.0, 1.0)
}

/// Draws a level bar from -60 to 0 dBFS (green to -12, yellow to -3, red above) with a clip light at its right end.
pub fn level_meter(ui: &mut Ui, peak_dbfs: f32, clipped: bool, width: f32) -> Response {
    let width = width.max(HEIGHT + GAP + 1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bar = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - HEIGHT - GAP, rect.max.y));
        painter.rect_filled(bar, ROUNDING, RECESSED);
        let bands = [(FLOOR_DBFS, YELLOW_FROM_DBFS, METER_GREEN), (YELLOW_FROM_DBFS, RED_FROM_DBFS, METER_YELLOW), (RED_FROM_DBFS, CEILING_DBFS, METER_RED)];
        for (from, to, color) in bands {
            let end = peak_dbfs.min(to);
            if end > from {
                let left = bar.left() + bar.width() * fraction(from);
                let right = bar.left() + bar.width() * fraction(end);
                painter.rect_filled(Rect::from_min_max(Pos2::new(left, bar.top()), Pos2::new(right, bar.bottom())), CornerRadius::ZERO, color);
            }
        }
        let light = Rect::from_min_size(Pos2::new(rect.max.x - HEIGHT, rect.top()), vec2(HEIGHT, HEIGHT));
        painter.rect_filled(light, ROUNDING, if clipped { METER_RED } else { CONTROL_BORDER });
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_maps_the_meter_range() {
        assert_eq!(fraction(-60.0), 0.0);
        assert_eq!(fraction(0.0), 1.0);
        assert_eq!(fraction(-30.0), 0.5);
        assert_eq!(fraction(-100.0), 0.0);
        assert_eq!(fraction(6.0), 1.0);
        assert_eq!(fraction(f32::NAN), 0.0);
    }
}
