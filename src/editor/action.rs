//! editor::action: Action, the single undoable edit path applied to the Project.

use crate::editor::ops::OpError;
use crate::editor::ops::{clip, track};
use crate::model::project::Project;

/// One user-facing edit, dispatched to a pure function in `editor::ops`.
#[derive(Debug, Clone)]
pub enum Action {
    /// Adds a track; `None` index appends, `None` name gives "Track N".
    AddTrack {
        index: Option<usize>,
        name: Option<String>,
    },
    /// Deletes the track with the given id.
    DeleteTrack { id: uuid::Uuid },
    /// Renames the track with the given id.
    RenameTrack { id: uuid::Uuid, name: String },
    /// Moves the track to a new zero-based index.
    MoveTrack { id: uuid::Uuid, new_index: usize },
    /// Sets track volume in dB, clamped to -60..+12.
    SetTrackVolume { id: uuid::Uuid, db: f32 },
    /// Sets track pan, clamped to -1..+1.
    SetTrackPan { id: uuid::Uuid, pan: f32 },
    /// Sets the track mute flag.
    SetTrackMute { id: uuid::Uuid, mute: bool },
    /// Sets the track solo flag.
    SetTrackSolo { id: uuid::Uuid, solo: bool },
    /// Solos this track and unsolos every other track.
    SoloExclusive { id: uuid::Uuid },
    /// Sets the master volume in dB, clamped to -60..+12.
    SetMasterVolume { db: f32 },
    /// Splits clips at the given sample position on the given tracks, or all tracks.
    SplitAt {
        /// Split position in samples.
        pos: u64,
        /// Track ids to split on, or all tracks when `None`.
        track_ids: Option<Vec<uuid::Uuid>>,
    },
}

impl Action {
    /// Menu label for this action, shown by undo and redo menu items.
    pub fn label(&self) -> &'static str {
        match self {
            Action::AddTrack { .. } => "Add Track",
            Action::DeleteTrack { .. } => "Delete Track",
            Action::RenameTrack { .. } => "Rename Track",
            Action::MoveTrack { .. } => "Move Track",
            Action::SetTrackVolume { .. } => "Track Volume",
            Action::SetTrackPan { .. } => "Track Pan",
            Action::SetTrackMute { .. } => "Mute",
            Action::SetTrackSolo { .. } => "Solo",
            Action::SoloExclusive { .. } => "Solo Exclusive",
            Action::SetMasterVolume { .. } => "Master Volume",
            Action::SplitAt { .. } => "Split",
        }
    }

    /// Runs the matching pure ops function on the given project.
    pub fn run(&self, project: &mut Project) -> Result<(), OpError> {
        match self {
            Action::AddTrack { index, name } => track::add_track(project, *index, name.clone()),
            Action::DeleteTrack { id } => track::delete_track(project, *id),
            Action::RenameTrack { id, name } => track::rename_track(project, *id, name.clone()),
            Action::MoveTrack { id, new_index } => track::move_track(project, *id, *new_index),
            Action::SetTrackVolume { id, db } => track::set_track_volume(project, *id, *db),
            Action::SetTrackPan { id, pan } => track::set_track_pan(project, *id, *pan),
            Action::SetTrackMute { id, mute } => track::set_track_mute(project, *id, *mute),
            Action::SetTrackSolo { id, solo } => track::set_track_solo(project, *id, *solo),
            Action::SoloExclusive { id } => track::solo_exclusive(project, *id),
            Action::SetMasterVolume { db } => track::set_master_volume(project, *db),
            Action::SplitAt { pos, track_ids } => clip::split_at(project, *pos, track_ids.clone()),
        }
    }
}
