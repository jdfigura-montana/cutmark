//! kittest tests for the transport deck (spec "Components": row "Transport
//! deck"): the play capsule click and glyph switch, and the sideways drag
//! clamp.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use egui::{Id, Pos2, Rect, vec2};
use egui_kittest::{Harness, kittest::Queryable};

/// A harness with the ui-kit font set, so the named Inter families the
/// widgets lay text out in are bound. The first pass only installs the
/// fonts (egui applies a font change at the start of the next pass); the
/// widgets are drawn from the second pass on.
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

/// The play button reports clicks and switches its glyph with the playing
/// state (the accessible name Play/Pause follows the glyph).
#[test]
fn clicking_the_play_button_toggles_it_and_switches_the_glyph() {
    let clicked = Rc::new(Cell::new(false));
    let playing = Rc::new(Cell::new(false));
    let (clicked_state, playing_state) = (clicked.clone(), playing.clone());
    let mut harness = new_harness(move |ui| {
        if ui_kit::play_button(ui, playing_state.get(), ui_kit::PlayKind::Play).clicked() {
            clicked_state.set(true);
            playing_state.set(!playing_state.get());
        }
    });
    harness.run();

    harness.get_by_label("Play").click();
    harness.run();
    assert!(clicked.get(), "the play button must report the click");
    assert!(playing.get(), "the click must flip the playing state");

    clicked.set(false);
    harness.get_by_label("Pause").click();
    harness.run();
    assert!(clicked.get(), "the pause glyph must be clickable");
    assert!(!playing.get(), "the second click must flip the playing state back");
}

/// Dragging the deck far past the content edge keeps it inside the content
/// rect: the sideways offset clamps.
#[test]
fn a_large_drag_keeps_the_deck_inside_the_content_rect() {
    let deck_rect = Rc::new(Cell::new(Rect::NOTHING));
    let watch = deck_rect.clone();
    let mut harness = new_harness(move |ui| {
        let content_rect = ui.max_rect();
        let area = ui_kit::transport_deck(ui, content_rect, Id::new("transport"), false, |ui| {
            ui_kit::deck_button(ui, egui_phosphor::variants::fill::SKIP_BACK, "Go to start");
            ui_kit::play_button(ui, false, ui_kit::PlayKind::Play);
            ui_kit::deck_button(ui, egui_phosphor::variants::fill::SKIP_FORWARD, "Go to end");
        });
        watch.set(area.response.rect);
    });
    harness.run();

    // The deck's controls are 44 tall inside the 56 capsule, so the empty
    // deck is the 8px gap between the first deck button and the play capsule,
    // around x = 367 at the deck's resting spot. Press there and drag far
    // right.
    harness.drag_at(Pos2::new(367.0, 555.0));
    harness.hover_at(Pos2::new(5_000.0, 555.0));
    harness.drop_at(Pos2::new(5_000.0, 555.0));
    harness.run();
    harness.run();

    let deck = deck_rect.get();
    assert!(deck.right() > 700.0, "the drag must have moved the deck right, deck was {deck:?}");
    assert!(deck.left() >= 0.0, "the deck must stay inside the left edge, deck was {deck:?}");
    assert!(deck.right() <= 800.0, "the deck must stay inside the right edge, deck was {deck:?}");
}
