//! Tests for the brand module (spec: Brand-Mark.md, sections 4 and 5): the
//! mark and wordmark draw for every app, and the title bar mark is 18 by 18.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui::{Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use ui_kit::brand::{self, App};

/// The sizes the mark and wordmark are drawn at.
const SIZES: [f32; 4] = [16.0, 18.0, 32.0, 64.0];

/// A harness with the ui-kit font set, which main_toolbar needs. The first pass
/// only installs the fonts.
fn new_harness(scale: f32, mut draw: impl FnMut(&mut egui::Ui) + 'static) -> Harness<'static> {
    let first_pass = Rc::new(Cell::new(true));
    let mut harness = Harness::builder().with_size(vec2(900.0, 700.0)).with_pixels_per_point(scale).build_ui(move |ui| {
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

#[test]
fn mark_and_wordmark_draw_for_every_app_and_size_on_dark_and_light() {
    for scale in [1.0, 1.5] {
        for dark in [true, false] {
            let drawn = Rc::new(Cell::new(0_u32));
            let count = drawn.clone();
            let mut harness = new_harness(scale, move |ui| {
                ui.visuals_mut().dark_mode = dark;
                let mut passed = 0;
                for app in App::ALL {
                    ui.horizontal(|ui| {
                        for size in SIZES {
                            let mark = brand::mark(ui, app, size).rect;
                            assert!((mark.size() - vec2(size, size)).length() < 0.01, "{app:?} mark at {size}: {mark:?}");
                            let word = brand::wordmark(ui, app, size).rect;
                            assert!((word.height() - size).abs() < 0.01, "{app:?} wordmark at {size}: {word:?}");
                            assert!(word.width() > 5.0 * size, "{app:?} wordmark is a wide lockup: {word:?}");
                            passed += 1;
                        }
                    });
                }
                count.set(passed);
            });
            harness.run();
            assert_eq!(drawn.get(), 16, "scale {scale}, dark {dark}");
        }
    }
}

#[test]
fn the_wordmark_is_read_as_the_app_name() {
    let harness = new_harness(1.0, |ui| {
        brand::wordmark(ui, App::Recorder, 32.0);
    });
    assert!(harness.query_by_label("CutMark Recorder").is_some());
}

#[test]
fn the_title_bar_mark_rect_is_18_by_18() {
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let seen = rect.clone();
    let _harness = new_harness(1.0, move |ui| {
        ui_kit::main_toolbar(ui, |ui| seen.set(brand::mark(ui, App::Audio, brand::TITLE_BAR_MARK).rect), |_| {}, |_| {});
    });
    assert_eq!(rect.get().size(), vec2(18.0, 18.0));
}
