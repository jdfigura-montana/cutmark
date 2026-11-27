//! The chord filter: which key downs are keyboard shortcuts. A key down that is not a shortcut
//! gets no name, and the logger drops it at once, so nothing typed is kept.

use super::Modifiers;

const VK_F1: u16 = 0x70;
const VK_F24: u16 = 0x87;
const VK_NUMPAD0: u16 = 0x60;
const VK_NUMPAD9: u16 = 0x69;

/// The key's name when this key down is a shortcut: Ctrl, Alt or Win is held, or the key is F1 to
/// F24. `altgr` says that right Alt and left Ctrl are both down, which is how Windows reports
/// AltGr.
///
/// Modifier keys, injected characters (VK_PACKET), input method keys and unknown keys have no name,
/// so they are never logged.
pub(crate) fn chord_key_name(vk: u16, modifiers: Modifiers, altgr: bool) -> Option<&'static str> {
    let name = key_name(vk)?;
    if (VK_F1..=VK_F24).contains(&vk) {
        return Some(name);
    }
    if !(modifiers.ctrl || modifiers.alt || modifiers.win) {
        return None;
    }
    // AltGr types characters on many layouts, for example @ on a German one.
    if altgr && !modifiers.win {
        return None;
    }
    // Alt with numpad digits types a character by its code.
    if (VK_NUMPAD0..=VK_NUMPAD9).contains(&vk) && modifiers.alt && !modifiers.ctrl && !modifiers.win {
        return None;
    }
    Some(name)
}

/// The name of a virtual key. It names the key, not the character the layout would type.
fn key_name(vk: u16) -> Option<&'static str> {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const LETTERS: [&str; 26] =
        ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"];
    const NUMPAD: [&str; 10] = ["Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7", "Numpad8", "Numpad9"];
    const FUNCTION: [&str; 24] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23",
        "F24",
    ];
    let from_table = |table: &[&'static str], first: u16| table.get(usize::from(vk - first)).copied();
    match vk {
        0x30..=0x39 => from_table(&DIGITS, 0x30),
        0x41..=0x5A => from_table(&LETTERS, 0x41),
        VK_NUMPAD0..=VK_NUMPAD9 => from_table(&NUMPAD, VK_NUMPAD0),
        VK_F1..=VK_F24 => from_table(&FUNCTION, VK_F1),
        0x03 => Some("Break"),
        0x08 => Some("Backspace"),
        0x09 => Some("Tab"),
        0x0C => Some("Clear"),
        0x0D => Some("Enter"),
        0x13 => Some("Pause"),
        0x14 => Some("CapsLock"),
        0x1B => Some("Escape"),
        0x20 => Some("Space"),
        0x21 => Some("PageUp"),
        0x22 => Some("PageDown"),
        0x23 => Some("End"),
        0x24 => Some("Home"),
        0x25 => Some("Left"),
        0x26 => Some("Up"),
        0x27 => Some("Right"),
        0x28 => Some("Down"),
        0x2C => Some("PrintScreen"),
        0x2D => Some("Insert"),
        0x2E => Some("Delete"),
        0x5D => Some("Menu"),
        0x6A => Some("NumpadMultiply"),
        0x6B => Some("NumpadAdd"),
        0x6D => Some("NumpadSubtract"),
        0x6E => Some("NumpadDecimal"),
        0x6F => Some("NumpadDivide"),
        0x90 => Some("NumLock"),
        0x91 => Some("ScrollLock"),
        0xBA => Some("Semicolon"),
        0xBB => Some("Equals"),
        0xBC => Some("Comma"),
        0xBD => Some("Minus"),
        0xBE => Some("Period"),
        0xBF => Some("Slash"),
        0xC0 => Some("Backquote"),
        0xDB => Some("LeftBracket"),
        0xDC => Some("Backslash"),
        0xDD => Some("RightBracket"),
        0xDE => Some("Quote"),
        0xE2 => Some("IntlBackslash"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Modifiers, chord_key_name};

    const NONE: Modifiers = Modifiers { ctrl: false, alt: false, shift: false, win: false };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const ALT: Modifiers = Modifiers { alt: true, ..NONE };
    const SHIFT: Modifiers = Modifiers { shift: true, ..NONE };
    const WIN: Modifiers = Modifiers { win: true, ..NONE };

    fn name(vk: u16, modifiers: Modifiers) -> Option<&'static str> {
        chord_key_name(vk, modifiers, false)
    }

    #[test]
    fn shortcuts_are_chords() {
        assert_eq!(name(u16::from(b'S'), CTRL), Some("S"));
        assert_eq!(name(0x09, ALT), Some("Tab"));
        assert_eq!(name(u16::from(b'E'), WIN), Some("E"));
        assert_eq!(name(u16::from(b'4'), Modifiers { shift: true, ..CTRL }), Some("4"));
    }

    #[test]
    fn typing_is_not_a_chord() {
        assert_eq!(name(u16::from(b'A'), NONE), None);
        assert_eq!(name(u16::from(b'A'), SHIFT), None);
        // Every key without Ctrl, Alt or Win, except the function keys.
        for vk in (0..=0xFF).filter(|vk| !(0x70..=0x87).contains(vk)) {
            assert_eq!(name(vk, NONE), None, "key {vk:#x}");
            assert_eq!(name(vk, SHIFT), None, "Shift and key {vk:#x}");
        }
    }

    #[test]
    fn function_keys_are_chords_alone() {
        assert_eq!(name(0x74, NONE), Some("F5"));
        assert_eq!(name(0x70, NONE), Some("F1"));
        assert_eq!(name(0x87, SHIFT), Some("F24"));
    }

    #[test]
    fn modifier_and_character_keys_are_never_chords() {
        // Shift, Ctrl, Alt, both Win keys, the sided modifiers, input method, VK_PACKET, no key.
        for vk in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xE5, 0xE7, 0xFF] {
            assert_eq!(name(vk, Modifiers { ctrl: true, alt: true, shift: true, win: true }), None, "key {vk:#x}");
        }
    }

    #[test]
    fn characters_typed_with_altgr_or_alt_codes_are_dropped() {
        let ctrl_alt = Modifiers { alt: true, ..CTRL };
        assert_eq!(chord_key_name(u16::from(b'Q'), ctrl_alt, true), None);
        assert_eq!(chord_key_name(u16::from(b'Q'), ctrl_alt, false), Some("Q"));
        assert_eq!(chord_key_name(0x74, ctrl_alt, true), Some("F5"));
        assert_eq!(name(0x61, ALT), None);
        assert_eq!(name(0x61, CTRL), Some("Numpad1"));
    }
}
