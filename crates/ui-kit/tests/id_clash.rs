//! kittest tests for egui widget ID hygiene and the main toolbar's zones:
//! duplicated widgets must never raise the red "First use / Second use of
//! widget ID" overlays, the right zone must read content then caption
//! buttons, and the left zone must fit the 44px bar.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui::{Rect, Shape};
use egui_kittest::{Harness, kittest::Queryable};

/// A harness with the ui-kit font set, so the named Inter families and the
/// phosphor icon fonts the widgets lay text out in are bound. The first pass
/// only installs the fonts (egui applies a font change at the start of the
/// next pass); the widgets are drawn from the second pass on.
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

/// Collects the texts of egui's debug overlay: a reused widget ID is reported
/// as "First use / Second use / Double use of widget ID ..." painted in red.
fn id_clash_texts(harness: &Harness<'static>) -> Vec<String> {
    fn walk(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, out);
                }
            }
            Shape::Text(text_shape) if text_shape.galley.text().contains("use of widget ID") => {
                out.push(text_shape.galley.text().to_owned());
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// Two segmented controls with identical labels on one page must not share
/// widget ids, and each control must still answer its own clicks.
#[test]
fn two_identical_segmented_controls_do_not_clash_ids() {
    let first = Rc::new(Cell::new(0usize));
    let second = Rc::new(Cell::new(0usize));
    let first_state = first.clone();
    let second_state = second.clone();
    let mut harness = new_harness(move |ui| {
        let mut a = first_state.get();
        ui_kit::segmented(ui, "modes-a", &mut a, &["Edit", "Mix"]);
        first_state.set(a);
        let mut b = second_state.get();
        ui_kit::segmented(ui, "modes-b", &mut b, &["Edit", "Mix"]);
        second_state.set(b);
    });
    harness.run();

    let clashes = id_clash_texts(&harness);
    assert!(clashes.is_empty(), "identical controls must not report id clashes: {clashes:?}");

    // Both controls expose their own "Mix" segment; clicking the first must
    // move only the first control's selection.
    let mixes: Vec<_> = harness.query_all_by_label("Mix").collect();
    assert_eq!(mixes.len(), 2, "both controls must expose a Mix segment");
    mixes[0].click();
    harness.run();
    assert_eq!(first.get(), 1, "the first control must answer the click");
    assert_eq!(second.get(), 0, "the second control must keep its own selection");
}

/// Two main_toolbar frames on one page (each drawing the page switch and the
/// actions twice: the invisible measuring run plus the real run) must not
/// share widget ids anywhere.
#[test]
fn two_main_toolbar_frames_do_not_clash_ids() {
    let mut harness = new_harness(|ui| {
        for salt in ["toolbar-a", "toolbar-b"] {
            let mode = Cell::new(0usize);
            ui_kit::main_toolbar(
                ui,
                |ui| {
                    ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE, "Undo");
                    ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_CLOCKWISE, "Redo");
                },
                |ui| {
                    let mut m = mode.get();
                    ui_kit::segmented(ui, salt, &mut m, &["Edit", "Mix"]);
                    mode.set(m);
                },
                |ui| {
                    ui_kit::primary_button(ui, "Share");
                },
            );
        }
    });
    harness.run();

    let clashes = id_clash_texts(&harness);
    assert!(clashes.is_empty(), "two toolbars must not report id clashes: {clashes:?}");
}

/// The right zone reads left to right (icon buttons, then the primary
/// button) and stays inside its zone at the window's right edge.
#[test]
fn the_right_zone_reads_content_then_sits_at_the_window_edge() {
    let undo = Rc::new(Cell::new(Rect::NOTHING));
    let share = Rc::new(Cell::new(Rect::NOTHING));
    let zone = Rc::new(Cell::new(Rect::NOTHING));
    let undo_watch = undo.clone();
    let share_watch = share.clone();
    let zone_watch = zone.clone();
    let mut harness = new_harness(move |ui| {
        let toolbar = ui_kit::main_toolbar(
            ui,
            |ui| {
                ui.label("Gallery");
            },
            |ui| {
                let mut m = 0;
                ui_kit::segmented(ui, "zones-modes", &mut m, &["Edit", "Mix"]);
            },
            |ui| {
                let response = ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE, "Undo");
                undo_watch.set(response.rect);
                let response = ui_kit::primary_button(ui, "Share");
                share_watch.set(response.rect);
            },
        );
        zone_watch.set(toolbar.right);
    });
    harness.run();

    let undo = undo.get();
    let share = share.get();
    let zone = zone.get();
    assert!(undo.is_positive() && share.is_positive(), "both right-zone widgets must be drawn");
    assert!(undo.right() < share.right(), "the icon button must come before the primary button, left to right");
    assert!(share.right() <= zone.right() + 0.5, "the primary button must end inside the right zone");
}

/// The left zone (app mark and menu bar) fits the 44px bar and stays
/// vertically centered.
#[test]
fn the_left_zone_fits_the_bar_centered() {
    let left = Rc::new(Cell::new(Rect::NOTHING));
    let bar = Rc::new(Cell::new(Rect::NOTHING));
    let left_watch = left.clone();
    let bar_watch = bar.clone();
    let mut harness = new_harness(move |ui| {
        let toolbar = ui_kit::main_toolbar(
            ui,
            |ui| {
                ui.add_space(ui_kit::tokens::space::SCALE_4);
                let mut menus = [ui_kit::Menu::new("File", |_| {}), ui_kit::Menu::new("View", |_| {})];
                ui_kit::menu_bar(ui, &mut menus);
            },
            |ui| {
                let mut m = 0;
                ui_kit::segmented(ui, "left-modes", &mut m, &["Edit", "Mix"]);
            },
            |ui| {
                ui_kit::primary_button(ui, "Share");
            },
        );
        left_watch.set(toolbar.left);
        bar_watch.set(toolbar.bar);
    });
    harness.run();

    let left = left.get();
    let bar = bar.get();
    assert!((bar.height() - ui_kit::tokens::size::TOOLBAR_MAIN_HEIGHT).abs() < 1e-3, "the bar must be the 44px token height, got {}", bar.height());
    assert!(left.height() <= bar.height(), "the left zone must fit the bar, got {} in {}", left.height(), bar.height());
    let drift = left.center().y - bar.center().y;
    assert!(drift.abs() <= 1.0, "the left zone must be vertically centered, off by {drift}px");
}

/// The toolbar keeps every zone inside the bar's bounds: nothing may be laid
/// out left of the bar (a negative x on the page) or past its right edge.
#[test]
fn the_toolbar_keeps_every_zone_inside_the_bar() {
    let zones = Rc::new(Cell::new(None));
    let zones_watch = zones.clone();
    let mut harness = new_harness(move |ui| {
        let toolbar = ui_kit::main_toolbar(
            ui,
            |ui| {
                ui.vertical(|ui| {
                    ui.label("Gallery");
                    ui.label("ui-kit 0.1.0");
                });
            },
            |ui| {
                let mut m = 0;
                ui_kit::segmented(ui, "bounds-modes", &mut m, &["Edit", "Mix", "Master"]);
            },
            |ui| {
                ui_kit::icon_button(ui, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE, "Undo");
                ui_kit::primary_button(ui, "Share");
            },
        );
        zones_watch.set(Some(toolbar));
    });
    harness.run();

    let zones = zones.take().expect("the toolbar zones must be reported");
    for (name, rect) in [("left", zones.left), ("center", zones.center), ("right", zones.right)] {
        assert!(rect.is_positive(), "the {name} zone must be laid out");
        assert!(rect.left() >= zones.bar.left() - 0.5, "the {name} zone must not start left of the bar");
        assert!(rect.right() <= zones.bar.right() + 0.5, "the {name} zone must not end past the bar");
    }
}
