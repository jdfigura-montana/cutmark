//! The Recorder's global hotkeys: defaults, registration, and turning key presses into commands.

use std::cell::RefCell;
use std::path::PathBuf;

use cutmark_hotkeys::{HotkeyMap, Registrar, altgr_conflict};
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};

use crate::engine::bus::{BusSender, Command, CommandKind, MarkKind};

/// The owner name the Recorder's bindings carry in the suite hotkey map.
const OWNER: &str = "recorder";

/// Track index the microphone is assumed to sit on until audio capture defines the track list.
const MIC_TRACK: usize = 0;

/// The Recorder's default `(action, chord)` pairs.
const DEFAULTS: [(&str, &str); 8] = [
    ("start_stop", "Ctrl+Alt+R"),
    ("pause_resume", "Ctrl+Alt+P"),
    ("mark", "F9"),
    ("mark_label", "Shift+F9"),
    ("highlight_last", "Ctrl+F9"),
    ("mute_mic", "Ctrl+Alt+M"),
    ("replay_toggle", "Ctrl+Alt+B"),
    ("replay_save", "Ctrl+Alt+S"),
];

/// A hotkey that Windows accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredHotkey {
    /// The action name, such as "start_stop".
    pub action: String,
    /// The chord in text form, such as "Ctrl+Alt+R".
    pub chord: String,
}

/// A hotkey that Windows refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedHotkey {
    /// The action name.
    pub action: String,
    /// The chord in text form.
    pub chord: String,
    /// Why it was refused.
    pub reason: String,
}

/// A chord that also types a character with AltGr on the active keyboard layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AltGrConflict {
    /// The action name.
    pub action: String,
    /// The chord in text form.
    pub chord: String,
    /// The character the chord types with AltGr.
    pub typed: char,
}

/// The registered hotkeys and the lists the UI shows about them. Key presses go straight to the controller's command bus.
pub struct Hotkeys {
    /// Hotkeys that are registered.
    pub registered: Vec<RegisteredHotkey>,
    /// Hotkeys Windows refused, for example because another program holds the chord.
    pub refused: Vec<RefusedHotkey>,
    /// Chords that clash with a character typed through AltGr.
    pub altgr: Vec<AltGrConflict>,
    /// Problems to show in a banner, such as an unreadable hotkey file.
    pub problems: Vec<String>,
}

impl Hotkeys {
    /// Loads the suite hotkey map, adds the Recorder's defaults, registers its chords and starts listening.
    ///
    /// Call it on the main thread, which runs the message loop that delivers hotkey events.
    /// A hotkey file that cannot be read is reported in `problems`, the defaults are used in memory and the file is not overwritten.
    pub fn start(bus: BusSender) -> Self {
        let mut problems = Vec::new();
        let (map, save_path) = load_map(&mut problems);
        let bindings = map.bindings_for(OWNER);

        let (registrar, refused) = match Registrar::new() {
            Ok(mut registrar) => {
                let refused = registrar.register(&bindings);
                (Some(registrar), refused)
            }
            Err(err) => {
                log::error!("{err}");
                problems.push(format!("Global hotkeys are unavailable: {err}"));
                (None, Vec::new())
            }
        };

        let mut registered = Vec::new();
        if registrar.is_some() {
            for (action, chord) in &bindings {
                if !refused.iter().any(|(refused_action, _, _)| refused_action == action) {
                    registered.push(RegisteredHotkey { action: action.clone(), chord: chord.to_string() });
                }
            }
        }
        let refused = refused.into_iter().map(|(action, chord, reason)| RefusedHotkey { action, chord: chord.to_string(), reason }).collect();
        let altgr = bindings
            .iter()
            .filter_map(|(action, chord)| Some(AltGrConflict { action: action.clone(), chord: chord.to_string(), typed: altgr_conflict(chord)? }))
            .collect();

        if let Some(err) = save_path.and_then(|path| map.save(path).err()) {
            log::error!("could not save the hotkey file: {err}");
            problems.push(format!("The hotkey file could not be saved: {err}"));
        }

        // The handler runs on the thread that pumps messages. It stamps the press with QPC right there and
        // sends the command straight to the controller, so hotkeys work while the window is hidden.
        // The registrar cannot move to another thread, and Windows delivers hotkey events on the thread that
        // registered them, so the handler reads it from a thread-local set on this thread.
        REGISTRAR.with(|slot| *slot.borrow_mut() = registrar);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state != HotKeyState::Pressed {
                return;
            }
            let press_qpc = capture::qpc_now();
            let Some(action) = REGISTRAR.with(|slot| slot.borrow().as_ref().and_then(|registrar| registrar.action_for(event.id).map(str::to_owned))) else {
                return;
            };
            let Some(kind) = command_for(&action) else {
                log::warn!("hotkey action {action} has no command");
                return;
            };
            log::info!("hotkey: {action}");
            if bus.send(Command::new(kind, press_qpc)).is_err() {
                log::error!("the command bus is closed, dropping hotkey {action}");
            }
        }));

        Self { registered, refused, altgr, problems }
    }
}

thread_local! {
    /// The registrations, kept alive on the thread that made them; the event handler looks hotkey ids up here.
    static REGISTRAR: RefCell<Option<Registrar>> = const { RefCell::new(None) };
}

/// Reads the suite hotkey map and adds the Recorder's defaults.
///
/// The second value is where to save the map, or `None` when the file could not be read and must be left alone.
fn load_map(problems: &mut Vec<String>) -> (HotkeyMap, Option<PathBuf>) {
    let path = HotkeyMap::default_path();
    let (mut map, save_path) = match path {
        None => {
            problems.push("The config folder could not be found, so the default hotkeys are used and not saved.".to_owned());
            (HotkeyMap::default(), None)
        }
        Some(path) => match HotkeyMap::load(&path) {
            Ok(map) => (map, Some(path)),
            Err(err) => {
                log::error!("{err}");
                problems.push(format!("The hotkey file could not be read, so the default hotkeys are used and the file is left alone: {err}"));
                (HotkeyMap::default(), None)
            }
        },
    };
    for conflict in map.ensure_defaults(OWNER, &DEFAULTS) {
        log::warn!("default {} for {} is already used by {}", conflict.chord, conflict.action, conflict.held_by);
        problems.push(format!("The default {} for {} is already used by {}, so it is not registered.", conflict.chord, conflict.action, conflict.held_by));
    }
    (map, save_path)
}

/// The command kind a hotkey action stands for, if the Recorder knows the action.
fn command_for(action: &str) -> Option<CommandKind> {
    Some(match action {
        "start_stop" => CommandKind::StartStop,
        "pause_resume" => CommandKind::PauseResume,
        "mark" => CommandKind::Mark(MarkKind::Silent),
        "mark_label" => CommandKind::Mark(MarkKind::WithLabel),
        "highlight_last" => CommandKind::Mark(MarkKind::HighlightLast),
        "mute_mic" => CommandKind::Mute { track: MIC_TRACK },
        "replay_toggle" => CommandKind::ReplayToggle,
        "replay_save" => CommandKind::ReplaySave,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_action_has_a_command() {
        for (action, _) in DEFAULTS {
            assert!(command_for(action).is_some(), "{action} has no command");
        }
    }
}
