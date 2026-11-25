//! The script library and the one open script. Unsaved text is saved before anything that
//! reads the open script's file or leaves it.

use std::path::Path;
use std::time::Instant;

use prompter_core::store::{Result, SnapshotMeta, Store};

use crate::autosave;
use crate::stats::WordCount;

/// The script in the editor.
pub struct OpenScript {
    pub id: String,
    pub text: String,
    /// When the unsaved text last changed, or `None` with nothing to save.
    unsaved_since: Option<Instant>,
    /// Counts openings, so that each one gets its own editor state.
    pub serial: u64,
    pub count: WordCount,
}

impl OpenScript {
    pub fn unsaved(&self) -> bool {
        self.unsaved_since.is_some()
    }
}

/// What [`Workspace::autosave`] did.
#[derive(Debug)]
pub enum Autosave {
    /// Nothing is unsaved, or a save went through.
    Done,
    /// A save is due after this long.
    Waiting(std::time::Duration),
    /// A save failed and will be tried again. `first` is false for a repeat of a failure
    /// already reported.
    Failed { error: prompter_core::store::StoreError, first: bool },
}

pub struct Workspace {
    store: Store,
    open: Option<OpenScript>,
    openings: u64,
    /// The last autosave failed.
    failing: bool,
}

impl Workspace {
    pub fn new(store: Store) -> Self {
        Self { store, open: None, openings: 0, failing: false }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn open_script(&self) -> Option<&OpenScript> {
        self.open.as_ref()
    }

    pub fn open_script_mut(&mut self) -> Option<&mut OpenScript> {
        self.open.as_mut()
    }

    /// A script's title, or "" for an id that is not listed.
    pub fn title(&self, id: &str) -> &str {
        self.store.library().scripts.iter().find(|script| script.id == id).map_or("", |script| script.title.as_str())
    }

    /// Notes that the editor changed the open script's text.
    pub fn text_changed(&mut self, now: Instant) {
        if let Some(open) = &mut self.open {
            open.unsaved_since = Some(now);
            open.count.text_changed();
        }
    }

    /// Saves the open script if its text has rested long enough.
    pub fn autosave(&mut self, now: Instant) -> Autosave {
        let Some(since) = self.open.as_ref().and_then(|open| open.unsaved_since) else {
            return Autosave::Done;
        };
        let wait = autosave::time_left(since, now);
        if !wait.is_zero() {
            return Autosave::Waiting(wait);
        }
        match self.flush() {
            Ok(()) => Autosave::Done,
            Err(error) => {
                if let Some(open) = &mut self.open {
                    open.unsaved_since = Some(now);
                }
                let first = !self.failing;
                self.failing = true;
                Autosave::Failed { error, first }
            }
        }
    }

    /// Saves any unsaved text now. On an error the text stays unsaved.
    pub fn flush(&mut self) -> Result<()> {
        if let Some(open) = &mut self.open
            && open.unsaved_since.is_some()
        {
            self.store.save(&open.id, &open.text)?;
            open.unsaved_since = None;
            self.failing = false;
        }
        Ok(())
    }

    /// Opens a script in place of the open one, which stays open if it cannot be saved or the
    /// new one cannot be read.
    pub fn open(&mut self, id: &str) -> Result<()> {
        if self.open.as_ref().is_some_and(|open| open.id == id) {
            return Ok(());
        }
        self.flush()?;
        let text = self.store.load(id)?;
        self.openings += 1;
        self.open = Some(OpenScript { id: id.to_owned(), text, unsaved_since: None, serial: self.openings, count: WordCount::new() });
        Ok(())
    }

    /// Makes an empty script and opens it.
    pub fn new_script(&mut self, title: &str) -> Result<()> {
        self.flush()?;
        let meta = self.store.new(title, None)?;
        self.open(&meta.id)
    }

    /// Copies a script and opens the copy.
    pub fn duplicate(&mut self, id: &str) -> Result<()> {
        self.flush()?;
        let meta = self.store.duplicate(id)?;
        self.open(&meta.id)
    }

    /// Deletes a script, closing it if it is the open one.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        // So that the copy the store keeps has the latest text.
        self.flush()?;
        self.store.delete(id)?;
        if self.open.as_ref().is_some_and(|open| open.id == id) {
            self.open = None;
        }
        Ok(())
    }

    /// Imports a file and opens the first script it made. Returns how many it made if the file
    /// was split.
    pub fn import(&mut self, source: &Path) -> Result<Option<usize>> {
        self.flush()?;
        let imported = self.store.import(source)?;
        if let Some(first) = imported.scripts.first() {
            self.open(&first.id)?;
        }
        Ok(imported.split.then_some(imported.scripts.len()))
    }

    pub fn export(&mut self, id: &str, dest: &Path) -> Result<()> {
        self.flush()?;
        self.store.export_md(id, dest)
    }

    /// Keeps a copy of a script's text as it is now.
    pub fn snapshot(&mut self, id: &str) -> Result<()> {
        self.flush()?;
        self.store.snapshot(id).map(|_| ())
    }

    /// Puts a snapshot's text back. The store keeps the current text as a snapshot first. An
    /// open script is opened afresh, so its editor starts over with the new text.
    pub fn restore(&mut self, id: &str, snapshot: SnapshotMeta) -> Result<()> {
        self.flush()?;
        let result = self.store.restore(id, snapshot);
        // Even after a failure, the file may hold the new text.
        if self.open.as_ref().is_some_and(|open| open.id == id) {
            let text = self.store.load(id)?;
            if self.open.as_ref().is_some_and(|open| open.text != text) {
                self.open = None;
                self.open(id)?;
            }
        }
        result
    }

    pub fn rename(&mut self, id: &str, title: &str) -> Result<()> {
        self.store.rename(id, title)
    }

    pub fn move_to_folder(&mut self, id: &str, folder: Option<&str>) -> Result<()> {
        self.store.move_to_folder(id, folder)
    }

    pub fn create_folder(&mut self, folder: &str) -> Result<()> {
        self.store.create_folder(folder)
    }

    pub fn rename_folder(&mut self, from: &str, to: &str) -> Result<()> {
        self.store.rename_folder(from, to)
    }

    pub fn delete_folder(&mut self, folder: &str) -> Result<()> {
        self.store.delete_folder(folder)
    }
}
