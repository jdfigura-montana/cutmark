//! kittest tests for the fader (spec "Components": rows "Slider" and "Meter").

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui_kittest::{Harness, kittest::Queryable};

/// A fader value shared between the test and the UI closure, so the test can
/// read what the widget wrote back.
fn shared_value(start: f32) -> (Rc<Cell<f32>>, Rc<Cell<f32>>) {
    let cell = Rc::new(Cell::new(start));
    (cell.clone(), cell)
}

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

/// Presses the pointer down and up twice at `at` within one pass, so egui's
/// double-click window (0.3 s; kittest steps are 0.25 s apart) sees a double-click.
fn double_click(harness: &mut Harness<'static>, at: egui::Pos2) {
    harness.hover_at(at);
    harness.step();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
    }
    harness.run();
}

/// A double-click sends the fader back to 0 dB.
#[test]
fn double_clicking_the_fader_resets_it_to_0_db() {
    let (watch, state) = shared_value(-10.0);
    let mut harness = new_harness(move |ui| {
        let mut db = state.get();
        ui_kit::fader(ui, &mut db, 120.0);
        state.set(db);
    });
    harness.run();

    let at = harness.get_by_role(egui::accesskit::Role::Slider).rect().center();
    double_click(&mut harness, at);

    assert_eq!(watch.get(), 0.0, "the double-click must reset the fader to 0 dB, got {}", watch.get());
}

/// With the fader focused, the Up and Down arrows step 0.5 dB.
#[test]
fn the_arrow_keys_step_the_focused_fader_by_half_a_db() {
    let (watch, state) = shared_value(0.0);
    let mut harness = new_harness(move |ui| {
        let mut db = state.get();
        ui_kit::fader(ui, &mut db, 120.0);
        state.set(db);
    });
    harness.run();

    harness.get_by_role(egui::accesskit::Role::Slider).focus();
    harness.run();
    harness.key_press(egui::Key::ArrowUp);
    harness.run();
    assert!((watch.get() - 0.5).abs() < 1e-4, "the Up arrow must step 0.5 dB up, got {}", watch.get());
    harness.key_press(egui::Key::ArrowDown);
    harness.run();
    assert!(watch.get().abs() < 1e-4, "the Down arrow must step 0.5 dB down, got {}", watch.get());
}

/// The fader value stays inside its -60 to +6 dB range no matter how far the
/// arrows push it.
#[test]
fn the_fader_value_stays_in_range() {
    let (watch, state) = shared_value(0.0);
    let mut harness = new_harness(move |ui| {
        let mut db = state.get();
        ui_kit::fader(ui, &mut db, 120.0);
        state.set(db);
    });
    harness.run();

    harness.get_by_role(egui::accesskit::Role::Slider).focus();
    harness.run();
    for _ in 0..40 {
        harness.key_press(egui::Key::ArrowUp);
        harness.run();
    }
    assert!(watch.get() <= 6.0, "the fader must stop at +6 dB, got {}", watch.get());
    assert!((watch.get() - 6.0).abs() < 1e-4, "the fader must reach +6 dB, got {}", watch.get());
    for _ in 0..140 {
        harness.key_press(egui::Key::ArrowDown);
        harness.run();
    }
    assert!(watch.get() >= -60.0, "the fader must stop at -60 dB, got {}", watch.get());
    assert!((watch.get() + 60.0).abs() < 1e-4, "the fader must reach -60 dB, got {}", watch.get());
}
