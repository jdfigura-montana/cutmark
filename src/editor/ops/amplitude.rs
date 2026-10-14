//! editor::ops::amplitude: gain, normalize, fade and silence processing.
//! editor::ops::amplitude: gain, normalize, fade and silence processing.

use uuid::Uuid;

use crate::editor::ops::OpError;
use crate::editor::ops::clip::{clear_range, split_clip_at};
use crate::model::clip::Clip;
use crate::model::project::Project;
use crate::model::track::Track;

/// What amplitude processing applies to: a time range on tracks, or whole clips.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// The range `start..end` on the listed tracks.
    Range {
        /// Range start in samples.
        start: u64,
        /// Exclusive range end in samples.
        end: u64,
        /// Track ids the range sits on.
        tracks: Vec<Uuid>,
    },
    /// The listed whole clips.
    Clips(Vec<Uuid>),
}

/// Returns a mutable reference to the track with the given id.
fn track_mut(project: &mut Project, id: Uuid) -> Result<&mut Track, OpError> {
    project
        .tracks
        .iter_mut()
        .find(|track| track.id == id)
        .ok_or(OpError::NotFound(id))
}

/// Returns a reference to the track with the given id.
fn track_ref(project: &Project, id: Uuid) -> Result<&Track, OpError> {
    project
        .tracks
        .iter()
        .find(|track| track.id == id)
        .ok_or(OpError::NotFound(id))
}

/// Returns a mutable reference to the clip with the given id anywhere in the project.
fn clip_mut(project: &mut Project, clip_id: Uuid) -> Option<&mut Clip> {
    project
        .tracks
        .iter_mut()
        .flat_map(|track| track.clips.iter_mut())
        .find(|clip| clip.id == clip_id)
}

/// Splits every listed track's clips at `start` and at `end`.
///
/// Unknown track ids return [`OpError::NotFound`]; splits inside the already
/// split tracks are no-ops.
pub fn split_edges(
    project: &mut Project,
    start: u64,
    end: u64,
    tracks: &[Uuid],
) -> Result<(), OpError> {
    for &id in tracks {
        let track = track_mut(project, id)?;
        split_clip_at(track, start);
        split_clip_at(track, end);
    }
    Ok(())
}

/// The (track id, clip id) pairs the target addresses.
///
/// For a range: every clip on the listed tracks lying fully inside
/// `start..end` (call after [`split_edges`]). For clips: the listed clips
/// with their track ids.
pub fn fragments(project: &Project, target: &Target) -> Vec<(Uuid, Uuid)> {
    let mut out = Vec::new();
    match target {
        Target::Range { start, end, tracks } => {
            for track in &project.tracks {
                if !tracks.contains(&track.id) {
                    continue;
                }
                for clip in &track.clips {
                    if clip.start >= *start && clip.end() <= *end {
                        out.push((track.id, clip.id));
                    }
                }
            }
        }
        Target::Clips(ids) => {
            for track in &project.tracks {
                for clip in &track.clips {
                    if ids.contains(&clip.id) {
                        out.push((track.id, clip.id));
                    }
                }
            }
        }
    }
    out
}

/// Validates clip ids for a whole-clips target, returning `NotFound` if any id
/// does not match a clip.
fn require_clips(project: &Project, target: &Target) -> Result<(), OpError> {
    if let Target::Clips(ids) = target {
        for id in ids {
            let found = project
                .tracks
                .iter()
                .any(|track| track.clips.iter().any(|clip| clip.id == *id));
            if !found {
                return Err(OpError::NotFound(*id));
            }
        }
    }
    Ok(())
}

/// Adds `delta_db` to each fragment's clip gain, clamped to -60..=+24.
///
/// Range targets split at the edges first; clips targets change the whole
/// clips without splitting. Unknown clip ids return [`OpError::NotFound`].
pub fn apply_gain(project: &mut Project, target: &Target, delta_db: f32) -> Result<(), OpError> {
    if let Target::Clips(_) = target {
        require_clips(project, target)?;
    }
    let frags = match target {
        Target::Range { start, end, tracks } => {
            split_edges(project, *start, *end, tracks)?;
            fragments(project, target)
        }
        Target::Clips(_) => fragments(project, target),
    };
    for (_, clip_id) in &frags {
        let clip = clip_mut(project, *clip_id).ok_or(OpError::NotFound(*clip_id))?;
        clip.gain_db = (clip.gain_db + delta_db).clamp(-60.0, 24.0);
    }
    Ok(())
}

/// Adds each track's delta from `deltas` to its fragments' clip gain, clamped
/// to -60..=+24.
///
/// Same splitting rule as [`apply_gain`]; fragments on tracks missing from
/// `deltas` get 0.0.
pub fn normalize(
    project: &mut Project,
    target: &Target,
    deltas: &[(Uuid, f32)],
) -> Result<(), OpError> {
    if let Target::Clips(_) = target {
        require_clips(project, target)?;
    }
    let frags = match target {
        Target::Range { start, end, tracks } => {
            split_edges(project, *start, *end, tracks)?;
            fragments(project, target)
        }
        Target::Clips(_) => fragments(project, target),
    };
    for (track_id, clip_id) in &frags {
        let delta = deltas
            .iter()
            .find(|(id, _)| id == track_id)
            .map(|(_, delta)| *delta)
            .unwrap_or(0.0);
        let clip = clip_mut(project, *clip_id).ok_or(OpError::NotFound(*clip_id))?;
        clip.gain_db = (clip.gain_db + delta).clamp(-60.0, 24.0);
    }
    Ok(())
}

/// Validates that a fade range sits inside one clip per track (or no clip at
/// all, which skips the track); returns the error otherwise.
fn validate_fade_range(
    project: &Project,
    start: u64,
    end: u64,
    tracks: &[Uuid],
) -> Result<(), OpError> {
    const MSG: &str = "Fade range must be inside a single clip on each track";
    for &id in tracks {
        let track = track_ref(project, id)?;
        let overlapping: Vec<&Clip> = track
            .clips
            .iter()
            .filter(|clip| clip.start < end && clip.end() > start)
            .collect();
        match overlapping.as_slice() {
            [] => {}
            [clip] => {
                if clip.start > start || clip.end() < end {
                    return Err(OpError::Invalid(MSG.to_string()));
                }
            }
            _ => return Err(OpError::Invalid(MSG.to_string())),
        }
    }
    Ok(())
}

/// Shared body of [`fade_in`] and [`fade_out`]; `fade_in` picks the edge.
fn fade(project: &mut Project, target: &Target, fade_in: bool) -> Result<(), OpError> {
    match target {
        Target::Range { start, end, tracks } => {
            validate_fade_range(project, *start, *end, tracks)?;
            split_edges(project, *start, *end, tracks)?;
            let length = end.saturating_sub(*start);
            for &id in tracks {
                let track = track_ref(project, id)?;
                let fragment = track
                    .clips
                    .iter()
                    .find(|clip| clip.start == *start && clip.end() == *end)
                    .map(|clip| clip.id);
                if let Some(clip_id) = fragment {
                    let clip = clip_mut(project, clip_id).ok_or(OpError::NotFound(clip_id))?;
                    if fade_in {
                        clip.fade_in = length;
                    } else {
                        clip.fade_out = length;
                    }
                    clip.clamp_fades();
                }
            }
            Ok(())
        }
        Target::Clips(ids) => {
            for id in ids.clone() {
                let track = project
                    .tracks
                    .iter()
                    .find(|track| track.clips.iter().any(|clip| clip.id == id))
                    .ok_or(OpError::NotFound(id))?;
                let length = track
                    .clips
                    .iter()
                    .find(|clip| clip.id == id)
                    .map(|clip| clip.len())
                    .unwrap_or(0);
                let (fade_in_len, fade_out_len) = if fade_in { (length, 0) } else { (0, length) };
                crate::editor::ops::clip::set_clip_fades(project, id, fade_in_len, fade_out_len)?;
            }
            Ok(())
        }
    }
}

/// Sets a fade-in over the target range (or the whole clips); see the SPEC's
/// "Range processing" table.
pub fn fade_in(project: &mut Project, target: &Target) -> Result<(), OpError> {
    fade(project, target, true)
}

/// Sets a fade-out over the target range (or the whole clips); see the SPEC's
/// "Range processing" table.
pub fn fade_out(project: &mut Project, target: &Target) -> Result<(), OpError> {
    fade(project, target, false)
}

/// Clears the clip content in `start..end` on each listed track; nothing
/// moves, whatever the Ripple toggle says.
pub fn silence(
    project: &mut Project,
    start: u64,
    end: u64,
    tracks: &[Uuid],
) -> Result<(), OpError> {
    for &id in tracks {
        let track = track_mut(project, id)?;
        clear_range(track, start, end);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::{project_with_clips, spans};

    #[test]
    fn split_edges_fragments_and_finds_fragments() {
        let mut project = project_with_clips(&[&[(100, 200)]]);
        let track = project.tracks[0].id;
        split_edges(&mut project, 150, 250, &[track]).unwrap();
        assert_eq!(spans(&project, 0), [(100, 150), (150, 250), (250, 300)]);
        let target = Target::Range {
            start: 150,
            end: 250,
            tracks: vec![track],
        };
        let frags = fragments(&project, &target);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].0, track);
    }

    #[test]
    fn split_edges_unknown_track_is_not_found() {
        let mut project = project_with_clips(&[&[]]);
        let missing = Uuid::new_v4();
        assert!(matches!(
            split_edges(&mut project, 0, 10, &[missing]),
            Err(OpError::NotFound(m)) if m == missing
        ));
    }

    #[test]
    fn fade_range_spanning_two_clips_is_invalid() {
        let mut project = project_with_clips(&[&[(100, 50), (150, 50)]]);
        let track = project.tracks[0].id;
        let before = project.clone();
        let target = Target::Range {
            start: 120,
            end: 180,
            tracks: vec![track],
        };
        assert!(matches!(
            fade_in(&mut project, &target),
            Err(OpError::Invalid(message)) if message.contains("single clip")
        ));
        assert_eq!(project, before);
    }

    #[test]
    fn fade_clips_target_fades_whole_clips() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        let clip = project.tracks[0].clips[0].id;
        fade_in(&mut project, &Target::Clips(vec![clip])).unwrap();
        assert_eq!(project.tracks[0].clips[0].fade_in, 100);
        assert_eq!(project.tracks[0].clips[0].fade_out, 0);
    }
}
