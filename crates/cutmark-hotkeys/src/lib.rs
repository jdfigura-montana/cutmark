//! Cutmark suite hotkey map.
//!
//! One file, `hotkeys.json` in the Cutmark folder of the user's config directory, lists every
//! hotkey in the suite: the chord, the app that owns it and the action it triggers.
//!
//! - Each app registers only the chords it owns, as global hotkeys ([`Registrar`]).
//! - Giving an app a chord another app owns is refused, naming the owner ([`MapError::OwnedBy`]).
//! - A chord Windows refuses, because another program holds it, is reported back so the app can flag it.
//! - A chord that types a character with AltGr on the active keyboard layout earns a warning ([`altgr_conflict`]).

#![warn(missing_docs)]

mod chord;
mod map;
mod register;

pub use chord::{Chord, ChordError, Key};
pub use map::{Binding, Conflict, HotkeyMap, MapError};
pub use register::{Registrar, RegistrarError, altgr_conflict};
