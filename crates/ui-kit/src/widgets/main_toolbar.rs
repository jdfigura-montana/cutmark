//! The main toolbar (spec "Components": row "Main toolbar"; "Layout rules"
//! items 2 and 3). Phase 7 adds the caption buttons and drag areas.

use egui::{Align, CornerRadius, Layout, Rect, Sense, Ui, UiBuilder, vec2};

use crate::tokens::{color, size};

/// The zone rects of a laid-out main toolbar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MainToolbar {
    /// The full 52px strip across the window (the title bar included).
    pub bar: Rect,
    /// The left zone: app mark, document name, Undo, Redo.
    pub left: Rect,
    /// The center zone (the mode switch), centered on the window itself
    /// (spec "Layout rules" item 3).
    pub center: Rect,
    /// The right zone: icon buttons and the primary button (the caption
    /// buttons join in phase 7).
    pub right: Rect,
}

/// The main toolbar: a 52px `bg.ground` strip with a left zone at the left
/// edge, a right zone at the right edge, and the center zone centered on the
/// window, not on the space between the side zones. No window chrome yet.
///
/// The center closure runs twice per frame (once to measure it); keep it to
/// drawing the mode switch.
///
/// Usage:
///
/// ```ignore
/// ui_kit::main_toolbar(ui, |ui| { /* app mark */ }, |ui| { /* modes */ }, |ui| { /* actions */ });
/// ```
pub fn main_toolbar<L, C, R>(ui: &mut Ui, left: L, center: C, right: R) -> MainToolbar
where
    L: FnOnce(&mut Ui),
    C: Fn(&mut Ui),
    R: FnOnce(&mut Ui),
{
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::TOOLBAR_MAIN_HEIGHT), Sense::hover());
    if ui.is_rect_visible(bar) {
        ui.painter().rect_filled(bar, CornerRadius::ZERO, color::BG_GROUND);
    }
    let mut left_ui = ui.new_child(UiBuilder::new().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
    left(&mut left_ui);
    let mut right_ui = ui.new_child(UiBuilder::new().max_rect(bar).layout(Layout::right_to_left(Align::Center)));
    right(&mut right_ui);
    // The center zone is measured in an invisible, disabled pass so it can be
    // centered on the window itself (spec "Layout rules" item 3); egui has no
    // centering layout for a horizontal run. The disabled widgets of the
    // measuring pass take no pointer events, and the real zone is registered
    // after them, on top.
    let mut measure_ui = ui.new_child(UiBuilder::new().invisible().disabled().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
    center(&mut measure_ui);
    let width = measure_ui.min_rect().width().min(bar.width());
    let center_rect = Rect::from_center_size(bar.center(), vec2(width, bar.height()));
    let mut center_ui = ui.new_child(UiBuilder::new().max_rect(center_rect).layout(Layout::left_to_right(Align::Center)));
    center(&mut center_ui);
    MainToolbar { bar, left: left_ui.min_rect(), center: center_ui.min_rect(), right: right_ui.min_rect() }
}
