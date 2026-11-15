//! Global hotkey registration for the chords one app owns, and the AltGr check.

use std::collections::HashMap;

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;
use thiserror::Error;

use crate::chord::Chord;

/// Why the hotkey manager could not be created.
#[derive(Debug, Error)]
pub enum RegistrarError {
    /// The operating system would not give the app a hotkey manager.
    #[error("could not start the global hotkey manager: {0}")]
    Manager(#[from] global_hotkey::Error),
}

/// Registers an app's own chords as global hotkeys and maps hotkey ids back to actions.
pub struct Registrar {
    manager: GlobalHotKeyManager,
    registered: Vec<HotKey>,
    actions: HashMap<u32, String>,
}

impl Registrar {
    /// Creates the manager. Call it on the thread that runs the app's message loop, which receives the hotkey events.
    pub fn new() -> Result<Self, RegistrarError> {
        Ok(Self { manager: GlobalHotKeyManager::new()?, registered: Vec::new(), actions: HashMap::new() })
    }

    /// Registers these `(action, chord)` pairs, replacing any set registered earlier.
    ///
    /// Returns `(action, chord, reason)` for every chord Windows refused, for example because
    /// another program holds it, so the app can flag it. The rest stay registered.
    pub fn register(&mut self, bindings: &[(String, Chord)]) -> Vec<(String, Chord, String)> {
        // Best effort: a hotkey that cannot be released is reported as refused below.
        let _ = self.manager.unregister_all(&self.registered);
        self.registered.clear();
        self.actions.clear();

        let mut refused = Vec::new();
        for (action, chord) in bindings {
            let hotkey = chord.to_global_hotkey();
            match self.manager.register(hotkey) {
                Ok(()) => {
                    self.registered.push(hotkey);
                    self.actions.insert(hotkey.id(), action.clone());
                }
                Err(err) => refused.push((action.clone(), *chord, refusal_reason(&err))),
            }
        }
        refused
    }

    /// The action registered under a hotkey event id, if any.
    pub fn action_for(&self, id: u32) -> Option<&str> {
        self.actions.get(&id).map(String::as_str)
    }
}

/// Plain-language reason for a refused registration.
fn refusal_reason(err: &global_hotkey::Error) -> String {
    match err {
        global_hotkey::Error::AlreadyRegistered(_) => "another binding of this app already uses it".to_owned(),
        global_hotkey::Error::FailedToRegister(detail) => {
            format!("Windows refused it, another program may hold it ({detail})")
        }
        other => other.to_string(),
    }
}

/// The character `chord` types on the active keyboard layout when Ctrl+Alt counts as AltGr, if any.
///
/// Only chords with both Ctrl and Alt can clash with AltGr. A warning is due when this returns a character.
#[cfg(windows)]
pub fn altgr_conflict(chord: &Chord) -> Option<char> {
    if !(chord.ctrl && chord.alt) {
        return None;
    }
    typed_with_ctrl_alt(u32::from(chord.key.virtual_key()), chord.shift)
}

/// The character `chord` types on the active keyboard layout when Ctrl+Alt counts as AltGr, if any.
///
/// Always `None` on targets other than Windows.
#[cfg(not(windows))]
pub fn altgr_conflict(_chord: &Chord) -> Option<char> {
    None
}

/// Asks the active layout what the virtual key types with Ctrl and Alt down.
#[cfg(windows)]
#[allow(unsafe_code)]
fn typed_with_ctrl_alt(virtual_key: u32, shift: bool) -> Option<char> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, MAPVK_VK_TO_VSC, MapVirtualKeyExW, ToUnicodeEx, VK_CONTROL, VK_MENU, VK_SHIFT};

    /// High bit of a key state byte: the key is down.
    const KEY_DOWN: u8 = 0x80;
    /// ToUnicodeEx flag: leave the keyboard state and any pending dead key untouched.
    const KEEP_STATE: u32 = 0x4;

    let mut state = [0u8; 256];
    for modifier in [VK_CONTROL, VK_MENU] {
        *state.get_mut(usize::from(modifier.0))? = KEY_DOWN;
    }
    if shift {
        *state.get_mut(usize::from(VK_SHIFT.0))? = KEY_DOWN;
    }

    let mut buffer = [0u16; 8];
    // SAFETY: these Win32 calls only read the key state array and write into the local buffer,
    // both of which outlive the calls. The layout handle comes from GetKeyboardLayout for this thread.
    let written = unsafe {
        let layout = GetKeyboardLayout(0);
        let scan_code = MapVirtualKeyExW(virtual_key, MAPVK_VK_TO_VSC, Some(layout));
        ToUnicodeEx(virtual_key, scan_code, &state, &mut buffer, KEEP_STATE, Some(layout))
    };
    if written == 0 {
        return None;
    }
    // A negative result is a dead key, whose character is still in the buffer.
    let typed = char::from_u32(u32::from(*buffer.first()?))?;
    // Ctrl+letter yields a control character, which is not typed text.
    (!typed.is_control()).then_some(typed)
}
