//! The shared keyboard-focus ring (spec "States": focus).

use egui::{CornerRadius, Rect, Stroke, StrokeKind, Ui};

use crate::Theme;

/// The ring stroke width (spec "States": 2px `accent` ring).
const RING_STROKE: f32 = 2.0;
/// The gap between the widget and the ring (spec "States": 1px offset).
const RING_OFFSET: f32 = 1.0;

/// Draws the 2px accent focus ring 1px outside `rect`.
///
/// Callers gate this on the widget response's keyboard focus, for example:
///
/// ```ignore
/// if response.has_focus() {
///     ui_kit::focus_ring(ui, response.rect, egui::CornerRadius::default());
/// }
/// ```
pub fn focus_ring(ui: &Ui, rect: Rect, radius: CornerRadius) {
    let accent = Theme::get(ui.ctx()).accent.accent;
    ui.painter().rect_stroke(rect.expand(RING_OFFSET), radius + RING_OFFSET as u8, Stroke::new(RING_STROKE, accent), StrokeKind::Outside);
}
