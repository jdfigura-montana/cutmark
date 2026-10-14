//! ui::widgets::fader: linear slider for dB and pan, with gesture events.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};

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

/// Draws a linear slider in `rect` (vertical when `vertical`, else horizontal).
///
/// The value maps linearly over `min..max` along the drag axis. Shows the
/// value text next to the handle. Double-click reports `reset`; Esc during a
/// drag reports `cancelled`.
pub fn slider(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    value: f32,
    range: (f32, f32),
    vertical: bool,
    fmt: impl Fn(f32) -> String,
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

    // New value from the pointer position along the drag axis.
    if dragging && let Some(p) = resp.interact_pointer_pos() {
        let t = if vertical {
            (p.y - rect.top()) / rect.height()
        } else {
            (p.x - rect.left()) / rect.width()
        };
        out.value = Some(min + t.clamp(0.0, 1.0) * (max - min));
    }

    let shown = out.value.unwrap_or(value);
    let t = ((shown - min) / (max - min)).clamp(0.0, 1.0);
    let visuals = ui.visuals().clone();
    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);

    // Slot along the drag axis and the handle across it.
    let handle_pos = if vertical {
        Pos2::new(rect.center().x, rect.top() + t * rect.height())
    } else {
        Pos2::new(rect.left() + t * rect.width(), rect.center().y)
    };
    let slot = Stroke::new(2.0, visuals.widgets.inactive.bg_stroke.color);
    if vertical {
        painter.line_segment(
            [
                Pos2::new(rect.center().x, rect.top() + 3.0),
                Pos2::new(rect.center().x, rect.bottom() - 3.0),
            ],
            slot,
        );
        painter.rect_filled(
            Rect::from_center_size(handle_pos, Vec2::new(rect.width() - 4.0, 4.0)),
            1.0,
            visuals.strong_text_color(),
        );
    } else {
        painter.line_segment(
            [
                Pos2::new(rect.left() + 3.0, rect.center().y),
                Pos2::new(rect.right() - 3.0, rect.center().y),
            ],
            slot,
        );
        painter.rect_filled(
            Rect::from_center_size(handle_pos, Vec2::new(4.0, rect.height() - 4.0)),
            1.0,
            visuals.strong_text_color(),
        );
    }

    // Value text next to the handle.
    let text_pos = if vertical {
        Pos2::new(rect.left() + 2.0, handle_pos.y - 7.0)
    } else {
        Pos2::new(handle_pos.x, rect.top() + 1.0)
    };
    let anchor = if vertical {
        egui::Align2::LEFT_BOTTOM
    } else {
        egui::Align2::CENTER_TOP
    };
    painter.text(
        text_pos,
        anchor,
        fmt(shown),
        egui::FontId::proportional(9.0),
        Color32::from_rgb(170, 175, 180),
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
