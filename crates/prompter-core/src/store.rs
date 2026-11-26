//! Scripts on disk (docs/suite/prompter.md, "Data and settings" and "Import").
//!
//! The caller passes the Prompter's data folder; this module never works out where it is.
//! Under it are `library.json`, `scripts/{id}.md`, `scripts/.history/{id}/` and
//! `cache/{id}.idx`. A [`Store`] assumes it is the only one writing to its folder.

mod import;

pub(crate) use import::{decode, escape_plan};

use std::cmp::Reverse;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::convert::DocumentError;
use crate::index::WordIndex;
use crate::script;

/// The `version` written to a new `library.json`.
pub const LIBRARY_VERSION: u32 = 1;

/// How many snapshots are kept for each script.
pub const MAX_SNAPSHOTS: usize = 10;

/// How many tabs can be open at once.
pub const MAX_OPEN_TABS: usize = 12;

/// The timestamp in a history file's name. UTC, and in name order from oldest to newest.
const STAMP: &str = "%Y%m%dT%H%M%S%.3fZ";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("{} is not a valid library file: {source}", path.display())]
    Library { path: PathBuf, source: serde_json::Error },
    #[error("no script has the id {0}")]
    UnknownScript(String),
    #[error("there is no folder named {0}")]
    UnknownFolder(String),
    #[error("a folder named {0} already exists")]
    FolderExists(String),
    #[error("the folder {0} still holds scripts")]
    FolderNotEmpty(String),
    #[error("a title or folder name cannot be blank")]
    BlankName,
    #[error("that snapshot no longer exists")]
    UnknownSnapshot,
    #[error("no more than {MAX_OPEN_TABS} tabs can be open")]
    TooManyTabs,
    #[error("the script {0} cannot have two tabs")]
    RepeatedTab(String),
    #[error("the script {0} has no open tab")]
    TabNotOpen(String),
    #[error("{}: only .md, .txt, .html, .htm, .docx and .odt files can be imported", .0.display())]
    UnsupportedImport(PathBuf),
    #[error("This file is not UTF-8 or UTF-16. Save it as UTF-8 and import again.")]
    ImportEncoding,
    #[error(transparent)]
    Document(#[from] DocumentError),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// The contents of `library.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub version: u32,
    pub scripts: Vec<ScriptMeta>,
    /// Folder names, in the order the user gave them. Folders do not nest.
    #[serde(default)]
    pub folders: Vec<String>,
    /// The ids of the scripts open in tabs, in tab order. At most [`MAX_OPEN_TABS`].
    #[serde(default)]
    pub open_tabs: Vec<String>,
    /// One of `open_tabs`.
    #[serde(default)]
    pub active_tab: Option<String>,
    /// How script lists are sorted.
    #[serde(default)]
    pub sort: Sort,
    /// The names of the folders shown closed.
    #[serde(default)]
    pub collapsed_folders: Vec<String>,
    /// Fields this version does not know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: LIBRARY_VERSION,
            scripts: Vec::new(),
            folders: Vec::new(),
            open_tabs: Vec::new(),
            active_tab: None,
            sort: Sort::default(),
            collapsed_folders: Vec::new(),
            extra: Map::new(),
        }
    }
}

impl Library {
    /// Drops tab and collapsed folder entries that do not fit the rest of the library, such as
    /// ones edited by hand.
    fn tidy(&mut self) {
        let mut seen = HashSet::new();
        self.open_tabs.retain(|tab| self.scripts.iter().any(|script| script.id == *tab) && seen.insert(tab.clone()));
        self.open_tabs.truncate(MAX_OPEN_TABS);
        self.active_tab = self.active_tab.take().filter(|active| self.open_tabs.contains(active));
        let mut seen = HashSet::new();
        self.collapsed_folders.retain(|folder| self.folders.contains(folder) && seen.insert(folder.clone()));
    }
}

/// The order of a script list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    /// By title, ignoring case.
    #[default]
    Name,
    /// The most recently edited first.
    Edited,
}

/// Sorts a script list. Favorites come first in either order. Scripts that tie keep the order
/// they were given in.
pub fn sort_scripts(scripts: &mut [&ScriptMeta], sort: Sort) {
    match sort {
        Sort::Name => scripts.sort_by_cached_key(|script| (!script.favorite, script.title.to_lowercase())),
        Sort::Edited => scripts.sort_by_key(|script| (!script.favorite, Reverse(script.edited))),
    }
}

/// What `library.json` holds about one script. Its text is in `scripts/{id}.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptMeta {
    /// A version 7 UUID.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub folder: Option<String>,
    pub created: DateTime<Utc>,
    /// When the text last changed. Renaming and moving do not change it.
    pub edited: DateTime<Utc>,
    /// Favorites sort first.
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub last_position: Option<u32>,
    /// Fields this version does not know, written back as they were read.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ScriptMeta {
    fn create(title: &str, folder: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7().to_string(),
            title: title.to_owned(),
            folder,
            created: now,
            edited: now,
            favorite: false,
            last_position: None,
            extra: Map::new(),
        }
    }
}

/// One saved copy of a script's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// When it was taken, to the millisecond. No two of a script's snapshots share one.
    pub taken: DateTime<Utc>,
}

/// What an import made.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The new scripts, in the file's reading order.
    pub scripts: Vec<ScriptMeta>,
    /// True if the file was over the word limit and was split. The user should be warned.
    pub split: bool,
}

/// The script library in one data folder.
#[derive(Debug)]
pub struct Store {
    base: PathBuf,
    library: Library,
}

impl Store {
    /// Opens the library in `base`, creating the folders it needs. A missing `library.json`
    /// gives an empty library. One that cannot be read is an error and is left as it is.
    pub fn open(base: &Path) -> Result<Self> {
        let mut store = Self { base: base.to_path_buf(), library: Library::default() };
        for folder in [store.scripts_dir(), store.history_root(), store.cache_dir()] {
            fs::create_dir_all(&folder).map_err(io_error(&folder))?;
        }
        let path = store.library_path();
        match fs::read(&path) {
            Ok(bytes) => {
                store.library = serde_json::from_slice(&bytes).map_err(|source| StoreError::Library { path, source })?;
                store.library.tidy();
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(&path)(error)),
        }
        Ok(store)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    /// Makes an empty script.
    #[allow(clippy::new_ret_no_self, reason = "this makes a new script, not a new Store")]
    pub fn new(&mut self, title: &str, folder: Option<&str>) -> Result<ScriptMeta> {
        let title = name(title)?;
        let folder = self.existing_folder(folder)?;
        self.add(ScriptMeta::create(title, folder), "")
    }

    /// The script's Markdown.
    pub fn load(&self, id: &str) -> Result<String> {
        self.position(id)?;
        read_text(&self.script_path(id))
    }

    /// Replaces the script's Markdown.
    pub fn save(&mut self, id: &str, text: &str) -> Result<()> {
        let at = self.position(id)?;
        write_atomic(&self.script_path(id), text.as_bytes())?;
        self.change(|library| library.scripts[at].edited = Utc::now())
    }

    pub fn rename(&mut self, id: &str, title: &str) -> Result<()> {
        let at = self.position(id)?;
        let title = name(title)?;
        self.change(|library| title.clone_into(&mut library.scripts[at].title))
    }

    /// Moves a script into an existing folder, or out of all folders with `None`.
    pub fn move_to_folder(&mut self, id: &str, folder: Option<&str>) -> Result<()> {
        let at = self.position(id)?;
        let folder = self.existing_folder(folder)?;
        self.change(|library| library.scripts[at].folder = folder)
    }

    /// Marks a script as a favorite, or takes the mark off.
    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<()> {
        let at = self.position(id)?;
        self.change(|library| library.scripts[at].favorite = favorite)
    }

    /// Replaces the open tabs and says which one is active. Each tab is a listed script's id,
    /// with no repeats, and `active` must be one of them.
    pub fn set_tabs(&mut self, open: &[String], active: Option<&str>) -> Result<()> {
        if open.len() > MAX_OPEN_TABS {
            return Err(StoreError::TooManyTabs);
        }
        for (at, id) in open.iter().enumerate() {
            self.position(id)?;
            if open[..at].contains(id) {
                return Err(StoreError::RepeatedTab(id.clone()));
            }
        }
        if let Some(active) = active.filter(|active| !open.iter().any(|id| id == active)) {
            return Err(StoreError::TabNotOpen(active.to_owned()));
        }
        self.change(|library| {
            library.open_tabs = open.to_vec();
            library.active_tab = active.map(str::to_owned);
        })
    }

    pub fn set_sort(&mut self, sort: Sort) -> Result<()> {
        self.change(|library| library.sort = sort)
    }

    /// Shows a folder closed or open.
    pub fn set_folder_collapsed(&mut self, folder: &str, collapsed: bool) -> Result<()> {
        if !self.has_folder(folder) {
            return Err(StoreError::UnknownFolder(folder.to_owned()));
        }
        self.change(|library| {
            library.collapsed_folders.retain(|other| other != folder);
            if collapsed {
                library.collapsed_folders.push(folder.to_owned());
            }
        })
    }

    /// Copies a script's text, in the same folder. The copy starts with no favorite mark, last
    /// position or unknown fields.
    pub fn duplicate(&mut self, id: &str) -> Result<ScriptMeta> {
        let original = &self.library.scripts[self.position(id)?];
        let meta = ScriptMeta::create(&format!("{} copy", original.title), original.folder.clone());
        let text = self.load(id)?;
        self.add(meta, &text)
    }

    /// Takes a script out of the library and closes its tab. If that tab was active, no tab
    /// is. Its file moves to `scripts/.history/{id}/deleted-<timestamp>.md` and nothing is
    /// erased. An entry whose file is already gone is removed too.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        let at = self.position(id)?;
        let history = self.history_dir(id);
        fs::create_dir_all(&history).map_err(io_error(&history))?;
        let from = self.script_path(id);
        let mut taken = now_to_the_millisecond();
        // A rename would replace an older file of the same name.
        let mut to = history.join(format!("deleted-{}.md", stamp(taken)));
        while to.exists() {
            taken += TimeDelta::milliseconds(1);
            to = history.join(format!("deleted-{}.md", stamp(taken)));
        }
        let moved = match fs::rename(&from, &to) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(io_error(&from)(error)),
        };
        let removed = self.change(|library| {
            library.scripts.remove(at);
            library.open_tabs.retain(|tab| tab != id);
            if library.active_tab.as_deref() == Some(id) {
                library.active_tab = None;
            }
        });
        if removed.is_err() && moved {
            // The script is still listed, so put its file back. If that fails too, the text
            // stays in the history folder.
            let _ = fs::rename(&to, &from);
        }
        removed
    }

    pub fn create_folder(&mut self, folder: &str) -> Result<()> {
        let folder = name(folder)?;
        if self.has_folder(folder) {
            return Err(StoreError::FolderExists(folder.to_owned()));
        }
        self.change(|library| library.folders.push(folder.to_owned()))
    }

    /// Moves a folder so that it is at `to_index` in the folder list. An index past the end
    /// puts it last.
    pub fn move_folder(&mut self, folder: &str, to_index: usize) -> Result<()> {
        let Some(from) = self.library.folders.iter().position(|other| other == folder) else {
            return Err(StoreError::UnknownFolder(folder.to_owned()));
        };
        self.change(|library| {
            let moved = library.folders.remove(from);
            library.folders.insert(to_index.min(library.folders.len()), moved);
        })
    }

    /// Renames a folder. Its scripts move with it.
    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        let to = name(to)?;
        if !self.has_folder(from) {
            return Err(StoreError::UnknownFolder(from.to_owned()));
        }
        if from == to {
            return Ok(());
        }
        if self.has_folder(to) {
            return Err(StoreError::FolderExists(to.to_owned()));
        }
        self.change(|library| {
            let folders = library.folders.iter_mut().chain(&mut library.collapsed_folders);
            let used = library.scripts.iter_mut().filter_map(|script| script.folder.as_mut());
            for folder in folders.chain(used).filter(|folder| *folder == from) {
                to.clone_into(folder);
            }
        })
    }

    /// Deletes a folder that holds no scripts.
    pub fn delete_folder(&mut self, folder: &str) -> Result<()> {
        if !self.has_folder(folder) {
            return Err(StoreError::UnknownFolder(folder.to_owned()));
        }
        if self.library.scripts.iter().any(|script| script.folder.as_deref() == Some(folder)) {
            return Err(StoreError::FolderNotEmpty(folder.to_owned()));
        }
        self.change(|library| {
            library.folders.retain(|other| other != folder);
            library.collapsed_folders.retain(|other| other != folder);
        })
    }

    /// Saves a copy of the script's current text, then drops all but the newest
    /// [`MAX_SNAPSHOTS`].
    pub fn snapshot(&self, id: &str) -> Result<SnapshotMeta> {
        let text = self.load(id)?;
        let history = self.history_dir(id);
        fs::create_dir_all(&history).map_err(io_error(&history))?;
        let mut taken = now_to_the_millisecond();
        // The new one must sort as the newest, even within one millisecond or after the clock
        // was set back. Otherwise it could replace a snapshot or be dropped at once.
        if let Some(newest) = self.snapshots(id)?.first() {
            taken = taken.max(newest.taken + TimeDelta::milliseconds(1));
        }
        let snapshot = SnapshotMeta { taken };
        write_atomic(&self.snapshot_path(id, snapshot), text.as_bytes())?;
        for old in self.snapshots(id)?.into_iter().skip(MAX_SNAPSHOTS) {
            // The new snapshot is safe, so an old one that will not go is not a failure.
            let _ = fs::remove_file(self.snapshot_path(id, old));
        }
        Ok(snapshot)
    }

    /// The script's snapshots, newest first.
    pub fn snapshots(&self, id: &str) -> Result<Vec<SnapshotMeta>> {
        self.position(id)?;
        let history = self.history_dir(id);
        let entries = match fs::read_dir(&history) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error(&history)(error)),
        };
        let mut snapshots = Vec::new();
        for entry in entries {
            let file = entry.map_err(io_error(&history))?.file_name();
            // Other files, such as a deleted script's text, are not snapshots.
            if let Some(taken) = file.to_str().and_then(|file| file.strip_suffix(".md")).and_then(parse_stamp) {
                snapshots.push(SnapshotMeta { taken });
            }
        }
        snapshots.sort_by_key(|snapshot| Reverse(snapshot.taken));
        Ok(snapshots)
    }

    /// The text a snapshot holds.
    pub fn snapshot_text(&self, id: &str, snapshot: SnapshotMeta) -> Result<String> {
        self.position(id)?;
        let path = self.snapshot_path(id, snapshot);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(StoreError::UnknownSnapshot),
            Err(error) => Err(io_error(&path)(error)),
        }
    }

    /// Replaces the script's text with a snapshot's, after taking a snapshot of the current
    /// text.
    pub fn restore(&mut self, id: &str, snapshot: SnapshotMeta) -> Result<()> {
        // Read it first: the new snapshot may push this one past the cap.
        let text = self.snapshot_text(id, snapshot)?;
        self.snapshot(id)?;
        self.save(id, &text)
    }

    /// Writes a copy of the script's Markdown to `dest`, replacing any file there.
    pub fn export_md(&self, id: &str, dest: &Path) -> Result<()> {
        let text = self.load(id)?;
        write_atomic(dest, text.as_bytes())
    }

    /// The word index of `text`, the script's current Markdown. It comes from the cache if
    /// that was built from the same text by this version. Otherwise it is built and cached.
    ///
    /// The cache is only a shortcut: a damaged file is rebuilt, and a failure to write it is
    /// ignored.
    pub fn cached_index(&self, id: &str, text: &str) -> WordIndex {
        let parsed = script::parse(text);
        if !is_id(id) {
            return WordIndex::build(&parsed);
        }
        let path = self.cache_dir().join(format!("{id}.idx"));
        let cached = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<WordIndex>(&bytes).ok());
        if let Some(index) = cached.filter(|index| index.is_current(&parsed)) {
            return index;
        }
        let index = WordIndex::build(&parsed);
        if let Ok(bytes) = serde_json::to_vec(&index) {
            let _ = write_atomic(&path, &bytes);
        }
        index
    }

    /// Writes a new script's text, then lists it.
    fn add(&mut self, meta: ScriptMeta, text: &str) -> Result<ScriptMeta> {
        write_atomic(&self.script_path(&meta.id), text.as_bytes())?;
        self.change(|library| library.scripts.push(meta.clone()))?;
        Ok(meta)
    }

    /// Applies a change to the library and writes it. The library in memory changes only if
    /// the write works.
    fn change(&mut self, edit: impl FnOnce(&mut Library)) -> Result<()> {
        let mut library = self.library.clone();
        edit(&mut library);
        let path = self.library_path();
        let bytes = serde_json::to_vec_pretty(&library).map_err(|source| StoreError::Library { path: path.clone(), source })?;
        write_atomic(&path, &bytes)?;
        self.library = library;
        Ok(())
    }

    /// Where the script is in the library. Ids become file names, so one that is not a UUID is
    /// refused even if a hand-edited `library.json` lists it.
    fn position(&self, id: &str) -> Result<usize> {
        let found = self.library.scripts.iter().position(|script| script.id == id);
        found.filter(|_| is_id(id)).ok_or_else(|| StoreError::UnknownScript(id.to_owned()))
    }

    fn has_folder(&self, folder: &str) -> bool {
        self.library.folders.iter().any(|other| other == folder)
    }

    fn existing_folder(&self, folder: Option<&str>) -> Result<Option<String>> {
        match folder {
            Some(folder) if !self.has_folder(folder) => Err(StoreError::UnknownFolder(folder.to_owned())),
            other => Ok(other.map(str::to_owned)),
        }
    }

    fn library_path(&self) -> PathBuf {
        self.base.join("library.json")
    }

    fn scripts_dir(&self) -> PathBuf {
        self.base.join("scripts")
    }

    fn script_path(&self, id: &str) -> PathBuf {
        self.scripts_dir().join(format!("{id}.md"))
    }

    fn history_root(&self) -> PathBuf {
        self.scripts_dir().join(".history")
    }

    fn history_dir(&self, id: &str) -> PathBuf {
        self.history_root().join(id)
    }

    fn snapshot_path(&self, id: &str, snapshot: SnapshotMeta) -> PathBuf {
        self.history_dir(id).join(format!("{}.md", stamp(snapshot.taken)))
    }

    fn cache_dir(&self) -> PathBuf {
        self.base.join("cache")
    }
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> StoreError {
    let path = path.to_path_buf();
    move |source| StoreError::Io { path, source }
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(io_error(path))
}

/// Writes to a temp file next to `target`, then renames it over `target`, so a crash leaves
/// either the old file or the new one.
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<()> {
    let temp = target.with_file_name(format!(".{}.tmp", Uuid::now_v7()));
    let written = File::create_new(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        // The data must be on disk before the rename makes it the real file.
        file.sync_all()
    });
    written.and_then(|()| fs::rename(&temp, target)).map_err(|error| {
        let _ = fs::remove_file(&temp);
        io_error(target)(error)
    })
}

/// A trimmed title or folder name.
fn name(text: &str) -> Result<&str> {
    let text = text.trim();
    if text.is_empty() { Err(StoreError::BlankName) } else { Ok(text) }
}

/// True for a UUID in the hyphenated form, which is safe to use in a file name.
fn is_id(id: &str) -> bool {
    id.len() == 36 && Uuid::try_parse(id).is_ok()
}

fn now_to_the_millisecond() -> DateTime<Utc> {
    let now = Utc::now();
    DateTime::from_timestamp_millis(now.timestamp_millis()).unwrap_or(now)
}

fn stamp(time: DateTime<Utc>) -> String {
    time.format(STAMP).to_string()
}

/// The time in a name that [`stamp`] made. Any other name gives `None`.
fn parse_stamp(text: &str) -> Option<DateTime<Utc>> {
    let time = NaiveDateTime::parse_from_str(text, STAMP).ok()?.and_utc();
    (stamp(time) == text).then_some(time)
}
