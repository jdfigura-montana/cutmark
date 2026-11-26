//! kittest tests for the stage panel (spec: E3 amendment, "1. The notch"): the
//! primary button in the notch, the bottom bar's empty center column, and the
//! status bar's center gap.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Id, Pos2, Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use ui_kit::{PlayKind, PrimaryButton, StageResponse};

/// The stage's height in these tests; the harness leaves room under it.
const STAGE_HEIGHT: f32 = 300.0;

/// A harness with the ui-kit font set. The first pass only installs the fonts;
/// the widgets are drawn from the second pass on.
fn new_harness(mut draw: impl FnMut(&mut egui::Ui) + 'static) -> Harness<'static> {
    let first_pass = Rc::new(Cell::new(true));
    let mut harness = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui(move |ui| {
        if first_pass.get() {
            first_pass.set(false);
            ui.ctx().set_fonts(ui_kit::fonts::definitions());
            return;
        }
        draw(ui);
    });
    harness.run();
    harness
}

/// What the tests read back from a drawn stage.
#[derive(Default)]
struct Seen {
    stage: RefCell<Option<StageResponse>>,
    clicks: Cell<u32>,
    bar_widgets: RefCell<Vec<Rect>>,
}

impl Seen {
    fn stage(&self) -> StageResponse {
        self.stage.borrow().clone().expect("the stage was drawn")
    }
}

/// A harness showing one stage whose parent is clipped to the panel's own
/// bottom edge, like a central panel above a status bar. `crowd` is the number
/// of buttons on each side of the bar.
fn stage_harness(kind: PlayKind, enabled: bool, crowd: usize) -> (Harness<'static>, Rc<Seen>) {
    let seen = Rc::new(Seen::default());
    let watch = seen.clone();
    let active = Rc::new(Cell::new(false));
    let mut harness = new_harness(move |ui| {
        let area = Rect::from_min_size(ui.max_rect().min, vec2(ui.available_width(), STAGE_HEIGHT));
        let mut parent = ui.new_child(egui::UiBuilder::new().max_rect(area));
        parent.set_clip_rect(area);
        let widgets = RefCell::new(Vec::new());
        let side = |ui: &mut egui::Ui, name: &str| {
            for index in 0..crowd {
                let button = ui_kit::bar_button(ui, ui_kit::icons::fill::STOP, &format!("{name} {index}"));
                widgets.borrow_mut().push(button.interact_rect);
            }
        };
        let primary = PrimaryButton { kind, active: active.get(), enabled };
        let stage = ui_kit::stage_panel(
            &mut parent,
            Id::new("test-stage"),
            primary,
            |ui| {
                ui.label("Content");
            },
            |ui| side(ui, "Left"),
            |ui| side(ui, "Right"),
        );
        if stage.clicked() {
            watch.clicks.set(watch.clicks.get() + 1);
            active.set(!active.get());
        }
        *watch.bar_widgets.borrow_mut() = widgets.into_inner();
        *watch.stage.borrow_mut() = Some(stage);
    });
    harness.run();
    (harness, seen)
}

/// Presses and releases the primary pointer button at `pos`.
fn click_at(harness: &mut Harness<'_>, pos: Pos2) {
    harness.hover_at(pos);
    harness.run();
    harness.drag_at(pos);
    harness.drop_at(pos);
    harness.run();
}

#[test]
fn clicking_the_primary_button_reports_clicked() {
    let (mut harness, seen) = stage_harness(PlayKind::Play, true, 1);
    harness.get_by_label("Play").click();
    harness.run();
    assert_eq!(seen.clicks.get(), 1);
    // The accessible name follows the state.
    harness.get_by_label("Pause").click();
    harness.run();
    assert_eq!(seen.clicks.get(), 2);
}

#[test]
fn the_record_button_reports_clicks_and_renames_itself() {
    let (mut harness, seen) = stage_harness(PlayKind::Record, true, 0);
    harness.get_by_label("Record").click();
    harness.run();
    assert_eq!(seen.clicks.get(), 1);
    harness.get_by_label("Stop recording");
}

#[test]
fn a_disabled_primary_button_reports_no_click() {
    let (mut harness, seen) = stage_harness(PlayKind::Play, false, 0);
    let center = seen.stage().primary.rect.center();
    click_at(&mut harness, center);
    assert_eq!(seen.clicks.get(), 0);
}

#[test]
fn the_primary_button_extends_below_the_panel_and_stays_clickable_there() {
    let (mut harness, seen) = stage_harness(PlayKind::Play, true, 1);
    let stage = seen.stage();
    let (button, panel) = (stage.primary.rect, stage.panel_rect);
    assert_eq!(button.size(), vec2(68.0, 68.0));
    assert!((button.center().x - panel.center().x).abs() < 0.5, "{button:?} {panel:?}");
    assert!((panel.bottom() - button.center().y - 8.0).abs() < 0.5, "{button:?} {panel:?}");
    assert!((button.bottom() - panel.bottom() - ui_kit::STAGE_PRIMARY_OVERHANG).abs() < 0.5, "{button:?} {panel:?}");
    // The parent clips at the panel's bottom edge; the overhang still clicks.
    click_at(&mut harness, Pos2::new(button.center().x, panel.bottom() + 15.0));
    assert_eq!(seen.clicks.get(), 1);
}

#[test]
fn the_bar_center_column_stays_free() {
    // Eight buttons a side are far wider than either side of an 800 px bar.
    for crowd in [2, 8] {
        let (_harness, seen) = stage_harness(PlayKind::Play, true, crowd);
        let stage = seen.stage();
        let center = stage.bar_center_rect;
        assert_eq!(center.width(), 112.0);
        assert_eq!(stage.bar_rect.height(), 64.0);
        let widgets = seen.bar_widgets.borrow();
        assert_eq!(widgets.len(), 2 * crowd);
        for rect in widgets.iter() {
            // A button clipped away entirely has an inverted interact rect.
            let overlap = rect.intersect(center);
            assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0, "{rect:?} reaches into {center:?}");
        }
    }
}

/// What the layer order tests read back.
#[derive(Default)]
struct Layers {
    stage_clicks: Cell<u32>,
    window_clicks: Cell<u32>,
    window_open: Cell<bool>,
    primary: Cell<Option<Rect>>,
}

impl Layers {
    fn primary_rect(&self) -> Rect {
        self.primary.get().expect("the stage was drawn")
    }
}

/// The strip painted under the stage after it, on the same layer, like the
/// app's status bar.
const STRIP_FILL: egui::Color32 = egui::Color32::from_rgb(1, 2, 3);
/// The fill of the one button inside the test window.
const WINDOW_FILL: egui::Color32 = egui::Color32::from_rgb(4, 5, 6);

/// A stage with a strip drawn under it afterwards and, when `window_open`, an
/// [`egui::Window`] whose single button covers the primary button.
fn layered_harness() -> (Harness<'static>, Rc<Layers>) {
    let seen = Rc::new(Layers::default());
    let watch = seen.clone();
    let mut harness = new_harness(move |ui| {
        let area = Rect::from_min_size(ui.max_rect().min, vec2(ui.available_width(), STAGE_HEIGHT));
        let mut parent = ui.new_child(egui::UiBuilder::new().max_rect(area));
        let primary = PrimaryButton { kind: PlayKind::Play, active: false, enabled: true };
        let stage = ui_kit::stage_panel(&mut parent, Id::new("test-stage"), primary, |_| {}, |_| {}, |_| {});
        if stage.clicked() {
            watch.stage_clicks.set(watch.stage_clicks.get() + 1);
        }
        watch.primary.set(Some(stage.primary.rect));
        let strip = Rect::from_min_size(area.left_bottom(), vec2(area.width(), 40.0));
        ui.painter().rect_filled(strip, 0.0, STRIP_FILL);
        let _ = ui.interact(strip, Id::new("strip"), egui::Sense::click());
        if watch.window_open.get() {
            egui::Window::new("Cover").title_bar(false).resizable(false).fixed_pos(stage.primary.rect.min - vec2(40.0, 40.0)).show(ui.ctx(), |ui| {
                if ui.add(egui::Button::new("Window button").fill(WINDOW_FILL).min_size(vec2(160.0, 160.0))).clicked() {
                    watch.window_clicks.set(watch.window_clicks.get() + 1);
                }
            });
        }
    });
    harness.run();
    (harness, seen)
}

/// The position in paint order of the first shape that matches.
fn paint_index(harness: &Harness<'_>, what: &str, matches: impl Fn(&egui::Shape) -> bool) -> usize {
    harness.output().shapes.iter().position(|clipped| matches(&clipped.shape)).expect(what)
}

/// The paint position of the primary button's circle.
fn primary_paint_index(harness: &Harness<'_>) -> usize {
    paint_index(harness, "the primary button", |shape| matches!(shape, egui::Shape::Circle(circle) if circle.radius == 34.0))
}

/// The paint position of the rect filled with `fill`.
fn fill_paint_index(harness: &Harness<'_>, fill: egui::Color32) -> usize {
    paint_index(harness, "the filled rect", |shape| matches!(shape, egui::Shape::Rect(rect) if rect.fill == fill))
}

#[test]
fn the_primary_button_stays_above_the_panels_drawn_after_it() {
    let (mut harness, seen) = layered_harness();
    assert!(primary_paint_index(&harness) > fill_paint_index(&harness, STRIP_FILL), "the strip covers the button");
    // A click on the panels does not raise them over the button.
    click_at(&mut harness, Pos2::new(20.0, seen.primary_rect().bottom()));
    assert!(primary_paint_index(&harness) > fill_paint_index(&harness, STRIP_FILL), "the strip covers the button after a click");
    // The overhang lies over the strip and still takes the click.
    let button = seen.primary_rect();
    click_at(&mut harness, Pos2::new(button.center().x, button.bottom() - 4.0));
    assert_eq!(seen.stage_clicks.get(), 1);
}

#[test]
fn a_window_over_the_primary_button_covers_it_and_gets_the_click() {
    let (mut harness, seen) = layered_harness();
    let center = seen.primary_rect().center();
    seen.window_open.set(true);
    harness.run();
    harness.run();
    assert!(fill_paint_index(&harness, WINDOW_FILL) > primary_paint_index(&harness), "the button draws over the window");
    click_at(&mut harness, center);
    assert_eq!((seen.window_clicks.get(), seen.stage_clicks.get()), (1, 0));
    // Closing the window gives the button its clicks back.
    seen.window_open.set(false);
    harness.run();
    click_at(&mut harness, center);
    assert_eq!((seen.window_clicks.get(), seen.stage_clicks.get()), (1, 1));
}

/// The rects of every painted text, with the clip applied.
fn text_rects(harness: &Harness<'_>) -> Vec<Rect> {
    fn walk(shape: &egui::Shape, clip: Rect, into: &mut Vec<Rect>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| walk(shape, clip, into)),
            egui::Shape::Text(text) => into.push(text.visual_bounding_rect().intersect(clip)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, clipped.clip_rect, &mut found);
    }
    found
}

#[test]
fn the_status_bar_gap_keeps_text_out_of_the_center() {
    let bar = Rc::new(Cell::new(Rect::NOTHING));
    let watch = bar.clone();
    // Enough items that, without the gap, several would cross the middle.
    let items = ["Ready", "48 kHz", "24 bit", "Stereo", "Autosave on", "Peak -3 dB", "Disk 120 GB free", "Voice track armed", "Snap on"];
    let harness = new_harness(move |ui| {
        watch.set(ui_kit::status_bar_with_gap(ui, &items).rect);
    });
    let bar = bar.get();
    assert_eq!(bar.height(), 30.0);
    let (gap_left, gap_right) = (bar.center().x - 60.0, bar.center().x + 60.0);
    let texts = text_rects(&harness);
    assert_eq!(texts.len(), items.len(), "every item is painted: {texts:?}");
    for text in &texts {
        // A glyph's soft edge may reach half a pixel past its layout box.
        assert!(text.right() <= gap_left + 0.5 || text.left() >= gap_right - 0.5, "{text:?} sits in the gap {gap_left}..{gap_right}");
    }
    assert!(texts.iter().any(|text| text.right() <= gap_left + 0.5), "items before the gap");
    assert!(texts.iter().any(|text| text.left() >= gap_right - 0.5), "items after the gap");
}
