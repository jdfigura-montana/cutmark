use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// A data folder of its own, removed afterwards.
struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let name = format!("cutmark-prompter-tabs-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed));
        Self(std::env::temp_dir().join(name))
    }

    fn workspace(&self) -> Workspace {
        Workspace::new(Store::open(&self.0).unwrap())
    }

    /// Puts a folder where a script's file goes, so that a save over it fails.
    fn block(&self, id: &str) {
        let file = self.0.join("scripts").join(format!("{id}.md"));
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        // A folder left behind in the temp folder does no harm.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opens a new script for each title, in order, and returns their ids.
fn add(workspace: &mut Workspace, titles: &[&str]) -> Vec<String> {
    let added = titles.iter().map(|title| {
        workspace.new_script(title).unwrap();
        workspace.open_script().unwrap().id.clone()
    });
    added.collect()
}

/// A workspace with tabs "1" to "12", the last one active.
fn full(folder: &Folder) -> (Workspace, Vec<String>) {
    let mut workspace = folder.workspace();
    let titles: Vec<String> = (1..=12).map(|number| number.to_string()).collect();
    let ids = add(&mut workspace, &titles.iter().map(String::as_str).collect::<Vec<_>>());
    (workspace, ids)
}

fn tabs(workspace: &Workspace) -> Vec<&str> {
    workspace.tabs().iter().map(|tab| workspace.title(&tab.id)).collect()
}

fn active(workspace: &Workspace) -> Option<&str> {
    workspace.open_script().map(|open| workspace.title(&open.id))
}

/// Edits a script that cannot be saved, so that its tab stays unsaved.
fn leave_unsaved(folder: &Folder, workspace: &mut Workspace, id: &str) {
    workspace.activate(id);
    folder.block(id);
    workspace.open_script_mut().unwrap().text.push_str("more");
    workspace.text_changed(Instant::now());
}

#[test]
fn a_thirteenth_tab_closes_the_least_recently_used_one() {
    let folder = Folder::new();
    let (mut workspace, ids) = full(&folder);
    // Opened first, but shown again since. "2" is now the one used longest ago.
    workspace.activate(&ids[0]);
    workspace.activate(&ids[11]);
    add(&mut workspace, &["13"]);
    assert_eq!(tabs(&workspace), ["1", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13"]);
    assert_eq!(active(&workspace), Some("13"));
    assert_eq!(workspace.store().library().open_tabs.len(), 12);
    // The script is only closed, and showing an open one closes nothing.
    assert_eq!(workspace.store().library().scripts.len(), 13);
    workspace.open(&ids[2]).unwrap();
    assert_eq!(workspace.tabs().len(), 12);
}

#[test]
fn eviction_passes_over_a_tab_with_a_pending_save() {
    let folder = Folder::new();
    let (mut workspace, ids) = full(&folder);
    leave_unsaved(&folder, &mut workspace, &ids[0]);
    // Leaving it fails to save it, and everything else is shown after it.
    for id in &ids[1..] {
        workspace.activate(id);
    }
    add(&mut workspace, &["13"]);
    assert_eq!(tabs(&workspace)[..3], ["1", "3", "4"]);
    assert!(workspace.tab(&ids[0]).unwrap().unsaved());
}

#[test]
fn with_nothing_but_unsaved_tabs_that_cannot_be_saved_nothing_is_closed() {
    let folder = Folder::new();
    let (mut workspace, ids) = full(&folder);
    for id in &ids {
        leave_unsaved(&folder, &mut workspace, id);
    }
    let before: Vec<String> = workspace.tabs().iter().map(|tab| tab.id.clone()).collect();
    assert!(workspace.new_script("13").is_err());
    let after: Vec<String> = workspace.tabs().iter().map(|tab| tab.id.clone()).collect();
    assert_eq!(after, before);
    assert_eq!(active(&workspace), Some("12"));
}

#[test]
fn closing_shows_a_neighbour_and_a_reopened_script_goes_to_the_end() {
    let folder = Folder::new();
    let mut workspace = folder.workspace();
    let ids = add(&mut workspace, &["a", "b", "c", "d"]);

    // Closing a tab that is not shown changes only the list.
    workspace.close(&ids[0]).unwrap();
    assert_eq!((tabs(&workspace), active(&workspace)), (vec!["b", "c", "d"], Some("d")));
    // With no tab to the right, the one to the left is shown. Else the one to the right.
    workspace.close(&ids[3]).unwrap();
    assert_eq!(active(&workspace), Some("c"));
    workspace.activate(&ids[1]);
    workspace.close(&ids[1]).unwrap();
    assert_eq!(active(&workspace), Some("c"));

    workspace.open(&ids[3]).unwrap();
    workspace.open(&ids[0]).unwrap();
    assert_eq!((tabs(&workspace), active(&workspace)), (vec!["c", "d", "a"], Some("a")));
    // Opening a script that has a tab leaves the order alone.
    workspace.open(&ids[2]).unwrap();
    assert_eq!((tabs(&workspace), active(&workspace)), (vec!["c", "d", "a"], Some("c")));

    for id in &ids {
        workspace.close(id).unwrap();
    }
    assert_eq!((tabs(&workspace), active(&workspace)), (vec![], None));
    assert!(workspace.store().library().open_tabs.is_empty());
}

#[test]
fn the_next_and_previous_tabs_go_round() {
    let folder = Folder::new();
    let mut workspace = folder.workspace();
    workspace.cycle_tab(true);
    add(&mut workspace, &["a", "b", "c"]);
    let mut step = |forward| {
        workspace.cycle_tab(forward);
        active(&workspace).map(str::to_owned).unwrap()
    };
    assert_eq!([step(true), step(true), step(false), step(false)], ["a", "b", "a", "c"]);
}

#[test]
fn the_tabs_and_the_active_one_come_back_from_the_library() {
    let folder = Folder::new();
    let mut workspace = folder.workspace();
    let ids = add(&mut workspace, &["a", "b", "c"]);
    workspace.activate(&ids[1]);
    workspace.open_script_mut().unwrap().text = "kept".to_owned();
    workspace.text_changed(Instant::now());
    workspace.flush().unwrap();
    drop(workspace);

    let mut workspace = folder.workspace();
    assert_eq!((tabs(&workspace), active(&workspace)), (vec!["a", "b", "c"], Some("b")));
    assert_eq!(workspace.open_script().unwrap().text, "kept");
    assert!(workspace.tabs().iter().all(|tab| !tab.unsaved() && !tab.live));
    // Each tab has an editor state of its own.
    let mut serials: Vec<u64> = workspace.tabs().iter().map(|tab| tab.serial).collect();
    serials.dedup();
    assert_eq!(serials.len(), 3);

    // A deleted script's tab does not come back, and neither does one that cannot be read.
    workspace.delete(&ids[1]).unwrap();
    assert_eq!(active(&workspace), Some("c"));
    drop(workspace);
    folder.block(&ids[2]);
    let workspace = folder.workspace();
    assert_eq!((tabs(&workspace), active(&workspace)), (vec!["a"], Some("a")));
}

#[test]
fn restored_tabs_count_as_used_from_left_to_right_with_the_active_one_last() {
    let folder = Folder::new();
    let (mut workspace, ids) = full(&folder);
    workspace.activate(&ids[0]);
    drop(workspace);
    let mut workspace = folder.workspace();
    add(&mut workspace, &["13"]);
    assert_eq!(tabs(&workspace)[..3], ["1", "3", "4"]);
}

#[test]
fn autosave_saves_every_due_tab_and_reports_a_failure_once() {
    let folder = Folder::new();
    let mut workspace = folder.workspace();
    let ids = add(&mut workspace, &["a", "b"]);
    leave_unsaved(&folder, &mut workspace, &ids[0]);
    workspace.activate(&ids[1]);
    workspace.open_script_mut().unwrap().text = "words".to_owned();
    let start = Instant::now();
    workspace.text_changed(start);

    let later = start + autosave::DELAY;
    assert!(matches!(workspace.autosave(later), Autosave::Failed { first: true, .. }));
    assert_eq!(workspace.store().load(&ids[1]).unwrap(), "words");
    assert!(matches!(workspace.autosave(later), Autosave::Waiting(_)));
    assert!(matches!(workspace.autosave(later + autosave::DELAY), Autosave::Failed { first: false, .. }));
    // A tab that cannot be saved cannot be closed.
    assert!(workspace.close(&ids[0]).is_err());
    assert_eq!(tabs(&workspace), ["a", "b"]);
}
