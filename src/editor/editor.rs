//! editor::editor: the Editor struct owning the Project, view, paths and dirty flag.

use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::editor::action::Action;
use crate::editor::clipboard::{Clipboard, copy_range};
use crate::editor::history::{History, HistoryEntry};
use crate::editor::ops::OpError;
use crate::editor::ops::paste;
use crate::io::IoError;
use crate::io::paths::{self, ProjectPaths};
use crate::io::project_file;
use crate::io::recent::{RecentEntry, RecentList};
use crate::model::invariants::{self, InvariantError};
use crate::model::project::{BitDepth, Project, ViewState};

/// Errors from creating, opening, saving or editing a project.
#[derive(Debug, Error)]
pub enum EditorError {
    /// Wrapped io error (project layout, project.json or recent list problems).
    #[error(transparent)]
    Io(#[from] IoError),
    /// A pure edit operation failed; the project was not changed.
    #[error(transparent)]
    Op(#[from] OpError),
    /// The edited project violates a model invariant; the change was discarded.
    #[error(transparent)]
    Invariant(#[from] InvariantError),
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
    /// Undo/redo stacks of whole-project snapshots.
    pub history: History,
    /// Snapshot taken when a continuous gesture began.
    pub gesture_base: Option<Arc<Project>>,
    /// Label remembered from the last previewed action of the current gesture.
    pub gesture_label: Option<&'static str>,
    /// Clipboard content from the last copy or cut, if any.
    pub clipboard: Option<Arc<Clipboard>>,
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
            history: History::new(),
            gesture_base: None,
            gesture_label: None,
            clipboard: None,
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
            history: History::new(),
            gesture_base: None,
            gesture_label: None,
            clipboard: None,
        })
    }

    /// Wraps an existing project for tests: no disk access, no history.
    pub fn with_project(project: Project, view: ViewState, paths: ProjectPaths) -> Editor {
        Editor {
            project: Arc::new(project),
            view,
            paths,
            dirty: false,
            history: History::new(),
            gesture_base: None,
            gesture_label: None,
            clipboard: None,
        }
    }

    /// Saves project.json (atomically) and clears the dirty flag.
    pub fn save(&mut self) -> Result<(), EditorError> {
        project_file::save(&self.paths, &self.project, &self.view)?;
        self.dirty = false;
        Ok(())
    }

    /// Copies the project folder to parent/new_name, renames the project,
    /// points the paths at the new root and saves there. The old folder is
    /// left untouched on disk.
    pub fn save_as(&mut self, parent: &Path, new_name: &str) -> Result<(), EditorError> {
        let new_paths = project_file::copy_project(&self.paths, parent, new_name)?;
        Arc::make_mut(&mut self.project).name = new_name.to_string();
        self.paths = new_paths;
        self.save()?;
        Self::touch_recent(&self.project.name, &self.paths.root);
        Ok(())
    }

    /// Copies the range `s..e` on the listed tracks to the clipboard.
    ///
    /// Not undoable and does not mark the project dirty.
    pub fn copy(&mut self, s: u64, e: u64, track_ids: &[uuid::Uuid]) {
        self.clipboard = Some(Arc::new(copy_range(&self.project, s, e, track_ids)));
    }

    /// Copies the range, then deletes it with the label "Cut".
    pub fn cut(
        &mut self,
        s: u64,
        e: u64,
        track_ids: Vec<uuid::Uuid>,
        ripple: bool,
    ) -> Result<(), EditorError> {
        self.clipboard = Some(Arc::new(copy_range(&self.project, s, e, &track_ids)));
        self.apply_labeled(
            Action::DeleteRange {
                s,
                e,
                track_ids,
                ripple,
            },
            "Cut",
        )
    }

    /// The clipboard content from the last copy or cut, if any.
    pub fn clipboard(&self) -> Option<Arc<Clipboard>> {
        self.clipboard.clone()
    }
    /// Applies one action: clone, run the op, check invariants, push undo, swap in.
    pub fn apply(&mut self, action: Action) -> Result<(), EditorError> {
        let label = action.label();
        self.apply_labeled(action, label)
    }

    /// Applies one action with an explicit label, overriding `action.label()`.
    pub fn apply_labeled(
        &mut self,
        action: Action,
        label: &'static str,
    ) -> Result<(), EditorError> {
        self.commit(label, |project| action.run(project))
    }

    /// Runs one pure edit op on a clone, checks invariants, pushes undo and swaps in.
    fn commit(
        &mut self,
        label: &'static str,
        run: impl FnOnce(&mut Project) -> Result<(), OpError>,
    ) -> Result<(), EditorError> {
        let mut next = (*self.project).clone();
        run(&mut next)?;
        invariants::check(&next)?;
        self.history.push_undo(HistoryEntry {
            label,
            before: Arc::clone(&self.project),
        });
        self.project = Arc::new(next);
        self.dirty = true;
        // publish to audio: phase 5
        Ok(())
    }

    /// Inserts the stored clipboard at `pos` on the receiving tracks (label "Paste").
    pub fn paste_insert_at(&mut self, pos: u64, target_track: usize) -> Result<(), EditorError> {
        let clipboard = self.require_clipboard()?;
        self.commit("Paste", |project| {
            paste::paste_insert(project, pos, target_track, &clipboard)
        })
    }

    /// Replaces the range `s..e` on the receiving tracks with the stored clipboard.
    pub fn paste_replace_range(
        &mut self,
        s: u64,
        e: u64,
        target_track: usize,
        ripple: bool,
    ) -> Result<(), EditorError> {
        let clipboard = self.require_clipboard()?;
        self.commit("Paste", |project| {
            paste::paste_replace(project, s, e, target_track, &clipboard, ripple)
        })
    }

    /// Returns the stored clipboard, or an error when nothing was copied or cut.
    fn require_clipboard(&self) -> Result<Arc<Clipboard>, OpError> {
        let clipboard = self
            .clipboard
            .clone()
            .ok_or_else(|| OpError::Invalid("empty clipboard".into()))?;
        if clipboard.tracks.iter().all(|clips| clips.is_empty()) {
            return Err(OpError::Invalid("empty clipboard".into()));
        }
        Ok(clipboard)
    }

    /// Restores the top undo entry; returns false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        match self.history.undo_step(Arc::clone(&self.project)) {
            Some(entry) => {
                self.project = entry.before;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// Restores the top redo entry; returns false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        match self.history.redo_step(Arc::clone(&self.project)) {
            Some(entry) => {
                self.project = entry.before;
                self.dirty = true;
                true
            }
            None => false,
        }
    }

    /// Label of the next undo, if any.
    pub fn undo_label(&self) -> Option<&'static str> {
        self.history.undo_label()
    }

    /// Label of the next redo, if any.
    pub fn redo_label(&self) -> Option<&'static str> {
        self.history.redo_label()
    }

    /// Starts a continuous gesture: stores the current project as the base snapshot.
    pub fn begin_gesture(&mut self) {
        self.gesture_base = Some(Arc::clone(&self.project));
    }

    /// Previews one action on a fresh clone of the gesture base; no history recorded.
    pub fn preview(&mut self, action: Action) -> Result<(), EditorError> {
        let base = self
            .gesture_base
            .as_ref()
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::clone(&self.project));
        let mut next = (*base).clone();
        action.run(&mut next)?;
        invariants::check(&next)?;
        self.project = Arc::new(next);
        self.gesture_label = Some(action.label());
        Ok(())
    }

    /// Ends the gesture: one history entry from the base if the project changed.
    pub fn end_gesture(&mut self) {
        if let Some(base) = self.gesture_base.take() {
            let label = self.gesture_label.take().unwrap_or("Edit");
            if base != self.project {
                self.history.push_undo(HistoryEntry {
                    label,
                    before: base,
                });
                self.dirty = true;
            }
        }
    }

    /// Cancels the gesture: restores the base snapshot and records nothing.
    pub fn cancel_gesture(&mut self) {
        if let Some(base) = self.gesture_base.take() {
            self.project = base;
        }
        self.gesture_label = None;
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
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    use crate::editor::action::Action;
    use crate::editor::ops::OpError;
    use crate::model::fixtures::{project_with_clips, spans, srcs};

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
    fn save_as_copies_folder_renames_and_keeps_old_project() {
        let _recent = lock_recent();
        let parent = tempdir().unwrap();
        let destination = tempdir().unwrap();
        let mut editor =
            Editor::create("Original", parent.path(), 48_000, BitDepth::Int24).unwrap();
        editor.save_as(destination.path(), "Renamed").unwrap();

        // New paths, new name, not dirty.
        assert_eq!(editor.paths.root, destination.path().join("Renamed"));
        assert_eq!(editor.project.name, "Renamed");
        assert!(!editor.dirty);
        let (paths, project, _) = project_file::load(&editor.paths.root).unwrap();
        assert_eq!(project.name, "Renamed");
        assert_eq!(paths.root, destination.path().join("Renamed"));

        // The old folder still exists with its old name and old project.json.
        let (old_paths, old_project, _) =
            project_file::load(&parent.path().join("Original")).unwrap();
        assert_eq!(old_project.name, "Original");
        assert_eq!(old_paths.root, parent.path().join("Original"));
        drop_from_recent(&editor.paths.root);
        drop_from_recent(&old_paths.root);
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

    /// Applies `action` to an in-memory editor, undoes and redoes it, returning
    /// the edited project.
    fn roundtrip(project: Project, action: Action) -> Project {
        let original = project.clone();
        let mut editor = Editor::with_project(
            project,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.apply(action).unwrap();
        assert!(invariants::check(&editor.project).is_ok());
        assert!(editor.undo(), "undo after apply");
        assert_eq!(*editor.project, original, "undo must restore the original");
        assert!(editor.redo(), "redo after undo");
        (*editor.project).clone()
    }

    #[test]
    fn add_track_roundtrip_appends_and_names() {
        let project = roundtrip(
            project_with_clips(&[&[]]),
            Action::AddTrack {
                index: None,
                name: None,
            },
        );
        assert_eq!(project.tracks.len(), 2);
        assert_eq!(project.tracks[1].name, "Track 2");
    }

    #[test]
    fn delete_track_roundtrip_removes_the_track() {
        let project = project_with_clips(&[&[], &[]]);
        let id = project.tracks[1].id;
        let project = roundtrip(project, Action::DeleteTrack { id });
        assert_eq!(project.tracks.len(), 1);
    }

    #[test]
    fn rename_track_roundtrip_sets_the_name() {
        let project = project_with_clips(&[&[]]);
        let id = project.tracks[0].id;
        let project = roundtrip(
            project,
            Action::RenameTrack {
                id,
                name: "Bass".into(),
            },
        );
        assert_eq!(project.tracks[0].name, "Bass");
    }

    #[test]
    fn move_track_roundtrip_reorders() {
        let project = project_with_clips(&[&[], &[], &[]]);
        let id = project.tracks[0].id;
        let project = roundtrip(project, Action::MoveTrack { id, new_index: 2 });
        assert_eq!(project.tracks[2].name, "Track 1");
        assert_eq!(project.tracks[0].name, "Track 2");
    }

    #[test]
    fn set_track_volume_roundtrip_sets_and_clamps() {
        let project = project_with_clips(&[&[]]);
        let id = project.tracks[0].id;
        let project = roundtrip(project, Action::SetTrackVolume { id, db: 3.0 });
        assert!((project.tracks[0].volume_db - 3.0).abs() < f32::EPSILON);
        // +20 dB clamps to the +12 dB maximum.
        let project = roundtrip(project, Action::SetTrackVolume { id, db: 20.0 });
        assert!((project.tracks[0].volume_db - 12.0).abs() < f32::EPSILON);
        // -100 dB clamps to the -60 dB minimum.
        let project = roundtrip(project, Action::SetTrackVolume { id, db: -100.0 });
        assert!((project.tracks[0].volume_db + 60.0).abs() < f32::EPSILON);
    }

    #[test]
    fn set_track_pan_roundtrip_sets_and_clamps() {
        let project = project_with_clips(&[&[]]);
        let id = project.tracks[0].id;
        let project = roundtrip(project, Action::SetTrackPan { id, pan: 0.5 });
        assert!((project.tracks[0].pan - 0.5).abs() < f32::EPSILON);
        let project = roundtrip(project, Action::SetTrackPan { id, pan: -3.0 });
        assert!((project.tracks[0].pan + 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn set_master_volume_roundtrip_sets_and_clamps() {
        let project = roundtrip(
            project_with_clips(&[&[]]),
            Action::SetMasterVolume { db: -3.0 },
        );
        assert!((project.master_volume_db + 3.0).abs() < f32::EPSILON);
        let project = roundtrip(project, Action::SetMasterVolume { db: 20.0 });
        assert!((project.master_volume_db - 12.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mute_solo_and_solo_exclusive_roundtrips() {
        let project = project_with_clips(&[&[], &[]]);
        let a = project.tracks[0].id;
        let b = project.tracks[1].id;
        let project = roundtrip(project, Action::SetTrackMute { id: a, mute: true });
        assert!(project.tracks[0].mute);
        let project = roundtrip(project, Action::SetTrackSolo { id: a, solo: true });
        assert!(project.tracks[0].solo);
        let project = roundtrip(project, Action::SoloExclusive { id: b });
        assert!(!project.tracks[0].solo);
        assert!(project.tracks[1].solo);
    }

    #[test]
    fn failed_action_leaves_project_and_history_unchanged() {
        let project = project_with_clips(&[&[]]);
        let mut editor = Editor::with_project(
            project.clone(),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let missing = uuid::Uuid::new_v4();
        let result = editor.apply(Action::SetTrackVolume {
            id: missing,
            db: 3.0,
        });
        assert!(matches!(result, Err(EditorError::Op(OpError::NotFound(id))) if id == missing));
        assert_eq!(*editor.project, project);
        assert!(!editor.dirty);
        assert!(editor.history.undo.is_empty());
        assert!(editor.history.redo.is_empty());
        assert!(!editor.undo());
    }

    #[test]
    fn apply_after_undo_clears_redo() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let id = editor.project.tracks[0].id;
        editor
            .apply(Action::SetTrackVolume { id, db: 1.0 })
            .unwrap();
        assert!(editor.undo());
        assert!(editor.redo_label().is_some());
        editor
            .apply(Action::SetTrackVolume { id, db: 2.0 })
            .unwrap();
        assert!(editor.redo_label().is_none());
        assert!(!editor.redo());
    }

    #[test]
    fn one_thousand_and_one_applies_keep_one_thousand_undo_steps() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let id = editor.project.tracks[0].id;
        for i in 0..1_001 {
            editor
                .apply(Action::SetTrackVolume {
                    id,
                    db: (i % 12) as f32,
                })
                .unwrap();
        }
        assert_eq!(editor.history.undo.len(), 1_000);
        for _ in 0..1_000 {
            assert!(editor.undo());
        }
        assert!(!editor.undo());
    }

    #[test]
    fn gesture_previews_end_in_one_undo_step() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let before = (*editor.project).clone();
        let id = editor.project.tracks[0].id;
        editor.begin_gesture();
        for db in [1.0, 2.0, 3.0] {
            editor.preview(Action::SetTrackVolume { id, db }).unwrap();
        }
        assert!(editor.history.undo.is_empty(), "previews record no history");
        editor.end_gesture();
        assert_eq!(editor.history.undo.len(), 1);
        assert_eq!(editor.undo_label(), Some("Track Volume"));
        assert!(editor.undo());
        assert_eq!(*editor.project, before);
        assert!(!editor.undo());
    }

    #[test]
    fn cancel_gesture_restores_and_records_nothing() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let before = (*editor.project).clone();
        let id = editor.project.tracks[0].id;
        editor.begin_gesture();
        editor
            .preview(Action::SetTrackVolume { id, db: 5.0 })
            .unwrap();
        editor.cancel_gesture();
        assert_eq!(*editor.project, before);
        assert!(editor.history.undo.is_empty());
        assert!(editor.history.redo.is_empty());
        assert!(!editor.undo());
    }

    #[test]
    fn action_labels_match_the_spec() {
        assert_eq!(
            Action::AddTrack {
                index: None,
                name: None
            }
            .label(),
            "Add Track"
        );
        assert_eq!(
            Action::DeleteTrack {
                id: uuid::Uuid::nil()
            }
            .label(),
            "Delete Track"
        );
        assert_eq!(
            Action::RenameTrack {
                id: uuid::Uuid::nil(),
                name: String::new()
            }
            .label(),
            "Rename Track"
        );
        assert_eq!(
            Action::MoveTrack {
                id: uuid::Uuid::nil(),
                new_index: 0
            }
            .label(),
            "Move Track"
        );
        assert_eq!(
            Action::SetTrackVolume {
                id: uuid::Uuid::nil(),
                db: 0.0
            }
            .label(),
            "Track Volume"
        );
        assert_eq!(
            Action::SetTrackPan {
                id: uuid::Uuid::nil(),
                pan: 0.0
            }
            .label(),
            "Track Pan"
        );
        assert_eq!(
            Action::SetTrackMute {
                id: uuid::Uuid::nil(),
                mute: true
            }
            .label(),
            "Mute"
        );
        assert_eq!(
            Action::SetTrackSolo {
                id: uuid::Uuid::nil(),
                solo: true
            }
            .label(),
            "Solo"
        );
        assert_eq!(
            Action::SoloExclusive {
                id: uuid::Uuid::nil()
            }
            .label(),
            "Solo Exclusive"
        );
        assert_eq!(Action::SetMasterVolume { db: 0.0 }.label(), "Master Volume");
        assert_eq!(
            Action::SplitAt {
                pos: 0,
                track_ids: None
            }
            .label(),
            "Split"
        );
        assert_eq!(
            Action::PlaceClip {
                track_id: uuid::Uuid::nil(),
                media_id: uuid::Uuid::nil(),
                start: 0,
                src_in: 0,
                src_out: 0,
                insert: false
            }
            .label(),
            "Add Clip"
        );
        assert_eq!(
            Action::MoveClips {
                clip_ids: vec![],
                delta_time: 0,
                delta_tracks: 0
            }
            .label(),
            "Move Clip"
        );
        assert_eq!(
            Action::TrimClip {
                clip_id: uuid::Uuid::nil(),
                edge: crate::editor::ops::clip::Edge::Start,
                new_pos: 0
            }
            .label(),
            "Trim Clip"
        );
        assert_eq!(
            Action::DeleteClips {
                clip_ids: vec![],
                ripple: false
            }
            .label(),
            "Delete Clip"
        );
        assert_eq!(
            Action::SetClipGain {
                clip_id: uuid::Uuid::nil(),
                db: 0.0
            }
            .label(),
            "Clip Gain"
        );
        assert_eq!(
            Action::SetClipFades {
                clip_id: uuid::Uuid::nil(),
                fade_in: 0,
                fade_out: 0
            }
            .label(),
            "Fades"
        );
    }

    #[test]
    fn split_at_roundtrip_all_tracks() {
        let project = roundtrip(
            project_with_clips(&[&[(100, 100)], &[(100, 100)]]),
            Action::SplitAt {
                pos: 150,
                track_ids: None,
            },
        );
        assert_eq!(spans(&project, 0), [(100, 150), (150, 200)]);
        assert_eq!(spans(&project, 1), [(100, 150), (150, 200)]);
    }

    #[test]
    fn split_at_roundtrip_single_track() {
        let target = project_with_clips(&[&[(100, 100)], &[(100, 100)]]);
        let id = target.tracks[1].id;
        let project = roundtrip(
            target,
            Action::SplitAt {
                pos: 150,
                track_ids: Some(vec![id]),
            },
        );
        assert_eq!(spans(&project, 0), [(100, 200)]);
        assert_eq!(spans(&project, 1), [(100, 150), (150, 200)]);
    }

    /// Wraps a project in an in-memory editor for error-path tests.
    fn with_editor(project: Project) -> Editor {
        Editor::with_project(
            project,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        )
    }

    #[test]
    fn place_clip_overwrite_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let track_id = target.tracks[0].id;
        let media_id = target.media[0].id;
        let project = roundtrip(
            target,
            Action::PlaceClip {
                track_id,
                media_id,
                start: 150,
                src_in: 0,
                src_out: 100,
                insert: false,
            },
        );
        assert_eq!(spans(&project, 0), [(100, 150), (150, 250)]);
    }

    #[test]
    fn place_clip_insert_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let track_id = target.tracks[0].id;
        let media_id = target.media[0].id;
        let project = roundtrip(
            target,
            Action::PlaceClip {
                track_id,
                media_id,
                start: 150,
                src_in: 0,
                src_out: 100,
                insert: true,
            },
        );
        assert_eq!(spans(&project, 0), [(100, 150), (150, 250), (250, 300)]);
    }

    #[test]
    fn move_clip_overwrites_neighbor_roundtrip() {
        let target = project_with_clips(&[&[(100, 100), (220, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(
            target,
            Action::MoveClips {
                clip_ids: vec![clip_id],
                delta_time: 50,
                delta_tracks: 0,
            },
        );
        assert_eq!(spans(&project, 0), [(150, 250), (250, 320)]);
    }

    #[test]
    fn move_clips_down_one_track_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)], &[(150, 100)], &[]]);
        let first = target.tracks[0].clips[0].id;
        let second = target.tracks[1].clips[0].id;
        let project = roundtrip(
            target,
            Action::MoveClips {
                clip_ids: vec![first, second],
                delta_time: 0,
                delta_tracks: 1,
            },
        );
        assert!(project.tracks[0].clips.is_empty());
        assert_eq!(spans(&project, 1), [(100, 200)]);
        assert_eq!(spans(&project, 2), [(150, 250)]);
    }

    #[test]
    fn move_clips_below_zero_errors_and_leaves_the_project_unchanged() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let original = target.clone();
        let mut editor = with_editor(target);
        let result = editor.apply(Action::MoveClips {
            clip_ids: vec![clip_id],
            delta_time: -200,
            delta_tracks: 0,
        });
        assert!(matches!(result, Err(EditorError::Op(OpError::Invalid(_)))));
        assert_eq!(*editor.project, original);
    }

    #[test]
    fn move_clips_off_the_last_track_errors_and_leaves_the_project_unchanged() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let original = target.clone();
        let mut editor = with_editor(target);
        let result = editor.apply(Action::MoveClips {
            clip_ids: vec![clip_id],
            delta_time: 0,
            delta_tracks: 1,
        });
        assert!(matches!(result, Err(EditorError::Op(OpError::Invalid(_)))));
        assert_eq!(*editor.project, original);
    }

    #[test]
    fn trim_start_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(
            target,
            Action::TrimClip {
                clip_id,
                edge: crate::editor::ops::clip::Edge::Start,
                new_pos: 120,
            },
        );
        assert_eq!(spans(&project, 0), [(120, 200)]);
        assert_eq!(srcs(&project, 0), [(20, 100)]);
    }

    #[test]
    fn trim_start_clamps_at_src_in_zero_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(
            target,
            Action::TrimClip {
                clip_id,
                edge: crate::editor::ops::clip::Edge::Start,
                new_pos: 50,
            },
        );
        assert_eq!(spans(&project, 0), [(100, 200)]);
        assert_eq!(srcs(&project, 0), [(0, 100)]);
    }

    #[test]
    fn trim_end_stops_at_neighbor_roundtrip() {
        let target = project_with_clips(&[&[(100, 150), (250, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(
            target,
            Action::TrimClip {
                clip_id,
                edge: crate::editor::ops::clip::Edge::End,
                new_pos: 400,
            },
        );
        assert_eq!(spans(&project, 0), [(100, 250), (250, 350)]);
    }

    #[test]
    fn delete_clips_ripple_roundtrip() {
        let target = project_with_clips(&[&[(0, 100), (100, 100), (200, 100)]]);
        let clip_id = target.tracks[0].clips[1].id;
        let project = roundtrip(
            target,
            Action::DeleteClips {
                clip_ids: vec![clip_id],
                ripple: true,
            },
        );
        assert_eq!(spans(&project, 0), [(0, 100), (100, 200)]);
    }

    #[test]
    fn delete_clips_without_ripple_leaves_the_gap_roundtrip() {
        let target = project_with_clips(&[&[(0, 100), (100, 100), (200, 100)]]);
        let clip_id = target.tracks[0].clips[1].id;
        let project = roundtrip(
            target,
            Action::DeleteClips {
                clip_ids: vec![clip_id],
                ripple: false,
            },
        );
        assert_eq!(spans(&project, 0), [(0, 100), (200, 300)]);
    }

    #[test]
    fn set_clip_gain_clamps_roundtrip() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(target, Action::SetClipGain { clip_id, db: 30.0 });
        assert_eq!(project.tracks[0].clips[0].gain_db, 24.0);
    }

    #[test]
    fn set_clip_fades_clamp_on_a_short_clip_roundtrip() {
        let target = project_with_clips(&[&[(100, 10)]]);
        let clip_id = target.tracks[0].clips[0].id;
        let project = roundtrip(
            target,
            Action::SetClipFades {
                clip_id,
                fade_in: 8,
                fade_out: 8,
            },
        );
        let clip = &project.tracks[0].clips[0];
        assert_eq!(clip.fade_in, 2);
        assert_eq!(clip.fade_out, 8);
    }
    #[test]
    fn delete_range_roundtrip() {
        let target = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let track_id = target.tracks[0].id;
        let project = roundtrip(
            target,
            Action::DeleteRange {
                s: 50,
                e: 150,
                track_ids: vec![track_id],
                ripple: true,
            },
        );
        assert_eq!(spans(&project, 0), [(0, 50), (100, 200)]);
    }

    #[test]
    fn delete_range_without_ripple_roundtrip() {
        let target = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let track_id = target.tracks[0].id;
        let project = roundtrip(
            target,
            Action::DeleteRange {
                s: 50,
                e: 150,
                track_ids: vec![track_id],
                ripple: false,
            },
        );
        assert_eq!(spans(&project, 0), [(0, 50), (200, 300)]);
    }

    #[test]
    fn trim_to_selection_roundtrip() {
        let target = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let track_id = target.tracks[0].id;
        let project = roundtrip(
            target,
            Action::TrimToSelection {
                s: 50,
                e: 250,
                track_ids: vec![track_id],
            },
        );
        assert_eq!(spans(&project, 0), [(50, 100), (200, 250)]);
    }

    #[test]
    fn cut_copies_to_the_clipboard_and_undo_restores() {
        let target = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let track_id = target.tracks[0].id;
        let original = target.clone();
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.cut(50, 150, vec![track_id], true).unwrap();
        assert!(editor.dirty);

        let clipboard = editor.clipboard().expect("cut stores the clipboard");
        assert_eq!(clipboard.length, 100);
        let clip = &clipboard.tracks[0][0];
        assert_eq!((clip.start, clip.start + clip.len()), (0, 50));
        assert_eq!((clip.src_in, clip.src_out), (50, 100));

        assert_eq!(editor.undo_label(), Some("Cut"));
        assert!(editor.undo(), "undo after cut");
        assert_eq!(*editor.project, original, "undo must restore the clips");
    }

    #[test]
    fn copy_is_not_undoable_and_not_dirty() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[(0, 100)]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let track_id = editor.project.tracks[0].id;
        editor.copy(0, 100, &[track_id]);
        assert!(!editor.dirty);
        assert!(editor.undo_label().is_none());
        let clipboard = editor.clipboard().expect("copy stores the clipboard");
        assert_eq!(clipboard.length, 100);
    }

    /// Wraps the target project in an editor with the clipboard already stored.
    fn paste_editor(target: Project, clipboard: Clipboard) -> Editor {
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.clipboard = Some(Arc::new(clipboard));
        editor
    }

    /// Builds a clipboard by copying `s..e` from a fixture project that shares
    /// the target's media id, so pasted clips pass the media invariant.
    fn clipboard_of(target: &Project, tracks: &[&[(u64, u64)]], s: u64, e: u64) -> Clipboard {
        let mut source = project_with_clips(tracks);
        let media_id = target.media[0].id;
        source.media[0].id = media_id;
        for track in &mut source.tracks {
            for clip in &mut track.clips {
                clip.media_id = media_id;
            }
        }
        let ids: Vec<uuid::Uuid> = source.tracks.iter().map(|t| t.id).collect();
        copy_range(&source, s, e, &ids)
    }

    /// Applies a paste through the editor, then undoes and redoes it.
    fn paste_roundtrip(mut editor: Editor, paste: impl FnOnce(&mut Editor)) -> Project {
        paste(&mut editor);
        assert!(editor.dirty);
        assert_eq!(editor.undo_label(), Some("Paste"));
        let redone = (*editor.project).clone();
        assert!(editor.undo(), "paste is undoable");
        assert!(editor.redo(), "paste is redoable");
        assert_eq!(*editor.project, redone, "undo and redo must roundtrip");
        redone
    }

    #[test]
    fn paste_insert_splits_and_shifts_the_receiving_track() {
        let target = project_with_clips(&[&[(100, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 100)]], 0, 100);
        assert_eq!(clipboard.length, 100);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| e.paste_insert_at(150, 0).unwrap());
        assert_eq!(spans(&project, 0), [(100, 150), (150, 250), (250, 300)]);
    }

    #[test]
    fn paste_insert_shifts_only_the_receiving_tracks() {
        let target = project_with_clips(&[&[(0, 100)], &[(0, 100)], &[(0, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| e.paste_insert_at(50, 0).unwrap());
        assert_eq!(spans(&project, 0), [(0, 50), (50, 100), (100, 150)]);
        assert_eq!(spans(&project, 1), [(0, 100)]);
        assert_eq!(spans(&project, 2), [(0, 100)]);
    }

    #[test]
    fn paste_insert_creates_missing_receiving_tracks() {
        let target = project_with_clips(&[&[(0, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50)], &[(0, 50)]], 0, 50);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| e.paste_insert_at(0, 0).unwrap());
        assert_eq!(project.tracks.len(), 2);
        assert_eq!(spans(&project, 0), [(0, 50), (50, 150)]);
        assert_eq!(spans(&project, 1), [(0, 50)]);
    }

    #[test]
    fn pasted_clips_get_new_ids() {
        let target = project_with_clips(&[&[]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let source_id = clipboard.tracks[0][0].id;
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| e.paste_insert_at(0, 0).unwrap());
        assert_ne!(project.tracks[0].clips[0].id, source_id);
    }

    #[test]
    fn paste_replace_ripple_shorter_shifts_later_clips_left() {
        let target = project_with_clips(&[&[(100, 100), (300, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| {
            e.paste_replace_range(100, 200, 0, true).unwrap()
        });
        assert_eq!(spans(&project, 0), [(100, 150), (250, 350)]);
    }

    #[test]
    fn paste_replace_ripple_longer_shifts_later_clips_right() {
        let target = project_with_clips(&[&[(100, 100), (300, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 150)]], 0, 150);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| {
            e.paste_replace_range(100, 200, 0, true).unwrap()
        });
        assert_eq!(spans(&project, 0), [(100, 250), (350, 450)]);
    }

    #[test]
    fn paste_replace_without_ripple_shorter_leaves_a_silent_tail() {
        let target = project_with_clips(&[&[(100, 100), (300, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| {
            e.paste_replace_range(100, 200, 0, false).unwrap()
        });
        assert_eq!(spans(&project, 0), [(100, 150), (300, 400)]);
    }

    #[test]
    fn paste_replace_without_ripple_longer_overwrites_past_the_range() {
        let target = project_with_clips(&[&[(100, 100), (200, 100)]]);
        let clipboard = clipboard_of(&target, &[&[(0, 150)]], 0, 150);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| {
            e.paste_replace_range(100, 200, 0, false).unwrap()
        });
        assert_eq!(spans(&project, 0), [(100, 250), (250, 300)]);
    }

    #[test]
    fn paste_keeps_gaps_from_the_clipboard() {
        let target = project_with_clips(&[&[]]);
        let clipboard = clipboard_of(&target, &[&[(0, 50), (100, 50)]], 0, 150);
        let editor = paste_editor(target, clipboard);
        let project = paste_roundtrip(editor, |e| e.paste_insert_at(100, 0).unwrap());
        assert_eq!(spans(&project, 0), [(100, 150), (200, 250)]);
    }

    #[test]
    fn paste_without_a_clipboard_is_an_error() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        assert!(matches!(
            editor.paste_insert_at(0, 0),
            Err(EditorError::Op(OpError::Invalid(_)))
        ));
        assert!(matches!(
            editor.paste_replace_range(0, 10, 0, false),
            Err(EditorError::Op(OpError::Invalid(_)))
        ));
        assert!(!editor.dirty);
        assert!(editor.undo_label().is_none());
    }
}
