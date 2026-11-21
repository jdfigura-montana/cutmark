//! kittest tests for the menu bar and the title bar around it (spec: Design
//! Language rev 3 "Title bar").

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use egui::{Key, Modifiers};
use egui_kittest::{Harness, kittest::Queryable};
use ui_kit::{Menu, Title};

/// A harness with the ui-kit font set. The first pass only installs the fonts
/// (egui applies a font change at the start of the next pass); the widgets are
/// drawn from the second pass on.
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

/// A title bar with a File and an Edit menu in the left zone and a document
/// title in the center; the toolbar zones land in the returned sink.
fn title_bar() -> (Harness<'static>, Rc<RefCell<Option<ui_kit::MainToolbar>>>) {
    let zones = Rc::new(RefCell::new(None));
    let sink = zones.clone();
    let mut harness = new_harness(move |ui| {
        let toolbar = ui_kit::main_toolbar(
            ui,
            |ui| {
                let mut menus = [
                    Menu::new("File", |ui| {
                        if ui.button("Open").clicked() {
                            ui.close();
                        }
                    }),
                    Menu::new("Edit", |ui| {
                        let _ = ui.button("Undo");
                    }),
                ];
                ui_kit::menu_bar(ui, &mut menus);
            },
            |ui| {
                ui_kit::title_block(ui, &Title::Document { name: "Interview take 3", unsaved: true, caption: Some("48 kHz \u{b7} 24-bit \u{b7} 4 tracks") });
            },
            |ui| {
                ui_kit::primary_button(ui, "Export");
            },
        );
        sink.borrow_mut().replace(toolbar);
    });
    harness.run();
    (harness, zones)
}

/// True when the label is in the accessibility tree.
fn shows(harness: &Harness<'static>, label: &str) -> bool {
    harness.query_by_label(label).is_some()
}

/// Clicking a title opens its menu as a popover; clicking it again closes it.
#[test]
fn a_click_on_a_title_opens_its_menu() {
    let (mut harness, _) = title_bar();
    assert!(!shows(&harness, "Open"), "no menu is open at rest");

    harness.get_by_label("File").click();
    harness.run();
    assert!(shows(&harness, "Open"), "clicking File must open the File menu");
    assert!(!shows(&harness, "Undo"), "only the clicked menu opens");

    harness.get_by_label("File").click();
    harness.run();
    assert!(!shows(&harness, "Open"), "clicking the open title again must close its menu");
}

/// While a menu is open, hovering another title switches to its menu.
#[test]
fn hovering_another_title_switches_menus_while_one_is_open() {
    let (mut harness, _) = title_bar();

    // Hovering with nothing open opens nothing.
    harness.get_by_label("Edit").hover();
    harness.run();
    assert!(!shows(&harness, "Undo"), "hover alone must not open a menu");

    harness.get_by_label("File").click();
    harness.run();
    assert!(shows(&harness, "Open"));

    harness.get_by_label("Edit").hover();
    harness.run();
    assert!(shows(&harness, "Undo"), "hovering Edit must switch to the Edit menu");
    assert!(!shows(&harness, "Open"), "the File menu must close when the Edit menu opens");
}

/// Esc closes the open menu.
#[test]
fn escape_closes_the_open_menu() {
    let (mut harness, _) = title_bar();
    harness.get_by_label("File").click();
    harness.run();
    assert!(shows(&harness, "Open"));

    harness.key_press(Key::Escape);
    harness.run();
    assert!(!shows(&harness, "Open"), "Esc must close the menu");
}

/// Clicking a menu item runs it and closes the menu.
#[test]
fn clicking_an_item_closes_the_menu() {
    let (mut harness, _) = title_bar();
    harness.get_by_label("File").click();
    harness.run();
    harness.get_by_label("Open").click();
    harness.run();
    assert!(!shows(&harness, "Open"), "an item click must close the menu");
}

/// Alt plus the first letter of a title opens that menu.
#[test]
fn alt_and_a_letter_open_the_matching_menu() {
    let (mut harness, _) = title_bar();
    harness.key_press_modifiers(Modifiers::ALT, Key::E);
    harness.run();
    assert!(shows(&harness, "Undo"), "Alt+E must open the Edit menu");
    assert!(!shows(&harness, "Open"), "Alt+E must not open the File menu");
}

/// A tap of Alt focuses the first title; Right moves to the next; Enter opens it.
#[test]
fn alt_then_the_arrow_keys_and_enter_drive_the_bar() {
    let (mut harness, _) = title_bar();
    harness.event(egui::Event::ModifiersChanged(Modifiers::ALT));
    harness.step();
    harness.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    harness.step();
    harness.run();

    harness.key_press(Key::ArrowRight);
    harness.run();
    harness.key_press(Key::Enter);
    harness.run();
    assert!(shows(&harness, "Undo"), "Alt, Right, Enter must open the second menu");

    harness.key_press(Key::Escape);
    harness.run();
    assert!(!shows(&harness, "Undo"), "Esc must close it");
}

/// The caption (drag) regions never cover a menu title: clicking a title must
/// not start a window drag.
#[test]
fn the_caption_regions_exclude_every_menu_title() {
    let (harness, zones) = title_bar();
    let toolbar = zones.borrow().clone().expect("the toolbar must report its zones");
    assert!(!toolbar.caption.is_empty(), "the bar must keep caption space");
    for title in ["File", "Edit"] {
        let rect = harness.get_by_label(title).rect();
        assert!(rect.is_positive(), "the {title} title must be laid out");
        for caption in &toolbar.caption {
            assert!(!caption.intersect(rect).is_positive(), "caption rect {caption:?} overlaps the {title} title {rect:?}");
        }
    }
    let gap = egui::pos2((toolbar.center.right() + toolbar.right.left()) / 2.0, toolbar.bar.center().y);
    assert!(toolbar.caption.iter().any(|r| r.contains(gap)), "empty bar space between the zones must stay caption");
}

/// The title block sits on the window's midpoint, within 1px, even though the
/// left and right zones differ in width.
#[test]
fn the_title_block_is_centered_on_the_window() {
    let (harness, zones) = title_bar();
    let toolbar = zones.borrow().clone().expect("the toolbar must report its zones");
    let block = harness.get_by_label("Interview take 3").rect();
    assert!(block.is_positive(), "the title block must be laid out");
    let drift = block.center().x - toolbar.bar.center().x;
    assert!(drift.abs() <= 1.0, "the title block must be centered on the window, off by {drift}px");
    let zone_drift = toolbar.center.center().x - toolbar.bar.center().x;
    assert!(zone_drift.abs() <= 1.0, "the center zone must be centered on the window, off by {zone_drift}px");
    // One line: the block is shorter than the 44px bar.
    assert!(block.height() < toolbar.bar.height(), "the title block must be a single line");
}
