//! ui::widgets::fader: linear slider for dB and pan, with gesture events.

use eframe::egui::{self, Pos2, Rect};

use crate::ui::theme::THEME;

/// Outcome of one frame of the slider widget.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SliderOutcome {
    /// Drag just started: the caller begins a gesture.
    pub started: bool,
    /// New value while dragging, mapped linearly over the range.
    pub value: Option<f32>,
    /// Drag released: the caller ends the gesture.
    pub ended: bool,
    /// Esc pressed during the drag: the caller cancels the gesture.
    pub cancelled: bool,
    /// Double-clicked: the caller resets to the default value.
    pub reset: bool,
}

/// Formats a dB slider value, e.g. "-3.5 dB"; values below -60 read "-60.0 dB".
pub fn db_text(db: f32) -> String {
    format!("{:.1} dB", db.max(-60.0))
}

/// Formats a pan value: "L50", "C" or "R30".
pub fn pan_text(pan: f32) -> String {
    let amount = (pan.abs() * 100.0).round() as i32;
    if amount < 5 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{amount}")
    } else {
        format!("R{amount}")
    }
}

/// Draws a bare horizontal custom slider in `rect`: a thin track, a filled
/// portion and a round thumb, with no value text (the caller shows the value).
///
/// The track is inset by the thumb radius at both ends so the thumb stays
/// fully visible at min and max. The value maps linearly over `min..max`.
/// When `fill_from_center`, the fill grows from the midpoint (used for pan).
/// Double-click reports `reset`; Esc during a drag reports `cancelled`.
pub fn slider_bare(ui: &mut egui::Ui, id: egui::Id, rect: Rect, value: f32, range: (f32, f32), fill_from_center: bool) -> SliderOutcome {
    let (min, max) = range;
    let resp = ui.interact(rect, id, egui::Sense::click_and_drag());

    let mut out = SliderOutcome::default();
    let dragging = resp.dragged();
    if resp.drag_started() && !resp.double_clicked() {
        out.started = true;
    }
    if resp.double_clicked() {
        out.reset = true;
    }
    if resp.drag_stopped() {
        out.ended = true;
    }
    if dragging && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        out.cancelled = true;
    }

    // Track travel: inset by the thumb radius at both ends.
    let thumb_r = THEME.track_slider_thumb * 0.5;
    let track_left = rect.left() + thumb_r;
    let track_right = rect.right() - thumb_r;
    let travel = (track_right - track_left).max(1.0);

    if dragging && let Some(p) = resp.interact_pointer_pos() {
        let t = (p.x - track_left) / travel;
        out.value = Some(min + t.clamp(0.0, 1.0) * (max - min));
    }

    let shown = out.value.unwrap_or(value);
    let t = ((shown - min) / (max - min)).clamp(0.0, 1.0);
    let painter = ui.painter_at(rect);
    let thumb_x = track_left + t * travel;
    let cy = rect.center().y;
    let half_track = THEME.track_slider_track_w * 0.5;

    // Track: full width for volume, from the center for pan.
    let fill_left = if fill_from_center { rect.center().x.min(thumb_x) } else { track_left };
    painter.rect_filled(Rect::from_min_max(Pos2::new(fill_left, cy - half_track), Pos2::new(thumb_x, cy + half_track)), half_track, THEME.slider_fill);
    // Unfilled remainder of the track.
    let rest_left = if fill_from_center { rect.center().x.max(thumb_x) } else { thumb_x };
    painter.rect_filled(Rect::from_min_max(Pos2::new(rest_left, cy - half_track), Pos2::new(track_right, cy + half_track)), half_track, THEME.slider_track);
    // Center tick for bipolar sliders.
    if fill_from_center {
        painter.rect_filled(
            Rect::from_min_max(Pos2::new(rect.center().x - 0.5, cy - 5.0), Pos2::new(rect.center().x + 0.5, cy + 5.0)),
            0.0,
            THEME.control_border,
        );
    }
    // Round thumb on top.
    painter.circle_filled(Pos2::new(thumb_x, cy), thumb_r, THEME.slider_thumb);

    out
}
