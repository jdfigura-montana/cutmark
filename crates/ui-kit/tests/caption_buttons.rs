//! kittest tests for the caption buttons and the toolbar chrome regions
//! (spec "Window chrome").

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui_kittest::{Harness, kittest::Queryable};

/// A harness with the ui-kit font set, so the named Instrument Sans families the
/// widgets lay text out in are bound. The first pass only installs the
/// fonts (egui applies a font change at the start of the next pass); the
/// widgets are drawn from the second pass on.
fn new_harness(mut draw: impl FnMut(&mut egui::Ui) + 'static) -> Harness<'static> {
    let first_pass = Rc::new(Cell::new(true));
    let mut harness = Harness::new_ui(move |ui| {
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

/// Draws a full main toolbar, reporting its zones into the returned sink.
fn toolbar_harness() -> (Harness<'static>, Rc<RefCell<Option<ui_kit::MainToolbar>>>) {
    let zones = Rc::new(RefCell::new(None));
    let sink = zones.clone();
    let mut harness = new_harness(move |ui| {
        let z = ui_kit::main_toolbar(
            ui,
            |ui| {
                ui_kit::icon_button(ui, "icon", "Undo");
            },
            |ui| {
                let mut mode = 0usize;
                ui_kit::segmented(ui, "caption-modes", &mut mode, &["Edit", "Mix"]);
            },
            |ui| {
                ui_kit::primary_button(ui, "Export");
            },
        );
        sink.borrow_mut().replace(z);
    });
    harness.run();
    (harness, zones)
}

/// The published caption regions never overlap an interactive widget rect:
/// not the zones' widgets, not the caption buttons (spec "Window chrome":
/// interactive toolbar elements never drag).
#[test]
fn the_published_caption_regions_miss_every_interactive_widget() {
    let (_harness, zones) = toolbar_harness();
    let z = zones.borrow().clone().expect("the toolbar must report its zones");
    assert!(!z.caption.is_empty(), "the bar must keep caption space");
    let interactive = [("left", z.left), ("center", z.center), ("right", z.right)];
    for caption in &z.caption {
        for (name, rect) in interactive {
            assert!(!caption.intersect(rect).is_positive(), "caption rect {caption:?} overlaps the {name} zone {rect:?}");
        }
    }
    let gap = egui::pos2((z.center.right() + z.right.left()) / 2.0, z.bar.center().y);
    assert!(z.caption.iter().any(|r| r.contains(gap)), "empty space between the zones must stay caption");
}

/// Without the custom chrome no caption buttons are drawn and none are
/// published (spec "Window chrome": only drawn when chrome::is_custom()).
#[test]
fn without_the_custom_chrome_no_caption_buttons_are_drawn() {
    let (harness, zones) = toolbar_harness();
    let z = zones.borrow().clone().expect("the toolbar must report its zones");
    assert!(z.minimize.is_none() && z.maximize.is_none() && z.close.is_none(), "no button rects without the custom chrome");
    for label in ["Minimize", "Maximize", "Restore", "Close"] {
        assert!(harness.query_by_label(label).is_none(), "no {label} button may exist without the custom chrome");
    }
}

/// Clicking the close button sends `ViewportCommand::Close`.
#[test]
fn clicking_close_sends_the_close_command() {
    let mut harness = new_harness(|ui| {
        ui_kit::caption_buttons(ui);
    });
    harness.run();
    harness.get_by_label("Close").click();
    let mut sent = false;
    for _ in 0..5 {
        harness.step();
        sent = harness.output().viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Close));
        if sent {
            break;
        }
    }
    assert!(sent, "the close button must send ViewportCommand::Close");
}

/// The three buttons are drawn with the caption button size from the tokens.
#[test]
fn the_caption_buttons_have_the_token_size() {
    let mut harness = new_harness(|ui| {
        ui_kit::caption_buttons(ui);
    });
    harness.run();
    let side = egui::vec2(ui_kit::tokens::size::CAPTION_BUTTON_WIDTH, ui_kit::tokens::size::CAPTION_BUTTON_HEIGHT);
    for label in ["Minimize", "Maximize", "Close"] {
        let rect = harness.get_by_label(label).rect();
        assert_eq!(rect.size(), side, "the {label} button must be {side:?}");
    }
}
