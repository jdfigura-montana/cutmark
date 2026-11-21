//! kittest tests for the track area painters (spec: Design Language rev 3
//! "Track area"): the header column, the empty area, the row separator and the
//! track header fills.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::{Color32, Rect, Shape, pos2};
use egui_kittest::Harness;
use ui_kit::tokens::color;

/// The fills of every rectangle shape in a harness output, with its rect.
fn rect_fills(harness: &Harness<'static>) -> Vec<(Rect, Color32)> {
    fn walk(shape: &Shape, out: &mut Vec<(Rect, Color32)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| walk(shape, out)),
            Shape::Rect(rect) => out.push((rect.rect, rect.fill)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// A harness that runs `draw` with the ui painter.
fn paint(draw: impl Fn(&egui::Painter) + 'static) -> Harness<'static> {
    let mut harness = Harness::new_ui(move |ui| draw(ui.painter()));
    harness.run();
    harness
}

const AREA: Rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(210.0, 120.0));

/// The header column is one `bg.track.header` strip with a 1px
/// `stroke.track.header` divider on its right edge.
#[test]
fn the_header_column_is_one_strip_with_a_right_divider() {
    let harness = paint(|painter| ui_kit::paint_track_header_column(painter, AREA));
    let fills = rect_fills(&harness);
    assert!(fills.contains(&(AREA, color::BG_TRACK_HEADER)), "the column must fill its whole rect with bg.track.header");
    let divider = fills.iter().find(|(_, fill)| *fill == color::STROKE_TRACK_HEADER).expect("the column must paint a divider").0;
    assert_eq!(divider, Rect::from_min_max(pos2(AREA.right() - 1.0, AREA.top()), AREA.right_bottom()), "the divider is 1px wide on the right edge");
}

/// The empty area below the last track is `bg.track.empty`.
#[test]
fn the_empty_area_is_bg_track_empty() {
    let harness = paint(|painter| ui_kit::paint_track_area_empty(painter, AREA));
    assert!(rect_fills(&harness).contains(&(AREA, color::BG_TRACK_EMPTY)), "the area below the last track must fill with bg.track.empty");
}

/// Every track row ends in a 1px `stroke.track` line, selected or not.
#[test]
fn every_track_row_ends_in_a_stroke_track_line() {
    for (index, selected) in [(0, false), (1, false), (2, true)] {
        let harness = paint(move |painter| ui_kit::paint_track_row(painter, AREA, index, selected));
        let line = Rect::from_min_max(pos2(AREA.left(), AREA.bottom() - 1.0), AREA.right_bottom());
        assert!(rect_fills(&harness).contains(&(line, color::STROKE_TRACK)), "row {index} (selected {selected}) must end in a stroke.track line");
    }
}

/// The selected row keeps `bg.track.selected`; an odd row shades with the
/// .025 `bg.track.alt`.
#[test]
fn rows_keep_their_lane_fills() {
    let selected = paint(|painter| ui_kit::paint_track_row(painter, AREA, 0, true));
    assert!(rect_fills(&selected).contains(&(AREA, color::BG_TRACK_SELECTED)), "the selected row keeps bg.track.selected");
    let odd = paint(|painter| ui_kit::paint_track_row(painter, AREA, 1, false));
    assert!(rect_fills(&odd).contains(&(AREA, color::BG_TRACK_ALT)), "an odd row shades with bg.track.alt");
}

/// The track header fills with `bg.track.header`, or `bg.track.header.selected`
/// when selected, and ends in the `stroke.track` line.
#[test]
fn the_track_header_uses_the_header_tokens() {
    for (selected, expected) in [(false, color::BG_TRACK_HEADER), (true, color::BG_TRACK_HEADER_SELECTED)] {
        let first_pass = std::rc::Rc::new(std::cell::Cell::new(true));
        let mut harness = Harness::new_ui(move |ui| {
            if first_pass.get() {
                first_pass.set(false);
                ui.ctx().set_fonts(ui_kit::fonts::definitions());
                return;
            }
            ui_kit::track_header(ui, 1, "Vocals", selected, &mut [], None);
        });
        harness.run();
        let fills = rect_fills(&harness);
        assert!(fills.iter().any(|(_, fill)| *fill == expected), "a header (selected {selected}) must fill with its header token");
        assert!(fills.iter().any(|(_, fill)| *fill == color::STROKE_TRACK), "a header must end in a stroke.track line");
    }
}
