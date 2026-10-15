//! ui::widgets::fader: linear slider for dB and pan, with gesture events.

use eframe::egui::{self, Pos2, Rect, Stroke, Vec2};

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
/// The value maps linearly over `min..max`. When `fill_from_center`, the fill
/// grows from the midpoint (used for pan). Double-click reports `reset`; Esc
/// during a drag reports `cancelled`.
pub fn slider_bare(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    value: f32,
    range: (f32, f32),
    fill_from_center: bool,
) -> SliderOutcome {
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
    if dragging && let Some(p) = resp.interact_pointer_pos() {
        let t = (p.x - rect.left()) / rect.width();
        out.value = Some(min + t.clamp(0.0, 1.0) * (max - min));
    }

    let shown = out.value.unwrap_or(value);
    let t = ((shown - min) / (max - min)).clamp(0.0, 1.0);
    let painter = ui.painter_at(rect);
    let thumb_x = rect.left() + t * rect.width();
    let cy = rect.center().y;
    let half_track = THEME.track_slider_track_w * 0.5;

    // Track: full width for volume, from the center for pan.
    let track_left = if fill_from_center {
        rect.center().x.min(thumb_x)
    } else {
        rect.left()
    };
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(track_left, cy - half_track),
            Pos2::new(thumb_x, cy + half_track),
        ),
        half_track,
        THEME.slider_fill,
    );
    // Unfilled remainder of the track.
    let rest_left = if fill_from_center {
        rect.center().x.max(thumb_x)
    } else {
        thumb_x
    };
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(rest_left, cy - half_track),
            Pos2::new(rect.right(), cy + half_track),
        ),
        half_track,
        THEME.slider_track,
    );
    // Center tick for bipolar sliders.
    if fill_from_center {
        painter.rect_filled(
            Rect::from_min_max(
                Pos2::new(rect.center().x - 0.5, cy - 5.0),
                Pos2::new(rect.center().x + 0.5, cy + 5.0),
            ),
            0.0,
            THEME.control_border,
        );
    }
    // Round thumb on top.
    let r = THEME.track_slider_thumb * 0.5;
    painter.circle_filled(Pos2::new(thumb_x, cy), r, THEME.slider_thumb);

    out
}

/// Draws a bare vertical fader in `rect`: a thin centered track and a wide,
/// short cap with a center line, no value text (the caller shows the value).
///
/// The value maps linearly over `min..max`, top = max. Double-click reports
/// `reset`; Esc during a drag reports `cancelled`.
pub fn fader_bare(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    value: f32,
    range: (f32, f32),
) -> SliderOutcome {
    let (min, max) = range;
    let painter = ui.painter_at(rect);
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

    // New value from the pointer position along the fader height.
    if dragging && let Some(p) = resp.interact_pointer_pos() {
        let t = (p.y - rect.top()) / rect.height();
        out.value = Some(max - t.clamp(0.0, 1.0) * (max - min));
    }

    let shown = out.value.unwrap_or(value);
    let t = ((max - shown) / (max - min)).clamp(0.0, 1.0);
    let cx = rect.center().x;
    let half_track = THEME.track_slider_track_w * 0.5;

    // Track: a thin line down the center of the fader.
    painter.rect_filled(
        Rect::from_min_max(
            Pos2::new(cx - half_track, rect.top()),
            Pos2::new(cx + half_track, rect.bottom()),
        ),
        half_track,
        THEME.slider_track,
    );

    // Cap: stays fully inside the rect as it travels.
    let cap_h = 12.0;
    let cap_y = rect.top() + t * (rect.height() - cap_h) + cap_h * 0.5;
    let cap = Rect::from_center_size(Pos2::new(cx, cap_y), Vec2::new(rect.width(), cap_h));
    // Small shadow under the cap.
    painter.rect_filled(
        cap.translate(egui::vec2(0.0, 1.0)),
        THEME.corner_r,
        egui::Color32::from_black_alpha(60),
    );
    painter.rect_filled(cap, THEME.corner_r, THEME.slider_thumb);
    // 1 px center line across the cap.
    painter.line_segment(
        [
            Pos2::new(cap.left() + 3.0, cap.center().y),
            Pos2::new(cap.right() - 3.0, cap.center().y),
        ],
        Stroke::new(THEME.hairline, THEME.slider_fill),
    );

    if out.reset {
        return SliderOutcome {
            started: false,
            value: None,
            ended: false,
            cancelled: false,
            reset: true,
        };
    }
    out
}
