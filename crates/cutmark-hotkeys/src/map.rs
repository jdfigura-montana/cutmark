//! The suite hotkey map: one JSON file listing the chord, owner app and action of every hotkey.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::chord::{Chord, ChordError};

/// Schema version written to the file.
const VERSION: u32 = 1;

/// Version assumed for a file that has no version field.
fn default_version() -> u32 {
    VERSION
}

/// Writes `text` to `tmp_path`, flushes it to disk, then renames it over `path`.
fn write_then_rename(tmp_path: &Path, path: &Path, text: &str) -> io::Result<()> {
    let mut file = File::create(tmp_path)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp_path, path)
}

/// Why the hotkey map could not be read, written or changed.
#[derive(Debug, Error)]
pub enum MapError {
    /// The file could not be read or written.
    #[error("hotkey file {}: {source}", .path.display())]
    Io {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// The file is not valid hotkey JSON.
    #[error("hotkey file {} is not valid: {source}", .path.display())]
    Json {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        source: serde_json::Error,
    },
    /// Another app already owns the chord.
    #[error("that chord is already owned by {owner}")]
    OwnedBy {
        /// Name of the app that owns the chord.
        owner: String,
    },
}

/// One line of the hotkey file: a chord, the app that owns it and the action it triggers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    /// The chord in canonical text form, such as "Ctrl+Alt+R".
    pub chord: String,
    /// The app that registers the chord.
    pub owner: String,
    /// The app's name for what the chord does.
    pub action: String,
}

/// A default that was skipped because another app already holds its chord.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The action whose default was skipped.
    pub action: String,
    /// The default chord, as given.
    pub chord: String,
    /// The app that holds the chord.
    pub held_by: String,
}

/// Every hotkey binding in the suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyMap {
    /// Schema version of the file.
    #[serde(default = "default_version")]
    pub version: u32,
    /// All bindings, for every app.
    #[serde(default)]
    pub bindings: Vec<Binding>,
}

impl Default for HotkeyMap {
    fn default() -> Self {
        Self { version: VERSION, bindings: Vec::new() }
    }
}

impl HotkeyMap {
    /// The suite-wide file, `hotkeys.json` in the Cutmark folder of the user's config directory.
    pub fn default_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("Cutmark").join("hotkeys.json"))
    }

    /// Reads the map. A missing file gives an empty map; a file that is not valid JSON is an error.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, MapError> {
        let path = path.as_ref();
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(MapError::Io { path: path.to_path_buf(), source });
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        serde_json::from_str(text).map_err(|source| MapError::Json { path: path.to_path_buf(), source })
    }

    /// Writes the map through a temp file in the same folder, then renames it over the target.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), MapError> {
        let path = path.as_ref();
        let io_error = |source: io::Error| MapError::Io { path: path.to_path_buf(), source };

        let mut text = serde_json::to_string_pretty(self).map_err(|source| MapError::Json { path: path.to_path_buf(), source })?;
        text.push('\n');

        if let Some(folder) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(folder).map_err(io_error)?;
        }
        let mut tmp_name = path.file_name().ok_or_else(|| io_error(io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name")))?.to_os_string();
        tmp_name.push(".tmp");
        let tmp_path = path.with_file_name(tmp_name);

        if let Err(source) = write_then_rename(&tmp_path, path, &text) {
            // Best effort: do not leave the temp file behind.
            let _ = fs::remove_file(&tmp_path);
            return Err(io_error(source));
        }
        Ok(())
    }

    /// Adds each `(action, chord)` default for `owner` that the owner has no binding for yet.
    ///
    /// A default whose chord another app holds is skipped and returned as a [`Conflict`].
    /// A default whose chord text does not parse is skipped. Existing bindings are never changed.
    pub fn ensure_defaults(&mut self, owner: &str, defaults: &[(&str, &str)]) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        for &(action, chord_text) in defaults {
            if self.has_binding(owner, action) {
                continue;
            }
            let Ok(chord) = chord_text.parse::<Chord>() else {
                continue;
            };
            if let Some(held_by) = self.holder_of(&chord, owner) {
                conflicts.push(Conflict { action: action.to_owned(), chord: chord_text.to_owned(), held_by });
                continue;
            }
            self.bindings.push(Binding { chord: chord.to_string(), owner: owner.to_owned(), action: action.to_owned() });
        }
        conflicts
    }

    /// Gives `chord` to `owner`'s `action`, replacing the owner's previous binding for that action.
    ///
    /// Refused with [`MapError::OwnedBy`], naming the owner, if another app holds the chord.
    pub fn assign(&mut self, owner: &str, action: &str, chord: &Chord) -> Result<(), MapError> {
        if let Some(holder) = self.holder_of(chord, owner) {
            return Err(MapError::OwnedBy { owner: holder });
        }
        let text = chord.to_string();
        match self.bindings.iter_mut().find(|b| b.owner == owner && b.action == action) {
            Some(binding) => binding.chord = text,
            None => self.bindings.push(Binding { chord: text, owner: owner.to_owned(), action: action.to_owned() }),
        }
        Ok(())
    }

    /// Removes `owner`'s binding for `action`. Returns whether there was one.
    pub fn unassign(&mut self, owner: &str, action: &str) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|b| !(b.owner == owner && b.action == action));
        self.bindings.len() != before
    }

    /// The `(action, chord)` pairs `owner` registers. Bindings whose chord does not parse are left out.
    pub fn bindings_for(&self, owner: &str) -> Vec<(String, Chord)> {
        self.bindings.iter().filter(|b| b.owner == owner).filter_map(|b| Some((b.action.clone(), b.chord.parse::<Chord>().ok()?))).collect()
    }

    /// Every binding, for any owner, whose chord text does not parse, with the reason.
    pub fn invalid_bindings(&self) -> Vec<(&Binding, ChordError)> {
        self.bindings.iter().filter_map(|b| b.chord.parse::<Chord>().err().map(|err| (b, err))).collect()
    }

    /// Whether `owner` already has a binding for `action`.
    fn has_binding(&self, owner: &str, action: &str) -> bool {
        self.bindings.iter().any(|b| b.owner == owner && b.action == action)
    }

    /// The name of an app other than `owner` that holds `chord`, if any.
    fn holder_of(&self, chord: &Chord, owner: &str) -> Option<String> {
        self.bindings.iter().filter(|b| b.owner != owner).find(|b| b.chord.parse::<Chord>().is_ok_and(|held| held == *chord)).map(|b| b.owner.clone())
    }
}
