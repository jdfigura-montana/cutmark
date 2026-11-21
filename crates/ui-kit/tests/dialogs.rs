//! kittest tests for the dialog (spec "Components": row "Dialogs"): Esc
//! closes, and the footer sits right-aligned with the primary button last,
//! bottom right.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui::{Id, vec2};
use egui_kittest::{Harness, kittest::Queryable};

/// A harness with the ui-kit font set and an 800 x 600 window, so the named
/// Inter families the widgets lay text out in are bound. The first pass only
/// installs the fonts (egui applies a font change at the start of the next
/// pass); the widgets are drawn from the second pass on.
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

/// Draws the confirm-delete dialog. The `open` flag round-trips through the
/// cell; the footer buttons write their clicks into `cancel` and `delete`
/// (closures cannot also borrow the `open` flag that `dialog` holds).
fn confirm_dialog(ui: &mut egui::Ui, open_cell: &Cell<bool>, cancel: &Cell<bool>, delete: &Cell<bool>) {
    let mut open = open_cell.get();
    ui_kit::dialog(
        ui.ctx(),
        Id::new("delete-confirm"),
        "Delete clip?",
        &mut open,
        |ui| {
            ui.label("This cannot be undone.");
        },
        |ui| {
            if ui.button("Cancel").clicked() {
                cancel.set(true);
                open_cell.set(false);
            }
            if ui.button("Delete").clicked() {
                delete.set(true);
                open_cell.set(false);
            }
        },
    );
    // The footer buttons close the dialog through the cell directly; only
    // write the Esc/backdrop result back when no button already closed it.
    if !cancel.get() && !delete.get() {
        open_cell.set(open);
    }
}

/// Escape closes the dialog without triggering the primary action.
#[test]
fn escape_closes_the_dialog() {
    let open = Rc::new(Cell::new(true));
    let cancelled = Rc::new(Cell::new(false));
    let deleted = Rc::new(Cell::new(false));
    let (open_cell, cancelled_cell, deleted_cell) = (open.clone(), cancelled.clone(), deleted.clone());
    let mut harness = new_harness(move |ui| {
        confirm_dialog(ui, &open_cell, &cancelled_cell, &deleted_cell);
    });
    harness.run();

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(!open.get(), "Esc must close the dialog");
    assert!(!cancelled.get(), "Esc must not click Cancel");
    assert!(!deleted.get(), "Esc must not trigger the primary action");
}

/// The footer buttons sit right of center with the primary button last: the
/// bottom right of the dialog.
#[test]
fn the_footer_sits_at_the_bottom_right_with_the_primary_button_last() {
    let open = Rc::new(Cell::new(true));
    let cancelled = Rc::new(Cell::new(false));
    let deleted = Rc::new(Cell::new(false));
    let (open_cell, cancelled_cell, deleted_cell) = (open.clone(), cancelled.clone(), deleted.clone());
    let mut harness = new_harness(move |ui| {
        confirm_dialog(ui, &open_cell, &cancelled_cell, &deleted_cell);
    });
    harness.run();

    let cancel = harness.get_by_label("Cancel").rect();
    let delete = harness.get_by_label("Delete").rect();
    assert!(delete.right() > cancel.right(), "the primary button must be the rightmost, cancel {cancel:?} delete {delete:?}");
    assert!(delete.right() > 400.0, "the footer must sit right of the window center, delete {delete:?}");
    assert!(cancel.left() < delete.left(), "the secondary button must come before the primary button");

    harness.get_by_label("Delete").click();
    harness.run();
    assert!(deleted.get(), "the primary button must report the click");
    assert!(!open.get(), "the dialog must have closed");
}
