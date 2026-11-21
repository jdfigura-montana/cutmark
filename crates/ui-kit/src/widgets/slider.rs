//! The slider (spec "Components": row "Slider").

use std::ops::RangeInclusive;

use egui::{Color32, Key, Pos2, Rect, Response, Sense, Ui, WidgetInfo, vec2};

use crate::Theme;
use crate::tokens::{color, size};
use crate::widgets::{capsule_radius, focus};

/// Height of the thin track (spec "Slider": thin track).
const TRACK_HEIGHT: f32 = 4.0;
/// Diameter of the white knob (spec "Slider": white knob).
const KNOB_DIAMETER: f32 = 14.0;
/// Keyboard step as a share of the range: the arrow keys move 1% of the range.
const KEY_STEP_FRACTION: f32 = 0.01;

/// The fill color of a slider: [`SliderKind::Accent`] and
/// [`SliderKind::Bipolar`] use the theme accent, [`SliderKind::Zoom`] uses
/// `text.primary`, because zoom is not a selection (spec "Slider").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderKind {
    /// Accent fill, for value sliders.
    Accent,
    /// `text.primary` fill, for zoom sliders.
    Zoom,
    /// Accent fill from the middle of the range to the value, for sliders
    /// whose center is the neutral position, such as pan.
    Bipolar,
}

impl SliderKind {
    /// The fill color for this kind, given the theme accent.
    fn fill_color(self, accent: Color32) -> Color32 {
        match self {
            Self::Accent | Self::Bipolar => accent,
            Self::Zoom => color::TEXT_PRIMARY,
        }
    }
}

/// The knob position along the track, from 0.0 at the range start to 1.0 at its end.
fn fraction(value: f32, range: RangeInclusive<f32>) -> f32 {
    let span = range.end() - range.start();
    if span <= 0.0 {
        return 0.0;
    }
    ((value - range.start()) / span).clamp(0.0, 1.0)
}

/// The filled part of the track as fractions of its width, `(from, to)` with
/// `from <= to`: from the range start to the value, or for
/// [`SliderKind::Bipolar`] between the range midpoint and the value.
fn fill_span(kind: SliderKind, fraction: f32) -> (f32, f32) {
    match kind {
        SliderKind::Bipolar => (fraction.min(0.5), fraction.max(0.5)),
        SliderKind::Accent | SliderKind::Zoom => (0.0, fraction),
    }
}

/// The slider: a thin `bg.selected` track, the fill up to the value in the
/// [`SliderKind`] color (from the range midpoint for [`SliderKind::Bipolar`]),
/// a white knob. Pointer clicks and drags set the value;
/// the arrow keys step it by 1% of the range when focused.
///
/// Usage:
///
/// ```ignore
/// let mut gain = 0.5;
/// ui_kit::slider(ui, &mut gain, 0.0..=1.0, ui_kit::SliderKind::Accent);
/// ```
pub fn slider(ui: &mut Ui, value: &mut f32, range: RangeInclusive<f32>, kind: SliderKind) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), size::SEGMENT_HEIGHT), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::slider(true, *value as f64, ""));
    if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let fraction = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        *value = range.start() + fraction * (range.end() - range.start());
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.has_focus() {
        let (right, left) = ui.input(|input| (input.key_pressed(Key::ArrowRight), input.key_pressed(Key::ArrowLeft)));
        let span = range.end() - range.start();
        let step = span * KEY_STEP_FRACTION;
        if right && span > 0.0 {
            *value = (*value + step).min(*range.end());
        }
        if left && span > 0.0 {
            *value = (*value - step).max(*range.start());
        }
    }
    if ui.is_rect_visible(rect) {
        let accent = Theme::get(ui.ctx()).accent.accent;
        let painter = ui.painter();
        let track = Rect::from_center_size(rect.center(), vec2(rect.width(), TRACK_HEIGHT));
        painter.rect_filled(track, capsule_radius(TRACK_HEIGHT), color::BG_SELECTED);
        let fraction = fraction(*value, range);
        let (from, to) = fill_span(kind, fraction);
        if to > from {
            let fill =
                Rect::from_min_max(Pos2::new(track.left() + track.width() * from, track.min.y), Pos2::new(track.left() + track.width() * to, track.max.y));
            painter.rect_filled(fill, capsule_radius(TRACK_HEIGHT), kind.fill_color(accent));
        }
        let knob_center = Pos2::new(track.left() + track.width() * fraction, track.center().y);
        painter.circle_filled(knob_center, KNOB_DIAMETER / 2.0, Color32::WHITE);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(size::SEGMENT_HEIGHT));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The knob fraction maps the ends and the middle of the range.
    #[test]
    fn fraction_maps_the_range() {
        assert_eq!(fraction(0.0, 0.0..=1.0), 0.0);
        assert_eq!(fraction(0.5, 0.0..=1.0), 0.5);
        assert_eq!(fraction(1.0, 0.0..=1.0), 1.0);
        assert_eq!(fraction(0.0, 40.0..=140.0), 0.0);
        assert_eq!(fraction(140.0, 40.0..=140.0), 1.0);
    }

    /// Values outside the range clamp, and an empty range reads 0.
    #[test]
    fn fraction_clamps_out_of_range_values() {
        assert_eq!(fraction(-1.0, 0.0..=1.0), 0.0);
        assert_eq!(fraction(2.0, 0.0..=1.0), 1.0);
        assert_eq!(fraction(0.5, 2.0..=2.0), 0.0);
    }

    /// The bipolar fill runs from the midpoint to the value, on either side.
    #[test]
    fn bipolar_fill_starts_at_the_midpoint() {
        assert_eq!(fill_span(SliderKind::Bipolar, 0.5), (0.5, 0.5));
        assert_eq!(fill_span(SliderKind::Bipolar, 0.25), (0.25, 0.5));
        assert_eq!(fill_span(SliderKind::Bipolar, 1.0), (0.5, 1.0));
        assert_eq!(fill_span(SliderKind::Accent, 0.25), (0.0, 0.25));
        assert_eq!(fill_span(SliderKind::Zoom, 0.0), (0.0, 0.0));
    }
}
