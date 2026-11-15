//! editor::action: Action, the single undoable edit path applied to the Project.

use crate::editor::ops::OpError;
use crate::editor::ops::{amplitude, clip, marker, range, track};
use crate::model::project::{BitDepth, Project};

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
    /// Appends a new track holding one new clip of the media item.
    PlaceClipOnNewTrack {
        /// Media item the clip plays.
        media_id: uuid::Uuid,
        /// Timeline start in samples.
        start: u64,
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
    /// Adds a marker at the given sample position.
    AddMarker {
        /// Position in samples.
        pos: u64,
    },
    /// Moves the marker with the given id to a new position.
    MoveMarker {
        /// Marker to move.
        id: uuid::Uuid,
        /// New position in samples.
        pos: u64,
    },
    /// Deletes the listed markers.
    DeleteMarkers {
        /// Markers to delete.
        ids: Vec<uuid::Uuid>,
    },
    /// Deletes the range `s..e` on the listed tracks, optionally rippling.
    DeleteRange {
        /// Range start in samples.
        s: u64,
        /// Exclusive range end in samples.
        e: u64,
        /// Track ids to delete on.
        track_ids: Vec<uuid::Uuid>,
        /// Shift later clips left by `e - s`.
        ripple: bool,
    },
    /// Removes everything outside `s..e` on the listed tracks.
    TrimToSelection {
        /// Range start in samples.
        s: u64,
        /// Exclusive range end in samples.
        e: u64,
        /// Track ids to trim on.
        track_ids: Vec<uuid::Uuid>,
    },
    /// Sets the project name and default export bit depth.
    SetProjectSettings {
        /// New project name.
        name: String,
        /// New default export bit depth.
        export_bit_depth: BitDepth,
    },
    /// Appends a bin entry to the media bin (audio already conformed).
    ImportMedia {
        /// The bin entry to add.
        item: crate::model::media::MediaItem,
    },
    /// Replaces an offline bin entry's file and stats after a relink.
    RelinkMedia {
        /// Media id to relink.
        id: uuid::Uuid,
        /// New file name inside media/.
        file_name: String,
        /// Original sample rate of the replacement file.
        original_rate: u32,
        /// Channel count of the replacement file.
        channels: u16,
        /// Length of the replacement in samples at the project rate.
        length: u64,
    },
    /// Adds a recorded take: its bin entry (named "Take <n>") and one clip of
    /// it on a track, in one step.
    RecordTake {
        /// The bin entry for the take (audio already in media/ and cached).
        item: crate::model::media::MediaItem,
        /// Track the take was recorded on.
        track_id: uuid::Uuid,
        /// Timeline start of the clip (where recording began), in samples.
        start: u64,
        /// Source in-point in samples: the hidden latency at the file's start.
        src_in: u64,
    },
    /// Removes the bin entry and every clip that uses it.
    RemoveMedia {
        /// Media id to remove.
        id: uuid::Uuid,
    },
    /// Renames a bin entry's display name.
    RenameMedia {
        /// Media id to rename.
        id: uuid::Uuid,
        /// New display name (must not be empty).
        name: String,
    },
    /// Adds gain in dB to a range's fragments or whole clips, clamped to -60..+24.
    ApplyGain {
        /// What the gain change applies to.
        target: amplitude::Target,
        /// Gain change in dB.
        delta_db: f32,
    },
    /// Adds each track's per-fragment gain delta to hit a normalize target peak.
    Normalize {
        /// What the normalize applies to.
        target: amplitude::Target,
        /// Per track id gain delta in dB.
        deltas: Vec<(uuid::Uuid, f32)>,
    },
    /// Sets a fade-in over the target range, or over the whole clips.
    FadeIn {
        /// What the fade applies to.
        target: amplitude::Target,
    },
    /// Sets a fade-out over the target range, or over the whole clips.
    FadeOut {
        /// What the fade applies to.
        target: amplitude::Target,
    },
    /// Removes clip content from `s..e` on the listed tracks; nothing moves.
    Silence {
        /// Range start in samples.
        s: u64,
        /// Exclusive range end in samples.
        e: u64,
        /// Track ids to silence.
        track_ids: Vec<uuid::Uuid>,
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
            Action::PlaceClipOnNewTrack { .. } => "Add Clip",
            Action::MoveClips { .. } => "Move Clip",
            Action::TrimClip { .. } => "Trim Clip",
            Action::DeleteClips { .. } => "Delete Clip",
            Action::SetClipGain { .. } => "Clip Gain",
            Action::SetClipFades { .. } => "Fades",
            Action::DeleteRange { .. } => "Delete",
            Action::TrimToSelection { .. } => "Trim to Selection",
            Action::AddMarker { .. } => "Add Marker",
            Action::MoveMarker { .. } => "Move Marker",
            Action::DeleteMarkers { .. } => "Delete Marker",
            Action::SetProjectSettings { .. } => "Project Settings",
            Action::ImportMedia { .. } => "Import",
            Action::RelinkMedia { .. } => "Relink",
            Action::RecordTake { .. } => "Record",
            Action::RemoveMedia { .. } => "Remove Media",
            Action::RenameMedia { .. } => "Rename Media",
            Action::ApplyGain { .. } => "Gain",
            Action::Normalize { .. } => "Normalize",
            Action::FadeIn { .. } => "Fade In",
            Action::FadeOut { .. } => "Fade Out",
            Action::Silence { .. } => "Silence",
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
            Action::PlaceClipOnNewTrack { media_id, start } => {
                clip::place_clip_on_new_track(project, *media_id, *start)
            }
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
            Action::AddMarker { pos } => marker::add_marker(project, *pos),
            Action::MoveMarker { id, pos } => marker::move_marker(project, *id, *pos),
            Action::DeleteMarkers { ids } => marker::delete_markers(project, ids),
            Action::DeleteRange {
                s,
                e,
                track_ids,
                ripple,
            } => range::delete_range(project, *s, *e, track_ids, *ripple),
            Action::TrimToSelection { s, e, track_ids } => {
                range::trim_to_selection(project, *s, *e, track_ids)
            }
            Action::SetProjectSettings {
                name,
                export_bit_depth,
            } => crate::editor::ops::set_project_settings(project, name, *export_bit_depth),
            Action::ImportMedia { item } => {
                crate::editor::ops::media::import_media(project, item.clone())
            }
            Action::RelinkMedia {
                id,
                file_name,
                original_rate,
                channels,
                length,
            } => crate::editor::ops::media::relink_media(
                project,
                *id,
                file_name.clone(),
                *original_rate,
                *channels,
                *length,
            ),
            Action::RecordTake {
                item,
                track_id,
                start,
                src_in,
            } => crate::editor::ops::media::record_take(
                project,
                item.clone(),
                *track_id,
                *start,
                *src_in,
            ),
            Action::RemoveMedia { id } => crate::editor::ops::media::remove_media(project, *id),
            Action::RenameMedia { id, name } => {
                crate::editor::ops::media::rename_media(project, *id, name)
            }
            Action::ApplyGain { target, delta_db } => {
                amplitude::apply_gain(project, target, *delta_db)
            }
            Action::Normalize { target, deltas } => amplitude::normalize(project, target, deltas),
            Action::FadeIn { target } => amplitude::fade_in(project, target),
            Action::FadeOut { target } => amplitude::fade_out(project, target),
            Action::Silence { s, e, track_ids } => amplitude::silence(project, *s, *e, track_ids),
        }
    }
}
