//! shortcuts: the app's one keyboard shortcut table (spec S1 Help "Shortcut
//! sheet"). The input handling, the menus and the generated sheet all read it,
//! so the keys can never drift apart. Key data only; no UI is drawn here.

use std::sync::LazyLock;

use eframe::egui::{Context, Key, KeyboardShortcut, Modifiers};
use ui_kit::shortcuts::{Shortcut, ShortcutTable};

/// The sheet group for the file commands.
const FILE: &str = "File";
/// The sheet group for the edit commands.
const EDIT: &str = "Edit";
/// The sheet group for the transport keys.
const PLAYBACK: &str = "Playback";
/// The sheet group for the marker and gesture keys.
const TIMELINE: &str = "Timeline";
/// The sheet group for the track keys.
const TRACKS: &str = "Tracks";
/// The sheet group for the zoom keys.
const VIEW: &str = "View";
/// The sheet group for the help keys.
const HELP: &str = "Help";

/// Context of a shortcut that works anywhere in the editor (the handler still
/// skips it while a text field has focus, an export locks the project or a take
/// is recorded).
const GLOBAL: &str = "global";
/// Context of a shortcut that only works over the timeline (the pointer must
/// hover it for the zoom keys).
const TIMELINE_ONLY: &str = "timeline";
/// Context of a key that only works while a take is recorded; every editor
/// shortcut is blocked then.
const RECORDING_BLOCKED: &str = "recording-blocked";
/// Context of a key that closes a hand-drawn context menu.
const MENU: &str = "menu";
/// Context of a key that commits or cancels an inline edit (a rename, the clip
/// gain editor).
const INLINE_EDIT: &str = "inline-edit";
/// Context of a key that confirms a dialog.
const DIALOG: &str = "dialog";

/// The app's shortcut table, built once on first use.
pub fn table() -> &'static ShortcutTable {
    static TABLE: LazyLock<ShortcutTable> = LazyLock::new(build);
    &TABLE
}

/// True when one of `id`'s keys was pressed this frame; the press is consumed,
/// so the shortcut fires once. An unknown id is always false.
pub fn pressed(ctx: &Context, id: &str) -> bool {
    table().pressed(ctx, id)
}

/// True when one of `id`'s keys went down this frame, without consuming it:
/// for the local checks whose old code left Esc or Enter for the handler behind
/// them (a menu closing on Esc still lets the same Esc clear the selection).
/// The modifiers match the way egui's own shortcut lookup matches them (an
/// extra Shift or Alt is ignored).
pub fn peek(ctx: &Context, id: &str) -> bool {
    table().get(id).is_some_and(|shortcut| {
        shortcut.keys.iter().any(|wanted| {
            ctx.input(|i| {
                i.events.iter().any(|event| {
                    matches!(
                        event,
                        eframe::egui::Event::Key { key, modifiers, pressed: true, .. }
                            if *key == wanted.logical_key && modifiers.matches_logically(wanted.modifiers)
                    )
                })
            })
        })
    })
}

/// The shortcut text a menu shows for `id` (its first key, in egui's
/// formatting). An unknown id is empty.
pub fn text(ctx: &Context, id: &str) -> String {
    table().text(ctx, id)
}

/// One entry with a single key.
fn one(id: &'static str, label: &'static str, group: &'static str, modifiers: Modifiers, key: Key, context: &'static str) -> Shortcut {
    Shortcut { id, label, group, keys: vec![KeyboardShortcut::new(modifiers, key)], context }
}

/// One entry with several keys; the first is the one menus show.
fn many(id: &'static str, label: &'static str, group: &'static str, keys: &[(Modifiers, Key)], context: &'static str) -> Shortcut {
    Shortcut { id, label, group, keys: keys.iter().map(|(modifiers, key)| KeyboardShortcut::new(*modifiers, *key)).collect(), context }
}
/// The table, in sheet order: File, Edit, Playback, Timeline, Tracks, View,
/// Help. Ctrl is `Modifiers::CTRL` (the app is Windows only), whose matching
/// in egui ignores an extra Shift or Alt, so entries that share a logical key
/// are checked most specific first in the input handling.
fn build() -> ShortcutTable {
    ShortcutTable::new(vec![
        // File.
        one("new_project", "New Project", FILE, Modifiers::CTRL, Key::N, GLOBAL),
        one("open_project", "Open Project", FILE, Modifiers::CTRL, Key::O, GLOBAL),
        one("save", "Save", FILE, Modifiers::CTRL, Key::S, GLOBAL),
        one("save_as", "Save As", FILE, Modifiers::CTRL | Modifiers::SHIFT, Key::S, GLOBAL),
        one("import_media", "Import Media", FILE, Modifiers::CTRL, Key::I, GLOBAL),
        one("export", "Export", FILE, Modifiers::CTRL, Key::E, GLOBAL),
        // Edit.
        one("undo", "Undo", EDIT, Modifiers::CTRL, Key::Z, GLOBAL),
        many("redo", "Redo", EDIT, &[(Modifiers::CTRL, Key::Y), (Modifiers::CTRL | Modifiers::SHIFT, Key::Z)], GLOBAL),
        one("cut", "Cut", EDIT, Modifiers::CTRL, Key::X, GLOBAL),
        one("copy", "Copy", EDIT, Modifiers::CTRL, Key::C, GLOBAL),
        one("paste", "Paste", EDIT, Modifiers::CTRL, Key::V, GLOBAL),
        many("delete_selection", "Delete", EDIT, &[(Modifiers::NONE, Key::Delete), (Modifiers::NONE, Key::Backspace)], GLOBAL),
        one("split_at_playhead", "Split at Playhead", EDIT, Modifiers::NONE, Key::S, GLOBAL),
        one("trim_to_selection", "Trim to Selection", EDIT, Modifiers::CTRL, Key::T, GLOBAL),
        one("select_all", "Select All", EDIT, Modifiers::CTRL, Key::A, GLOBAL),
        one("clear_selection", "Clear Selection", EDIT, Modifiers::NONE, Key::Escape, GLOBAL),
        one("toggle_ripple", "Ripple", EDIT, Modifiers::NONE, Key::R, GLOBAL),
        one("toggle_snap", "Snap", EDIT, Modifiers::NONE, Key::N, GLOBAL),
        one("process_gain", "Gain", EDIT, Modifiers::CTRL, Key::G, GLOBAL),
        one("process_normalize", "Normalize", EDIT, Modifiers::CTRL | Modifiers::SHIFT, Key::N, GLOBAL),
        one("fade_in", "Fade In", EDIT, Modifiers::CTRL | Modifiers::SHIFT, Key::I, GLOBAL),
        one("fade_out", "Fade Out", EDIT, Modifiers::CTRL | Modifiers::SHIFT, Key::O, GLOBAL),
        one("silence", "Silence", EDIT, Modifiers::CTRL, Key::L, GLOBAL),
        one("preferences", "Preferences", EDIT, Modifiers::CTRL, Key::Comma, GLOBAL),
        one("close_menu", "Close Menu", EDIT, Modifiers::NONE, Key::Escape, MENU),
        one("cancel_edit", "Cancel Edit", EDIT, Modifiers::NONE, Key::Escape, INLINE_EDIT),
        one("confirm_edit", "Confirm Edit", EDIT, Modifiers::NONE, Key::Enter, INLINE_EDIT),
        one("dialog_confirm", "Confirm Dialog", EDIT, Modifiers::NONE, Key::Enter, DIALOG),
        // Playback.
        one("play_pause", "Play / Pause", PLAYBACK, Modifiers::NONE, Key::Space, GLOBAL),
        one("play_selection", "Play Selection", PLAYBACK, Modifiers::SHIFT, Key::Space, GLOBAL),
        one("pause", "Pause", PLAYBACK, Modifiers::NONE, Key::P, GLOBAL),
        one("jump_to_start", "Jump to Start", PLAYBACK, Modifiers::NONE, Key::Home, GLOBAL),
        one("jump_to_end", "Jump to End", PLAYBACK, Modifiers::NONE, Key::End, GLOBAL),
        one("record", "Record", PLAYBACK, Modifiers::CTRL, Key::R, GLOBAL),
        one("record_stop_keep", "Stop and Keep the Take", PLAYBACK, Modifiers::NONE, Key::Space, RECORDING_BLOCKED),
        one("record_discard", "Stop and Discard the Take", PLAYBACK, Modifiers::NONE, Key::Escape, RECORDING_BLOCKED),
        // Timeline.
        one("add_marker", "Add Marker", TIMELINE, Modifiers::NONE, Key::M, GLOBAL),
        one("previous_marker", "Previous Marker", TIMELINE, Modifiers::NONE, Key::Comma, GLOBAL),
        one("next_marker", "Next Marker", TIMELINE, Modifiers::NONE, Key::Period, GLOBAL),
        one("cancel_gesture", "Cancel Gesture", TIMELINE, Modifiers::NONE, Key::Escape, TIMELINE_ONLY),
        // Tracks.
        one("add_track", "Add Track", TRACKS, Modifiers::CTRL | Modifiers::SHIFT, Key::T, GLOBAL),
        // View.
        many("zoom_in", "Zoom In", VIEW, &[(Modifiers::NONE, Key::Plus), (Modifiers::NONE, Key::Equals)], TIMELINE_ONLY),
        one("zoom_out", "Zoom Out", VIEW, Modifiers::NONE, Key::Minus, TIMELINE_ONLY),
        one("zoom_to_fit", "Zoom to Fit", VIEW, Modifiers::NONE, Key::Backslash, TIMELINE_ONLY),
        one("zoom_to_selection", "Zoom to Selection", VIEW, Modifiers::CTRL, Key::Backslash, TIMELINE_ONLY),
        // Help: reserved for phase 14d, not bound to an action yet.
        one("help_topics", "Help Topics", HELP, Modifiers::NONE, Key::F1, GLOBAL),
        one("keyboard_shortcuts", "Keyboard Shortcuts", HELP, Modifiers::CTRL, Key::Slash, GLOBAL),
    ])
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Ctrl and Shift together, as a const (the `|` operator is not const).
    const CTRL_SHIFT: Modifiers = Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: false };

    /// Every binding listed in inventory section 7, with the keys it lists
    /// there. An id that section lists more than once (Esc and Enter appear
    /// under several local areas, Ctrl+S, Space and M under the recording
    /// paragraph) is listed once, with the keys it has.
    const INVENTORY: &[(&str, &[(Modifiers, Key)])] = &[
        ("select_all", &[(Modifiers::CTRL, Key::A)]),
        ("import_media", &[(Modifiers::CTRL, Key::I)]),
        ("undo", &[(Modifiers::CTRL, Key::Z)]),
        ("redo", &[(Modifiers::CTRL, Key::Y), (CTRL_SHIFT, Key::Z)]),
        ("new_project", &[(Modifiers::CTRL, Key::N)]),
        ("open_project", &[(Modifiers::CTRL, Key::O)]),
        ("save_as", &[(CTRL_SHIFT, Key::S)]),
        ("export", &[(Modifiers::CTRL, Key::E)]),
        ("record", &[(Modifiers::CTRL, Key::R)]),
        ("add_track", &[(CTRL_SHIFT, Key::T)]),
        ("save", &[(Modifiers::CTRL, Key::S)]),
        ("delete_selection", &[(Modifiers::NONE, Key::Delete), (Modifiers::NONE, Key::Backspace)]),
        ("clear_selection", &[(Modifiers::NONE, Key::Escape)]),
        ("toggle_ripple", &[(Modifiers::NONE, Key::R)]),
        ("toggle_snap", &[(Modifiers::NONE, Key::N)]),
        ("split_at_playhead", &[(Modifiers::NONE, Key::S)]),
        ("cut", &[(Modifiers::CTRL, Key::X)]),
        ("copy", &[(Modifiers::CTRL, Key::C)]),
        ("paste", &[(Modifiers::CTRL, Key::V)]),
        ("trim_to_selection", &[(Modifiers::CTRL, Key::T)]),
        ("add_marker", &[(Modifiers::NONE, Key::M)]),
        ("previous_marker", &[(Modifiers::NONE, Key::Comma)]),
        ("next_marker", &[(Modifiers::NONE, Key::Period)]),
        ("process_gain", &[(Modifiers::CTRL, Key::G)]),
        ("process_normalize", &[(CTRL_SHIFT, Key::N)]),
        ("fade_in", &[(CTRL_SHIFT, Key::I)]),
        ("fade_out", &[(CTRL_SHIFT, Key::O)]),
        ("silence", &[(Modifiers::CTRL, Key::L)]),
        ("play_pause", &[(Modifiers::NONE, Key::Space)]),
        ("play_selection", &[(Modifiers::SHIFT, Key::Space)]),
        ("pause", &[(Modifiers::NONE, Key::P)]),
        ("jump_to_start", &[(Modifiers::NONE, Key::Home)]),
        ("jump_to_end", &[(Modifiers::NONE, Key::End)]),
        // While a take is recorded: Ctrl+S (the same save), Space, Esc and M.
        ("record_stop_keep", &[(Modifiers::NONE, Key::Space)]),
        ("record_discard", &[(Modifiers::NONE, Key::Escape)]),
        // Timeline keys.
        ("zoom_to_fit", &[(Modifiers::NONE, Key::Backslash)]),
        ("zoom_to_selection", &[(Modifiers::CTRL, Key::Backslash)]),
        ("zoom_in", &[(Modifiers::NONE, Key::Plus), (Modifiers::NONE, Key::Equals)]),
        ("zoom_out", &[(Modifiers::NONE, Key::Minus)]),
        // Local Esc and Enter handling.
        ("cancel_gesture", &[(Modifiers::NONE, Key::Escape)]),
        ("cancel_edit", &[(Modifiers::NONE, Key::Escape)]),
        ("close_menu", &[(Modifiers::NONE, Key::Escape)]),
        ("confirm_edit", &[(Modifiers::NONE, Key::Enter)]),
    ];

    /// The table has no two entries sharing a key in the same context.
    #[test]
    fn validate_reports_no_clash() {
        let problems = table().validate();
        assert!(problems.is_empty(), "the shortcut table has clashes: {problems:?}");
    }

    /// Every inventory section 7 binding is in the table with its keys.
    #[test]
    fn every_inventory_binding_is_in_the_table_with_its_keys() {
        let table = table();
        for (id, keys) in INVENTORY {
            let entry = table.get(id).unwrap_or_else(|| panic!("the table has no {id} entry"));
            let expected: Vec<KeyboardShortcut> = keys.iter().map(|(modifiers, key)| KeyboardShortcut::new(*modifiers, *key)).collect();
            assert_eq!(entry.keys, expected, "{id} has different keys than inventory section 7");
        }
    }
    /// Every entry has an id, a label and a group, and no id repeats.
    #[test]
    fn ids_are_unique_and_entries_are_labelled() {
        let table = table();
        for entry in table.entries() {
            assert!(!entry.id.is_empty() && !entry.label.is_empty() && !entry.group.is_empty(), "{entry:?} is missing its id, label or group");
            assert!(!entry.keys.is_empty(), "{} has no keys", entry.id);
            let repeats = table.entries().iter().filter(|other| other.id == entry.id).count();
            assert_eq!(repeats, 1, "{} is defined {repeats} times", entry.id);
        }
    }

    /// The table fills the sheet's seven groups, and only those.
    #[test]
    fn every_group_is_one_of_the_seven() {
        let groups: Vec<&str> = table().entries().iter().map(|entry| entry.group).collect();
        for group in &groups {
            assert!([FILE, EDIT, PLAYBACK, TIMELINE, TRACKS, VIEW, HELP].contains(group), "{group} is not one of the seven groups");
        }
        for group in [FILE, EDIT, PLAYBACK, TIMELINE, TRACKS, VIEW, HELP] {
            assert!(groups.contains(&group), "the {group} group is empty");
        }
    }

    /// The text a menu shows is the entry's first key.
    #[test]
    fn menu_text_is_the_first_key() {
        let ctx = Context::default();
        assert_eq!(text(&ctx, "undo"), "Ctrl+Z");
        assert_eq!(text(&ctx, "redo"), "Ctrl+Y", "the menu shows the first key");
        assert_eq!(text(&ctx, "help_topics"), "F1");
        assert_eq!(text(&ctx, "keyboard_shortcuts"), "Ctrl+Slash", "egui names the key, the menu shows it verbatim");
        assert_eq!(text(&ctx, "missing"), "", "an unknown id has no text");
    }

    /// The Help entries are reserved: their keys are set, nothing binds them.
    #[test]
    fn help_entries_are_reserved_with_their_keys() {
        let table = table();
        let help_topics = table.get("help_topics").expect("help_topics is reserved");
        assert_eq!(help_topics.group, HELP);
        assert_eq!(help_topics.keys, [KeyboardShortcut::new(Modifiers::NONE, Key::F1)]);
        let sheet = table.get("keyboard_shortcuts").expect("keyboard_shortcuts is reserved");
        assert_eq!(sheet.group, HELP);
        assert_eq!(sheet.keys, [KeyboardShortcut::new(Modifiers::CTRL, Key::Slash)]);
    }

    /// An egui input carrying one Esc key-down.
    fn esc() -> eframe::egui::RawInput {
        let mut input = eframe::egui::RawInput::default();
        input.events.push(eframe::egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
        input
    }

    /// `peek` sees a key without consuming it, so the handler behind it still
    /// fires; `pressed` consumes it, so a second entry does not.
    #[test]
    fn pressed_consumes_the_key_and_peek_does_not() {
        let ctx = Context::default();
        let (mut peeked, mut fired_after) = (false, false);
        let mut output = ctx.run_ui(esc(), |ui| {
            peeked = peek(ui.ctx(), "close_menu");
            fired_after = pressed(ui.ctx(), "clear_selection");
        });
        output.textures_delta.clear();
        assert!(peeked, "peek sees Esc");
        assert!(fired_after, "peek left Esc for the handler behind it");

        let (mut first, mut second) = (false, true);
        let mut output = ctx.run_ui(esc(), |ui| {
            first = pressed(ui.ctx(), "clear_selection");
            second = pressed(ui.ctx(), "close_menu");
        });
        output.textures_delta.clear();
        assert!(first, "the pressed entry fires");
        assert!(!second, "a consumed press cannot fire a second entry");
    }
}
