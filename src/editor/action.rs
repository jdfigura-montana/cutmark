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
    /// Places a new clip on a track (overwrite, or insert when `insert`).
    PlaceClip {
        /// Destination track id.
        track_id: uuid::Uuid,
        /// Media item the clip plays.
        media_id: uuid::Uuid,
        /// Timeline start in samples.
        start: u64,
        /// Source in-point in samples.
        src_in: u64,
        /// Exclusive source out-point in samples.
        src_out: u64,
        /// Insert (ripple right) instead of overwrite.
        insert: bool,
    },
    /// Moves the listed clips in time and across tracks.
    MoveClips {
        /// Clips to move.
        clip_ids: Vec<uuid::Uuid>,
        /// Time shift in samples (may be negative).
        delta_time: i64,
        /// Track shift (may be negative).
        delta_tracks: i32,
    },
    /// Trims one edge of a clip.
    TrimClip {
        /// Clip to trim.
        clip_id: uuid::Uuid,
        /// Which edge to trim.
        edge: clip::Edge,
        /// New edge position in samples.
        new_pos: u64,
    },
    /// Deletes the listed clips, optionally closing the gaps (ripple).
    DeleteClips {
        /// Clips to delete.
        clip_ids: Vec<uuid::Uuid>,
        /// Close the gaps left behind.
        ripple: bool,
    },
    /// Sets clip gain in dB, clamped to -60..+24.
    SetClipGain {
        /// Clip to change.
        clip_id: uuid::Uuid,
        /// New gain in dB.
        db: f32,
    },
    /// Sets the clip's fade in/out lengths, clamped to the clip length.
    SetClipFades {
        /// Clip to change.
        clip_id: uuid::Uuid,
        /// Fade-in length in samples.
        fade_in: u64,
        /// Fade-out length in samples.
        fade_out: u64,
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
            Action::PlaceClip { .. } => "Add Clip",
            Action::MoveClips { .. } => "Move Clip",
            Action::TrimClip { .. } => "Trim Clip",
            Action::DeleteClips { .. } => "Delete Clip",
            Action::SetClipGain { .. } => "Clip Gain",
            Action::SetClipFades { .. } => "Fades",
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
            Action::PlaceClip {
                track_id,
                media_id,
                start,
                src_in,
                src_out,
                insert,
            } => clip::place_clip(
                project, *track_id, *media_id, *start, *src_in, *src_out, *insert,
            )
            .map(|_| ()),
            Action::MoveClips {
                clip_ids,
                delta_time,
                delta_tracks,
            } => clip::move_clips(project, clip_ids, *delta_time, *delta_tracks),
            Action::TrimClip {
                clip_id,
                edge,
                new_pos,
            } => clip::trim_clip(project, *clip_id, *edge, *new_pos),
            Action::DeleteClips { clip_ids, ripple } => {
                clip::delete_clips(project, clip_ids, *ripple)
            }
            Action::SetClipGain { clip_id, db } => clip::set_clip_gain(project, *clip_id, *db),
            Action::SetClipFades {
                clip_id,
                fade_in,
                fade_out,
            } => clip::set_clip_fades(project, *clip_id, *fade_in, *fade_out),
        }
    }
}
