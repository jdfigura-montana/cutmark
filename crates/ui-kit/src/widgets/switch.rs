//! The switch (spec "Components": row "Switch").

use egui::{Color32, Context, Id, Pos2, Rect, Response, Sense, Ui, WidgetInfo, WidgetType, vec2};

use crate::Theme;
use crate::tokens::{color, motion, size, space};
use crate::widgets::{capsule_radius, focus};

/// The accessible name of the switch (it has no visible label).
const ACCESSIBLE_NAME: &str = "Switch";
/// The knob color (spec "Switch": white knob).
const KNOB_COLOR: Color32 = Color32::WHITE;

/// The switch: a 40 x 24 capsule, off `bg.selected`, on `accent`, with a
/// white knob that slides over 120ms (instant when the theme asks for
/// reduced motion). The response reports `changed()` on the frame a click
/// or key press flips the value.
///
/// Usage:
///
/// ```ignore
/// let mut solo = false;
/// ui_kit::switch(ui, &mut solo);
/// ```
pub fn switch(ui: &mut Ui, on: &mut bool) -> Response {
    let width = size::SWITCH_WIDTH;
    let height = size::SWITCH_HEIGHT;
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, *on, ACCESSIBLE_NAME));
    // `clicked` also covers Space and Enter on the focused switch.
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let theme = Theme::get(ui.ctx());
    let inset = space::SCALE_2;
    let knob_diameter = height - 2.0 * inset;
    let travel = width - 2.0 * inset - knob_diameter;
    if ui.is_rect_visible(rect) {
        let fill = if *on { theme.accent.accent } else { color::BG_SELECTED };
        ui.painter().rect_filled(rect, capsule_radius(height), fill);
        let knob_x = knob_offset(ui.ctx(), response.id, *on, theme.reduce_motion, travel);
        let knob_rect = Rect::from_min_size(Pos2::new(rect.left() + inset + knob_x, rect.top() + inset), vec2(knob_diameter, knob_diameter));
        ui.painter().circle_filled(knob_rect.center(), knob_diameter / 2.0, KNOB_COLOR);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(height));
    }
    response
}

/// The knob's x offset from its off position, animated over the hover-fill
/// motion token (spec "Motion") or instant when reduce motion is on.
pub(crate) fn knob_offset(ctx: &Context, id: Id, on: bool, reduce_motion: bool, travel: f32) -> f32 {
    let target = if on { travel } else { 0.0 };
    let duration = if reduce_motion { 0.0 } else { motion::HOVER_FILL_MS as f32 / 1000.0 };
    ctx.animate_value_with_time(id, target, duration)
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// A click flips the value, and the response reports `changed()` on
    /// that one frame only.
    #[test]
    fn clicking_the_switch_flips_the_value_and_reports_changed() {
        // The state is (value, frames that reported `changed`).
        let mut harness = Harness::new_ui_state(
            |ui, (on, changed_frames): &mut (bool, u32)| {
                if switch(ui, on).changed() {
                    *changed_frames += 1;
                }
            },
            (false, 0),
        );
        harness.run();
        assert_eq!(*harness.state(), (false, 0), "an untouched switch must not report a change");
        harness.get_by_label(ACCESSIBLE_NAME).click();
        harness.run();
        assert_eq!(*harness.state(), (true, 1), "the click must flip the value and report changed once");
    }

    /// With reduce motion the knob reaches the end position in one frame;
    /// with full motion it is still mid-travel there.
    #[test]
    fn the_knob_jumps_in_one_frame_when_reduce_motion_is_on() {
        let ctx = Context::default();
        let id = Id::new("knob-test");
        let travel = 16.0;
        let mut time = 0.0f64;
        let mut sample = |on: bool, reduce_motion: bool| {
            time += 0.05;
            step(&ctx, time, on, reduce_motion, id, travel)
        };
        sample(false, false);
        sample(false, false); // the knob is registered at the off position
        assert!(sample(true, false) < travel, "the animated knob must still be mid-travel one frame after the toggle");
        sample(false, false);
        assert_eq!(sample(true, true), travel, "with reduce motion the knob must reach the end in one frame");
    }

    /// Runs one pass, dropping the font texture deltas the default egui
    /// context produces (no renderer consumes them in a plain test).
    fn step(ctx: &Context, time: f64, on: bool, reduce_motion: bool, id: Id, travel: f32) -> f32 {
        let mut offset = 0.0f32;
        let raw = egui::RawInput { time: Some(time), ..Default::default() };
        let mut output = ctx.run_ui(raw, |_| {
            offset = knob_offset(ctx, id, on, reduce_motion, travel);
        });
        output.textures_delta.clear();
        offset
    }
}
