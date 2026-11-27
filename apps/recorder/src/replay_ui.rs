//! What the window shows for the replay buffer outside the deck, the status bar and the
//! tray: the Capture menu's two items and the replay toasts.
//!
//! See docs/suite/recorder-replay-buffer.md.

use egui::Ui;

use crate::app::{Chords, menu_button, menu_item_enabled};
use crate::engine::bus::CommandKind;
use crate::engine::controller::{ControllerState, ReplayEvent};
use crate::engine::stats::GIB;
use crate::profile_ui::take_label;

/// One toast waiting to be shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    /// The toast's text.
    pub text: String,
    /// The take its Review button opens, for a toast that has one.
    pub review: Option<String>,
}

/// The toast for one replay event.
fn notice(event: &ReplayEvent) -> Notice {
    match event {
        ReplayEvent::Saved(saved) => Notice { text: format!("Replay saved: {}", take_label(&saved.name)), review: Some(saved.name.clone()) },
        ReplayEvent::SaveFailed(reason) => Notice { text: format!("Replay could not be saved: {reason}"), review: None },
        ReplayEvent::RamWarning(warning) => {
            let gb = |bytes: u64| bytes as f64 / GIB as f64;
            let text = format!(
                "The replay buffer may use {:.1} GB of RAM, more than 25% of the {:.1} GB that is free",
                gb(warning.need_bytes),
                gb(warning.free_bytes)
            );
            Notice { text, review: None }
        }
    }
}

/// The toasts for the replay events raised after number `seen`, oldest first; `seen` moves
/// to the newest event.
pub(crate) fn new_notices(state: &ControllerState, seen: &mut u64) -> Vec<Notice> {
    let notices = state.replay_events.iter().filter(|note| note.seq > *seen).map(|note| notice(&note.event)).collect();
    *seen = state.replay_event_seq;
    notices
}

/// The Capture menu's replay items: Replay buffer, checked while the buffer is on, and Save
/// replay, enabled only then. Returns the command of the clicked item.
pub(crate) fn menu_items(ui: &mut Ui, on: bool, chords: &Chords) -> Option<CommandKind> {
    let mut clicked = None;
    if ui.add(menu_button("Replay buffer", &chords.replay_toggle).selected(on)).clicked() {
        clicked = Some(CommandKind::ReplayToggle);
    }
    if menu_item_enabled(ui, on, "Save replay", &chords.replay_save).clicked() {
        clicked = Some(CommandKind::ReplaySave);
    }
    clicked
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::rc::Rc;

    use egui::accesskit::Toggled;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::engine::controller::{ReplayRamWarning, ReplaySaved};

    /// The two menu items with the buffer on or off, and what their clicks asked for.
    fn menu(on: bool) -> (Harness<'static>, Rc<RefCell<Vec<CommandKind>>>) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let clicks = Rc::clone(&sent);
        let chords = Chords { replay_toggle: "Ctrl+Alt+B".to_owned(), replay_save: "Ctrl+Alt+S".to_owned(), ..Chords::default() };
        let first_pass = Cell::new(true);
        let mut harness = Harness::new_ui(move |ui| {
            // The fonts a style asks for are only there from the next pass on.
            if first_pass.replace(false) {
                ui_kit::apply(ui.ctx());
                return;
            }
            clicks.borrow_mut().extend(menu_items(ui, on, &chords));
        });
        harness.run();
        (harness, sent)
    }

    #[test]
    fn the_replay_menu_items_follow_the_buffer() {
        for on in [false, true] {
            let (mut harness, sent) = menu(on);
            let toggle = harness.get_by_label_contains("Replay buffer").accesskit_node();
            assert!(!toggle.is_disabled(), "the buffer can always be switched");
            assert_eq!(toggle.toggled() == Some(Toggled::True), on, "checked while on");
            assert_eq!(harness.get_by_label_contains("Save replay").accesskit_node().is_disabled(), !on);

            harness.get_by_label_contains("Save replay").click();
            harness.run();
            harness.get_by_label_contains("Replay buffer").click();
            harness.run();
            let expected: &[CommandKind] = if on { &[CommandKind::ReplaySave, CommandKind::ReplayToggle] } else { &[CommandKind::ReplayToggle] };
            assert_eq!(sent.borrow().as_slice(), expected);
        }
    }

    #[test]
    fn each_replay_event_becomes_one_toast_and_only_a_saved_one_offers_review() {
        let mut state = ControllerState::default();
        state.push_replay_event(ReplayEvent::Saved(ReplaySaved { take_path: PathBuf::from("show_take07.mkv"), name: "show_take07".to_owned() }));
        state.push_replay_event(ReplayEvent::SaveFailed("the disk is full".to_owned()));
        let mut seen = 0;
        assert_eq!(
            new_notices(&state, &mut seen),
            [
                Notice { text: "Replay saved: take07".to_owned(), review: Some("show_take07".to_owned()) },
                Notice { text: "Replay could not be saved: the disk is full".to_owned(), review: None },
            ]
        );
        assert!(new_notices(&state, &mut seen).is_empty(), "an event is shown once");

        state.push_replay_event(ReplayEvent::RamWarning(ReplayRamWarning { need_bytes: 3 * GIB, free_bytes: 8 * GIB }));
        assert_eq!(
            new_notices(&state, &mut seen),
            [Notice { text: "The replay buffer may use 3.0 GB of RAM, more than 25% of the 8.0 GB that is free".to_owned(), review: None }]
        );
    }
}
