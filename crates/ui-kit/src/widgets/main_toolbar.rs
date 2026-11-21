//! The main toolbar (spec "Components": row "Main toolbar"; "Layout rules"
//! items 2 and 3; "Window chrome"). The toolbar is the title bar: it appends
//! the caption buttons and publishes the chrome regions every frame, so any
//! app gets a working title bar from this call alone.

use egui::{Align, CornerRadius, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};

use crate::chrome::{self, Regions};
use crate::tokens::{color, size};
use crate::widgets::caption_buttons::{self, CaptionRects};

/// The zone rects of a laid-out main toolbar, plus the chrome regions it
/// published for the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MainToolbar {
    /// The full 52px strip across the window (the title bar included).
    pub bar: Rect,
    /// The left zone: app mark, document name, Undo, Redo.
    pub left: Rect,
    /// The center zone (the mode switch), centered on the window itself
    /// (spec "Layout rules" item 3).
    pub center: Rect,
    /// The right zone: the app content (icon buttons, then the primary
    /// button). The caption buttons sit between it and the window edge;
    /// nothing is drawn to the right of the close button.
    pub right: Rect,
    /// The minimize button rect, when the custom chrome drew the buttons.
    pub minimize: Option<Rect>,
    /// The maximize or restore button rect, when the custom chrome drew it.
    pub maximize: Option<Rect>,
    /// The close button rect, when the custom chrome drew the buttons.
    pub close: Option<Rect>,
    /// The caption (drag) rectangles published to the chrome hit test this
    /// frame: the bar minus every interactive widget rect.
    pub caption: Vec<Rect>,
}

/// The main toolbar: a 52px `bg.ground` strip with a left zone at the left
/// edge, a right zone at the right edge, and the center zone centered on the
/// window, not on the space between the side zones. Every zone is vertically
/// centered in the bar, so the left zone's two-line block (type.headline over
/// type.caption) is never clipped. In the right zone the app content (icon
/// buttons, then the primary button) reads left to right and the caption
/// buttons join at the far right, so nothing is ever drawn to the right of
/// the close button; the chrome regions (caption drag areas, maximize
/// button) are published every frame, so the toolbar is the title bar
/// (spec "Window chrome").
///
/// When the window is not focused, toolbar text drops to `text.tertiary`;
/// the primary button keeps its color, because it paints its own accent text.
///
/// The left, center and right closures run twice per frame (once in an
/// invisible sizing pass to measure them); keep them to drawing their
/// widgets.
///
/// Usage:
///
/// ```ignore
/// ui_kit::main_toolbar(ui, |ui| { /* app mark */ }, |ui| { /* modes */ }, |ui| { /* actions */ });
/// ```
pub fn main_toolbar<L, C, R>(ui: &mut Ui, left: L, center: C, right: R) -> MainToolbar
where
    L: Fn(&mut Ui),
    C: Fn(&mut Ui),
    R: Fn(&mut Ui),
{
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::TOOLBAR_MAIN_HEIGHT), Sense::hover());
    if ui.is_rect_visible(bar) {
        ui.painter().rect_filled(bar, CornerRadius::ZERO, color::BG_GROUND);
    }
    // Inactive window: toolbar text drops to text.tertiary (spec "Window
    // chrome", "Both"). The primary button keeps its color: it lays its own
    // accent-colored galley, which an override does not touch.
    let unfocused = ui.ctx().input(|i| i.viewport().focused) == Some(false);
    // The left zone is measured in an invisible sizing pass under its own id
    // scope (the measuring run creates no live widget ids), then drawn in a
    // bar-centered slice, so the document name's two-line block is vertically
    // centered in the 52px bar and never clipped.
    let left_height = ui
        .push_id("measure-left", |ui| {
            let mut measure_ui = ui.new_child(UiBuilder::new().sizing_pass().invisible().max_rect(bar).layout(Layout::left_to_right(Align::Min)));
            left(&mut measure_ui);
            measure_ui.min_rect().height()
        })
        .inner
        .min(bar.height());
    let left_rect = Rect::from_min_max(pos2(bar.left(), bar.center().y - left_height / 2.0), pos2(bar.right(), bar.center().y + left_height / 2.0));
    let mut left_ui = ui.new_child(UiBuilder::new().max_rect(left_rect).layout(Layout::left_to_right(Align::Center)));
    if unfocused {
        left_ui.style_mut().visuals.override_text_color = Some(color::TEXT_TERTIARY);
    }
    left(&mut left_ui);
    // The caption buttons join at the far right, only with the custom chrome
    // (spec "Window chrome": the window has no native title bar). They are
    // drawn first in a right-to-left child, so the close button is the
    // rightmost thing in the window.
    let mut buttons_ui = ui.new_child(UiBuilder::new().max_rect(bar).layout(Layout::right_to_left(Align::Center)));
    let buttons = if chrome::is_custom() { caption_buttons::caption_buttons(&mut buttons_ui) } else { None };
    let buttons_width = if buttons.is_some() { 3.0 * size::CAPTION_BUTTON_WIDTH } else { 0.0 };
    // The right zone is measured in an invisible sizing pass under its own id
    // scope, so the measuring run creates no live widget ids (egui reports
    // reused ids as red overlays), then drawn left to right so the content
    // reads left to right and ends where the caption buttons begin.
    let content_width = ui
        .push_id("measure-right", |ui| {
            let mut measure_ui = ui.new_child(UiBuilder::new().sizing_pass().invisible().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
            right(&mut measure_ui);
            measure_ui.min_rect().width()
        })
        .inner
        .min(bar.width() - buttons_width);
    let content_rect = Rect::from_min_max(pos2(bar.right() - buttons_width - content_width, bar.top()), pos2(bar.right() - buttons_width, bar.bottom()));
    let mut right_ui = ui.new_child(UiBuilder::new().max_rect(content_rect).layout(Layout::left_to_right(Align::Center)));
    if unfocused {
        right_ui.style_mut().visuals.override_text_color = Some(color::TEXT_TERTIARY);
    }
    right(&mut right_ui);
    let right_content = right_ui.min_rect();
    // The center zone is measured in an invisible sizing pass under its own
    // id scope so it can be centered on the window itself (spec "Layout
    // rules" item 3); egui has no centering layout for a horizontal run. The
    // measuring run creates no live widget ids; the real zone is registered
    // after the side zones, on top.
    let measured_width = ui
        .push_id("measure", |ui| {
            let mut measure_ui = ui.new_child(UiBuilder::new().sizing_pass().invisible().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
            center(&mut measure_ui);
            measure_ui.min_rect().width()
        })
        .inner;
    let width = measured_width.min(bar.width());
    let center_rect = Rect::from_center_size(bar.center(), vec2(width, bar.height()));
    let mut center_ui = ui.new_child(UiBuilder::new().max_rect(center_rect).layout(Layout::left_to_right(Align::Center)));
    if unfocused {
        center_ui.style_mut().visuals.override_text_color = Some(color::TEXT_TERTIARY);
    }
    center(&mut center_ui);

    // Chrome regions: the whole bar is caption except every interactive
    // widget rect (spec "Window chrome": empty toolbar space drags,
    // interactive toolbar elements never drag). The maximize button is also
    // carved out and published on its own, for Snap Layouts.
    let (minimize, maximize, close) = match buttons {
        Some(CaptionRects { minimize, maximize, close }) => (Some(minimize), Some(maximize), Some(close)),
        None => (None, None, None),
    };
    let mut holes = vec![left_ui.min_rect(), center_ui.min_rect(), right_content];
    holes.extend(minimize);
    holes.extend(close);
    holes.extend(maximize);
    let caption = caption_rects(bar, &holes);
    let mut regions = Regions::new(caption.clone());
    if let Some(maximize) = maximize {
        regions = regions.with_maximize_button(maximize);
    }
    chrome::publish(ui.ctx(), &regions);
    MainToolbar { bar, left: left_ui.min_rect(), center: center_ui.min_rect(), right: right_ui.min_rect(), minimize, maximize, close, caption }
}

/// Subtracts the interactive widget rects from the bar and returns the
/// remaining caption rectangles.
fn caption_rects(bar: Rect, holes: &[Rect]) -> Vec<Rect> {
    let mut rects = vec![bar];
    for &hole in holes {
        let hole = hole.intersect(bar);
        if !hole.is_positive() {
            continue;
        }
        let mut next = Vec::new();
        for rect in rects {
            next.extend(subtract(rect, hole));
        }
        rects = next;
    }
    rects.retain(Rect::is_positive);
    rects
}

/// Returns the parts of `rect` outside `hole`, up to four rectangles.
fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
    if !rect.intersects(hole) {
        return vec![rect];
    }
    // The hole may reach past the rect on any side; the strips stay inside
    // the rect.
    let top = hole.top().max(rect.top());
    let bottom = hole.bottom().min(rect.bottom());
    let left = hole.left().max(rect.left());
    let right = hole.right().min(rect.right());
    let mut parts = Vec::new();
    if top > rect.top() {
        parts.push(Rect::from_min_max(pos2(rect.left(), rect.top()), pos2(rect.right(), top)));
    }
    if bottom < rect.bottom() {
        parts.push(Rect::from_min_max(pos2(rect.left(), bottom), pos2(rect.right(), rect.bottom())));
    }
    if left > rect.left() {
        parts.push(Rect::from_min_max(pos2(rect.left(), top), pos2(left, bottom)));
    }
    if right < rect.right() {
        parts.push(Rect::from_min_max(pos2(right, top), pos2(rect.right(), bottom)));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The carved caption rects leave exactly the bar minus the holes: their
    /// total area matches, none overlaps a hole and the gaps still drag.
    #[test]
    fn caption_rects_carve_around_the_holes() {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 52.0));
        let holes = [
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(120.0, 52.0)),       // left zone
            Rect::from_min_size(egui::pos2(200.0, 12.0), egui::vec2(80.0, 28.0)), // center zone
            Rect::from_min_size(egui::pos2(400.0, 8.0), egui::vec2(240.0, 36.0)), // right content
            Rect::from_min_size(egui::pos2(662.0, 0.0), egui::vec2(46.0, 52.0)),  // minimize
            Rect::from_min_size(egui::pos2(708.0, 0.0), egui::vec2(46.0, 52.0)),  // maximize
            Rect::from_min_size(egui::pos2(754.0, 0.0), egui::vec2(46.0, 52.0)),  // close
        ];
        let caption = caption_rects(bar, &holes);
        assert!(!caption.is_empty(), "the bar must keep caption space");
        let area: f32 = caption.iter().map(Rect::area).sum();
        let hole_area: f32 = holes.iter().map(|h| h.intersect(bar).area()).sum();
        assert!((area - (bar.area() - hole_area)).abs() < 1.0, "carved area {area} must equal the bar minus the holes {}", bar.area() - hole_area);
        for rect in &caption {
            for hole in &holes {
                assert!(!rect.intersect(*hole).is_positive(), "{rect:?} overlaps the interactive rect {hole:?}");
            }
        }
        let gap = egui::pos2(330.0, 26.0);
        assert!(caption.iter().any(|r| r.contains(gap)), "empty space between the zones must stay caption");
    }

    /// Holes outside the bar are ignored: the bar stays whole.
    #[test]
    fn caption_rects_ignore_holes_outside_the_bar() {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 52.0));
        let holes = [Rect::from_min_size(egui::pos2(100.0, 60.0), egui::vec2(50.0, 20.0))];
        let caption = caption_rects(bar, &holes);
        assert_eq!(caption, vec![bar], "a hole outside the bar must leave the bar whole");
    }
}
