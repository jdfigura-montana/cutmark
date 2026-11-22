//! ui::timeline::deck: the transport deck over the timeline panel: timecode,
//! back to start, Play (Pause while playing), Stop, Record, forward to the
//! end and the total length. Same editor actions as the old toolbar buttons;
//! the keyboard shortcuts live in app.rs.

use eframe::egui;
use ui_kit::PlayKind;
use ui_kit::icons::fill;
use ui_kit::tokens::{color, space, typography};

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

    let deck = ui_kit::transport_deck(ui, content_rect, egui::Id::new(DECK_ID), scrubbing, |ui| {
        ui_kit::timecode(ui, main, fraction);
        ui.add_space(space::SCALE_8);

        ui.add_enabled_ui(!recording, |ui| {
            if ui_kit::deck_button(ui, fill::SKIP_BACK, "Go to start (Home)").clicked() {
                editor.jump_start();
            }
        });
        // Play shows Pause while playing and then pauses; it plays otherwise,
        // and while a take runs it ends the take (the editor routes it).
        if ui_kit::play_button(ui, playing, PlayKind::Play).on_hover_text(if playing { "Pause (P)" } else { "Play (Space)" }).clicked() {
            if playing {
                editor.toggle_pause();
            } else {
                editor.toggle_play();
            }
        }
        if ui_kit::deck_button(ui, fill::STOP, "Stop").clicked() {
            editor.stop();
        }
        // Record starts a take when idle and ends it (keeping it) while
        // recording; a take that is already being saved ignores clicks.
        let saving = recording && !live;
        if ui_kit::play_button(ui, live, PlayKind::Record).on_hover_text("Record (Ctrl+R)").clicked() {
            if live {
                editor.stop_recording();
            } else if !saving && let Err(error) = editor.start_recording() {
                messages.push(Message::recording(error.to_string()));
            }
        }
        ui.add_enabled_ui(!recording, |ui| {
            if ui_kit::deck_button(ui, fill::SKIP_FORWARD, "Go to end (End)").clicked() {
                editor.jump_end();
            }
        });

        ui.add_space(space::SCALE_8);
        ui.label(egui::RichText::new(length).font(ui_kit::fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_TERTIARY));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timecode_splits_before_the_fraction() {
        assert_eq!(split_timecode("00:01:02.345"), ("00:01:02", ".345"));
        assert_eq!(split_timecode("00:01:02"), ("00:01:02", ""));
    }
}
