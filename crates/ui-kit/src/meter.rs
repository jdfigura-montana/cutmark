//! Level meters with a clip light, horizontal and vertical, drawn only from the design tokens.

use egui::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Ui, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{color, typography};

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
/// Gap between the two bars of the stereo meter.
const STEREO_GAP: f32 = 3.0;
/// Height of the clip light above the vertical bars.
const CLIP_HEIGHT: f32 = 3.0;
/// Gap between the dB scale labels and the bars.
const SCALE_GAP: f32 = 4.0;
/// The dB levels the scale labels name (spec "Meter": dB scale in `type.value`).
const SCALE_LABELS: [f32; 5] = [-60.0, -24.0, -12.0, -6.0, 0.0];
/// The ok, warn and clip zones with their colors, shared by every meter.
const BANDS: [(f32, f32, Color32); 3] = [
    (FLOOR_DBFS, YELLOW_FROM_DBFS, color::SEM_METER_OK),
    (YELLOW_FROM_DBFS, RED_FROM_DBFS, color::SEM_METER_WARN),
    (RED_FROM_DBFS, CEILING_DBFS, color::SEM_METER_CLIP),
];

/// Where a level sits along the bar, from 0.0 at -60 dBFS to 1.0 at 0 dBFS.
fn fraction(dbfs: f32) -> f32 {
    if dbfs.is_nan() {
        return 0.0;
    }
    ((dbfs - FLOOR_DBFS) / (CEILING_DBFS - FLOOR_DBFS)).clamp(0.0, 1.0)
}

/// The filled span of the band between `from` and `to` for a peak level, as
/// fractions of the bar from its start: the left edge of a horizontal bar,
/// the bottom edge of a vertical one. Both orientations share this mapping.
fn band_span(from: f32, to: f32, peak_dbfs: f32) -> (f32, f32) {
    let low = fraction(from);
    let high = fraction(peak_dbfs.min(to));
    (low, high.max(low))
}

/// Draws a level bar from -60 to 0 dBFS (ok to -12, warn to -3, clip above) with a clip light at its right end.
pub fn level_meter(ui: &mut Ui, peak_dbfs: f32, clipped: bool, width: f32) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let width = width.max(HEIGHT + GAP + 1.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bar = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - HEIGHT - GAP, rect.max.y));
        painter.rect_filled(bar, ROUNDING, neutrals.bg_control);
        for (from, to, band_color) in BANDS {
            let (low, high) = band_span(from, to, peak_dbfs);
            if high > low {
                let left = bar.left() + bar.width() * low;
                let right = bar.left() + bar.width() * high;
                painter.rect_filled(Rect::from_min_max(Pos2::new(left, bar.top()), Pos2::new(right, bar.bottom())), CornerRadius::ZERO, band_color);
            }
        }
        let light = Rect::from_min_size(Pos2::new(rect.max.x - HEIGHT, rect.top()), vec2(HEIGHT, HEIGHT));
        painter.rect_filled(light, ROUNDING, if clipped { color::SEM_METER_CLIP } else { neutrals.stroke_hairline });
    }
    response
}

/// Paints the recessed bar and the ok, warn and clip bands of a vertical bar,
/// filling from the bottom up to the peak level.
fn paint_bands_vertical(painter: &egui::Painter, bar: Rect, peak_dbfs: f32) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    painter.rect_filled(bar, ROUNDING, neutrals.bg_control);
    for (from, to, band_color) in BANDS {
        let (low, high) = band_span(from, to, peak_dbfs);
        if high > low {
            let top = bar.bottom() - bar.height() * high;
            let bottom = bar.bottom() - bar.height() * low;
            painter.rect_filled(Rect::from_min_max(Pos2::new(bar.left(), top), Pos2::new(bar.right(), bottom)), CornerRadius::ZERO, band_color);
        }
    }
}

/// Draws a vertical level bar from -60 to 0 dBFS with a small clip light above it.
///
/// `height` is the full allocated height including the clip light and the gap.
pub fn level_meter_vertical(ui: &mut Ui, peak_dbfs: f32, clipped: bool, height: f32) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let height = height.max(CLIP_HEIGHT + GAP + HEIGHT);
    let (rect, response) = ui.allocate_exact_size(vec2(HEIGHT, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bar = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + CLIP_HEIGHT + GAP), rect.max);
        paint_bands_vertical(painter, bar, peak_dbfs);
        let light = Rect::from_min_size(rect.min, vec2(HEIGHT, CLIP_HEIGHT));
        painter.rect_filled(light, ROUNDING, if clipped { color::SEM_METER_CLIP } else { neutrals.stroke_hairline });
    }
    response
}

/// Draws two vertical bars 3px apart (left, then right) with one clip light
/// spanning both above them, and optional dB scale labels beside them
/// (labels that would overlap their neighbour hide; 0 and -60 always show).
///
/// `height` is the full allocated height including the clip light and the gap.
pub fn stereo_meter_vertical(ui: &mut Ui, left_dbfs: f32, right_dbfs: f32, clipped: bool, height: f32, show_scale: bool) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let bars_width = HEIGHT * 2.0 + STEREO_GAP;
    let scale_gap = if show_scale { SCALE_GAP } else { 0.0 };
    let label_width =
        if show_scale { ui.painter().layout_no_wrap("-60".to_owned(), fonts::mono(typography::VALUE_SIZE), neutrals.text_tertiary).size().x } else { 0.0 };
    let height = height.max(CLIP_HEIGHT + GAP + HEIGHT);
    let (rect, response) = ui.allocate_exact_size(vec2(label_width + scale_gap + bars_width, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let bars_left = rect.left() + label_width + scale_gap;
        let bar_top = rect.top() + CLIP_HEIGHT + GAP;
        let left_bar = Rect::from_min_max(Pos2::new(bars_left, bar_top), Pos2::new(bars_left + HEIGHT, rect.bottom()));
        let right_bar = Rect::from_min_max(Pos2::new(left_bar.right() + STEREO_GAP, bar_top), Pos2::new(left_bar.right() + STEREO_GAP + HEIGHT, rect.bottom()));
        paint_bands_vertical(painter, left_bar, left_dbfs);
        paint_bands_vertical(painter, right_bar, right_dbfs);
        let light = Rect::from_min_size(Pos2::new(bars_left, rect.top()), vec2(bars_width, CLIP_HEIGHT));
        painter.rect_filled(light, ROUNDING, if clipped { color::SEM_METER_CLIP } else { neutrals.stroke_hairline });
        if show_scale {
            let font = fonts::mono(typography::VALUE_SIZE);
            let text_height = painter.layout_no_wrap("-60".to_owned(), font.clone(), neutrals.text_tertiary).size().y;
            for (db, (center_y, shown)) in SCALE_LABELS.iter().zip(scale_label_centers(bar_top, rect.bottom(), text_height)) {
                if !shown {
                    continue;
                }
                let galley = painter.layout_no_wrap(format!("{db}"), font.clone(), neutrals.text_tertiary);
                let pos = Pos2::new(bars_left - SCALE_GAP - galley.size().x, center_y - galley.size().y / 2.0);
                painter.galley(pos, galley, neutrals.text_tertiary);
            }
        }
    }
    response
}

/// The center y of each dB scale label in a vertical bar and whether it is
/// drawn: each label sits at its dB position, a label closer than its text
/// height to the previously drawn one hides, and the first (-60) and last
/// (0) labels always show.
fn scale_label_centers(bar_top: f32, bar_bottom: f32, text_height: f32) -> Vec<(f32, bool)> {
    let half = text_height / 2.0;
    let centers: Vec<f32> =
        SCALE_LABELS.iter().map(|db| (bar_bottom - (bar_bottom - bar_top) * fraction(*db)).clamp(bar_top + half, bar_bottom - half)).collect();
    let mut drawn = vec![false; centers.len()];
    let mut previous: Option<f32> = None;
    for (index, &center) in centers.iter().enumerate() {
        let forced = index == 0 || index == centers.len() - 1;
        let far_enough = previous.is_none_or(|prev| prev - center >= text_height);
        if forced || far_enough {
            drawn[index] = true;
            previous = Some(center);
        }
    }
    centers.into_iter().zip(drawn).collect()
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

    /// The vertical bar measures its fill from the bottom with the same
    /// `fraction` the horizontal bar measures from the left, so both meters
    /// light up identically for the same peak level.
    #[test]
    fn the_vertical_meter_uses_the_same_fraction_as_the_horizontal_one() {
        for db in [-60.0, -24.0, -12.0, -6.0, -3.0, -1.0, 0.0] {
            let (low, high) = band_span(FLOOR_DBFS, CEILING_DBFS, db);
            assert_eq!(low, fraction(FLOOR_DBFS), "every band starts at the bar's bottom edge");
            assert_eq!(high, fraction(db), "the vertical fill at {db} dBFS must sit where the horizontal fill sits");
        }
    }

    /// A peak inside a band fills only up to the peak, never past it.
    #[test]
    fn band_span_stops_at_the_peak() {
        assert_eq!(band_span(YELLOW_FROM_DBFS, RED_FROM_DBFS, -6.0), (fraction(-12.0), fraction(-6.0)));
        let unreached = band_span(RED_FROM_DBFS, CEILING_DBFS, -12.0);
        assert_eq!(unreached.0, unreached.1, "an unreached band stays empty");
    }

    /// The scale labels never overlap: every hideable label sits at least one
    /// text height below the previously drawn one, and 0 and -60 always show
    /// (they are exempt from the distance rule, so in a very short meter the
    /// forced 0 may sit close to its neighbour).
    #[test]
    fn scale_labels_never_overlap_and_0_and_minus60_always_show() {
        let text_height = 15.0;
        for bar_height in [40.0, 60.0, 90.0, 140.0, 300.0] {
            let labels = scale_label_centers(0.0, bar_height, text_height);
            assert_eq!(labels.len(), SCALE_LABELS.len());
            assert!(labels.first().is_some_and(|(_, shown)| *shown), "-60 always shows");
            assert!(labels.last().is_some_and(|(_, shown)| *shown), "0 always shows");
            for (index, &(center, shown)) in labels.iter().enumerate() {
                if !shown || index == 0 || index == labels.len() - 1 {
                    continue;
                }
                let previous = labels[..index].iter().rev().find(|(_, shown)| *shown).map(|(above, _)| *above);
                let previous = previous.unwrap_or(center);
                assert!(previous - center >= text_height, "label at {} sits only {} below the drawn label at {}", center, previous - center, previous);
            }
        }
    }

    /// A tall meter shows every scale label.
    #[test]
    fn a_tall_meter_shows_every_scale_label() {
        let labels = scale_label_centers(0.0, 200.0, 15.0);
        assert!(labels.iter().all(|(_, shown)| *shown), "every label fits at 200px, got {labels:?}");
    }
}
