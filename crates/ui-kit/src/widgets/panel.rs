//! The panel container (spec "Components": row "Panel").

use egui::{InnerResponse, Margin, Ui};

use crate::HAIRLINE;
use crate::tokens::{color, radius, space};
#[cfg(test)]
use crate::widgets;

/// A panel: `bg.panel` fill, `radius.panel`, a 1px hairline rim and 8px
/// padding, flat (no drop shadow, spec "Elevation and glass").
///
/// Usage:
///
/// ```ignore
/// ui_kit::panel(ui, |ui| {
///     ui.label("Inspector group");
/// });
/// ```
pub fn panel<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    egui::Frame::new()
        .fill(color::BG_PANEL)
        .corner_radius(radius::PANEL)
        .inner_margin(Margin::same(space::PANEL_PADDING as i8))
        .stroke(egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE))
        .show(ui, add_contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel keeps its inner value and returns the frame response.
    #[test]
    fn panel_returns_the_inner_value() {
        let ctx = egui::Context::default();
        let mut value = 0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let inner = panel(ui, |ui| {
                ui.label("x");
                7
            });
            value = inner.inner;
        });
        output.textures_delta.clear();
        assert_eq!(value, 7);
    }

    /// The `widgets` helper module is reachable for the shared radius rule.
    #[test]
    fn capsule_radius_is_half_the_height() {
        let radius = widgets::capsule_radius(26.0);
        assert_eq!(radius.nw, 13);
        assert_eq!(radius.se, 13);
    }
}
