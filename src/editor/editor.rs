//! editor::editor: the Editor struct owning the Project, view, paths and dirty flag.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use thiserror::Error;

use crate::audio::engine::Engine;
use crate::audio::meters::{MeterBank, MeterReading, StereoMeter};
use crate::audio::mix_plan::MixPlan;
use crate::audio::player::Command;
use crate::editor::action::Action;
use crate::editor::clipboard::{Clipboard, copy_clips, copy_range};
use crate::editor::history::{History, HistoryEntry};
use crate::editor::jobs::{ExportDecision, ExportSettings, JobChannels, JobMessage};
use crate::editor::media_store::MediaStore;
use crate::editor::ops::OpError;
use crate::editor::ops::amplitude::{self, Target};
use crate::editor::ops::marker;
use crate::editor::ops::paste;
use crate::editor::selection::Selection;
use crate::editor::transport::TransportState;
use crate::io::IoError;
use crate::io::autosave;
use crate::io::export::ExportKind;
use crate::io::paths::{self, ProjectPaths, conformed_path, peaks_path};
use crate::io::project_file;
use crate::io::recent::{RecentEntry, RecentList};
use crate::model::invariants::{self, InvariantError};
use crate::model::project::{BitDepth, Project, ViewState};
use crate::util::db::{db_to_linear, linear_to_db};
use crate::util::fade::fade_gain;
use uuid::Uuid;

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
    /// The editor is busy with an export; the operation was refused.
    #[error("{0}")]
    Locked(String),
}

/// Message shown when an operation is refused during an export.
const EXPORT_LOCKED: &str = "Export in progress";

/// Result of opening with an autosave-recovery check.
pub enum OpenOutcome {
    /// The project opened normally.
    Opened(Box<Editor>),
    /// The autosave is newer than project.json; the UI must ask the user.
    NeedsRecoveryChoice(PathBuf),
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
    /// Conformed audio for bin items, kept for the whole session.
    pub media_store: MediaStore,
    /// Pending background jobs: job id -> (label, progress 0.0..1.0).
    pub pending: BTreeMap<u64, (String, f32)>,
    /// Id of the next background job to start.
    pub next_job: u64,
    /// Messages from failed background jobs, read and cleared by the UI.
    job_errors: Vec<String>,
    /// Worker-thread job channel pair.
    pub jobs: JobChannels,
    /// Display-only vertical waveform scale, 1.0..=16.0 (not saved).
    pub wave_scale: f32,
    /// The live audio engine, when it started successfully.
    pub audio: Option<Engine>,
    /// Mix plans retired from the ArcSwap, kept 2 s so the audio thread
    /// never frees the last reference.
    pub retired: Vec<(Instant, Arc<MixPlan>)>,
    /// Message from a failed `Engine::start`, shown by `audio_status`.
    pub audio_error: Option<String>,
    /// Session master meter, shared with every published mix plan.
    pub master_meter: Arc<StereoMeter>,
    /// Current per-track meter bank; reused while the track list is unchanged.
    pub meters: Arc<MeterBank>,
    /// Transport state from the previous `poll_transport`, to detect a stop.
    pub(crate) transport_prev: Option<TransportState>,
    /// Bin item currently previewed, if the UI started one.
    preview_item: Option<Uuid>,
    /// Current range or clip selection; not undoable.
    selection: Selection,
    /// Currently selected marker, if any; not undoable.
    selected_marker: Option<Uuid>,
    /// Track id of the last track clicked, for the target track fallback.
    last_clicked_track: Option<Uuid>,
    /// State of the running export job, if any.
    exporting: Option<ExportState>,
    /// Result of the finished export job, taken by `take_export_result`.
    export_result: Option<Result<PathBuf, String>>,
    /// Minimum time between autosave writes; the real app uses 120 s.
    pub autosave_interval: Duration,
    /// When the last autosave write happened (or the editor was created).
    last_autosave: Instant,
    /// True while the one-time missing-media notice is still pending.
    missing_notice: bool,
    /// Bumped by every change that marks the project dirty.
    autosave_generation: u64,
    /// Generation captured by the last successful autosave write.
    autosaved_generation: u64,
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
        Ok(Self::assemble(project_paths, project, view, false))
    }

    /// Builds an Editor from loaded parts; used by create, open and recovery.
    fn assemble(paths: ProjectPaths, project: Project, view: ViewState, dirty: bool) -> Editor {
        Self::touch_recent(&project.name, &paths.root);
        let mut editor = Editor {
            project: Arc::new(project),
            view,
            paths,
            dirty,
            history: History::new(),
            gesture_base: None,
            gesture_label: None,
            clipboard: None,
            media_store: MediaStore::new(),
            pending: BTreeMap::new(),
            next_job: 1,
            job_errors: Vec::new(),
            jobs: JobChannels::new(),
            wave_scale: 1.0,
            audio: None,
            retired: Vec::new(),
            audio_error: None,
            master_meter: Arc::new(StereoMeter::new()),
            meters: Arc::new(MeterBank {
                track_ids: Vec::new(),
                tracks: Vec::new(),
            }),
            transport_prev: None,
            preview_item: None,
            selection: Selection::None,
            selected_marker: None,
            last_clicked_track: None,
            exporting: None,
            export_result: None,
            autosave_interval: Duration::from_secs(120),
            last_autosave: Instant::now(),
            autosave_generation: 0,
            autosaved_generation: 0,
            missing_notice: false,
        };
        editor.start_media_loads();
        editor
    }

    /// Loads a project from a project.json file path or a project folder path.
    pub fn open(path: &Path) -> Result<Editor, EditorError> {
        let (project_paths, project, view) = project_file::load(path)?;
        Ok(Self::assemble(project_paths, project, view, false))
    }

    /// Opens a project, checking whether an autosave is newer than the last
    /// save. Returns `NeedsRecoveryChoice` so the UI can ask the user.
    pub fn open_with_recovery_check(path: &Path) -> Result<OpenOutcome, EditorError> {
        let (project_paths, _, _) = project_file::load(path)?;
        if autosave::newer_than_saved(&project_paths) {
            return Ok(OpenOutcome::NeedsRecoveryChoice(path.to_path_buf()));
        }
        Ok(OpenOutcome::Opened(Box::new(Self::open(path)?)))
    }

    /// Opens the project from its autosave snapshot; the editor starts dirty
    /// so the title shows the unsaved-changes mark.
    pub fn open_recovered(path: &Path) -> Result<Editor, EditorError> {
        let (project_paths, _, _) = project_file::load(path)?;
        let (project, view) = autosave::load(&project_paths)?;
        Ok(Self::assemble(project_paths, project, view, true))
    }

    /// Renames the autosave aside (recovery is refused for this session) and
    /// opens the project from its last save.
    pub fn open_discarding_autosave(path: &Path) -> Result<Editor, EditorError> {
        let (project_paths, _, _) = project_file::load(path)?;
        autosave::discard(&project_paths)?;
        Self::open(path)
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
            media_store: MediaStore::new(),
            pending: BTreeMap::new(),
            next_job: 1,
            job_errors: Vec::new(),
            jobs: JobChannels::new(),
            wave_scale: 1.0,
            audio: None,
            retired: Vec::new(),
            audio_error: None,
            master_meter: Arc::new(StereoMeter::new()),
            meters: Arc::new(MeterBank {
                track_ids: Vec::new(),
                tracks: Vec::new(),
            }),
            transport_prev: None,
            preview_item: None,
            selection: Selection::None,
            selected_marker: None,
            last_clicked_track: None,
            exporting: None,
            export_result: None,
            autosave_interval: Duration::from_secs(120),
            last_autosave: Instant::now(),
            autosave_generation: 0,
            autosaved_generation: 0,
            missing_notice: false,
        }
    }

    /// Saves project.json (atomically) and clears the dirty flag.
    pub fn save(&mut self) -> Result<(), EditorError> {
        project_file::save(&self.paths, &self.project, &self.view)?;
        self.dirty = false;
        // A successful manual save makes the autosave obsolete.
        let _ = autosave::delete(&self.paths);
        self.cleanup_unused_media();
        Ok(())
    }

    /// Copies the project folder to parent/new_name, renames the project,
    /// points the paths at the new root and saves there. The old folder is
    /// left untouched on disk.
    pub fn save_as(&mut self, parent: &Path, new_name: &str) -> Result<(), EditorError> {
        // The old folder's autosave belongs to the already-saved session.
        let _ = autosave::delete(&self.paths);
        let new_paths = project_file::copy_project(&self.paths, parent, new_name)?;
        Arc::make_mut(&mut self.project).name = new_name.to_string();
        self.paths = new_paths;
        self.save()?;
        Self::touch_recent(&self.project.name, &self.paths.root);
        Ok(())
    }

    /// Read-only access to the view state.
    pub fn view(&self) -> &ViewState {
        &self.view
    }

    /// Ids and file names of every MediaItem in the project and in every
    /// snapshot of the undo and redo stacks.
    ///
    /// Cleanup keeps media that a single undo (or redo) could bring back.
    pub fn referenced_media(&self) -> (HashSet<Uuid>, HashSet<String>) {
        let mut ids = HashSet::new();
        let mut files = HashSet::new();
        let mut add = |project: &Project| {
            for item in &project.media {
                ids.insert(item.id);
                files.insert(item.file_name.clone());
            }
        };
        add(&self.project);
        for entry in self.history.undo.iter().chain(self.history.redo.iter()) {
            add(&entry.before);
        }
        (ids, files)
    }

    /// Removes media and cache files that nothing references, after a Save.
    ///
    /// Skipped while any background job (import, load, relink or export) is
    /// still running, so a file a just-started job writes is never mistaken
    /// for junk. Messages about removed files and failures go into the job
    /// error list; failures never fail the Save. Tests delete directly
    /// instead of using the real Recycle Bin.
    fn cleanup_unused_media(&mut self) {
        if self.has_pending_jobs() {
            return;
        }
        let (keep_ids, keep_files) = self.referenced_media();
        let recycle = !cfg!(test);
        for message in paths::cleanup_unused(&self.paths, &keep_ids, &keep_files, recycle) {
            self.job_errors.push(message);
        }
    }

    /// Moves the playhead. Not undoable and does not mark the project dirty,
    /// but the view state is stored in project.json on Save.
    pub fn set_playhead(&mut self, samples: u64) {
        self.view.playhead = samples;
    }

    /// Sets horizontal zoom and scroll. Not undoable and does not mark the
    /// project dirty, but the view state is stored in project.json on Save.
    pub fn set_view_x(&mut self, spp: f64, scroll_x: u64) {
        self.view.samples_per_pixel = spp;
        self.view.scroll_x = scroll_x;
    }

    /// Sets the vertical scroll position in pixels. Not undoable and does not
    /// mark the project dirty.
    pub fn set_scroll_y(&mut self, y: f32) {
        self.view.scroll_y = y.max(0.0);
    }

    /// Sets one track's lane height in points. UI only: clamped to 40..=400,
    /// no undo history entry and no dirty flag.
    pub fn set_track_height(&mut self, id: Uuid, height: f32) {
        let mut next = (*self.project).clone();
        let Some(track) = next.tracks.iter_mut().find(|t| t.id == id) else {
            return;
        };
        track.height = height.clamp(40.0, 400.0);
        if next != *self.project {
            self.project = Arc::new(next);
        }
    }

    /// End of the last clip on any track, in samples; 0 when there are no clips.
    pub fn project_length(&self) -> u64 {
        self.project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.start + (c.src_out - c.src_in))
            .max()
            .unwrap_or(0)
    }

    /// Display-only vertical waveform scale, 1.0..=16.0.
    pub fn wave_scale(&self) -> f32 {
        self.wave_scale
    }

    /// Sets the display-only waveform scale, clamped to 1.0..=16.0.
    pub fn set_wave_scale(&mut self, scale: f32) {
        self.wave_scale = scale.clamp(1.0, 16.0);
    }
    /// The current selection; not undoable and never marks the project dirty.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Sets the range selection on the given tracks.
    ///
    /// `a` and `b` are ordered; the tracks are sorted into project order with
    /// duplicates removed. Equal ends or no tracks gives `None`.
    pub fn set_range(&mut self, a: u64, b: u64, tracks: Vec<Uuid>) {
        if a == b || tracks.is_empty() {
            self.selection = Selection::None;
            return;
        }
        let (start, end) = if a < b { (a, b) } else { (b, a) };
        let mut ranked: Vec<(usize, Uuid)> = tracks
            .into_iter()
            .filter_map(|id| {
                self.project
                    .tracks
                    .iter()
                    .position(|t| t.id == id)
                    .map(|index| (index, id))
            })
            .collect();
        ranked.sort_by_key(|&(index, _)| index);
        ranked.dedup_by_key(|(index, _)| *index);
        self.selection = Selection::Range {
            start,
            end,
            tracks: ranked.into_iter().map(|(_, id)| id).collect(),
        };
    }

    /// Ctrl+click on a track header: adds or removes a track from the current
    /// range selection. Does nothing when there is no range selection.
    pub fn toggle_range_track(&mut self, id: Uuid) {
        let Selection::Range { tracks, .. } = &mut self.selection else {
            return;
        };
        if let Some(position) = tracks.iter().position(|t| *t == id) {
            tracks.remove(position);
            if tracks.is_empty() {
                self.selection = Selection::None;
            }
            return;
        }
        if let Some(index) = self.project.tracks.iter().position(|t| t.id == id) {
            let insert_at = tracks
                .iter()
                .filter(|t| {
                    self.project
                        .tracks
                        .iter()
                        .position(|p| p.id == **t)
                        .is_some_and(|p| p < index)
                })
                .count();
            tracks.insert(insert_at, id);
        }
    }

    /// Replaces the clip selection with the given ids.
    pub fn select_clips(&mut self, ids: Vec<Uuid>) {
        if ids.is_empty() {
            self.selection = Selection::None;
        } else {
            self.selection = Selection::Clips(ids);
        }
    }

    /// Toggles one clip in the clip selection; selects it alone when another
    /// kind of selection (or none) is active. No-op for unknown clips.
    pub fn toggle_clip(&mut self, id: Uuid) {
        let exists = self
            .project
            .tracks
            .iter()
            .any(|t| t.clips.iter().any(|c| c.id == id));
        if !exists {
            return;
        }
        match &mut self.selection {
            Selection::Clips(ids) => {
                if let Some(position) = ids.iter().position(|c| *c == id) {
                    ids.remove(position);
                    if ids.is_empty() {
                        self.selection = Selection::None;
                    }
                } else {
                    ids.push(id);
                }
            }
            _ => self.selection = Selection::Clips(vec![id]),
        }
    }

    /// Esc: clears the selection.
    pub fn clear_selection(&mut self) {
        self.selection = Selection::None;
    }

    /// The selected marker, if any.
    pub fn selected_marker(&self) -> Option<Uuid> {
        self.selected_marker
    }

    /// Selects a marker. Not undoable, never dirty, and it does not touch
    /// the range or clip selection.
    pub fn select_marker(&mut self, id: Option<Uuid>) {
        self.selected_marker = id;
    }

    /// M: adds a marker at the playhead. Does nothing (without an error)
    /// when a marker already sits exactly at the playhead.
    pub fn add_marker_at_playhead(&mut self) -> Result<(), EditorError> {
        let pos = self.view.playhead;
        if self.project.markers.iter().any(|m| m.position == pos) {
            return Ok(());
        }
        self.apply(Action::AddMarker { pos })
    }

    /// `,`: seeks to the last marker strictly before the playhead.
    /// Nothing happens when there is none.
    pub fn jump_prev_marker(&mut self) {
        if let Some(pos) = marker::prev_marker(&self.project, self.view.playhead) {
            self.seek(pos);
        }
    }

    /// `.`: seeks to the first marker strictly after the playhead.
    /// Nothing happens when there is none.
    pub fn jump_next_marker(&mut self) {
        if let Some(pos) = marker::next_marker(&self.project, self.view.playhead) {
            self.seek(pos);
        }
    }

    /// Selects everything: range 0..project_length on all tracks, or `None`
    /// when the project is empty.
    pub fn select_all(&mut self) {
        let tracks: Vec<Uuid> = self.project.tracks.iter().map(|t| t.id).collect();
        let length = self.project_length();
        if tracks.is_empty() || length == 0 {
            self.selection = Selection::None;
            return;
        }
        self.selection = Selection::Range {
            start: 0,
            end: length,
            tracks,
        };
    }

    /// The track id of the last track clicked, if any.
    pub fn last_clicked_track(&self) -> Option<Uuid> {
        self.last_clicked_track
    }

    /// Remembers which track was last clicked.
    pub fn set_last_clicked_track(&mut self, id: Uuid) {
        self.last_clicked_track = Some(id);
    }

    /// Index of the target track: the topmost range track, else the last
    /// clicked track, else track 0. Missing tracks fall through.
    pub fn target_track(&self) -> usize {
        if let Selection::Range { tracks, .. } = &self.selection
            && let Some(index) = tracks
                .first()
                .and_then(|id| self.project.tracks.iter().position(|t| t.id == *id))
        {
            return index;
        }
        if let Some(index) = self
            .last_clicked_track
            .and_then(|id| self.project.tracks.iter().position(|t| t.id == id))
        {
            return index;
        }
        0
    }

    /// N: toggles snapping in the view state; not undoable, not dirty.
    pub fn toggle_snap(&mut self) {
        self.view.snap = !self.view.snap;
    }

    /// Toggles ripple editing in the view state; not undoable, not dirty.
    pub fn toggle_ripple(&mut self) {
        self.view.ripple = !self.view.ripple;
    }

    /// Drops selection ids that no longer exist after a project change.
    fn tidy_selection(&mut self) {
        self.selection = std::mem::take(&mut self.selection).tidy(&self.project);
        if let Some(id) = self.selected_marker
            && !self.project.markers.iter().any(|m| m.id == id)
        {
            self.selected_marker = None;
        }
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

    /// Copy: with a range, copies it (see `copy`); with clips selected, copies
    /// them whole with their track offsets; with nothing selected, does nothing.
    pub fn copy_selection(&mut self) {
        match self.selection.clone() {
            Selection::Range { start, end, tracks } => self.copy(start, end, &tracks),
            Selection::Clips(ids) => {
                self.clipboard = Some(Arc::new(copy_clips(&self.project, &ids)));
            }
            Selection::None => {}
        }
    }

    /// Cut: with a range, copies then deletes it with the current ripple,
    /// clears the selection and puts the playhead at the range start; with
    /// clips selected, copies them then deletes them (label "Cut").
    pub fn cut_selection(&mut self) -> Result<(), EditorError> {
        match self.selection.clone() {
            Selection::Range { start, end, tracks } => {
                self.cut(start, end, tracks, self.view.ripple)?;
                self.clear_selection();
                self.set_playhead(start);
                Ok(())
            }
            Selection::Clips(ids) => {
                self.clipboard = Some(Arc::new(copy_clips(&self.project, &ids)));
                self.apply_labeled(
                    Action::DeleteClips {
                        clip_ids: ids,
                        ripple: self.view.ripple,
                    },
                    "Cut",
                )
            }
            Selection::None => Ok(()),
        }
    }

    /// Paste: does nothing with an empty clipboard; with a range selected,
    /// replaces it and puts the playhead at the range start; otherwise inserts
    /// at the playhead on the target track.
    pub fn paste(&mut self) -> Result<(), EditorError> {
        if self
            .clipboard()
            .is_none_or(|c| c.tracks.iter().all(|t| t.is_empty()))
        {
            return Ok(());
        }
        match self.selection.clone() {
            Selection::Range { start, end, .. } => {
                let target = self.target_track();
                self.paste_replace_range(start, end, target, self.view.ripple)?;
                self.clear_selection();
                self.set_playhead(start);
                Ok(())
            }
            _ => {
                let pos = self.view.playhead;
                let target = self.target_track();
                self.paste_insert_at(pos, target)
            }
        }
    }

    /// Split: splits clips under the playhead on the range tracks, or on all
    /// tracks when nothing (or only clips) is selected.
    pub fn split(&mut self) -> Result<(), EditorError> {
        let pos = self.view.playhead;
        let track_ids = match &self.selection {
            Selection::Range { tracks, .. } => Some(tracks.clone()),
            _ => None,
        };
        self.apply(Action::SplitAt { pos, track_ids })
    }

    /// Trim to selection: removes everything outside the range on the range
    /// tracks; keeps the selection and does nothing without a range.
    pub fn trim_to_selection(&mut self) -> Result<(), EditorError> {
        match self.selection.clone() {
            Selection::Range { start, end, tracks } => self.apply(Action::TrimToSelection {
                s: start,
                e: end,
                track_ids: tracks,
            }),
            _ => Ok(()),
        }
    }

    /// What amplitude processing applies to: the range if any, else the
    /// selected clips, else `None`.
    pub fn process_target(&self) -> Option<Target> {
        match &self.selection {
            Selection::Range { start, end, tracks } => Some(Target::Range {
                start: *start,
                end: *end,
                tracks: tracks.clone(),
            }),
            Selection::Clips(ids) => Some(Target::Clips(ids.clone())),
            Selection::None => None,
        }
    }

    /// Highest absolute sample per track id across the fragments, after clip
    /// gain and fades. Offline or missing media counts as silence.
    pub fn measure_peak(&self, project: &Project, frags: &[(Uuid, Uuid)]) -> HashMap<Uuid, f32> {
        let mut peaks: HashMap<Uuid, f32> = HashMap::new();
        for (track_id, clip_id) in frags {
            let entry = peaks.entry(*track_id).or_insert(0.0);
            let Some(clip) = project
                .tracks
                .iter()
                .flat_map(|track| track.clips.iter())
                .find(|clip| clip.id == *clip_id)
            else {
                continue;
            };
            if self.media_store.is_offline(clip.media_id) {
                continue;
            }
            let (Some(loaded), Some(item)) = (
                self.media_store.get(clip.media_id),
                project.media.iter().find(|item| item.id == clip.media_id),
            ) else {
                continue;
            };
            let channels = usize::from(item.channels).max(1);
            let len = clip.len();
            for local in 0..len {
                let frame = (clip.src_in + local) as usize;
                let env =
                    db_to_linear(clip.gain_db) * fade_gain(local, len, clip.fade_in, clip.fade_out);
                for ch in 0..channels {
                    if let Some(sample) = loaded.samples.get(frame * channels + ch) {
                        *entry = entry.max(sample.abs() * env);
                    }
                }
            }
        }
        peaks
    }

    /// Normalizes the selection so its peak hits `target_dbfs`; with
    /// `per_track` each track gets its own gain change.
    pub fn normalize(&mut self, target_dbfs: f32, per_track: bool) -> Result<(), EditorError> {
        let Some(target) = self.process_target() else {
            return Ok(());
        };
        let mut scratch = (*self.project).clone();
        if let Target::Range { start, end, tracks } = &target {
            amplitude::split_edges(&mut scratch, *start, *end, tracks)?;
        }
        let frags = amplitude::fragments(&scratch, &target);
        let peaks = self.measure_peak(&scratch, &frags);
        let highest = peaks.values().copied().fold(0.0f32, f32::max);
        if highest <= 0.0 {
            return Err(OpError::Invalid(
                "Nothing to normalize: the selection is silent".to_string(),
            )
            .into());
        }
        let deltas: Vec<(Uuid, f32)> = if per_track {
            peaks
                .iter()
                .map(|(id, peak)| {
                    let delta = if *peak > 0.0 {
                        target_dbfs - linear_to_db(*peak)
                    } else {
                        0.0
                    };
                    (*id, delta)
                })
                .collect()
        } else {
            let delta = target_dbfs - linear_to_db(highest);
            peaks.keys().map(|id| (*id, delta)).collect()
        };
        self.apply(Action::Normalize { target, deltas })
    }

    /// Adds `delta_db` of gain to the selected range or clips.
    pub fn apply_gain(&mut self, delta_db: f32) -> Result<(), EditorError> {
        match self.process_target() {
            Some(target) => self.apply(Action::ApplyGain { target, delta_db }),
            None => Ok(()),
        }
    }

    /// Fades in over the selected range or whole selected clips.
    pub fn fade_in(&mut self) -> Result<(), EditorError> {
        match self.process_target() {
            Some(target) => self.apply(Action::FadeIn { target }),
            None => Ok(()),
        }
    }

    /// Fades out over the selected range or whole selected clips.
    pub fn fade_out(&mut self) -> Result<(), EditorError> {
        match self.process_target() {
            Some(target) => self.apply(Action::FadeOut { target }),
            None => Ok(()),
        }
    }

    /// Silences the selected range on its tracks; needs a range selection.
    pub fn silence(&mut self) -> Result<(), EditorError> {
        match self.selection.clone() {
            Selection::Range { start, end, tracks } => self.apply(Action::Silence {
                s: start,
                e: end,
                track_ids: tracks,
            }),
            _ => Ok(()),
        }
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
        if self.exporting.is_some() {
            return Err(EditorError::Locked(EXPORT_LOCKED.to_string()));
        }
        self.commit(label, |project| action.run(project))
    }

    /// Runs one pure edit op on a clone, checks invariants, pushes undo and swaps in.
    fn commit(
        &mut self,
        label: &'static str,
        run: impl FnOnce(&mut Project) -> Result<(), OpError>,
    ) -> Result<(), EditorError> {
        if self.exporting.is_some() {
            return Err(EditorError::Locked(EXPORT_LOCKED.to_string()));
        }
        let mut next = (*self.project).clone();
        run(&mut next)?;
        invariants::check(&next)?;
        self.history.push_undo(HistoryEntry {
            label,
            before: Arc::clone(&self.project),
        });
        self.project = Arc::new(next);
        self.tidy_selection();
        self.dirty = true;
        self.autosave_generation += 1;
        self.publish();
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
        if self.exporting.is_some() {
            return false;
        }
        match self.history.undo_step(Arc::clone(&self.project)) {
            Some(entry) => {
                self.project = entry.before;
                self.tidy_selection();
                self.dirty = true;
                self.autosave_generation += 1;
                self.publish();
                true
            }
            None => false,
        }
    }

    /// Restores the top redo entry; returns false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        if self.exporting.is_some() {
            return false;
        }
        match self.history.redo_step(Arc::clone(&self.project)) {
            Some(entry) => {
                self.project = entry.before;
                self.tidy_selection();
                self.dirty = true;
                self.autosave_generation += 1;
                self.publish();
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
        if self.exporting.is_some() {
            return;
        }
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
        self.publish();
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
                self.autosave_generation += 1;
                self.publish();
            }
        }
    }

    /// Cancels the gesture: restores the base snapshot and records nothing.
    pub fn cancel_gesture(&mut self) {
        if let Some(base) = self.gesture_base.take() {
            self.project = base;
            self.publish();
        }
        self.gesture_label = None;
    }

    /// Starts the audio engine for the project rate; stores the error on failure.
    pub fn start_audio(&mut self) {
        if self.audio.is_some() {
            return;
        }
        match Engine::start(self.project.sample_rate) {
            Ok(engine) => self.audio = Some(engine),
            Err(error) => self.audio_error = Some(error.to_string()),
        }
    }

    /// Publishes a fresh MixPlan to the audio engine; a no-op without audio.
    /// Reuses the meter bank while the track id list is unchanged.
    pub fn publish(&mut self) {
        self.ensure_meters();
        let Some(engine) = &self.audio else {
            return;
        };
        let plan = self.build_plan();
        let old = engine.publish_timeline(plan);
        self.retired.push((Instant::now(), old));
    }

    /// Recreates the meter bank when the track list changed.
    fn ensure_meters(&mut self) {
        let ids: Vec<Uuid> = self.project.tracks.iter().map(|t| t.id).collect();
        if self.meters.track_ids != ids {
            self.meters = Arc::new(MeterBank {
                track_ids: ids,
                tracks: self
                    .project
                    .tracks
                    .iter()
                    .map(|_| StereoMeter::new())
                    .collect(),
            });
        }
    }

    /// Builds a fresh MixPlan snapshot from the project and the media store.
    fn build_plan(&self) -> Arc<MixPlan> {
        let media = &self.media_store;
        Arc::new(MixPlan::build(
            &self.project,
            &|id| media.get(id).map(|l| Arc::clone(&l.samples)),
            Arc::clone(&self.meters),
            Arc::clone(&self.master_meter),
        ))
    }

    /// Drops retired mix plans older than 2 s and runs the autosave; called
    /// every frame.
    pub fn tick(&mut self) {
        self.retired
            .retain(|(at, _)| at.elapsed() < Duration::from_secs(2));
        self.missing_media_notice();
        self.autosave_tick();
    }

    /// Shows the missing-media notice once, after all media loads finished.
    ///
    /// The message goes through the job error list so the app's message modal
    /// surfaces it.
    fn missing_media_notice(&mut self) {
        if !self.missing_notice || self.has_pending_jobs() {
            return;
        }
        self.missing_notice = false;
        let count = self.offline_count();
        if count > 0 {
            self.job_errors.push(format!(
                "{count} media files are missing. Their clips are shown in red and play silence. Right-click an item in the bin and choose Relink to fix it."
            ));
        }
    }

    /// Writes the autosave when the project is dirty, an edit happened since
    /// the last autosave and the interval has passed. Errors are recorded in
    /// the job error list, never panicked. Runs during an export too (the
    /// autosave only reads the project).
    fn autosave_tick(&mut self) {
        if !self.dirty || self.autosaved_generation == self.autosave_generation {
            return;
        }
        if self.last_autosave.elapsed() < self.autosave_interval {
            return;
        }
        self.last_autosave = Instant::now();
        if let Err(error) = autosave::write(&self.paths, &self.project, &self.view) {
            self.job_errors.push(format!("Autosave failed: {error}"));
            return;
        }
        self.autosaved_generation = self.autosave_generation;
    }

    /// Takes the current meter readings for every track and the master.
    /// Each value leaves its accumulator zeroed.
    pub fn read_meters(&self) -> (Vec<(Uuid, [MeterReading; 2])>, [MeterReading; 2]) {
        let tracks = self
            .meters
            .track_ids
            .iter()
            .zip(&self.meters.tracks)
            .map(|(id, m)| (*id, [m.l.take(), m.r.take()]))
            .collect();
        let master = [self.master_meter.l.take(), self.master_meter.r.take()];
        (tracks, master)
    }

    /// One-line audio summary for the status bar.
    pub fn audio_status(&self) -> String {
        match &self.audio {
            Some(engine) => engine.device_info().to_string(),
            None => match &self.audio_error {
                Some(error) => format!("No audio: {error}"),
                None => "No audio".to_string(),
            },
        }
    }

    /// Previews a bin item on its own: publishes a preview MixPlan, starts it
    /// and stops timeline playback. A no-op without audio or unloaded media.
    pub fn preview_media(&mut self, id: Uuid) {
        if self.exporting.is_some() {
            return;
        }
        let engine = match &self.audio {
            Some(engine) => engine,
            None => return,
        };
        let Some(loaded) = self.media_store.get(id) else {
            return;
        };
        let Some(item) = self.project.media.iter().find(|m| m.id == id) else {
            return;
        };
        let plan = MixPlan::preview(
            item.length,
            item.channels,
            Arc::clone(&loaded.samples),
            self.project.sample_rate,
            self.project.master_volume_db,
            Arc::clone(&self.master_meter),
        );
        let old = engine.publish_preview(Arc::new(plan));
        self.retired.push((Instant::now(), old));
        engine.send(Command::Preview);
        self.preview_item = Some(id);
    }

    /// Stops the running bin preview, if any.
    pub fn stop_preview(&mut self) {
        if self.transport_state() == TransportState::Previewing
            && let Some(engine) = &self.audio
        {
            engine.send(Command::StopPreview);
        }
        self.preview_item = None;
    }

    /// The item being previewed while the transport state is Previewing.
    pub fn previewing(&self) -> Option<Uuid> {
        if self.transport_state() == TransportState::Previewing {
            self.preview_item
        } else {
            None
        }
    }

    /// Bin preview progress 0.0..1.0; 0.0 when not previewing.
    pub fn preview_progress(&self) -> f32 {
        let Some(engine) = &self.audio else {
            return 0.0;
        };
        if self.transport_state() != TransportState::Previewing {
            return 0.0;
        }
        let Some(item) = self
            .preview_item
            .and_then(|id| self.project.media.iter().find(|m| m.id == id))
        else {
            return 0.0;
        };
        if item.length == 0 {
            return 0.0;
        }
        let pos = engine.shared().preview_pos.load(Ordering::Relaxed);
        (pos as f32 / item.length as f32).clamp(0.0, 1.0)
    }

    /// Starts background import jobs for the supported files.
    ///
    /// Returns the file names that were rejected because their extension is
    /// not supported; one import job is started per supported file.
    pub fn import_files(&mut self, files: Vec<PathBuf>) -> Vec<String> {
        if self.exporting.is_some() {
            return Vec::new();
        }
        let mut rejected = Vec::new();
        for file in files {
            if crate::io::decode::is_supported(&file) {
                let job = self.next_job;
                self.next_job += 1;
                self.pending.insert(job, ("Import".to_string(), 0.0));
                self.jobs.spawn_import(
                    job,
                    file,
                    self.paths.clone(),
                    Uuid::new_v4(),
                    self.project.sample_rate,
                );
            } else {
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string());
                rejected.push(name);
            }
        }
        rejected
    }

    /// Starts a relink job: copies a replacement file in for an offline item.
    ///
    /// A no-op during an export or for an id that is not in the bin. The job
    /// copies the file into media/, rebuilds the cache and, on success, checks
    /// that every clip using the item still fits inside the replacement.
    pub fn relink(&mut self, id: Uuid, src: PathBuf) {
        if self.exporting.is_some() {
            return;
        }
        let Some(item) = self.project.media.iter().find(|m| m.id == id) else {
            return;
        };
        let item = item.clone();
        let job = self.next_job;
        self.next_job += 1;
        self.pending.insert(job, ("Relink".to_string(), 0.0));
        self.jobs
            .spawn_relink(job, src, self.paths.clone(), item, self.project.sample_rate);
    }

    /// Applies a finished relink job: the length rule first, then the swap.
    ///
    /// When a clip using the item reaches past the replacement's length, the
    /// new copy and its cache files are deleted and the SPEC message is
    /// reported; nothing else changes. Otherwise the loaded audio replaces the
    /// store entry (clearing the offline mark) and the bin entry is updated as
    /// one undoable "Relink" step.
    fn finish_relink(
        &mut self,
        item: crate::model::media::MediaItem,
        loaded: crate::io::import::Loaded,
    ) {
        // Length rule: run the pure op on a clone; on failure undo the copy.
        let mut probe = (*self.project).clone();
        if let Err(error) = crate::editor::ops::media::relink_media(
            &mut probe,
            item.id,
            item.file_name.clone(),
            item.original_rate,
            item.channels,
            item.length,
        ) {
            let _ = std::fs::remove_file(self.paths.media().join(&item.file_name));
            let _ = std::fs::remove_file(conformed_path(&self.paths, item.id));
            let _ = std::fs::remove_file(peaks_path(&self.paths, item.id));
            self.job_errors.push(
                error
                    .to_string()
                    .strip_prefix("invalid operation: ")
                    .unwrap_or(&error.to_string())
                    .to_string(),
            );
            return;
        }
        self.media_store.insert(item.id, loaded);
        if let Err(error) = self.apply(Action::RelinkMedia {
            id: item.id,
            file_name: item.file_name.clone(),
            original_rate: item.original_rate,
            channels: item.channels,
            length: item.length,
        }) {
            self.job_errors.push(error.to_string());
        }
    }

    /// Number of bin items whose media file is missing from media/.
    pub fn offline_count(&self) -> usize {
        self.project
            .media
            .iter()
            .filter(|m| self.media_store.is_offline(m.id))
            .count()
    }

    /// Drains worker-thread messages without blocking.
    pub fn poll_jobs(&mut self) {
        while let Ok(message) = self.jobs.rx.try_recv() {
            match message {
                JobMessage::Progress { job, frac } => {
                    if let Some(entry) = self.pending.get_mut(&job) {
                        entry.1 = frac;
                    }
                }
                JobMessage::ImportDone { job, result } => {
                    self.pending.remove(&job);
                    match result {
                        Ok((item, loaded)) => {
                            self.media_store.insert(item.id, loaded);
                            if let Err(error) = self.apply(Action::ImportMedia { item }) {
                                self.job_errors.push(error.to_string());
                            }
                        }
                        Err(error) => self.job_errors.push(error.to_string()),
                    }
                }
                JobMessage::LoadDone {
                    job,
                    media_id,
                    result,
                } => {
                    self.pending.remove(&job);
                    match result {
                        Ok(loaded) => {
                            self.media_store.insert(media_id, loaded);
                            self.publish();
                        }
                        Err(IoError::MediaOffline(id)) => {
                            self.media_store.set_offline(id);
                            self.publish();
                        }
                        Err(error) => {
                            self.media_store.set_offline(media_id);
                            self.job_errors.push(error.to_string());
                        }
                    }
                }
                JobMessage::RelinkDone { job, result, .. } => {
                    self.pending.remove(&job);
                    match result {
                        Ok((item, loaded)) => self.finish_relink(item, loaded),
                        Err(IoError::MediaOffline(id)) => {
                            self.media_store.set_offline(id);
                        }
                        Err(error) => self.job_errors.push(error.to_string()),
                    }
                }
                JobMessage::ExportProgress { job, pass, frac } => {
                    if let Some(entry) = self.pending.get_mut(&job) {
                        entry.1 = frac;
                    }
                    if let Some(state) = &mut self.exporting {
                        state.pass = pass;
                        state.frac = frac;
                    }
                }
                JobMessage::ExportPeak { job: _, peak_db } => {
                    if let Some(state) = &mut self.exporting {
                        state.peak_warning = Some(peak_db);
                    }
                }
                JobMessage::ExportDone { job, result } => {
                    self.pending.remove(&job);
                    self.exporting = None;
                    self.export_result = Some(result);
                }
            }
        }
    }

    /// Starts one background load job per media item (used on open).
    fn start_media_loads(&mut self) {
        if !self.project.media.is_empty() {
            self.missing_notice = true;
        }
        for item in self.project.media.clone() {
            let job = self.next_job;
            self.next_job += 1;
            self.pending.insert(job, ("Load".to_string(), 0.0));
            self.jobs
                .spawn_load(job, item, self.paths.clone(), self.project.sample_rate);
        }
    }

    /// Pending background jobs as (label, progress 0.0..1.0), oldest first.
    pub fn pending_jobs(&self) -> Vec<(String, f32)> {
        self.pending.values().cloned().collect()
    }

    /// True while any background job is still running.
    pub fn has_pending_jobs(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Takes the messages recorded from failed background jobs.
    pub fn take_job_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.job_errors)
    }

    /// Loaded audio for the media id, if it has been imported or loaded.
    pub fn media(&self, id: Uuid) -> Option<&crate::io::import::Loaded> {
        self.media_store.get(id)
    }

    /// True when the media item's file is missing from media/.
    pub fn is_media_offline(&self, id: Uuid) -> bool {
        self.media_store.is_offline(id)
    }

    /// Number of clips on the timeline that use the media item.
    pub fn media_usage(&self, id: Uuid) -> usize {
        self.project
            .tracks
            .iter()
            .map(|t| t.clips.iter().filter(|c| c.media_id == id).count())
            .sum()
    }

    /// Reveals the media item's file in media/ in Windows Explorer.
    pub fn reveal_media(&self, id: Uuid) {
        if let Some(item) = self.project.media.iter().find(|m| m.id == id) {
            paths::reveal_in_explorer(&self.paths.media().join(&item.file_name));
        }
    }

    /// Reveals any file on disk (e.g. a finished export) in Windows Explorer.
    pub fn reveal_file(&self, path: &std::path::Path) {
        paths::reveal_in_explorer(path);
    }

    /// Places a clip of the given media item at `start` (drop from the bin).
    ///
    /// `Some(track_index)` places on that track, overwriting what it covers, or
    /// inserting and shifting later clips right when `insert`. `None` appends a
    /// new track holding the clip (`insert` is ignored). The clip spans the full
    /// media (`src_in` 0, `src_out` = media length). Returns the error unchanged
    /// and without touching the project for unknown media or a bad track index.
    pub fn drop_media(
        &mut self,
        media_id: Uuid,
        track_index: Option<usize>,
        start: u64,
        insert: bool,
    ) -> Result<(), EditorError> {
        let media_len = self
            .project
            .media
            .iter()
            .find(|media| media.id == media_id)
            .map(|media| media.length)
            .ok_or(EditorError::Op(OpError::NotFound(media_id)))?;
        match track_index {
            Some(index) => {
                let track_id = self
                    .project
                    .tracks
                    .get(index)
                    .map(|track| track.id)
                    .ok_or(EditorError::Op(OpError::Overflow))?;
                self.apply(Action::PlaceClip {
                    track_id,
                    media_id,
                    start,
                    src_in: 0,
                    src_out: media_len,
                    insert,
                })
            }
            None => self.apply(Action::PlaceClipOnNewTrack { media_id, start }),
        }
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

/// Progress of the running export, read by the UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExportStatus {
    /// Render pass, 1 (peak scan) or 2 (write).
    pub pass: u8,
    /// Fraction of the current pass done, 0.0..=1.0.
    pub frac: f32,
    /// Peak level in dBFS when pass 1 found the mix above 0 dBFS.
    pub peak_warning: Option<f32>,
}

/// Bookkeeping for the running export job.
struct ExportState {
    /// Current render pass.
    pass: u8,
    /// Fraction of the current pass done.
    frac: f32,
    /// Peak level in dBFS reported by pass 1, if it found clipping.
    peak_warning: Option<f32>,
    /// Sends the Editor's clip-warning decision to the job.
    decision_tx: Sender<ExportDecision>,
    /// Set to abort the job within one render block.
    cancel: Arc<AtomicBool>,
}

impl Editor {
    /// True while an export job runs: editing, undo and transport are locked.
    pub fn is_locked(&self) -> bool {
        self.exporting.is_some()
    }

    /// Starts an export job on its own worker thread; stops playback first.
    ///
    /// `settings.end` is clamped to the project length. When `settings.tracks`
    /// is given, every other track becomes inaudible for the export (mute and
    /// solo still apply to the rest). The job reports progress, an optional
    /// clip warning and its result back through `poll_jobs`.
    pub fn start_export(&mut self, settings: ExportSettings) -> Result<(), EditorError> {
        if self.exporting.is_some() {
            return Err(EditorError::Locked(EXPORT_LOCKED.to_string()));
        }
        self.stop();
        self.stop_preview();
        let end = settings.end.min(self.project_length());
        if settings.start >= end {
            return Err(EditorError::Io(IoError::Export(
                "empty export range".to_string(),
            )));
        }
        let mut settings = settings;
        settings.end = end;
        self.ensure_meters();
        let mut plan = self.build_plan();
        if let Some(keep) = &settings.tracks {
            let mut scoped = (*plan).clone();
            for (index, track) in scoped.tracks.iter_mut().enumerate() {
                track.audible = self
                    .project
                    .tracks
                    .get(index)
                    .is_some_and(|t| keep.contains(&t.id));
            }
            plan = Arc::new(scoped);
        }
        if let Some(parent) = settings.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let job = self.next_job;
        self.next_job += 1;
        self.pending.insert(job, ("Export".to_string(), 0.0));
        let (decision_tx, decision_rx) = crossbeam_channel::bounded(1);
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs
            .spawn_export(job, plan, settings, decision_rx, Arc::clone(&cancel));
        self.exporting = Some(ExportState {
            pass: 1,
            frac: 0.0,
            peak_warning: None,
            decision_tx,
            cancel,
        });
        Ok(())
    }

    /// Progress of the running export, if any.
    pub fn export_status(&self) -> Option<ExportStatus> {
        self.exporting.as_ref().map(|state| ExportStatus {
            pass: state.pass,
            frac: state.frac,
            peak_warning: state.peak_warning,
        })
    }

    /// Export Anyway on the clip warning; harmless when there is no warning.
    pub fn export_continue(&mut self) {
        if let Some(state) = &self.exporting {
            let _ = state.decision_tx.send(ExportDecision::Continue);
        }
    }

    /// Cancels the running export; its result becomes Err("Cancelled").
    pub fn export_cancel(&mut self) {
        if let Some(state) = &self.exporting {
            state.cancel.store(true, Ordering::Relaxed);
            let _ = state.decision_tx.send(ExportDecision::Cancel);
        }
    }

    /// Takes the finished export's result, if poll_jobs has received one.
    pub fn take_export_result(&mut self) -> Option<Result<PathBuf, String>> {
        self.export_result.take()
    }

    /// Default export file: exports/<project name>.wav or .mp3.
    pub fn default_export_path(&self, kind: ExportKind) -> PathBuf {
        let extension = match kind {
            ExportKind::Wav(_) => "wav",
            ExportKind::Mp3 { .. } => "mp3",
        };
        self.paths
            .exports()
            .join(format!("{}.{}", self.project.name, extension))
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

    /// Builds a real project layout holding one bin item ("test.wav") with a
    /// clip, writes its media file and returns the editor and the media id.
    fn cleanup_editor() -> (tempfile::TempDir, Editor, Uuid) {
        let parent = tempdir().unwrap();
        let paths = crate::io::paths::create_layout(parent.path(), "Cleanup").unwrap();
        let project = project_with_clips(&[&[(0, 100)]]);
        let id = project.media[0].id;
        std::fs::write(paths.media().join("test.wav"), b"x").unwrap();
        let editor = Editor::with_project(project, ViewState::default(), paths);
        (parent, editor, id)
    }

    #[test]
    fn save_removes_unreferenced_media_and_cache_files() {
        let (_parent, mut editor, id) = cleanup_editor();
        let gone_id = Uuid::new_v4();
        std::fs::write(editor.paths.media().join("gone.wav"), b"x").unwrap();
        std::fs::write(editor.paths.cache().join(format!("{id}.f32.wav")), b"x").unwrap();
        std::fs::write(editor.paths.cache().join(format!("{id}.peaks")), b"x").unwrap();
        std::fs::write(
            editor.paths.cache().join(format!("{gone_id}.f32.wav")),
            b"x",
        )
        .unwrap();
        std::fs::write(editor.paths.cache().join(format!("{gone_id}.peaks")), b"x").unwrap();

        editor.save().unwrap();

        // Referenced files stay, unreferenced ones are removed and reported.
        assert!(editor.paths.media().join("test.wav").is_file());
        assert!(!editor.paths.media().join("gone.wav").exists());
        assert!(editor.paths.cache().join(format!("{id}.f32.wav")).is_file());
        assert!(editor.paths.cache().join(format!("{id}.peaks")).is_file());
        assert!(
            !editor
                .paths
                .cache()
                .join(format!("{gone_id}.f32.wav"))
                .exists()
        );
        assert!(
            !editor
                .paths
                .cache()
                .join(format!("{gone_id}.peaks"))
                .exists()
        );
        let messages = editor.take_job_errors().join("\n");
        assert!(messages.contains("gone.wav"), "got: {messages}");
        assert!(messages.contains(&gone_id.to_string()), "got: {messages}");
    }

    #[test]
    fn save_keeps_media_that_undo_can_restore() {
        let (_parent, mut editor, id) = cleanup_editor();

        // Remove the bin item; the file must survive the next Save because
        // the undo history still holds it.
        editor.apply(Action::RemoveMedia { id }).unwrap();
        assert!(editor.project.media.is_empty());
        editor.save().unwrap();
        assert!(
            editor.paths.media().join("test.wav").is_file(),
            "undo history must keep the media file"
        );

        // Once the history no longer holds it (as after closing the project),
        // the same Save cleans it up.
        editor.history = History::new();
        editor.save().unwrap();
        assert!(!editor.paths.media().join("test.wav").exists());
        assert!(!editor.paths.cache().join(format!("{id}.f32.wav")).exists());
        assert!(!editor.paths.cache().join(format!("{id}.peaks")).exists());
    }

    #[test]
    fn save_skips_cleanup_while_a_job_is_pending() {
        let (_parent, mut editor, _id) = cleanup_editor();
        std::fs::write(editor.paths.media().join("gone.wav"), b"x").unwrap();

        editor.pending.insert(99, ("Import".to_string(), 0.5));
        editor.save().unwrap();
        assert!(
            editor.paths.media().join("gone.wav").is_file(),
            "cleanup must be skipped while a job is running"
        );

        editor.pending.clear();
        editor.save().unwrap();
        assert!(!editor.paths.media().join("gone.wav").exists());
    }

    #[test]
    fn referenced_media_includes_undo_and_redo_history() {
        let (_parent, mut editor, id) = cleanup_editor();
        let extra = crate::model::media::MediaItem {
            id: Uuid::new_v4(),
            display_name: "Extra".to_string(),
            file_name: "extra.wav".to_string(),
            original_rate: 48_000,
            channels: 2,
            length: 100,
        };

        // A snapshot holding the extra item sits on the redo stack.
        let mut redo_project = (*editor.project).clone();
        redo_project.media.push(extra.clone());
        editor
            .history
            .redo
            .push(crate::editor::history::HistoryEntry {
                label: "Import",
                before: std::sync::Arc::new(redo_project),
            });
        let (ids, files) = editor.referenced_media();
        assert!(ids.contains(&id));
        assert!(files.contains("test.wav"));
        assert!(ids.contains(&extra.id), "redo snapshot media must be kept");
        assert!(
            files.contains("extra.wav"),
            "redo snapshot media must be kept"
        );

        // Removing the item leaves it only on the undo stack, but it is kept.
        editor.history.redo.clear();
        editor.apply(Action::RemoveMedia { id }).unwrap();
        let (ids, files) = editor.referenced_media();
        assert!(ids.contains(&id), "undo snapshot media must be kept");
        assert!(
            files.contains("test.wav"),
            "undo snapshot media must be kept"
        );
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

    fn range_target(project: &Project, start: u64, end: u64, tracks: &[usize]) -> Target {
        Target::Range {
            start,
            end,
            tracks: tracks.iter().map(|&i| project.tracks[i].id).collect(),
        }
    }

    fn test_editor(project: Project) -> Editor {
        Editor::with_project(
            project,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        )
    }

    /// Stores `frames` stereo frames of constant `value` for the media id.
    fn put_constant(editor: &mut Editor, media_id: Uuid, value: f32, frames: usize) {
        editor.media_store.insert(
            media_id,
            crate::io::import::Loaded {
                samples: vec![value; frames * 2].into(),
                peaks: crate::model::media::Peaks {
                    channels: 2,
                    frames: frames as u64,
                    levels: Vec::new(),
                }
                .into(),
            },
        );
    }

    #[test]
    fn apply_gain_range_splits_and_gains_only_the_middle() {
        let project = project_with_clips(&[&[(100, 200)]]);
        let target = range_target(&project, 150, 250, &[0]);
        let result = roundtrip(
            project,
            Action::ApplyGain {
                target,
                delta_db: 6.0,
            },
        );
        assert_eq!(spans(&result, 0), [(100, 150), (150, 250), (250, 300)]);
        let gains: Vec<f32> = result.tracks[0].clips.iter().map(|c| c.gain_db).collect();
        assert_eq!(gains, [0.0, 6.0, 0.0]);
    }

    #[test]
    fn apply_gain_clamps_at_plus_24() {
        let project = project_with_clips(&[&[(100, 200)]]);
        let clip = project.tracks[0].clips[0].id;
        let result = roundtrip(
            project,
            Action::ApplyGain {
                target: Target::Clips(vec![clip]),
                delta_db: 30.0,
            },
        );
        assert_eq!(result.tracks[0].clips[0].gain_db, 24.0);
    }

    #[test]
    fn apply_gain_clips_target_changes_whole_clips_without_splitting() {
        let project = project_with_clips(&[&[(100, 100), (300, 100)]]);
        let ids: Vec<Uuid> = project.tracks[0].clips.iter().map(|c| c.id).collect();
        let result = roundtrip(
            project,
            Action::ApplyGain {
                target: Target::Clips(ids),
                delta_db: -3.0,
            },
        );
        assert_eq!(spans(&result, 0), [(100, 200), (300, 400)]);
        assert!(result.tracks[0].clips.iter().all(|c| c.gain_db == -3.0));
    }

    #[test]
    fn fade_in_inside_one_clip_sets_fragment_fade() {
        let project = project_with_clips(&[&[(100, 100)]]);
        let target = range_target(&project, 120, 180, &[0]);
        let result = roundtrip(project, Action::FadeIn { target });
        assert_eq!(spans(&result, 0), [(100, 120), (120, 180), (180, 200)]);
        let mid = &result.tracks[0].clips[1];
        assert_eq!((mid.fade_in, mid.fade_out), (60, 0));
    }

    #[test]
    fn fade_out_inside_one_clip_sets_fragment_fade() {
        let project = project_with_clips(&[&[(100, 100)]]);
        let target = range_target(&project, 120, 180, &[0]);
        let result = roundtrip(project, Action::FadeOut { target });
        let mid = &result.tracks[0].clips[1];
        assert_eq!((mid.fade_in, mid.fade_out), (0, 60));
    }

    #[test]
    fn fade_in_across_two_clips_errors_and_changes_nothing() {
        let project = project_with_clips(&[&[(100, 50), (150, 50)]]);
        let target = range_target(&project, 120, 180, &[0]);
        let mut editor = test_editor(project.clone());
        let result = editor.apply(Action::FadeIn { target });
        assert!(matches!(
            result,
            Err(EditorError::Op(OpError::Invalid(ref m)))
                if m == "Fade range must be inside a single clip on each track"
        ));
        assert_eq!(*editor.project, project);
    }

    #[test]
    fn fade_in_skips_track_with_no_clip_in_range() {
        let project = project_with_clips(&[&[(100, 100)], &[(500, 100)]]);
        let target = range_target(&project, 120, 180, &[0, 1]);
        let result = roundtrip(project, Action::FadeIn { target });
        assert_eq!(spans(&result, 1), [(500, 600)]);
        assert_eq!(result.tracks[1].clips[0].fade_in, 0);
        assert_eq!(result.tracks[0].clips[1].fade_in, 60);
    }

    #[test]
    fn silence_leaves_a_gap_and_moves_nothing_with_ripple_on() {
        let project = project_with_clips(&[&[(100, 100), (300, 50)]]);
        let track = project.tracks[0].id;
        let mut editor = test_editor(project);
        editor.view.ripple = true;
        editor.set_range(120, 180, vec![track]);
        editor.silence().unwrap();
        assert_eq!(
            spans(&editor.project, 0),
            [(100, 120), (180, 200), (300, 350)]
        );
        assert!(editor.undo());
        assert_eq!(spans(&editor.project, 0), [(100, 200), (300, 350)]);
    }

    #[test]
    fn normalize_constant_quarter_to_minus_one_dbfs() {
        let project = project_with_clips(&[&[(0, 100)]]);
        let track = project.tracks[0].id;
        let media = project.media[0].id;
        let mut editor = test_editor(project);
        put_constant(&mut editor, media, 0.25, 100);
        editor.set_range(0, 100, vec![track]);
        editor.normalize(-1.0, false).unwrap();
        let gain = editor.project.tracks[0].clips[0].gain_db;
        assert!((gain - 11.04).abs() < 0.01, "gain was {gain}");
        assert!(editor.undo());
        assert_eq!(editor.project.tracks[0].clips[0].gain_db, 0.0);
    }

    /// Two tracks with constant media peaking at 0.25 and 0.5, both selected.
    fn two_track_editor() -> Editor {
        let mut project = project_with_clips(&[&[(0, 100)], &[(0, 100)]]);
        let quiet = project.media[0].id;
        let loud = Uuid::new_v4();
        let mut item = project.media[0].clone();
        item.id = loud;
        project.media.push(item);
        project.tracks[1].clips[0].media_id = loud;
        let tracks = vec![project.tracks[0].id, project.tracks[1].id];
        let mut editor = test_editor(project);
        put_constant(&mut editor, quiet, 0.25, 100);
        put_constant(&mut editor, loud, 0.5, 100);
        editor.set_range(0, 100, tracks);
        editor
    }

    #[test]
    fn normalize_together_gives_both_tracks_the_same_delta() {
        let mut editor = two_track_editor();
        editor.normalize(-1.0, false).unwrap();
        let g0 = editor.project.tracks[0].clips[0].gain_db;
        let g1 = editor.project.tracks[1].clips[0].gain_db;
        assert!((g0 - g1).abs() < 1e-4);
        // Target comes from the 0.5 peak: -1 - (-6.0206) = 5.0206.
        assert!((g0 - 5.0206).abs() < 0.01, "gain was {g0}");
    }

    #[test]
    fn normalize_per_track_gives_different_deltas() {
        let mut editor = two_track_editor();
        editor.normalize(-1.0, true).unwrap();
        let g0 = editor.project.tracks[0].clips[0].gain_db;
        let g1 = editor.project.tracks[1].clips[0].gain_db;
        assert!((g0 - 11.04).abs() < 0.01, "gain was {g0}");
        assert!((g1 - 5.02).abs() < 0.01, "gain was {g1}");
    }

    #[test]
    fn normalize_on_silence_errors() {
        let project = project_with_clips(&[&[(0, 100)]]);
        let track = project.tracks[0].id;
        let media = project.media[0].id;
        let mut editor = test_editor(project);
        put_constant(&mut editor, media, 0.0, 100);
        editor.set_range(0, 100, vec![track]);
        let result = editor.normalize(-1.0, false);
        assert!(matches!(
            result,
            Err(EditorError::Op(OpError::Invalid(ref m)))
                if m == "Nothing to normalize: the selection is silent"
        ));
    }

    #[test]
    fn set_track_height_clamps_without_history_or_dirty() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let id = editor.project.tracks[0].id;
        editor.set_track_height(id, 1_000.0);
        assert_eq!(editor.project.tracks[0].height, 400.0);
        editor.set_track_height(id, 10.0);
        assert_eq!(editor.project.tracks[0].height, 40.0);
        editor.set_track_height(id, 120.0);
        assert_eq!(editor.project.tracks[0].height, 120.0);
        editor.set_track_height(Uuid::new_v4(), 200.0);
        assert_eq!(editor.project.tracks[0].height, 120.0);
        assert!(
            editor.history.undo_label().is_none(),
            "track height must not create a history entry"
        );
        assert!(!editor.dirty);
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

    #[test]
    fn place_clip_on_new_track_roundtrip() {
        let target = project_with_clips(&[&[]]);
        let media_id = target.media[0].id;
        let track_count = target.tracks.len();
        let project = roundtrip(
            target,
            Action::PlaceClipOnNewTrack {
                media_id,
                start: 500,
            },
        );
        // One undo step removed both the new track and its clip, so the
        // roundtrip helper (apply -> undo -> original -> redo) proves it.
        assert_eq!(project.tracks.len(), track_count + 1);
        assert_eq!(project.tracks[track_count].name, "Track 2");
        assert_eq!(spans(&project, track_count), [(500, 1_000_500)]);
        assert_eq!(srcs(&project, track_count), [(0, 1_000_000)]);
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

    #[test]
    fn copy_selection_of_clips_keeps_track_offsets() {
        let target = project_with_clips(&[&[(100, 50)], &[], &[(200, 50)]]);
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        let clip_ids = vec![
            editor.project.tracks[0].clips[0].id,
            editor.project.tracks[2].clips[0].id,
        ];
        editor.select_clips(clip_ids);
        editor.copy_selection();
        let clipboard = editor.clipboard().expect("copy stores the clipboard");
        assert_eq!(clipboard.tracks.len(), 3, "track 1 stays as an empty gap");
        assert_eq!(clipboard.length, 150);
        assert_eq!(clipboard.tracks[0][0].start, 0);
        assert_eq!(clipboard.tracks[2][0].start, 100);
    }

    #[test]
    fn cut_selection_of_clips_is_one_cut_step_and_undoes() {
        let target = project_with_clips(&[&[(0, 100), (200, 100), (400, 100)]]);
        let original = target.clone();
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.view.ripple = true;
        let clip_id = editor.project.tracks[0].clips[1].id;
        editor.select_clips(vec![clip_id]);

        editor.cut_selection().unwrap();
        assert_eq!(editor.undo_label(), Some("Cut"));
        assert_eq!(
            spans(&editor.project, 0),
            [(0, 100), (300, 400)],
            "ripple on closes the gap",
        );

        assert!(editor.undo(), "one undo step restores the clips");
        assert_eq!(*editor.project, original);
    }

    #[test]
    fn paste_with_a_range_selected_replaces_it() {
        let target = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let track_id = target.tracks[0].id;
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let mut editor = paste_editor(target, clipboard);
        editor.set_range(200, 300, vec![track_id]);
        editor.set_playhead(0);

        editor.paste().unwrap();
        assert_eq!(
            spans(&editor.project, 0),
            [(0, 100), (200, 250)],
            "replace puts the clipboard at the range start",
        );
        assert_eq!(editor.selection(), &Selection::None, "selection cleared");
        assert_eq!(editor.view.playhead, 200, "playhead at the range start");
    }

    #[test]
    fn paste_without_selection_inserts_at_the_playhead_on_the_target_track() {
        let target = project_with_clips(&[&[], &[(0, 100)]]);
        let track1_id = target.tracks[1].id;
        let clipboard = clipboard_of(&target, &[&[(0, 50)]], 0, 50);
        let mut editor = paste_editor(target, clipboard);
        editor.set_last_clicked_track(track1_id);
        editor.set_playhead(100);

        editor.paste().unwrap();
        assert_eq!(spans(&editor.project, 0), [], "track 0 untouched");
        assert_eq!(
            spans(&editor.project, 1),
            [(0, 100), (100, 150)],
            "insert at the playhead on the last clicked track",
        );
    }

    #[test]
    fn split_with_a_range_splits_only_the_range_tracks() {
        let target = project_with_clips(&[&[(0, 100)], &[(0, 100)]]);
        let track1_id = target.tracks[1].id;
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.set_range(0, 100, vec![track1_id]);
        editor.set_playhead(50);

        editor.split().unwrap();
        assert_eq!(spans(&editor.project, 0), [(0, 100)], "track 0 untouched");
        assert_eq!(spans(&editor.project, 1), [(0, 50), (50, 100)]);
    }

    #[test]
    fn split_without_selection_splits_every_track() {
        let target = project_with_clips(&[&[(0, 100)], &[(0, 100)]]);
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.set_playhead(50);

        editor.split().unwrap();
        assert_eq!(spans(&editor.project, 0), [(0, 50), (50, 100)]);
        assert_eq!(spans(&editor.project, 1), [(0, 50), (50, 100)]);
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

    #[test]
    fn add_markers_roundtrip_sorts_and_labels() {
        let project = project_with_clips(&[&[]]);
        let project = roundtrip(project, Action::AddMarker { pos: 500 });
        assert_eq!(
            project
                .markers
                .iter()
                .map(|m| m.position)
                .collect::<Vec<_>>(),
            [500]
        );
        let project = roundtrip(project, Action::AddMarker { pos: 100 });
        let project = roundtrip(project, Action::AddMarker { pos: 300 });
        assert_eq!(
            project
                .markers
                .iter()
                .map(|m| m.position)
                .collect::<Vec<_>>(),
            [100, 300, 500]
        );
        assert_eq!(Action::AddMarker { pos: 0 }.label(), "Add Marker");
    }

    #[test]
    fn add_marker_on_existing_position_errors_and_undoes_cleanly() {
        let project = project_with_clips(&[&[]]);
        let mut editor = Editor::with_project(
            project.clone(),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.apply(Action::AddMarker { pos: 250 }).unwrap();
        let before = editor.project.clone();
        assert!(matches!(
            editor.apply(Action::AddMarker { pos: 250 }),
            Err(EditorError::Op(OpError::Invalid(_)))
        ));
        assert_eq!(*editor.project, *before, "failed add must change nothing");
        assert_eq!(*editor.project, *before, "failed add must change nothing");
    }

    #[test]
    fn add_marker_at_playhead_twice_adds_one_and_no_error() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.set_playhead(250);
        editor.add_marker_at_playhead().unwrap();
        editor.add_marker_at_playhead().unwrap();
        assert_eq!(editor.project.markers.len(), 1, "still one marker");
        assert_eq!(editor.project.markers[0].position, 250);
    }

    #[test]
    fn jump_next_and_prev_markers_seek_strictly() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.apply(Action::AddMarker { pos: 100 }).unwrap();
        editor.apply(Action::AddMarker { pos: 500 }).unwrap();

        editor.set_playhead(100);
        editor.jump_next_marker();
        assert_eq!(editor.view.playhead, 500);
        editor.jump_next_marker();
        assert_eq!(editor.view.playhead, 500, "no marker after 500");
        editor.jump_prev_marker();
        assert_eq!(editor.view.playhead, 100);
        editor.jump_prev_marker();
        assert_eq!(editor.view.playhead, 100, "no marker strictly before 100");
    }

    #[test]
    fn delete_selected_marker_clears_selection_and_undo_restores() {
        let mut editor = Editor::with_project(
            project_with_clips(&[&[]]),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.set_playhead(100);
        editor.add_marker_at_playhead().unwrap();
        let id = editor.project.markers[0].id;
        editor.select_marker(Some(id));

        editor
            .apply(Action::DeleteMarkers { ids: vec![id] })
            .unwrap();
        assert_eq!(editor.selected_marker(), None, "selection tidied");
        assert!(editor.project.markers.is_empty());

        assert!(editor.undo());
        assert_eq!(editor.project.markers.len(), 1, "marker restored");
        assert_eq!(editor.project.markers[0].position, 100);
    }

    #[test]
    fn move_marker_roundtrip_re_sorts() {
        let project = project_with_clips(&[&[]]);
        let project = roundtrip(project, Action::AddMarker { pos: 100 });
        let project = roundtrip(project, Action::AddMarker { pos: 300 });
        let id = project.markers[1].id;
        let project = roundtrip(project, Action::MoveMarker { id, pos: 50 });
        assert_eq!(
            project
                .markers
                .iter()
                .map(|m| m.position)
                .collect::<Vec<_>>(),
            [50, 100]
        );
        assert_eq!(Action::MoveMarker { id, pos: 0 }.label(), "Move Marker");
    }

    #[test]
    fn delete_markers_roundtrip_removes_two_at_once() {
        let project = project_with_clips(&[&[]]);
        let project = roundtrip(project, Action::AddMarker { pos: 100 });
        let project = roundtrip(project, Action::AddMarker { pos: 200 });
        let project = roundtrip(project, Action::AddMarker { pos: 300 });
        let ids = vec![project.markers[0].id, project.markers[2].id];
        let project = roundtrip(project, Action::DeleteMarkers { ids });
        assert_eq!(
            project
                .markers
                .iter()
                .map(|m| m.position)
                .collect::<Vec<_>>(),
            [200]
        );
        assert_eq!(
            Action::DeleteMarkers { ids: Vec::new() }.label(),
            "Delete Marker"
        );
    }

    #[test]
    fn ripple_delete_range_does_not_move_markers() {
        let project = project_with_clips(&[&[(0, 100), (300, 100)]]);
        let track_id = project.tracks[0].id;
        let project = roundtrip(project, Action::AddMarker { pos: 500 });
        let project = roundtrip(project, Action::AddMarker { pos: 350 });
        let project = roundtrip(
            project,
            Action::DeleteRange {
                s: 200,
                e: 300,
                track_ids: vec![track_id],
                ripple: true,
            },
        );
        assert_eq!(spans(&project, 0), [(0, 100), (200, 300)]);
        assert_eq!(
            project
                .markers
                .iter()
                .map(|m| m.position)
                .collect::<Vec<_>>(),
            [350, 500],
            "ripple must never move markers"
        );
    }

    #[test]
    fn set_project_settings_roundtrip_sets_name_and_depth() {
        let project = roundtrip(
            project_with_clips(&[&[]]),
            Action::SetProjectSettings {
                name: "Renamed".to_string(),
                export_bit_depth: BitDepth::Float32,
            },
        );
        assert_eq!(project.name, "Renamed");
        assert_eq!(project.export_bit_depth, BitDepth::Float32);
        assert_eq!(
            Action::SetProjectSettings {
                name: String::new(),
                export_bit_depth: BitDepth::Int16
            }
            .label(),
            "Project Settings"
        );
    }

    // ---- Task 3.5: media store, background import, media actions ----

    #[test]
    fn import_media_action_roundtrip() {
        let target = project_with_clips(&[&[]]);
        let mut new_item = target.media[0].clone();
        new_item.id = uuid::Uuid::new_v4();
        new_item.display_name = "Extra".to_string();
        let project = roundtrip(target, Action::ImportMedia { item: new_item });
        assert_eq!(project.media.len(), 2);
        assert_eq!(project.media[1].display_name, "Extra");
    }

    #[test]
    fn rename_media_action_roundtrip() {
        let target = project_with_clips(&[&[]]);
        let id = target.media[0].id;
        let project = roundtrip(
            target,
            Action::RenameMedia {
                id,
                name: "New Name".to_string(),
            },
        );
        assert_eq!(project.media[0].display_name, "New Name");
    }

    #[test]
    fn remove_media_removes_clips_and_undo_restores_both() {
        let target = project_with_clips(&[&[(0, 100)], &[(200, 100)]]);
        let original = target.clone();
        let id = target.media[0].id;
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        assert_eq!(editor.media_usage(id), 2);
        editor.apply(Action::RemoveMedia { id }).unwrap();
        assert!(editor.project.media.is_empty());
        assert!(editor.project.tracks.iter().all(|t| t.clips.is_empty()));
        assert!(editor.undo(), "remove media is undoable");
        assert_eq!(
            *editor.project, original,
            "undo restores the item and its clips"
        );
    }

    #[test]
    fn remove_media_action_roundtrip() {
        let target = project_with_clips(&[&[(0, 100)]]);
        let id = target.media[0].id;
        let project = roundtrip(target, Action::RemoveMedia { id });
        assert!(project.media.is_empty());
        assert!(project.tracks.iter().all(|t| t.clips.is_empty()));
    }

    #[test]
    fn rename_media_empty_name_errors() {
        let target = project_with_clips(&[&[]]);
        let id = target.media[0].id;
        let mut editor = Editor::with_project(
            target,
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        assert!(
            editor
                .apply(Action::RenameMedia {
                    id,
                    name: "   ".to_string(),
                })
                .is_err()
        );
        assert_eq!(editor.project.media[0].display_name, "Test Media");
    }

    /// Writes a 16-bit stereo WAV with `frames` frames at 48 kHz.
    fn write_test_wav(path: &Path, frames: usize) {
        use hound::{SampleFormat, WavSpec, WavWriter};
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            let v = (i % 100) as i16;
            writer.write_sample(v).unwrap();
            writer.write_sample(-v).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Wraps a fresh 48 kHz project over a real project folder layout.
    fn editor_over_layout(parent: &Path, name: &str) -> Editor {
        let paths = paths::create_layout(parent, name).unwrap();
        let project = Project::new(name.to_string(), 48_000, BitDepth::Int24);
        Editor::with_project(project, ViewState::default(), paths)
    }

    #[test]
    fn import_job_end_to_end() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("tone.wav");
        write_test_wav(&src, 500);

        let proj_dir = tempdir().unwrap();
        let mut editor = editor_over_layout(proj_dir.path(), "Imp");

        let rejected = editor.import_files(vec![src]);
        assert!(rejected.is_empty());
        assert!(editor.has_pending_jobs());

        let start = std::time::Instant::now();
        while editor.has_pending_jobs() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(10),
                "import job timed out"
            );
            editor.poll_jobs();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(
            editor.take_job_errors().is_empty(),
            "no job errors expected"
        );
        assert_eq!(editor.project.media.len(), 1);
        let (media_id, display_name, length) = {
            let item = &editor.project.media[0];
            (item.id, item.display_name.clone(), item.length)
        };
        assert_eq!(display_name, "tone");
        assert_eq!(length, 500);
        let loaded = editor.media(media_id).expect("imported audio is loaded");
        assert_eq!(loaded.samples.len(), 500 * 2);
        assert!(!editor.is_media_offline(media_id));

        assert!(editor.undo(), "import is undoable");
        assert!(editor.project.media.is_empty());
        assert!(
            editor.media(media_id).is_some(),
            "store keeps audio for undo"
        );
    }

    #[test]
    fn relink_media_roundtrip() {
        let project = project_with_clips(&[&[(0, 100)]]);
        let id = project.media[0].id;
        let new = roundtrip(
            project,
            Action::RelinkMedia {
                id,
                file_name: "new.wav".to_string(),
                original_rate: 44_100,
                channels: 1,
                length: 500_000,
            },
        );
        let item = new.media.iter().find(|m| m.id == id).unwrap();
        assert_eq!(item.file_name, "new.wav");
        assert_eq!(item.original_rate, 44_100);
        assert_eq!(item.channels, 1);
        assert_eq!(item.length, 500_000);
    }

    #[test]
    fn import_files_rejects_unsupported_extensions() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("song.flac");
        std::fs::write(&src, b"flac").unwrap();

        let proj_dir = tempdir().unwrap();
        let mut editor = editor_over_layout(proj_dir.path(), "Rej");

        let rejected = editor.import_files(vec![src]);
        assert_eq!(rejected, vec!["song.flac".to_string()]);
        assert!(!editor.has_pending_jobs());
        assert!(editor.project.media.is_empty());
    }

    #[test]
    fn publish_without_audio_is_a_noop() {
        let mut editor = with_editor(project_with_clips(&[&[(100, 100)]]));
        assert!(editor.audio.is_none());
        assert!(editor.audio_status().starts_with("No audio"));
        // An edit with no engine running must not panic; publish is a no-op.
        assert!(
            editor
                .apply(Action::AddTrack {
                    index: None,
                    name: None
                })
                .is_ok()
        );
        editor.publish();
        editor.tick();
        assert!(editor.retired.is_empty());
    }

    /// An in-memory editor with no audio engine.
    fn selection_editor(tracks: &[&[(u64, u64)]]) -> Editor {
        Editor::with_project(
            project_with_clips(tracks),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        )
    }

    #[test]
    fn set_range_orders_positions_and_tracks() {
        let mut editor = selection_editor(&[&[], &[], &[]]);
        let t0 = editor.project.tracks[0].id;
        let t2 = editor.project.tracks[2].id;
        editor.set_range(500, 100, vec![t2, t0]);
        assert_eq!(
            editor.selection(),
            &Selection::Range {
                start: 100,
                end: 500,
                tracks: vec![t0, t2]
            }
        );
    }

    #[test]
    fn set_range_with_equal_ends_or_no_tracks_is_none() {
        let mut editor = selection_editor(&[&[]]);
        let t0 = editor.project.tracks[0].id;
        editor.set_range(100, 100, vec![t0]);
        assert_eq!(editor.selection(), &Selection::None);
        editor.set_range(0, 100, vec![]);
        assert_eq!(editor.selection(), &Selection::None);
    }

    #[test]
    fn apply_undo_and_redo_tidy_the_selection() {
        let mut editor = selection_editor(&[&[(0, 100)], &[]]);
        let t0 = editor.project.tracks[0].id;
        let t1 = editor.project.tracks[1].id;
        let clip = editor.project.tracks[0].clips[0].id;

        // A range whose only track is deleted becomes None.
        editor.set_range(0, 100, vec![t1]);
        editor.apply(Action::DeleteTrack { id: t1 }).unwrap();
        assert_eq!(editor.selection(), &Selection::None);

        // A clip selection on a deleted track is tidied away too.
        editor.select_clips(vec![clip]);
        editor.apply(Action::DeleteTrack { id: t0 }).unwrap();
        assert_eq!(editor.selection(), &Selection::None);

        // Undo and redo tidy as well; the restored track has nothing selected.
        assert!(editor.undo());
        assert_eq!(editor.selection(), &Selection::None);
        assert!(editor.redo());
        assert_eq!(editor.selection(), &Selection::None);

        // With the track back, a clip selection is tidied by delete, undo and redo.
        assert!(editor.undo());
        editor.select_clips(vec![clip]);
        editor.apply(Action::DeleteTrack { id: t0 }).unwrap();
        assert_eq!(editor.selection(), &Selection::None);
        assert!(editor.undo());
        assert_eq!(editor.selection(), &Selection::None);
        assert!(editor.redo());
        assert_eq!(editor.selection(), &Selection::None);
    }

    #[test]
    fn toggle_range_track_adds_in_order_and_removes() {
        let mut editor = selection_editor(&[&[], &[], &[]]);
        let t0 = editor.project.tracks[0].id;
        let t1 = editor.project.tracks[1].id;
        let t2 = editor.project.tracks[2].id;
        editor.set_range(0, 100, vec![t2]);
        editor.toggle_range_track(t0);
        assert!(matches!(
            editor.selection(),
            Selection::Range { tracks, .. } if *tracks == vec![t0, t2]
        ));
        editor.toggle_range_track(t1);
        assert!(matches!(
            editor.selection(),
            Selection::Range { tracks, .. } if *tracks == vec![t0, t1, t2]
        ));
        editor.toggle_range_track(t1);
        assert!(matches!(
            editor.selection(),
            Selection::Range { tracks, .. } if *tracks == vec![t0, t2]
        ));
        editor.toggle_range_track(t0);
        editor.toggle_range_track(t2);
        assert_eq!(editor.selection(), &Selection::None);
        // With no range, toggling does nothing.
        editor.toggle_range_track(t0);
        assert_eq!(editor.selection(), &Selection::None);
    }

    #[test]
    fn clip_selection_toggles_and_clears() {
        let mut editor = selection_editor(&[&[(0, 100), (200, 200)]]);
        let clip0 = editor.project.tracks[0].clips[0].id;
        let clip1 = editor.project.tracks[0].clips[1].id;
        let gone = uuid::Uuid::new_v4();

        editor.toggle_clip(clip0);
        assert_eq!(editor.selection(), &Selection::Clips(vec![clip0]));
        editor.toggle_clip(clip1);
        assert_eq!(editor.selection(), &Selection::Clips(vec![clip0, clip1]));
        editor.toggle_clip(clip0);
        assert_eq!(editor.selection(), &Selection::Clips(vec![clip1]));
        editor.toggle_clip(clip1);
        assert_eq!(editor.selection(), &Selection::None);
        editor.toggle_clip(gone);
        assert_eq!(editor.selection(), &Selection::None);

        editor.select_clips(vec![clip0, clip1]);
        editor.clear_selection();
        assert_eq!(editor.selection(), &Selection::None);
    }

    #[test]
    fn select_all_covers_the_whole_project_and_every_track() {
        let mut editor = selection_editor(&[&[(0, 100)], &[(300, 100)]]);
        editor.select_all();
        let tracks = editor.project.tracks.iter().map(|t| t.id).collect();
        assert_eq!(
            editor.selection(),
            &Selection::Range {
                start: 0,
                end: 400,
                tracks
            }
        );

        let mut empty = selection_editor(&[]);
        empty.select_all();
        assert_eq!(empty.selection(), &Selection::None);
    }

    #[test]
    fn target_track_prefers_range_then_last_clicked_then_zero() {
        let mut editor = selection_editor(&[&[], &[], &[]]);
        let t0 = editor.project.tracks[0].id;
        let t1 = editor.project.tracks[1].id;
        let t2 = editor.project.tracks[2].id;
        editor.set_range(0, 100, vec![t1, t2]);
        assert_eq!(editor.target_track(), 1);
        editor.clear_selection();
        editor.set_last_clicked_track(t2);
        assert_eq!(editor.target_track(), 2);
        editor.set_last_clicked_track(t0);
        assert_eq!(editor.target_track(), 0);
    }

    #[test]
    fn snap_toggles_flip_the_view_without_dirty() {
        let mut editor = selection_editor(&[&[]]);
        editor.view.snap = true;
        editor.view.ripple = false;
        editor.toggle_snap();
        editor.toggle_ripple();
        assert!(!editor.view.snap);
        assert!(editor.view.ripple);
        assert!(!editor.dirty);
        assert!(editor.history.undo.is_empty());
    }

    /// A real on-disk project (tempdir) plus an Editor wrapping it, with the
    /// autosave interval set to zero so every tick may write.
    fn disk_editor(name: &str) -> (tempfile::TempDir, Editor) {
        let parent = tempdir().unwrap();
        let paths = crate::io::paths::create_layout(parent.path(), name).unwrap();
        let project = Project::new(name.to_string(), 48_000, BitDepth::Int16);
        let view = ViewState::default();
        project_file::save(&paths, &project, &view).unwrap();
        let mut editor = Editor::with_project(project, view, paths);
        editor.autosave_interval = Duration::ZERO;
        (parent, editor)
    }

    /// Marks the editor dirty the way an edit would.
    fn make_dirty(editor: &mut Editor) {
        editor.dirty = true;
        editor.autosave_generation += 1;
    }

    #[test]
    fn autosave_tick_does_nothing_when_not_dirty() {
        let parent = tempdir().unwrap();
        let paths = crate::io::paths::create_layout(parent.path(), "Clean").unwrap();
        let project = Project::new("Clean".to_string(), 48_000, BitDepth::Int16);
        project_file::save(&paths, &project, &ViewState::default()).unwrap();
        let mut editor = Editor::with_project(project, ViewState::default(), paths);
        editor.autosave_interval = Duration::ZERO;
        editor.tick();
        assert!(!autosave::autosave_path(&editor.paths).exists());
        assert!(editor.take_job_errors().is_empty());
    }

    #[test]
    fn autosave_tick_writes_once_after_the_interval() {
        let (_parent, mut editor) = disk_editor("Ticky");
        make_dirty(&mut editor);
        editor.tick();
        let first = autosave::autosave_path(&editor.paths);
        assert!(first.is_file());
        let mtime = std::fs::metadata(&first)
            .and_then(|m| m.modified())
            .unwrap();
        // A second tick with no new edit must not write again.
        editor.tick();
        let mtime2 = std::fs::metadata(&first)
            .and_then(|m| m.modified())
            .unwrap();
        assert_eq!(mtime, mtime2);
        // A new edit makes the next tick write again.
        make_dirty(&mut editor);
        editor.tick();
        let mtime3 = std::fs::metadata(&first)
            .and_then(|m| m.modified())
            .unwrap();
        assert_ne!(mtime2, mtime3);
    }

    #[test]
    fn autosave_tick_records_errors_in_job_errors() {
        let (_parent, mut editor) = disk_editor("Bad");
        // Make the atomic rename fail: project.autosave.json exists as a
        // directory, so the rename over it cannot succeed.
        std::fs::create_dir_all(editor.paths.autosave()).unwrap();
        std::fs::create_dir(autosave::autosave_path(&editor.paths)).unwrap();
        make_dirty(&mut editor);
        editor.tick();
        assert!(
            editor
                .take_job_errors()
                .iter()
                .any(|e| e.contains("Autosave failed")),
            "expected an autosave job error"
        );
    }

    #[test]
    fn save_removes_the_autosave() {
        let (_parent, mut editor) = disk_editor("Saver");
        make_dirty(&mut editor);
        editor.tick();
        assert!(autosave::autosave_path(&editor.paths).is_file());
        editor.save().unwrap();
        assert!(!autosave::autosave_path(&editor.paths).exists());
    }

    #[test]
    fn open_with_recovery_check_requests_recovery_for_newer_autosave() {
        let _recent = lock_recent();
        let (_parent, mut editor) = disk_editor("Recov");
        make_dirty(&mut editor);
        editor.tick();
        // The autosave is now newer than project.json.
        assert!(matches!(
            Editor::open_with_recovery_check(&editor.paths.root),
            Ok(OpenOutcome::NeedsRecoveryChoice(_))
        ));
        // After a manual save the prompt is gone.
        editor.save().unwrap();
        assert!(matches!(
            Editor::open_with_recovery_check(&editor.paths.root),
            Ok(OpenOutcome::Opened(_))
        ));
        drop_from_recent(&editor.paths.root);
    }

    #[test]
    fn open_recovered_loads_the_autosave_marked_dirty() {
        let _recent = lock_recent();
        let (_parent, mut editor) = disk_editor("Recovered");
        editor
            .apply(Action::AddTrack {
                index: None,
                name: None,
            })
            .unwrap();
        assert!(editor.dirty);
        editor.tick();
        assert!(autosave::autosave_path(&editor.paths).is_file());
        let track_count = editor.project.tracks.len();

        let recovered = Editor::open_recovered(&editor.paths.root).unwrap();
        assert!(recovered.dirty, "recovered project must show as unsaved");
        assert_eq!(recovered.project.tracks.len(), track_count);
        assert_eq!(recovered.paths.root, editor.paths.root);

        // Discarding opens the last saved project instead (no extra track).
        let discarded = Editor::open_discarding_autosave(&editor.paths.root).unwrap();
        assert!(!discarded.dirty);
        assert!(discarded.project.tracks.is_empty());
        drop_from_recent(&recovered.paths.root);
    }
}
