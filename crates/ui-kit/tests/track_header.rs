//! kittest tests for the track header (spec "Components": row "Track header",
//! "Layout rules" item 5).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui_kittest::{Harness, kittest::Queryable};
use ui_kit::track_header::{Toggle, ToggleKind, track_header};

/// A toggle bool shared between the test and the UI closure.
fn shared_bool() -> (Rc<Cell<bool>>, Rc<Cell<bool>>) {
    let cell = Rc::new(Cell::new(false));
    (cell.clone(), cell)
}

/// A harness with the ui-kit font set, so the named Inter families the
/// widgets lay text out in are bound. The first pass only installs the
/// fonts; the widgets are drawn from the second pass on. `selected` feeds
/// the header's selection state.
fn new_harness(selected: bool, mute: Rc<Cell<bool>>, arm: Rc<Cell<bool>>) -> Harness<'static> {
    let first_pass = Rc::new(Cell::new(true));
    let mut harness = Harness::new_ui(move |ui| {
        if first_pass.get() {
            first_pass.set(false);
            ui.ctx().set_fonts(ui_kit::fonts::definitions());
            return;
        }
        let mut mute_on = mute.get();
        let mut arm_on = arm.get();
        let mut toggles = [Toggle::hidden("M", &mut mute_on), Toggle { label: "R", on: &mut arm_on, kind: ToggleKind::RecordArm, always_visible: false }];
        track_header(ui, 1, "Vocals", selected, &mut toggles, Some((-6.0, false)));
        mute.set(mute_on);
        arm.set(arm_on);
    });
    harness.run();
    harness.fit_contents();
    harness.run();
    harness
}

/// A hidden toggle is not in the accessibility tree until the header is
/// hovered ("Layout rules" item 5).
#[test]
fn a_hidden_toggle_is_absent_until_the_header_is_hovered() {
    let (_, mute) = shared_bool();
    let (_, arm) = shared_bool();
    let mut harness = new_harness(false, mute, arm);

    assert!(harness.root().query_by_label("M").is_none(), "the hidden toggle must not be in the tree while the header is idle");

    harness.get_by_label("Vocals").hover();
    harness.run();
    assert!(harness.root().query_by_label("M").is_some(), "hovering the header must reveal the hidden toggle");
}

/// A hidden toggle shows without hover when the header is selected.
#[test]
fn a_hidden_toggle_shows_when_the_header_is_selected() {
    let (_, mute) = shared_bool();
    let (_, arm) = shared_bool();
    let harness = new_harness(true, mute, arm);

    assert!(harness.root().query_by_label("M").is_some(), "the selected header must show its hidden toggles without hover");
}

/// Clicking a toggle flips its bool, off and back on.
#[test]
fn clicking_a_toggle_flips_its_bool() {
    let (_, mute) = shared_bool();
    let (arm_watch, arm) = shared_bool();
    let mut harness = new_harness(true, mute, arm);

    harness.get_by_label("R").click();
    harness.run();
    assert!(arm_watch.get(), "the record arm must be on after the first click");
    harness.get_by_label("R").click();
    harness.run();
    assert!(!arm_watch.get(), "the record arm must be off after the second click");
}
