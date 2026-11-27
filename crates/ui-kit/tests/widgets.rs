//! kittest tests for the core widgets (spec "Components").

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
        ui_kit::segmented(ui, "test-segments", &mut value, &["Video", "Audio"]);
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
        ui_kit::segmented(ui, "test-segments", &mut value, &["Video", "Audio"]);
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

/// Clicking the third tool strip cell selects its tool.
#[test]
fn clicking_the_third_tool_strip_cell_selects_it() {
    let selected = Rc::new(Cell::new(0usize));
    let state = selected.clone();
    let mut harness = new_harness(move |ui| {
        let mut value = state.get();
        ui_kit::tool_strip(
            ui,
            &mut value,
            &[
                (0, ui_kit::icons::regular::CURSOR, "Select"),
                (1, ui_kit::icons::regular::SCISSORS, "Razor"),
                (2, ui_kit::icons::regular::CROP, "Crop"),
                (3, ui_kit::icons::regular::TEXT_T, "Text"),
                (4, ui_kit::icons::regular::WAVE_SINE, "Fade"),
            ],
        );
        state.set(value);
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Crop").click();
    harness.run();
    assert_eq!(selected.get(), 2, "clicking the third cell must select its tool");
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

/// Dragging the slider across its track changes the value, staying in range.
#[test]
fn dragging_the_slider_changes_the_value_within_range() {
    let value = Rc::new(Cell::new(0.25f32));
    let state = value.clone();
    let mut harness = new_harness(move |ui| {
        let mut v = state.get();
        ui_kit::slider(ui, &mut v, 0.0..=1.0, ui_kit::SliderKind::Accent);
        state.set(v);
    });
    harness.run();

    let rect = harness.get_by_role(egui::accesskit::Role::Slider).rect();
    let start = egui::Pos2::new(rect.left() + rect.width() * 0.3, rect.center().y);
    let end = egui::Pos2::new(rect.right() - rect.width() * 0.1, rect.center().y);
    harness.drag_at(start);
    harness.run();
    harness.hover_at(end);
    harness.run();
    harness.drop_at(end);
    harness.run();

    let v = value.get();
    assert!((0.0..=1.0).contains(&v), "the dragged value must stay within the range, got {v}");
    assert!(v > 0.6, "dragging near the right end must raise the value, got {v}");
}

/// With the slider focused, the Right arrow steps the value by 1% of the range.
#[test]
fn the_right_arrow_steps_the_focused_slider() {
    let value = Rc::new(Cell::new(0.5f32));
    let state = value.clone();
    let mut harness = new_harness(move |ui| {
        let mut v = state.get();
        ui_kit::slider(ui, &mut v, 0.0..=1.0, ui_kit::SliderKind::Zoom);
        state.set(v);
    });
    harness.run();

    harness.get_by_role(egui::accesskit::Role::Slider).focus();
    harness.run();
    let before = value.get();
    harness.key_press(egui::Key::ArrowRight);
    harness.run();
    let after = value.get();
    assert!((after - before - 0.01).abs() < 1e-4, "the Right arrow must step the value by 1% of the range, moved {before} to {after}");
}

/// Typing into the text field updates the string.
#[test]
fn typing_into_the_text_field_updates_the_string() {
    let text = Rc::new(std::cell::RefCell::new(String::new()));
    let state = text.clone();
    let mut harness = new_harness(move |ui| {
        let mut value = state.borrow_mut();
        ui_kit::text_field(ui, &mut value, "Project name");
    });
    harness.run();

    harness.get_by_label("Project name").click();
    harness.run();
    harness.get_by_label("Project name").type_text("abc");
    harness.run();
    assert_eq!(text.borrow().as_str(), "abc", "typing must update the string");
}

/// Clicking a list row reports clicked.
#[test]
fn clicking_a_list_row_reports_clicked() {
    let (watch, state) = shared_bool();
    let mut harness = new_harness(move |ui| {
        if ui_kit::list_row(ui, false, Some(ui_kit::tokens::color::MARKER_CUT), "Interview take 3", Some("02:14")).clicked() {
            state.set(true);
        }
    });
    harness.run();
    harness.fit_contents();
    harness.run();

    harness.get_by_label("Interview take 3").click();
    harness.run();
    assert!(watch.get(), "the list row must report the click");
}

/// The main toolbar centers the center zone on the window midpoint, even when
/// the side zones have different widths (spec "Layout rules" item 3).
#[test]
fn the_main_toolbar_centers_the_center_zone_on_the_window() {
    let mode = Rc::new(Cell::new(0usize));
    let drift = Rc::new(Cell::new(f32::NAN));
    let mode_state = mode.clone();
    let drift_watch = drift.clone();
    let mut harness = new_harness(move |ui| {
        let zones = ui_kit::main_toolbar(
            ui,
            |ui| {
                ui_kit::icon_button(ui, "icon", "Undo");
                ui_kit::icon_button(ui, "icon", "Redo");
            },
            |ui| {
                let mut m = mode_state.get();
                ui_kit::segmented(ui, "toolbar-modes", &mut m, &["Edit", "Mix"]);
                mode_state.set(m);
            },
            |ui| {
                ui_kit::icon_button(ui, "icon", "Export");
                ui_kit::primary_button(ui, "Share");
            },
        );
        drift_watch.set(zones.center.center().x - zones.bar.center().x);
    });
    harness.run();

    let drift = drift.get();
    assert!(drift.is_finite(), "the toolbar zones must be reported");
    assert!(drift.abs() <= 1.0, "the center zone must sit on the window midpoint, off by {drift}px");
}

/// Every widget's on-screen height matches its size token (spec
/// "Spacing and sizes": primary button 32, icon button 32, chip 26,
/// segment 28 inside a track with 3 padding).
#[test]
fn the_widget_heights_match_their_size_tokens() {
    use ui_kit::tokens::size;

    let chip_on = Rc::new(Cell::new(false));
    let selected = Rc::new(Cell::new(0usize));
    let segment_state = selected.clone();
    let mut harness = new_harness(move |ui| {
        ui_kit::primary_button(ui, "Export");
        ui_kit::icon_button(ui, "icon", "Settings");
        let mut on = chip_on.get();
        ui_kit::chip(ui, &mut on, "Snap");
        chip_on.set(on);
        let mut segment = segment_state.get();
        ui_kit::segmented(ui, "size-tokens", &mut segment, &["Video", "Audio"]);
        segment_state.set(segment);
    });
    harness.run();

    let near = |measured: f32, token: f32| (measured - token).abs() < 1e-3;
    assert!(near(harness.get_by_label("Export").rect().height(), size::TOOLBAR_BUTTON), "the primary button must be {} tall", size::TOOLBAR_BUTTON);
    assert!(near(harness.get_by_label("Settings").rect().height(), size::ICON_BUTTON), "the icon button must be {} tall", size::ICON_BUTTON);
    assert!(near(harness.get_by_label("Snap").rect().height(), size::CHIP_HEIGHT), "the chip must be {} tall", size::CHIP_HEIGHT);
    assert!(near(harness.get_by_label("Video").rect().height(), size::SEGMENT_HEIGHT), "the segment must be {} tall", size::SEGMENT_HEIGHT);
    let track_padding = 3.0; // spec "Segmented control": track padding 3
    assert!(
        near(harness.get_by_label("Segmented control").rect().height(), size::SEGMENT_HEIGHT + 2.0 * track_padding),
        "the track must add {track_padding} padding around the segment"
    );
}

/// Height of the slider's thin track, for finding its rects in the painted shapes.
const SLIDER_TRACK_HEIGHT: f32 = 4.0;

/// The painted rectangles of the track and the fill: every 4 px high rect, in paint order.
fn slider_track_rects(harness: &Harness<'_>) -> Vec<egui::Rect> {
    fn walk(shape: &egui::Shape, into: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Vec(inner) => inner.iter().for_each(|shape| walk(shape, into)),
            egui::Shape::Rect(rect) if (rect.rect.height() - SLIDER_TRACK_HEIGHT).abs() < 1e-3 => into.push(rect.rect),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut found);
    }
    found
}

/// A bipolar slider fills from the middle of its range to the value, on both sides of the middle.
#[test]
fn the_bipolar_slider_fills_from_the_center() {
    for (value, from, to) in [(-0.5f32, 0.25f32, 0.5f32), (0.5, 0.5, 0.75)] {
        let mut harness = new_harness(move |ui| {
            let mut v = value;
            ui_kit::slider(ui, &mut v, -1.0..=1.0, ui_kit::SliderKind::Bipolar);
        });
        harness.run();
        let rects = slider_track_rects(&harness);
        assert_eq!(rects.len(), 2, "the track and the fill are painted: {rects:?}");
        let (track, fill) = (rects[0], rects[1]);
        let width = track.width();
        assert!((fill.left() - (track.left() + width * from)).abs() < 0.5, "the fill starts at {from} of the track for {value}: {fill:?} in {track:?}");
        assert!((fill.right() - (track.left() + width * to)).abs() < 0.5, "the fill ends at {to} of the track for {value}: {fill:?} in {track:?}");
    }
}

/// At the middle of its range a bipolar slider paints no fill at all.
#[test]
fn the_bipolar_slider_has_no_fill_at_the_center() {
    let mut harness = new_harness(|ui| {
        let mut v = 0.0f32;
        ui_kit::slider(ui, &mut v, -1.0..=1.0, ui_kit::SliderKind::Bipolar);
    });
    harness.run();
    assert_eq!(slider_track_rects(&harness).len(), 1, "only the track is painted at the center");
}

/// The text shapes of the last frame that read `text`.
fn texts_reading<'a>(harness: &'a Harness<'static>, text: &str) -> Vec<&'a egui::epaint::TextShape> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(shape) if shape.galley.text() == text => Some(shape),
            _ => None,
        })
        .collect()
}

/// A checkable menu item reports its state and draws the check only while on;
/// every shortcut ends at the same x; a disabled item draws at 40%.
#[test]
fn menu_items_draw_the_check_and_align_their_shortcuts() {
    use egui::accesskit::Toggled;
    use egui_kittest::kittest::NodeT;

    for on in [false, true] {
        let harness = new_harness(move |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                ui_kit::menu_check_slot(ui);
                ui.add(ui_kit::MenuItem::new("Replay buffer").shortcut("Ctrl+Alt+B").checked(on));
                ui.add_enabled(false, ui_kit::MenuItem::new("Save").shortcut("S"));
            });
        });
        let node = harness.get_by_label("Replay buffer").accesskit_node();
        assert_eq!(node.toggled() == Some(Toggled::True), on, "reports checked while on");
        assert_eq!(texts_reading(&harness, ui_kit::icons::regular::CHECK).len(), usize::from(on), "the check mark");

        let right = |text: &str| {
            let shapes = texts_reading(&harness, text);
            assert_eq!(shapes.len(), 1, "{text}");
            shapes[0].pos.x + shapes[0].galley.size().x
        };
        assert!((right("Ctrl+Alt+B") - right("S")).abs() < 0.5, "shortcuts share one right edge");

        let left = |text: &str| texts_reading(&harness, text)[0].pos.x;
        assert!((left("Replay buffer") - left("Save")).abs() < 0.5, "labels line up");
        assert_eq!(texts_reading(&harness, "Save")[0].fallback_color.a(), 102, "disabled at 40%");
    }
}
