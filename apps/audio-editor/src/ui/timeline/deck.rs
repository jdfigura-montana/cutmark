//! ui::timeline::deck: the transport deck over the timeline panel: timecode,
//! back to start, Play (Pause while playing), Stop, Record, forward to the
//! end and the total length. Same editor actions as the old toolbar buttons;
//! the keyboard shortcuts live in app.rs.

use eframe::egui::{self, Align, Layout, vec2};
use ui_kit::PlayKind;
use ui_kit::icons::fill;
use ui_kit::tokens::{color, size, space, typography};

use crate::editor::Editor;
use crate::editor::message::Message;
use crate::editor::transport::TransportState;
use crate::util::time::samples_to_timecode;

/// Id of the deck (its egui area and the saved sideways offset).
const DECK_ID: &str = "transport_deck";

/// Id of the deck rectangle drawn on the last frame in the egui context data.
#[cfg(test)]
const RECT_KEY: &str = "transport_deck_rect";

/// The deck rectangle as drawn on the last frame, or None before its first
/// draw. Test hook, so the F1 context test can point at the deck.
#[cfg(test)]
pub fn rect(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(RECT_KEY)))
}

/// True when the deck, as drawn on the last frame, covers `pos`. The pointer
/// handlers of the lanes use it to leave a press on the deck to the deck.
pub fn covers(ctx: &egui::Context, pos: egui::Pos2) -> bool {
    ctx.layer_id_at(pos) == Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::new(DECK_ID)))
}

/// Splits `HH:MM:SS.mmm` into the main part and the `.mmm` fraction.
fn split_timecode(timecode: &str) -> (&str, &str) {
    timecode.split_at(timecode.find('.').unwrap_or(timecode.len()))
}

/// Gaps between the deck's items (like the Recorder's deck): the timecode sits
/// 12 px from the deck's left edge and 12 px from the first button, and the
/// total length 12 px after the last button.
const DECK_GAP: f32 = space::SCALE_12;
/// Gap between the deck's buttons.
const DECK_BUTTON_GAP: f32 = space::SCALE_4;
/// Space after the total length; the space scale has no 14 px step.
const DECK_TAIL: f32 = 14.0;

/// What the deck shows this frame, read from the editor before drawing so the
/// layout can be drawn (and tested) without an editor.
struct DeckData<'a> {
    /// True while a take is recorded: back and forward are disabled.
    recording: bool,
    /// True while the take is live: the Record capsule shows Stop.
    live: bool,
    /// True while playback runs: Play shows Pause.
    playing: bool,
    /// The playhead timecode's main part.
    main: &'a str,
    /// The playhead timecode's `.mmm` fraction.
    fraction: &'a str,
    /// The project's total length as a timecode.
    length: &'a str,
}

/// What one deck control asks the editor to do when clicked.
#[derive(Debug, Clone, Copy)]
enum DeckAction {
    /// Back to the start.
    JumpStart,
    /// Play, or pause while playing.
    Play,
    /// Stop playback.
    Stop,
    /// Start a take, or end it (keeping it) while one is live.
    Record,
    /// Forward to the end.
    JumpEnd,
}

/// Draws the deck at the bottom center of `content_rect`; returns the
/// messages for the user (a Record click that could not start).
///
/// While a take is recorded the Play and Stop buttons and the Record button
/// end it, back and forward are disabled and the timecode shows the record
/// position. `scrubbing` fades the deck while the pointer scrubs the ruler
/// near it.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, content_rect: egui::Rect, scrubbing: bool) -> Vec<Message> {
    let mut messages = Vec::new();
    let recording = editor.is_recording();
    let live = editor.take_is_live();
    let playing = editor.transport_state() == TransportState::Playing && !recording;
    let rate = editor.project.sample_rate;
    let position = editor.record_playhead().unwrap_or(editor.view.playhead);
    let timecode = samples_to_timecode(position, rate);
    let length = samples_to_timecode(editor.project_length(), rate);
    let (main, fraction) = split_timecode(&timecode);
    let data = DeckData { recording, live, playing, main, fraction, length: &length };

    let deck = ui_kit::transport_deck(ui, content_rect, egui::Id::new(DECK_ID), scrubbing, |ui| {
        // Record starts a take when idle and ends it (keeping it) while
        // recording; a take that is already being saved ignores clicks.
        let saving = recording && !live;
        deck_contents(ui, &data, &mut |action| match action {
            DeckAction::JumpStart => editor.jump_start(),
            // Play pauses while playing and plays otherwise; while a take
            // runs it ends the take (the editor routes it).
            DeckAction::Play => {
                if playing {
                    editor.toggle_pause();
                } else {
                    editor.toggle_play();
                }
            }
            DeckAction::Stop => editor.stop(),
            DeckAction::Record => {
                if live {
                    editor.stop_recording();
                } else if !saving && let Err(error) = editor.start_recording() {
                    messages.push(Message::recording(error.to_string()));
                }
            }
            DeckAction::JumpEnd => editor.jump_end(),
        });
    });
    let deck_rect = deck.response.rect;
    #[cfg(test)]
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new(RECT_KEY), deck_rect));
    // F1 over the deck opens Playback and the transport; the tour spotlights
    // the deck. The tag sits on a ui as large as the deck itself, so it wins
    // over the whole timeline panel behind it.
    ui_kit::help::context(&ui.new_child(egui::UiBuilder::new().max_rect(deck_rect)), crate::help::CONTEXT_TRANSPORT);
    ui_kit::tour::anchor(ui, crate::help::ANCHOR_TRANSPORT, deck_rect);
    messages
}

/// The deck's items on one center line (like the Recorder's deck): everything
/// is laid out in one row the height of a deck button, so the timecode, the
/// buttons, the capsules and the total length all center on the deck's
/// center. The gaps are the deck's own (12 px left of the timecode and before
/// the first button and the total length, 4 px between the buttons, 14 px
/// after the total length), so egui's item spacing is off inside the row.
fn deck_contents(ui: &mut egui::Ui, d: &DeckData<'_>, send: &mut dyn FnMut(DeckAction)) {
    let height = size::TRANSPORT_BUTTON;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(DECK_GAP);
        let timecode = ui_kit::timecode(ui, d.main, d.fraction);
        note_deck_item(timecode.response.rect);
        ui.add_space(DECK_GAP);
        let back = ui.add_enabled_ui(!d.recording, |ui| ui_kit::deck_button(ui, fill::SKIP_BACK, "Go to start (Home)")).inner;
        if back.clicked() {
            send(DeckAction::JumpStart);
        }
        note_deck_item(back.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let play = ui_kit::play_button(ui, d.playing, PlayKind::Play).on_hover_text(if d.playing { "Pause (P)" } else { "Play (Space)" });
        if play.clicked() {
            send(DeckAction::Play);
        }
        note_deck_item(play.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let stop = ui_kit::deck_button(ui, fill::STOP, "Stop");
        if stop.clicked() {
            send(DeckAction::Stop);
        }
        note_deck_item(stop.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let record = ui_kit::play_button(ui, d.live, PlayKind::Record).on_hover_text("Record (Ctrl+R)");
        if record.clicked() {
            send(DeckAction::Record);
        }
        note_deck_item(record.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let forward = ui.add_enabled_ui(!d.recording, |ui| ui_kit::deck_button(ui, fill::SKIP_FORWARD, "Go to end (End)")).inner;
        if forward.clicked() {
            send(DeckAction::JumpEnd);
        }
        note_deck_item(forward.rect);
        ui.add_space(DECK_GAP);
        // The total length: the same mono size as the timecode and vertically
        // centered in the same row, so it sits on the timecode's baseline.
        let length = ui.label(egui::RichText::new(d.length).font(ui_kit::fonts::mono(typography::TIME_LARGE_SIZE)).color(color::TEXT_TERTIARY));
        note_deck_item(length.rect);
        ui.add_space(DECK_TAIL);
    });
}

// The rects of the deck's items, recorded for the center-line test (test hook).
#[cfg(test)]
thread_local! {
    static DECK_ITEM_RECTS: std::cell::RefCell<Vec<egui::Rect>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Notes one deck item's rect for the center-line test; a no-op outside tests.
#[cfg(not(test))]
fn note_deck_item(_: egui::Rect) {}

/// Notes one deck item's rect for the center-line test (test builds only).
#[cfg(test)]
fn note_deck_item(rect: egui::Rect) {
    DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().push(rect));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timecode_splits_before_the_fraction() {
        assert_eq!(split_timecode("00:01:02.345"), ("00:01:02", ".345"));
        assert_eq!(split_timecode("00:01:02"), ("00:01:02", ""));
    }

    /// The deck's items all sit on one center line (like the Recorder's
    /// `the_deck_items_sit_on_one_center_line`): in a headless frame, every
    /// deck item's rect center y is within 1 px of the deck's center y, idle
    /// and while a take is recorded.
    #[test]
    fn the_deck_items_sit_on_one_center_line() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        for recording in [false, true] {
            DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().clear());
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            let data = DeckData { recording, live: false, playing: false, main: "00:01:02", fraction: ".345", length: "00:03:04" };
            let mut deck_rect = egui::Rect::NOTHING;
            // Two frames: the first settles the deck area's rect, the second
            // is the frame the items are read from.
            let mut output = ctx.run_ui(input.clone(), |ui| {
                let deck = ui_kit::transport_deck(
                    ui,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0)),
                    egui::Id::new("deck-center-line"),
                    false,
                    |ui| {
                        deck_contents(ui, &data, &mut |_| {});
                    },
                );
                deck_rect = deck.response.rect;
            });
            output.textures_delta.clear();
            DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().clear());
            let mut output = ctx.run_ui(input, |ui| {
                let deck = ui_kit::transport_deck(
                    ui,
                    egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0)),
                    egui::Id::new("deck-center-line"),
                    false,
                    |ui| {
                        deck_contents(ui, &data, &mut |_| {});
                    },
                );
                deck_rect = deck.response.rect;
            });
            output.textures_delta.clear();
            let items = DECK_ITEM_RECTS.with(|slot| slot.borrow().clone());
            assert!(!items.is_empty(), "the deck drew its items");
            for rect in items {
                assert!((rect.center().y - deck_rect.center().y).abs() <= 1.0, "the deck item {rect:?} must sit on the deck's center line {deck_rect:?}");
            }
        }
    }
}
