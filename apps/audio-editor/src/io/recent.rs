//! io::recent: the recent-projects list in %APPDATA%.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::IoError;

/// Maximum number of entries kept in the recent list.
const MAX_ENTRIES: usize = 10;

/// One recent project: display name and project root folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    /// Project name shown on the start screen.
    pub name: String,
    /// Project root folder path.
    pub root: PathBuf,
    /// Unix timestamp (seconds) of the last open; None for pre-existing
    /// entries written before the field existed.
    #[serde(default)]
    pub last_opened: Option<i64>,
}

impl RecentEntry {
    /// True if the project root still contains project.json.
    pub fn exists(&self) -> bool {
        self.root.join("project.json").is_file()
    }
}

/// The current time as a Unix timestamp in seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The recent-projects list, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentList {
    /// Entries, newest first.
    pub entries: Vec<RecentEntry>,
}

/// Old per-user config folder name used before the app was renamed.
const OLD_APP_DIR: &str = "AudioEditor";

/// Default location of recent.json in %APPDATA%, if the config dir can be found.
pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "CutMark").map(|dirs| dirs.config_dir().join("recent.json"))
}

/// One-time migration: if the current recent.json does not exist but the one
/// from the old AudioEditor config folder does, copy it over (the old file is
/// kept). Any failure is silent and non-fatal.
pub fn migrate_old_config() {
    if let (Some(new), Some(old)) = (default_path(), directories::ProjectDirs::from("", "", OLD_APP_DIR).map(|dirs| dirs.config_dir().join("recent.json"))) {
        migrate(&old, &new);
    }
}

/// Copies `old` to `new` when `new` is missing and `old` exists; true if the
/// copy happened. Errors are swallowed (migration is best-effort).
fn migrate(old: &Path, new: &Path) -> bool {
    if new.exists() || !old.is_file() {
        return false;
    }
    if let Some(parent) = new.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    fs::copy(old, new).is_ok()
}

impl RecentList {
    /// Loads the list from `path`; returns an empty list if the file is missing or unreadable.
    pub fn load(path: &Path) -> RecentList {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => return RecentList::default(),
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Saves the list pretty-printed: writes recent.json.tmp next to the target,
    /// creating the parent folder if needed, then renames it over the target.
    pub fn save(&self, path: &Path) -> Result<(), IoError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Moves the entry with the given root to the top (or inserts it there),
    /// stamping it with the current time, keeping at most MAX_ENTRIES entries.
    pub fn touch(&mut self, name: String, root: PathBuf) {
        self.entries.retain(|e| e.root != root);
        self.entries.insert(0, RecentEntry { name, root, last_opened: Some(now_unix()) });
        self.entries.truncate(MAX_ENTRIES);
    }

    /// Removes the entry with the given root, if present.
    pub fn remove(&mut self, root: &Path) {
        self.entries.retain(|e| e.root != root);
    }

    /// Removes every entry whose project folder no longer exists.
    pub fn remove_missing(&mut self) {
        self.entries.retain(|e| e.exists());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// Creates a fake project root containing project.json.
    fn make_project(parent: &Path, name: &str) -> PathBuf {
        let root = parent.join(name);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("project.json"), "{}").unwrap();
        root
    }

    #[test]
    fn touch_puts_newest_first() {
        let dir = tempdir().unwrap();
        let mut list = RecentList::default();
        list.touch("A".into(), dir.path().join("a"));
        list.touch("B".into(), dir.path().join("b"));
        assert_eq!(list.entries[0].name, "B");
        assert_eq!(list.entries[1].name, "A");
    }

    #[test]
    fn touch_moves_existing_root_to_top_without_duplicate() {
        let dir = tempdir().unwrap();
        let mut list = RecentList::default();
        let root_a = dir.path().join("a");
        list.touch("A".into(), root_a.clone());
        list.touch("B".into(), dir.path().join("b"));
        list.touch("A2".into(), root_a);
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.entries[0].name, "A2");
        assert!(list.entries[1..].iter().all(|e| e.name != "A2"));
    }

    #[test]
    fn eleventh_touch_drops_oldest() {
        let dir = tempdir().unwrap();
        let mut list = RecentList::default();
        for i in 0..11 {
            list.touch(format!("P{i}"), dir.path().join(format!("p{i}")));
        }
        assert_eq!(list.entries.len(), 10);
        assert_eq!(list.entries[0].name, "P10");
        assert_eq!(list.entries[9].name, "P1");
        assert!(list.entries.iter().all(|e| e.name != "P0"));
    }

    #[test]
    fn remove_deletes_entry() {
        let dir = tempdir().unwrap();
        let mut list = RecentList::default();
        let root = dir.path().join("a");
        list.touch("A".into(), root.clone());
        list.touch("B".into(), dir.path().join("b"));
        list.remove(&root);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].name, "B");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sub").join("recent.json");
        let mut list = RecentList::default();
        list.touch("A".into(), dir.path().join("a"));
        list.touch("B".into(), dir.path().join("b"));
        list.save(&path).unwrap();
        let loaded = RecentList::load(&path);
        assert_eq!(loaded, list);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn load_missing_file_gives_empty_list() {
        let dir = tempdir().unwrap();
        let list = RecentList::load(&dir.path().join("nope.json"));
        assert!(list.entries.is_empty());
    }

    #[test]
    fn load_unreadable_file_gives_empty_list() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recent.json");
        fs::write(&path, "not json").unwrap();
        let list = RecentList::load(&path);
        assert!(list.entries.is_empty());
    }

    #[test]
    fn exists_false_for_deleted_folder() {
        let dir = tempdir().unwrap();
        let root = make_project(dir.path(), "gone");
        let entry = RecentEntry { name: "gone".into(), root: root.clone(), last_opened: None };
        assert!(entry.exists());
        fs::remove_dir_all(&root).unwrap();
        assert!(!entry.exists());
    }

    #[test]
    fn touch_stamps_last_opened() {
        let dir = tempdir().unwrap();
        let mut list = RecentList::default();
        list.touch("A".into(), dir.path().join("a"));
        let ts = list.entries[0].last_opened.expect("stamped");
        let now = now_unix();
        assert!((now - ts).abs() < 60, "timestamp {ts} vs now {now}");
    }

    #[test]
    fn remove_missing_removes_only_missing_entries() {
        let dir = tempdir().unwrap();
        let kept = make_project(dir.path(), "kept");
        let gone = make_project(dir.path(), "gone");
        fs::remove_dir_all(&gone).unwrap();
        let mut list = RecentList {
            entries: vec![
                RecentEntry { name: "gone".into(), root: gone, last_opened: Some(0) },
                RecentEntry { name: "kept".into(), root: kept, last_opened: None },
            ],
        };
        list.remove_missing();
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].name, "kept");
    }

    #[test]
    fn old_config_without_last_opened_still_loads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recent.json");
        fs::write(&path, r#"{"entries":[{"name":"A","root":"C:\\x"}]}"#).unwrap();
        let list = RecentList::load(&path);
        assert_eq!(list.entries.len(), 1);
        assert_eq!(list.entries[0].last_opened, None);
    }

    #[test]
    fn migrate_copies_old_to_missing_new() {
        let dir = tempdir().unwrap();
        let old = dir.path().join("old").join("recent.json");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, "old list").unwrap();
        let new = dir.path().join("new").join("recent.json");
        assert!(migrate(&old, &new));
        assert_eq!(fs::read_to_string(&new).unwrap(), "old list");
        assert!(old.is_file(), "the old file is kept");
    }

    #[test]
    fn migrate_leaves_existing_new_file_untouched() {
        let dir = tempdir().unwrap();
        let old = dir.path().join("old.json");
        fs::write(&old, "old list").unwrap();
        let new = dir.path().join("new.json");
        fs::write(&new, "new list").unwrap();
        assert!(!migrate(&old, &new));
        assert_eq!(fs::read_to_string(&new).unwrap(), "new list");
    }

    #[test]
    fn migrate_without_old_file_is_no_error() {
        let dir = tempdir().unwrap();
        let old = dir.path().join("missing.json");
        let new = dir.path().join("new.json");
        assert!(!migrate(&old, &new));
        assert!(!new.exists());
    }
}
