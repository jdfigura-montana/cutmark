//! The main toolbar, which is the title bar (spec: Design Language rev 3
//! "Title bar"). The toolbar appends the caption buttons and publishes the
//! chrome regions every frame, so any app gets a working title bar from this
//! call alone.

use egui::{Align, CornerRadius, Layout, Rect, Sense, Ui, UiBuilder, pos2, vec2};

use crate::HAIRLINE;
use crate::chrome::{self, Regions};
use crate::tokens::{color, size, space};
use crate::widgets::caption_buttons::{self, CaptionRects};

/// Padding before the left zone's first element (the app mark).
const LEFT_PADDING: f32 = space::SCALE_12;
/// Gap between the app actions and the separator before the caption buttons
/// (spec rev 3: 12).
const ACTIONS_GAP: f32 = space::SCALE_12;
/// Height of the separator before the caption buttons (spec rev 3: 1 x 20).
const SEPARATOR_HEIGHT: f32 = 20.0;
/// Free space kept between the center zone and the zones beside it.
const CENTER_CLEARANCE: f32 = space::SCALE_12;

/// The zone rects of a laid-out main toolbar, plus the chrome regions it
/// published for the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct MainToolbar {
    /// The full 44px strip across the window (the title bar included).
    pub bar: Rect,
    /// The left zone: app mark, then the optional menu bar.
    pub left: Rect,
    /// The center zone (the title block), centered on the window itself
    /// (spec "Layout rules" item 3).
    pub center: Rect,
    /// The right zone: the app actions. The separator and the caption buttons
    /// sit between it and the window edge; nothing is drawn to the right of
    /// the close button.
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

/// The main toolbar: a 44px `bg.ground` title bar with a hairline along its
/// bottom edge. The left zone starts at the left edge (app mark, then an
/// optional [`crate::widgets::menu_bar`]), the right zone ends at the caption
/// buttons, and the center zone (usually a [`crate::widgets::title_block`]) is
/// centered on the window, not on the space between the side zones. In the
/// right zone the app actions read left to right, then come a 12px gap, a 1 x
/// 20 `stroke.separator.strong` line and the caption buttons, so nothing is
/// ever drawn to the right of the close button. The chrome regions (caption
/// drag areas, maximize button) are published every frame, with every
/// interactive rect (menu titles and all buttons) carved out, so the toolbar is
/// the title bar (spec "Window chrome").
///
/// When the window is not focused, toolbar text drops to `text.tertiary`;
/// the primary button keeps its color, because it paints its own accent text.
///
/// The center and right closures run twice per frame (once in an invisible
/// sizing pass to measure them); keep them to drawing their widgets. The left
/// closure runs once.
///
/// Usage:
///
/// ```ignore
/// ui_kit::main_toolbar(
///     ui,
///     |ui| { /* app mark, then ui_kit::menu_bar */ },
///     |ui| {
///         ui_kit::title_block(ui, &ui_kit::Title::App("Cutmark Audio"));
///     },
///     |ui| { /* app actions */ },
/// );
/// ```
pub fn main_toolbar<L, C, R>(ui: &mut Ui, left: L, center: C, right: R) -> MainToolbar
where
    L: Fn(&mut Ui),
    C: Fn(&mut Ui),
    R: Fn(&mut Ui),
{
    toolbar(ui, chrome::is_custom(), left, center, right)
}

/// [`main_toolbar`] with the custom-chrome switch passed in, so the tests can
/// lay out the caption buttons without hooking a real window.
fn toolbar<L, C, R>(ui: &mut Ui, custom: bool, left: L, center: C, right: R) -> MainToolbar
where
    L: Fn(&mut Ui),
    C: Fn(&mut Ui),
    R: Fn(&mut Ui),
{
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), size::TOOLBAR_MAIN_HEIGHT), Sense::hover());
    if ui.is_rect_visible(bar) {
        ui.painter().rect_filled(bar, CornerRadius::ZERO, color::BG_GROUND);
        let hairline = Rect::from_min_max(pos2(bar.left(), bar.bottom() - HAIRLINE), bar.right_bottom());
        ui.painter().rect_filled(hairline, CornerRadius::ZERO, color::STROKE_HAIRLINE);
    }
    // Inactive window: toolbar text drops to text.tertiary (spec "Window
    // chrome", "Both"). The primary button keeps its color: it lays its own
    // accent-colored galley, which an override does not touch.
    let unfocused = ui.ctx().input(|i| i.viewport().focused) == Some(false);
    // The left zone runs once: it is laid out in the bar's full height with
    // its content centered, so the menu bar owns its state and popovers.
    let left_rect = Rect::from_min_max(pos2(bar.left() + LEFT_PADDING, bar.top()), bar.right_bottom());
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
    // The three buttons sit flush, 46px each, so their hover fills touch.
    buttons_ui.spacing_mut().item_spacing.x = 0.0;
    let buttons = if custom { caption_buttons::caption_buttons(&mut buttons_ui) } else { None };
    let buttons_width = if buttons.is_some() { 3.0 * size::CAPTION_BUTTON_WIDTH } else { 0.0 };
    // The separator sits against the minimize button; the app actions end a
    // 12px gap before it.
    let mut actions_right = bar.right() - buttons_width;
    if buttons.is_some() {
        let separator_left = actions_right - HAIRLINE;
        let separator = Rect::from_center_size(pos2(separator_left + HAIRLINE / 2.0, bar.center().y), vec2(HAIRLINE, SEPARATOR_HEIGHT));
        if ui.is_rect_visible(separator) {
            ui.painter().rect_filled(separator, CornerRadius::ZERO, color::STROKE_SEPARATOR_STRONG);
        }
        actions_right = separator_left - ACTIONS_GAP;
    }
    // The right zone is measured in an invisible sizing pass under its own id
    // scope, so the measuring run creates no live widget ids (egui reports
    // reused ids as red overlays), then drawn left to right so the content
    // reads left to right and ends where the separator begins.
    let content_width = ui
        .push_id("measure-right", |ui| {
            let mut measure_ui = ui.new_child(UiBuilder::new().sizing_pass().invisible().max_rect(bar).layout(Layout::left_to_right(Align::Center)));
            right(&mut measure_ui);
            measure_ui.min_rect().width()
        })
        .inner
        .min(actions_right - bar.left());
    let content_rect = Rect::from_min_max(pos2(actions_right - content_width, bar.top()), pos2(actions_right, bar.bottom()));
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
    // after the side zones, on top. It never gets wider than the room both
    // side zones leave around the window's midpoint.
    let room = (bar.center().x - left_ui.min_rect().right()).min(right_content.left() - bar.center().x) - CENTER_CLEARANCE;
    let max_width = (2.0 * room).clamp(0.0, bar.width());
    let measure_rect = Rect::from_center_size(bar.center(), vec2(max_width, bar.height()));
    let measured_width = ui
        .push_id("measure", |ui| {
            let mut measure_ui = ui.new_child(UiBuilder::new().sizing_pass().invisible().max_rect(measure_rect).layout(Layout::left_to_right(Align::Center)));
            center(&mut measure_ui);
            measure_ui.min_rect().width()
        })
        .inner;
    let width = measured_width.min(max_width);
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
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use egui_kittest::Harness;

    /// The carved caption rects leave exactly the bar minus the holes: their
    /// total area matches, none overlaps a hole and the gaps still drag.
    #[test]
    fn caption_rects_carve_around_the_holes() {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 44.0));
        let holes = [
            Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(120.0, 44.0)),       // left zone
            Rect::from_min_size(egui::pos2(200.0, 8.0), egui::vec2(80.0, 28.0)),  // center zone
            Rect::from_min_size(egui::pos2(400.0, 6.0), egui::vec2(240.0, 32.0)), // right content
            Rect::from_min_size(egui::pos2(662.0, 0.0), egui::vec2(46.0, 44.0)),  // minimize
            Rect::from_min_size(egui::pos2(708.0, 0.0), egui::vec2(46.0, 44.0)),  // maximize
            Rect::from_min_size(egui::pos2(754.0, 0.0), egui::vec2(46.0, 44.0)),  // close
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
        let gap = egui::pos2(330.0, 22.0);
        assert!(caption.iter().any(|r| r.contains(gap)), "empty space between the zones must stay caption");
    }

    /// Holes outside the bar are ignored: the bar stays whole.
    #[test]
    fn caption_rects_ignore_holes_outside_the_bar() {
        let bar = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 44.0));
        let holes = [Rect::from_min_size(egui::pos2(100.0, 60.0), egui::vec2(50.0, 20.0))];
        let caption = caption_rects(bar, &holes);
        assert_eq!(caption, vec![bar], "a hole outside the bar must leave the bar whole");
    }

    /// Lays out one toolbar with the custom chrome switched on (the caption
    /// buttons drawn) and returns its zones plus every painted shape's right
    /// edge.
    fn custom_chrome_toolbar() -> (MainToolbar, f32) {
        let zones = Rc::new(RefCell::new(None));
        let sink = zones.clone();
        let first_pass = Rc::new(Cell::new(true));
        let mut harness = Harness::new_ui(move |ui| {
            if first_pass.get() {
                first_pass.set(false);
                ui.ctx().set_fonts(crate::fonts::definitions());
                return;
            }
            let toolbar = toolbar(
                ui,
                true,
                |ui| {
                    crate::icon_button(ui, "icon", "Undo");
                },
                |ui| {
                    crate::title_block(ui, &crate::Title::App("Cutmark"));
                },
                |ui| {
                    crate::primary_button(ui, "Export");
                },
            );
            sink.borrow_mut().replace(toolbar);
        });
        harness.run();
        let toolbar = zones.borrow().clone().expect("the toolbar must report its zones");
        let mut right_edge = f32::NEG_INFINITY;
        for clipped in &harness.output().shapes {
            right_edge = right_edge.max(clipped.shape.visual_bounding_rect().right());
        }
        (toolbar, right_edge)
    }

    /// The right zone ends: app actions, a 12px gap, the strong separator,
    /// then the three 46 x 44 caption buttons, and nothing is drawn to the
    /// right of the close button.
    #[test]
    fn nothing_is_drawn_right_of_the_close_button() {
        let (toolbar, right_edge) = custom_chrome_toolbar();
        let (minimize, maximize, close) = (toolbar.minimize.expect("minimize"), toolbar.maximize.expect("maximize"), toolbar.close.expect("close"));
        let tolerance = 0.5;
        assert!((close.right() - toolbar.bar.right()).abs() <= tolerance, "close must end at the window edge: {} vs {}", close.right(), toolbar.bar.right());
        assert!(right_edge <= close.right() + tolerance, "a shape reaches {right_edge}, right of the close button's edge {}", close.right());
        assert!(minimize.right() <= maximize.left() + tolerance && maximize.right() <= close.left() + tolerance, "buttons read minimize, maximize, close");
        assert_eq!(close.size(), vec2(size::CAPTION_BUTTON_WIDTH, size::CAPTION_BUTTON_HEIGHT));
        // The actions end at least the 12px gap plus the 1px separator before the buttons.
        let room = minimize.left() - toolbar.right.right();
        assert!(room >= ACTIONS_GAP + HAIRLINE - tolerance, "actions must end {ACTIONS_GAP}px plus the separator before the buttons, got {room}");
        for zone in [toolbar.left, toolbar.center, toolbar.right] {
            assert!(zone.right() <= minimize.left() + tolerance, "zone {zone:?} reaches into the caption buttons");
        }
    }
}
