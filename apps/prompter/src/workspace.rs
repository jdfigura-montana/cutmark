//! The script library and the scripts open in tabs. A tab keeps its text while it is open.
//! Unsaved text is saved when its tab is left or closed, and before anything that reads its
//! file.

use std::path::Path;
use std::time::{Duration, Instant};

use prompter_core::store::{MAX_OPEN_TABS, Result, SnapshotMeta, Sort, Store, StoreError};

use crate::autosave;
use crate::stats::WordCount;

/// A script open in a tab.
pub struct OpenScript {
    pub id: String,
    pub text: String,
    /// When the unsaved text last changed, or `None` with nothing to save.
    unsaved_since: Option<Instant>,
    /// The last save failed.
    save_failed: bool,
    /// Counts openings, so that each one gets its own editor state.
    pub serial: u64,
    pub count: WordCount,
    /// A running session is on this script, so its tab shows a Live badge. Nothing sets it
    /// until sessions exist (Phase 4).
    pub live: bool,
    /// When the tab was last made the active one. The lowest was used the longest ago.
    used: u64,
}

impl OpenScript {
    pub fn unsaved(&self) -> bool {
        self.unsaved_since.is_some()
    }
}

/// What [`Workspace::autosave`] did.
#[derive(Debug)]
pub enum Autosave {
    /// Nothing is unsaved, or every save that was due went through.
    Done,
    /// A save is due after this long.
    Waiting(Duration),
    /// A save failed and will be tried again. `first` is false for a repeat of a failure
    /// already reported.
    Failed { error: StoreError, first: bool },
}

pub struct Workspace {
    store: Store,
    /// In tab order. At most [`MAX_OPEN_TABS`].
    tabs: Vec<OpenScript>,
    /// The id of the tab that is shown. `None` only with no tabs.
    active: Option<String>,
    openings: u64,
    /// Counts the times a tab was made active, for [`OpenScript::used`].
    uses: u64,
}

impl Workspace {
    /// The library with the tabs it remembers open again.
    pub fn new(store: Store) -> Self {
        let mut workspace = Self { store, tabs: Vec::new(), active: None, openings: 0, uses: 0 };
        workspace.restore_tabs();
        workspace
    }

    /// A script that cannot be read gets no tab. With no remembered active tab, the first is
    /// shown. Tabs further left count as used longer ago.
    fn restore_tabs(&mut self) {
        let library = self.store.library();
        let (open, active) = (library.open_tabs.clone(), library.active_tab.clone());
        for id in open {
            match self.store.load(&id) {
                Ok(text) => self.add_tab(&id, text),
                Err(error) => log::warn!("a tab could not be restored: {error}"),
            }
        }
        if let Some(at) = active.and_then(|id| self.tab_at(&id)).or_else(|| (!self.tabs.is_empty()).then_some(0)) {
            self.show_tab(at);
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The tabs in order.
    pub fn tabs(&self) -> &[OpenScript] {
        &self.tabs
    }

    /// The active tab.
    pub fn open_script(&self) -> Option<&OpenScript> {
        self.active_at().and_then(|at| self.tabs.get(at))
    }

    pub fn open_script_mut(&mut self) -> Option<&mut OpenScript> {
        self.active_at().and_then(|at| self.tabs.get_mut(at))
    }

    /// The tab holding a script, if it has one.
    pub fn tab(&self, id: &str) -> Option<&OpenScript> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    #[cfg(test)]
    pub fn tab_mut(&mut self, id: &str) -> Option<&mut OpenScript> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// A script's title, or "" for an id that is not listed.
    pub fn title(&self, id: &str) -> &str {
        self.store.library().scripts.iter().find(|script| script.id == id).map_or("", |script| script.title.as_str())
    }

    /// Notes that the editor changed the active tab's text.
    pub fn text_changed(&mut self, now: Instant) {
        if let Some(open) = self.open_script_mut() {
            open.unsaved_since = Some(now);
            open.count.text_changed();
        }
    }

    /// Saves each tab whose text has rested long enough.
    pub fn autosave(&mut self, now: Instant) -> Autosave {
        let mut wait: Option<Duration> = None;
        let mut failure: Option<(StoreError, bool)> = None;
        for at in 0..self.tabs.len() {
            let Some(since) = self.tabs[at].unsaved_since else {
                continue;
            };
            let left = autosave::time_left(since, now);
            if !left.is_zero() {
                wait = Some(wait.map_or(left, |wait| wait.min(left)));
                continue;
            }
            let reported = self.tabs[at].save_failed;
            if let Err(error) = self.save_tab(at) {
                self.tabs[at].unsaved_since = Some(now);
                self.tabs[at].save_failed = true;
                // A failure not yet reported is the one to return.
                if failure.as_ref().is_none_or(|(_, first)| !first && !reported) {
                    failure = Some((error, !reported));
                }
            }
        }
        match (failure, wait) {
            (Some((error, first)), _) => Autosave::Failed { error, first },
            (None, Some(wait)) => Autosave::Waiting(wait),
            (None, None) => Autosave::Done,
        }
    }

    /// Saves all unsaved text now. Text that cannot be saved stays unsaved, and the first
    /// error is returned.
    pub fn flush(&mut self) -> Result<()> {
        let mut result = Ok(());
        for at in 0..self.tabs.len() {
            let saved = self.save_tab(at);
            result = result.and(saved);
        }
        result
    }

    /// Shows a script, in its tab if it has one, or else in a new tab at the end. With no room
    /// for one, the least recently used tab with nothing to save is closed. On an error the
    /// tabs stay as they are.
    pub fn open(&mut self, id: &str) -> Result<()> {
        if self.tab_at(id).is_none() {
            // Read first, so that a script that cannot be read costs no tab.
            let text = self.store.load(id)?;
            // So that the tab being left can be closed too, if it comes to that.
            self.save_active();
            self.make_room()?;
            self.add_tab(id, text);
        }
        self.activate(id);
        Ok(())
    }

    /// Shows another tab. The one being left is saved.
    pub fn activate(&mut self, id: &str) {
        if self.active.as_deref() == Some(id) {
            return;
        }
        let Some(at) = self.tab_at(id) else {
            return;
        };
        self.save_active();
        self.show_tab(at);
        self.remember_tabs();
    }

    /// Shows the tab after the active one, or the one before it, going round at the ends.
    pub fn cycle_tab(&mut self, forward: bool) {
        let Some(at) = self.active_at() else {
            return;
        };
        let count = self.tabs.len();
        let next = (at + if forward { 1 } else { count - 1 }) % count;
        let id = self.tabs[next].id.clone();
        self.activate(&id);
    }

    /// Saves a tab and closes it. One that cannot be saved stays open.
    pub fn close(&mut self, id: &str) -> Result<()> {
        let Some(at) = self.tab_at(id) else {
            return Ok(());
        };
        self.save_tab(at)?;
        self.remove_tab(at);
        self.remember_tabs();
        Ok(())
    }

    /// Makes an empty script and opens it.
    pub fn new_script(&mut self, title: &str) -> Result<()> {
        let meta = self.store.new(title, None)?;
        self.open(&meta.id)
    }

    /// Copies a script and opens the copy.
    pub fn duplicate(&mut self, id: &str) -> Result<()> {
        self.save_script(id)?;
        let meta = self.store.duplicate(id)?;
        self.open(&meta.id)
    }

    /// Deletes a script, closing its tab.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        // So that the copy the store keeps has the latest text.
        self.save_script(id)?;
        self.store.delete(id)?;
        if let Some(at) = self.tab_at(id) {
            self.remove_tab(at);
        }
        self.remember_tabs();
        Ok(())
    }

    /// Imports a file into `folder`, or into no folder, and opens the first script it made.
    /// Returns how many it made if the file was split.
    pub fn import(&mut self, source: &Path, folder: Option<&str>) -> Result<Option<usize>> {
        let imported = self.store.import(source)?;
        if let Some(folder) = folder {
            for script in &imported.scripts {
                self.store.move_to_folder(&script.id, Some(folder))?;
            }
        }
        if let Some(first) = imported.scripts.first() {
            self.open(&first.id)?;
        }
        Ok(imported.split.then_some(imported.scripts.len()))
    }

    pub fn export(&mut self, id: &str, dest: &Path) -> Result<()> {
        self.save_script(id)?;
        self.store.export_md(id, dest)
    }

    /// Keeps a copy of a script's text as it is now.
    pub fn snapshot(&mut self, id: &str) -> Result<()> {
        self.save_script(id)?;
        self.store.snapshot(id).map(|_| ())
    }

    /// Puts a snapshot's text back. The store keeps the current text as a snapshot first. An
    /// open script's tab keeps its place, and its editor starts over with the new text.
    pub fn restore(&mut self, id: &str, snapshot: SnapshotMeta) -> Result<()> {
        self.save_script(id)?;
        let result = self.store.restore(id, snapshot);
        // Even after a failure, the file may hold the new text.
        if let Some(at) = self.tab_at(id) {
            let text = self.store.load(id)?;
            if self.tabs[at].text != text {
                self.openings += 1;
                let tab = &mut self.tabs[at];
                (tab.text, tab.serial, tab.count) = (text, self.openings, WordCount::new());
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

    pub fn move_folder(&mut self, folder: &str, to_index: usize) -> Result<()> {
        self.store.move_folder(folder, to_index)
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<()> {
        self.store.set_favorite(id, favorite)
    }

    pub fn set_sort(&mut self, sort: Sort) -> Result<()> {
        self.store.set_sort(sort)
    }

    pub fn set_folder_collapsed(&mut self, folder: &str, collapsed: bool) -> Result<()> {
        self.store.set_folder_collapsed(folder, collapsed)
    }

    fn tab_at(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn active_at(&self) -> Option<usize> {
        self.active.as_deref().and_then(|id| self.tab_at(id))
    }

    /// Adds a tab at the end, without showing it.
    fn add_tab(&mut self, id: &str, text: String) {
        self.openings += 1;
        self.uses += 1;
        self.tabs.push(OpenScript {
            id: id.to_owned(),
            text,
            unsaved_since: None,
            save_failed: false,
            serial: self.openings,
            count: WordCount::new(),
            live: false,
            used: self.uses,
        });
    }

    /// Makes the tab at `at` the active one.
    fn show_tab(&mut self, at: usize) {
        self.uses += 1;
        self.tabs[at].used = self.uses;
        self.active = Some(self.tabs[at].id.clone());
    }

    /// Takes out a tab. If it was the active one, the tab that moves into its place is shown,
    /// or else the one before it.
    fn remove_tab(&mut self, at: usize) {
        let removed = self.tabs.remove(at);
        if self.active.as_deref() == Some(removed.id.as_str()) {
            self.active = None;
            if let Some(last) = self.tabs.len().checked_sub(1) {
                self.show_tab(at.min(last));
            }
        }
    }

    /// With every tab taken, closes the least recently used one that has nothing to save. If
    /// they all have, the least recently used one is saved and then closed.
    fn make_room(&mut self) -> Result<()> {
        if self.tabs.len() < MAX_OPEN_TABS {
            return Ok(());
        }
        let oldest = |saved_only: bool| {
            let tabs = self.tabs.iter().enumerate().filter(|(_, tab)| !saved_only || !tab.unsaved());
            tabs.min_by_key(|(_, tab)| tab.used).map(|(at, _)| at)
        };
        if let Some(at) = oldest(true) {
            self.remove_tab(at);
        } else if let Some(at) = oldest(false) {
            self.save_tab(at)?;
            self.remove_tab(at);
        }
        Ok(())
    }

    fn save_tab(&mut self, at: usize) -> Result<()> {
        let tab = &mut self.tabs[at];
        if tab.unsaved_since.is_some() {
            self.store.save(&tab.id, &tab.text)?;
            tab.unsaved_since = None;
            tab.save_failed = false;
        }
        Ok(())
    }

    /// Saves a script's unsaved text, if it is open and has any.
    fn save_script(&mut self, id: &str) -> Result<()> {
        match self.tab_at(id) {
            Some(at) => self.save_tab(at),
            None => Ok(()),
        }
    }

    /// Saves the active tab before another is shown. A failure leaves it unsaved, for
    /// autosave to try again and report.
    fn save_active(&mut self) {
        if let Some(at) = self.active_at()
            && let Err(error) = self.save_tab(at)
        {
            log::warn!("a tab was not saved when it was left: {error}");
        }
    }

    /// Writes the tabs to the library, for the next launch. They work without it, so a
    /// failure is only logged.
    fn remember_tabs(&mut self) {
        let open: Vec<String> = self.tabs.iter().map(|tab| tab.id.clone()).collect();
        let library = self.store.library();
        if library.open_tabs == open && library.active_tab == self.active {
            return;
        }
        if let Err(error) = self.store.set_tabs(&open, self.active.as_deref()) {
            log::warn!("the open tabs were not remembered: {error}");
        }
    }
}

#[cfg(test)]
mod tests;
