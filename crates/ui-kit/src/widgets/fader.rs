//! The fader (spec "Components": rows "Slider" and "Meter"): a vertical gain slider.

use egui::{Color32, Key, Pos2, Rect, Response, Sense, Ui, WidgetInfo, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{size, space, typography};
use crate::widgets::{capsule_radius, focus};

/// The level at the bottom of the fader travel, in dB.
const MIN_DB: f32 = -60.0;
/// The level at the top of the fader travel, in dB.
const MAX_DB: f32 = 6.0;
/// Where 0 dB sits along the travel (spec: 0 dB at about 75% of the travel).
const ZERO_FRACTION: f32 = 0.75;
/// Width of the thin track (spec "Slider": thin track).
const TRACK_WIDTH: f32 = 4.0;
/// Diameter of the white knob (spec "Slider": white knob).
const KNOB_DIAMETER: f32 = 14.0;
/// The arrow-key step, in dB.
const DB_STEP: f32 = 0.5;
/// The 0 dB level the double-click resets to.
const ZERO_DB: f32 = 0.0;

/// The knob position along the travel, from 0.0 at `MIN_DB` to 1.0 at `MAX_DB`,
/// piecewise linear so that 0 dB sits at [`ZERO_FRACTION`] of the travel.
fn fraction(db: f32) -> f32 {
    let db = db.clamp(MIN_DB, MAX_DB);
    if db <= ZERO_DB { (db - MIN_DB) / (ZERO_DB - MIN_DB) * ZERO_FRACTION } else { ZERO_FRACTION + db / MAX_DB * (1.0 - ZERO_FRACTION) }
}

/// The inverse of [`fraction`]: the level a point on the travel stands for.
/// Fractions outside 0.0..=1.0 clamp to the ends of the range.
fn db_at(fraction: f32) -> f32 {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= ZERO_FRACTION { MIN_DB + fraction / ZERO_FRACTION * (ZERO_DB - MIN_DB) } else { (fraction - ZERO_FRACTION) / (1.0 - ZERO_FRACTION) * MAX_DB }
}

/// The readout text under the fader, signed to one decimal.
fn readout(db: f32) -> String {
    format!("{db:+.1} dB")
}

/// The fader: a vertical `bg.selected` track with the accent filling from the
/// bottom up to the value, a white knob, and the dB readout under it in
/// `type.value`. Pointer clicks and drags set the value; double-click resets
/// to 0 dB; the Up and Down arrows step 0.5 dB when focused.
///
/// Usage:
///
/// ```ignore
/// let mut gain = 0.0;
/// ui_kit::fader(ui, &mut gain, 96.0);
/// ```
pub fn fader(ui: &mut Ui, db: &mut f32, height: f32) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let galley = ui.painter().layout_no_wrap(readout(*db), fonts::mono(typography::VALUE_SIZE), neutrals.text_primary);
    let width = galley.size().x.max(KNOB_DIAMETER);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::slider(true, *db as f64, ""));
    let track = Rect::from_min_max(
        Pos2::new(rect.center().x - TRACK_WIDTH / 2.0, rect.top()),
        Pos2::new(rect.center().x + TRACK_WIDTH / 2.0, rect.bottom() - galley.size().y - space::SCALE_6),
    );
    let track = Rect::from_min_max(track.min, Pos2::new(track.max.x, track.max.y.max(track.min.y + 1.0)));
    if response.double_clicked() {
        *db = ZERO_DB;
    } else if (response.clicked() || response.dragged())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let fraction = (1.0 - (pos.y - track.top()) / track.height()).clamp(0.0, 1.0);
        *db = db_at(fraction);
    }
    if response.clicked() || response.drag_started() {
        response.request_focus();
    }
    if response.has_focus() {
        let (up, down) = ui.input(|input| (input.key_pressed(Key::ArrowUp), input.key_pressed(Key::ArrowDown)));
        if up {
            *db = (*db + DB_STEP).min(MAX_DB);
        }
        if down {
            *db = (*db - DB_STEP).max(MIN_DB);
        }
    }
    if ui.is_rect_visible(rect) {
        let accent = Theme::get(ui.ctx()).accent.accent;
        let painter = ui.painter();
        painter.rect_filled(track, capsule_radius(TRACK_WIDTH), neutrals.bg_selected);
        let fraction = fraction(*db);
        let knob_y = track.bottom() - track.height() * fraction;
        let fill = Rect::from_min_max(Pos2::new(track.left(), knob_y), track.max);
        painter.rect_filled(fill, capsule_radius(TRACK_WIDTH), accent);
        painter.circle_filled(Pos2::new(track.center().x, knob_y), KNOB_DIAMETER / 2.0, Color32::WHITE);
        let pos = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y);
        painter.galley(pos, galley, neutrals.text_primary);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(size::SEGMENT_HEIGHT));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fader curve pins the ends and puts 0 dB at 75% of the travel.
    #[test]
    fn fraction_maps_the_fader_curve() {
        assert_eq!(fraction(MIN_DB), 0.0);
        assert_eq!(fraction(ZERO_DB), ZERO_FRACTION);
        assert_eq!(fraction(MAX_DB), 1.0);
    }

    /// The pointer mapping inverts the knob mapping, and clamps past the ends.
    #[test]
    fn db_at_inverts_fraction_and_clamps() {
        for db in [-60.0, -45.0, -30.0, -12.0, -6.0, -0.5, 0.0, 3.0, 6.0] {
            let back = db_at(fraction(db));
            assert!((back - db).abs() < 1e-4, "db_at(fraction({db})) must round-trip, got {back}");
        }
        assert_eq!(db_at(-0.5), MIN_DB);
        assert_eq!(db_at(1.5), MAX_DB);
    }

    /// The readout is signed to one decimal.
    #[test]
    fn readout_formats_signed_db() {
        assert_eq!(readout(0.0), "+0.0 dB");
        assert_eq!(readout(-6.25), "-6.2 dB");
        assert_eq!(readout(6.0), "+6.0 dB");
    }

    /// Values the fader is handed stay inside its range on the curve.
    #[test]
    fn fraction_clamps_out_of_range_levels() {
        assert_eq!(fraction(-120.0), 0.0);
        assert_eq!(fraction(24.0), 1.0);
        assert!((MIN_DB..=MAX_DB).contains(&db_at(fraction(-120.0))));
        assert!((MIN_DB..=MAX_DB).contains(&db_at(fraction(24.0))));
    }
}
