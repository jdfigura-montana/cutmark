//! Chords: modifiers plus one main key, with a canonical text form.

use std::fmt;
use std::str::FromStr;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use thiserror::Error;

/// Why a chord string could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ChordError {
    /// The text is empty.
    #[error("the chord is empty")]
    Empty,
    /// Two plus signs in a row, or a plus sign at either end.
    #[error("the chord {0:?} has an empty part")]
    EmptyPart(String),
    /// A part before the last one is not Ctrl, Alt, Shift or Win.
    #[error("unknown modifier {0:?}")]
    UnknownModifier(String),
    /// The same modifier appears twice.
    #[error("the modifier {0} is given twice")]
    DuplicateModifier(&'static str),
    /// The chord names only modifiers.
    #[error("the chord {0:?} has no key")]
    NoKey(String),
    /// The last part is not a supported key.
    #[error("unknown key {0:?}")]
    UnknownKey(String),
}

/// Declares the `Key` enum and its lookup tables from one list of keys.
macro_rules! keys {
    ($($variant:ident, $text:literal, $code:ident, $vk:literal;)*) => {
        /// The main key of a chord.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Key {
            $(
                #[doc = concat!("The ", $text, " key.")]
                $variant,
            )*
        }

        impl Key {
            /// Every supported key, in canonical order.
            pub const ALL: &'static [Key] = &[$(Key::$variant),*];

            /// Canonical text of the key.
            fn text(self) -> &'static str {
                match self {
                    $(Key::$variant => $text,)*
                }
            }

            /// Physical key code understood by the global hotkey manager.
            fn code(self) -> Code {
                match self {
                    $(Key::$variant => Code::$code,)*
                }
            }

            /// Windows virtual-key code of the key.
            #[cfg_attr(not(windows), allow(dead_code))]
            pub(crate) fn virtual_key(self) -> u16 {
                match self {
                    $(Key::$variant => $vk,)*
                }
            }
        }
    };
}

keys! {
    A, "A", KeyA, 0x41;
    B, "B", KeyB, 0x42;
    C, "C", KeyC, 0x43;
    D, "D", KeyD, 0x44;
    E, "E", KeyE, 0x45;
    F, "F", KeyF, 0x46;
    G, "G", KeyG, 0x47;
    H, "H", KeyH, 0x48;
    I, "I", KeyI, 0x49;
    J, "J", KeyJ, 0x4A;
    K, "K", KeyK, 0x4B;
    L, "L", KeyL, 0x4C;
    M, "M", KeyM, 0x4D;
    N, "N", KeyN, 0x4E;
    O, "O", KeyO, 0x4F;
    P, "P", KeyP, 0x50;
    Q, "Q", KeyQ, 0x51;
    R, "R", KeyR, 0x52;
    S, "S", KeyS, 0x53;
    T, "T", KeyT, 0x54;
    U, "U", KeyU, 0x55;
    V, "V", KeyV, 0x56;
    W, "W", KeyW, 0x57;
    X, "X", KeyX, 0x58;
    Y, "Y", KeyY, 0x59;
    Z, "Z", KeyZ, 0x5A;
    D0, "0", Digit0, 0x30;
    D1, "1", Digit1, 0x31;
    D2, "2", Digit2, 0x32;
    D3, "3", Digit3, 0x33;
    D4, "4", Digit4, 0x34;
    D5, "5", Digit5, 0x35;
    D6, "6", Digit6, 0x36;
    D7, "7", Digit7, 0x37;
    D8, "8", Digit8, 0x38;
    D9, "9", Digit9, 0x39;
    F1, "F1", F1, 0x70;
    F2, "F2", F2, 0x71;
    F3, "F3", F3, 0x72;
    F4, "F4", F4, 0x73;
    F5, "F5", F5, 0x74;
    F6, "F6", F6, 0x75;
    F7, "F7", F7, 0x76;
    F8, "F8", F8, 0x77;
    F9, "F9", F9, 0x78;
    F10, "F10", F10, 0x79;
    F11, "F11", F11, 0x7A;
    F12, "F12", F12, 0x7B;
    F13, "F13", F13, 0x7C;
    F14, "F14", F14, 0x7D;
    F15, "F15", F15, 0x7E;
    F16, "F16", F16, 0x7F;
    F17, "F17", F17, 0x80;
    F18, "F18", F18, 0x81;
    F19, "F19", F19, 0x82;
    F20, "F20", F20, 0x83;
    F21, "F21", F21, 0x84;
    F22, "F22", F22, 0x85;
    F23, "F23", F23, 0x86;
    F24, "F24", F24, 0x87;
    Space, "Space", Space, 0x20;
    Enter, "Enter", Enter, 0x0D;
    Tab, "Tab", Tab, 0x09;
    Escape, "Escape", Escape, 0x1B;
    Up, "Up", ArrowUp, 0x26;
    Down, "Down", ArrowDown, 0x28;
    Left, "Left", ArrowLeft, 0x25;
    Right, "Right", ArrowRight, 0x27;
    Home, "Home", Home, 0x24;
    End, "End", End, 0x23;
    PageUp, "PageUp", PageUp, 0x21;
    PageDown, "PageDown", PageDown, 0x22;
    Insert, "Insert", Insert, 0x2D;
    Delete, "Delete", Delete, 0x2E;
    Minus, "Minus", Minus, 0xBD;
    Equals, "Equals", Equal, 0xBB;
    LeftBracket, "LeftBracket", BracketLeft, 0xDB;
    RightBracket, "RightBracket", BracketRight, 0xDD;
    Backslash, "Backslash", Backslash, 0xDC;
    Semicolon, "Semicolon", Semicolon, 0xBA;
    Quote, "Quote", Quote, 0xDE;
    Comma, "Comma", Comma, 0xBC;
    Period, "Period", Period, 0xBE;
    Slash, "Slash", Slash, 0xBF;
    Backquote, "Backquote", Backquote, 0xC0;
}

/// Alternative spellings accepted when parsing a key, given in lower case.
fn key_alias(lower: &str) -> Option<Key> {
    Some(match lower {
        "esc" => Key::Escape,
        "return" => Key::Enter,
        "pgup" => Key::PageUp,
        "pgdn" => Key::PageDown,
        "ins" => Key::Insert,
        "del" => Key::Delete,
        "-" => Key::Minus,
        "=" => Key::Equals,
        "[" => Key::LeftBracket,
        "]" => Key::RightBracket,
        "\\" => Key::Backslash,
        ";" => Key::Semicolon,
        "'" => Key::Quote,
        "," => Key::Comma,
        "." => Key::Period,
        "/" => Key::Slash,
        "`" => Key::Backquote,
        _ => return None,
    })
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

impl FromStr for Key {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let token = s.trim();
        Key::ALL
            .iter()
            .copied()
            .find(|key| key.text().eq_ignore_ascii_case(token))
            .or_else(|| key_alias(&token.to_ascii_lowercase()))
            .ok_or_else(|| ChordError::UnknownKey(token.to_owned()))
    }
}

/// Modifiers plus one main key. Two chords are equal when they press the same keys, however they were typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Ctrl is held.
    pub ctrl: bool,
    /// Alt is held.
    pub alt: bool,
    /// Shift is held.
    pub shift: bool,
    /// The Windows key is held.
    pub win: bool,
    /// The main key.
    pub key: Key,
}

impl Chord {
    /// Builds the hotkey the global hotkey manager registers for this chord.
    pub fn to_global_hotkey(&self) -> HotKey {
        let mut modifiers = Modifiers::empty();
        if self.ctrl {
            modifiers |= Modifiers::CONTROL;
        }
        if self.alt {
            modifiers |= Modifiers::ALT;
        }
        if self.shift {
            modifiers |= Modifiers::SHIFT;
        }
        if self.win {
            modifiers |= Modifiers::META;
        }
        let modifiers = (!modifiers.is_empty()).then_some(modifiers);
        HotKey::new(modifiers, self.key.code())
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (held, name) in [(self.ctrl, "Ctrl"), (self.alt, "Alt"), (self.shift, "Shift"), (self.win, "Win")] {
            if held {
                f.write_str(name)?;
                f.write_str("+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().is_empty() {
            return Err(ChordError::Empty);
        }
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(ChordError::EmptyPart(s.to_owned()));
        }
        let (key_part, modifier_parts) = parts.split_last().ok_or(ChordError::Empty)?;

        let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
        for part in modifier_parts {
            let (flag, name) = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => (&mut ctrl, "Ctrl"),
                "alt" => (&mut alt, "Alt"),
                "shift" => (&mut shift, "Shift"),
                "win" => (&mut win, "Win"),
                _ => return Err(ChordError::UnknownModifier((*part).to_owned())),
            };
            if *flag {
                return Err(ChordError::DuplicateModifier(name));
            }
            *flag = true;
        }

        let lower = key_part.to_ascii_lowercase();
        if matches!(lower.as_str(), "ctrl" | "control" | "alt" | "shift" | "win") {
            return Err(ChordError::NoKey(s.to_owned()));
        }
        let key = key_part.parse::<Key>()?;
        Ok(Chord { ctrl, alt, shift, win, key })
    }
}
