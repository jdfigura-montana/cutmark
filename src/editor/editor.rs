//! editor::editor: the Editor struct owning the Project, view, paths and dirty flag.
// undo/redo: phase 2

use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::io::IoError;
use crate::io::paths::{self, ProjectPaths};
use crate::io::project_file;
use crate::io::recent::{RecentEntry, RecentList};
use crate::model::project::{BitDepth, Project, ViewState};

/// Errors from creating, opening or saving a project.
#[derive(Debug, Error)]
pub enum EditorError {
    /// Wrapped io error (project layout, project.json or recent list problems).
    #[error(transparent)]
    Io(#[from] IoError),
}

/// Owns the open project; the only gateway between the UI and project state.
pub struct Editor {
    /// Current project state, shared read-only (audio thread later).
    pub project: Arc<Project>,
    /// UI view state (zoom, scroll, playhead, snapping, ripple).
    pub view: ViewState,
    /// Locations of the project folder and its subfolders.
    pub paths: ProjectPaths,
    /// True when the project changed since the last save.
    pub dirty: bool,
}

impl Editor {
    /// Creates a project folder layout, a new Project and saves project.json.
    pub fn create(
        name: &str,
        parent: &Path,
        sample_rate: u32,
        bit_depth: BitDepth,
    ) -> Result<Editor, EditorError> {
        let project_paths = paths::create_layout(parent, name)?;
        let project = Project::new(name.to_string(), sample_rate, bit_depth);
        let view = ViewState::default();
        project_file::save(&project_paths, &project, &view)?;
        Self::touch_recent(&project.name, &project_paths.root);
        Ok(Editor {
            project: Arc::new(project),
            view,
            paths: project_paths,
            dirty: false,
        })
    }

    /// Loads a project from a project.json file path or a project folder path.
    pub fn open(path: &Path) -> Result<Editor, EditorError> {
        let (project_paths, project, view) = project_file::load(path)?;
        Self::touch_recent(&project.name, &project_paths.root);
        Ok(Editor {
            project: Arc::new(project),
            view,
            paths: project_paths,
            dirty: false,
        })
    }

    /// Saves project.json (atomically) and clears the dirty flag.
    pub fn save(&mut self) -> Result<(), EditorError> {
        project_file::save(&self.paths, &self.project, &self.view)?;
        self.dirty = false;
        Ok(())
    }

    /// Adds the project to the recent list; a write failure is ignored.
    fn touch_recent(name: &str, root: &Path) {
        if let Some(list_path) = crate::io::recent::default_path() {
            let mut list = RecentList::load(&list_path);
            list.touch(name.to_string(), root.to_path_buf());
            let _ = list.save(&list_path);
        }
    }
}

/// The recent-projects list, newest first, for the start screen.
pub fn recent_projects() -> Vec<RecentEntry> {
    match crate::io::recent::default_path() {
        Some(list_path) => RecentList::load(&list_path).entries,
        None => Vec::new(),
    }
}

/// Removes one entry from the recent list; a write failure is ignored.
pub fn remove_recent(root: &Path) {
    if let Some(list_path) = crate::io::recent::default_path() {
        let mut list = RecentList::load(&list_path);
        list.remove(root);
        let _ = list.save(&list_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Serializes tests that read/write the real recent list, which race otherwise.
    static RECENT_LOCK: Mutex<()> = Mutex::new(());

    /// Locks the recent list for the calling test.
    fn lock_recent() -> std::sync::MutexGuard<'static, ()> {
        RECENT_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Cleans the temp project root out of the real recent list after a test.
    fn drop_from_recent(root: &Path) {
        remove_recent(root);
    }

    #[test]
    fn create_makes_layout_saves_and_is_not_dirty() {
        let _recent = lock_recent();
        let parent = tempdir().unwrap();
        let editor = Editor::create("My Project", parent.path(), 48_000, BitDepth::Int24).unwrap();
        assert!(editor.paths.project_json().is_file());
        assert!(editor.paths.media().is_dir());
        assert!(editor.paths.cache().is_dir());
        assert!(editor.paths.autosave().is_dir());
        assert!(editor.paths.exports().is_dir());
        assert!(!editor.dirty);
        assert_eq!(editor.project.name, "My Project");
        assert_eq!(editor.project.sample_rate, 48_000);
        drop_from_recent(&editor.paths.root);
    }

    #[test]
    fn open_round_trips_a_created_project() {
        let _recent = lock_recent();
        let parent = tempdir().unwrap();
        let editor = Editor::create("Round", parent.path(), 44_100, BitDepth::Float32).unwrap();
        let reopened = Editor::open(&editor.paths.root).unwrap();
        assert_eq!(reopened.project, editor.project);
        assert_eq!(reopened.view, editor.view);
        assert_eq!(reopened.paths.root, editor.paths.root);
        assert!(!reopened.dirty);
        drop_from_recent(&editor.paths.root);
    }

    #[test]
    fn open_errors_on_missing_project() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            Editor::open(&dir.path().join("nothing")),
            Err(EditorError::Io(IoError::NotAProject(_)))
        ));
    }

    #[test]
    fn create_errors_on_invalid_name() {
        let parent = tempdir().unwrap();
        assert!(matches!(
            Editor::create("a/b", parent.path(), 48_000, BitDepth::Int24),
            Err(EditorError::Io(IoError::InvalidProjectName(_)))
        ));
    }

    #[test]
    fn save_clears_dirty_flag() {
        let _recent = lock_recent();
        let parent = tempdir().unwrap();
        let mut editor = Editor::create("Dirty", parent.path(), 48_000, BitDepth::Int16).unwrap();
        editor.dirty = true;
        editor.save().unwrap();
        assert!(!editor.dirty);
        drop_from_recent(&editor.paths.root);
    }

    #[test]
    fn create_and_remove_update_the_recent_list() {
        let _recent = lock_recent();
        let parent = tempdir().unwrap();
        let editor = Editor::create("RecentTest", parent.path(), 48_000, BitDepth::Int24).unwrap();
        let list = recent_projects();
        assert!(list.iter().any(|e| e.root == editor.paths.root));
        remove_recent(&editor.paths.root);
        let list = recent_projects();
        assert!(list.iter().all(|e| e.root != editor.paths.root));
    }
}
