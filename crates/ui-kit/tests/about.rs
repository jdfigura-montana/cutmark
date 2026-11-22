//! kittest tests for the About box (spec: S1 Help "Open questions" item 1): Esc
//! closes it, and the licenses view opens and returns.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};

/// The about info the tests show.
const INFO: ui_kit::AboutInfo =
    ui_kit::AboutInfo { app_name: "Cutmark Gallery", version: "0.1.0", copyright: "(c) 2026 Cutmark", notices: "Inter: SIL OFL 1.1." };

/// A harness with the ui-kit font set and an 800 x 600 window, so the named
/// Inter families the dialog lays text out in are bound. The first pass only
/// installs the fonts (egui applies a font change at the start of the next
/// pass).
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

/// Draws the About box, writing the open flag back through the cell.
fn about(ui: &mut egui::Ui, open_cell: &Cell<bool>) {
    let mut open = open_cell.get();
    ui_kit::about_dialog(ui.ctx(), &mut open, &INFO);
    open_cell.set(open);
}

/// Escape closes the About box.
#[test]
fn escape_closes_the_about_box() {
    let open = Rc::new(Cell::new(true));
    let open_cell = open.clone();
    let mut harness = new_harness(move |ui| about(ui, &open_cell));
    harness.run();
    assert!(open.get(), "the box must be open to start");

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!open.get(), "Esc must close the About box");
}

/// Close closes the box, and the licenses view opens and goes back.
#[test]
fn the_licenses_view_opens_and_close_closes_the_box() {
    let open = Rc::new(Cell::new(true));
    let open_cell = open.clone();
    let mut harness = new_harness(move |ui| about(ui, &open_cell));
    harness.run();

    harness.get_by_label("Third-party licenses").click();
    harness.run();
    assert!(open.get(), "opening the licenses view must not close the box");
    assert!(harness.query_by_label("Back").is_some(), "the licenses view has a Back button");

    harness.get_by_label("Back").click();
    harness.run();
    assert!(harness.query_by_label("Third-party licenses").is_some(), "Back returns to the main view");

    harness.get_by_label("Close").click();
    harness.run();
    assert!(!open.get(), "Close must close the About box");
}
