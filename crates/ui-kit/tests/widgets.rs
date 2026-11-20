//! kittest tests for the core widgets (spec "Components": the first six rows).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui_kittest::{Harness, kittest::Queryable};

/// A bool shared between the test and the UI closure, so the test can read
/// what the widget wrote back.
fn shared_bool() -> (Rc<Cell<bool>>, Rc<Cell<bool>>) {
    let cell = Rc::new(Cell::new(false));
    (cell.clone(), cell)
}

/// A harness with the ui-kit font set, so the named Inter families the
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

/// The switch toggles on click, on and back off.
#[test]
fn clicking_the_switch_toggles_it() {
    let (watch, state) = shared_bool();
    let mut harness = new_harness(move |ui| {
        let mut value = state.get();
        ui_kit::switch(ui, &mut value);
        state.set(value);
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Switch").click();
    harness.run();
    assert!(watch.get(), "the switch must be on after the first click");
    harness.get_by_label("Switch").click();
    harness.run();
    assert!(!watch.get(), "the switch must be off after the second click");
}

/// The chip toggles on click, on and back off.
#[test]
fn clicking_the_chip_toggles_it() {
    let (watch, state) = shared_bool();
    let mut harness = new_harness(move |ui| {
        let mut value = state.get();
        ui_kit::chip(ui, &mut value, "Snap");
        state.set(value);
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Snap").click();
    harness.run();
    assert!(watch.get(), "the chip must be on after the first click");
    harness.get_by_label("Snap").click();
    harness.run();
    assert!(!watch.get(), "the chip must be off after the second click");
}

/// Clicking the second segment of the control selects it.
#[test]
fn clicking_the_second_segment_selects_it() {
    let selected = Rc::new(Cell::new(0usize));
    let state = selected.clone();
    let mut harness = new_harness(move |ui| {
        let mut value = state.get();
        ui_kit::segmented(ui, &mut value, &["Video", "Audio"]);
        state.set(value);
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Audio").click();
    harness.run();
    assert_eq!(selected.get(), 1, "clicking the second segment must select it");
}

/// With the control focused, the Right arrow moves the selection.
#[test]
fn right_arrow_moves_the_selection_when_focused() {
    let selected = Rc::new(Cell::new(0usize));
    let state = selected.clone();
    let mut harness = new_harness(move |ui| {
        let mut value = state.get();
        ui_kit::segmented(ui, &mut value, &["Video", "Audio"]);
        state.set(value);
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Video").focus();
    harness.run();
    assert_eq!(selected.get(), 0, "focusing a segment must not change the selection");
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    assert_eq!(selected.get(), 1, "the Right arrow must move the selection");
}

/// The primary button reports clicks.
#[test]
fn the_primary_button_reports_clicks() {
    let (watch, state) = shared_bool();
    let mut harness = new_harness(move |ui| {
        if ui_kit::primary_button(ui, "Export").clicked() {
            state.set(true);
        }
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Export").click();
    harness.run();
    assert!(watch.get(), "the primary button must report the click");
}

/// Hovering the icon button shows its tooltip text.
#[test]
fn the_icon_button_shows_its_tooltip_on_hover() {
    let mut harness = new_harness(|ui| {
        ui_kit::icon_button(ui, "icon", "Settings");
    });
    harness.run();
    harness.fit_contents();
    harness.ctx.all_styles_mut(|style| style.interaction.tooltip_delay = 0.0);
    harness.run();
    assert_eq!(harness.query_all_by_label("Settings").count(), 1, "before the hover only the button itself carries the label");

    harness.get_by_label("Settings").hover();
    harness.run();
    assert!(harness.query_all_by_label("Settings").count() > 1, "the tooltip text must appear in the tree on hover");
}
