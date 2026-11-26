//! The panel container (spec "Components": row "Panel").

use egui::{InnerResponse, Margin, Ui};

use crate::HAIRLINE;
use crate::Theme;
use crate::tokens::{radius, space};
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
    let neutrals = Theme::get(ui.ctx()).neutrals;
    egui::Frame::new()
        .fill(neutrals.bg_panel)
        .corner_radius(radius::PANEL)
        .inner_margin(Margin::same(space::PANEL_PADDING as i8))
        .stroke(egui::Stroke::new(HAIRLINE, neutrals.stroke_hairline))
        .show(ui, add_contents)
}

/// A side panel: `bg.side` fill, `radius.side`, no rim, and the same 8px
/// padding as [`panel`] (spec: E3 amendment, "4. No generic tells").
///
/// Usage:
///
/// ```ignore
/// ui_kit::side_panel(ui, |ui| {
///     ui.label("Library");
/// });
/// ```
pub fn side_panel<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    egui::Frame::new().fill(neutrals.bg_side).corner_radius(radius::SIDE).inner_margin(Margin::same(space::PANEL_PADDING as i8)).show(ui, add_contents)
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
