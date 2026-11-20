//! A horizontal level meter with a clip light, drawn only from the design tokens.

use egui::{CornerRadius, Pos2, Rect, Response, Sense, Ui, vec2};

use crate::tokens::color;

/// The level at the left end of the bar, in dBFS.
const FLOOR_DBFS: f32 = -60.0;
/// The level where the bar turns from green to yellow, in dBFS.
const YELLOW_FROM_DBFS: f32 = -12.0;
/// The level where the bar turns from yellow to red, in dBFS.
const RED_FROM_DBFS: f32 = -3.0;
/// The level at the right end of the bar, in dBFS.
const CEILING_DBFS: f32 = 0.0;
/// Height of the bar and side of the clip light (spec "Meter": 6px bars).
const HEIGHT: f32 = 6.0;
/// Gap between the bar and the clip light.
const GAP: f32 = 4.0;
/// Corner radius of the bar and the clip light (spec "Meter": 3px radius).
const ROUNDING: f32 = 3.0;

/// Where a level sits along the bar, from 0.0 at -60 dBFS to 1.0 at 0 dBFS.
fn fraction(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs - FLOOR_DBFS) / (CEILING_DBFS - FLOOR_DBFS)).clamp(0.0, 1.0)
}

/// Draws a level bar from -60 to 0 dBFS (ok to -12, warn to -3, clip above) with a clip light at its right end.
pub fn level_meter(ui: &mut Ui, peak_dbfs: f32, clipped: bool, width: f32) -> Response {
    let width = width.max(HEIGHT + GAP + 1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bar = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - HEIGHT - GAP, rect.max.y));
        painter.rect_filled(bar, ROUNDING, color::BG_CONTROL);
        let bands = [
            (FLOOR_DBFS, YELLOW_FROM_DBFS, color::SEM_METER_OK),
            (YELLOW_FROM_DBFS, RED_FROM_DBFS, color::SEM_METER_WARN),
            (RED_FROM_DBFS, CEILING_DBFS, color::SEM_METER_CLIP),
        ];
        for (from, to, band_color) in bands {
            let end = peak_dbfs.min(to);
            if end > from {
                let left = bar.left() + bar.width() * fraction(from);
                let right = bar.left() + bar.width() * fraction(end);
                painter.rect_filled(Rect::from_min_max(Pos2::new(left, bar.top()), Pos2::new(right, bar.bottom())), CornerRadius::ZERO, band_color);
            }
        }
        let light = Rect::from_min_size(Pos2::new(rect.max.x - HEIGHT, rect.top()), vec2(HEIGHT, HEIGHT));
        painter.rect_filled(light, ROUNDING, if clipped { color::SEM_METER_CLIP } else { color::STROKE_HAIRLINE });
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
