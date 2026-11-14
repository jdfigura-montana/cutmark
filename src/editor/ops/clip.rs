//! editor::ops::clip: clip moves, trims, splits, gain and fades.

use uuid::Uuid;

use crate::editor::ops::OpError;
use crate::model::clip::Clip;
use crate::model::project::Project;
use crate::model::track::Track;

/// Splits the clip covering `pos` into two; the left keeps the original id.
///
/// Returns `(left id, right id)` or `None` if no clip covers `pos` strictly inside.
pub fn split_clip_at(track: &mut Track, pos: u64) -> Option<(Uuid, Uuid)> {
    let index = track
        .clips
        .iter()
        .position(|clip| clip.start < pos && pos < clip.end())?;
    let left_id = track.clips[index].id;

    let mut left = track.clips[index].clone();
    left.src_out = left.src_in + (pos - left.start);
    left.fade_out = 0;
    left.clamp_fades();

    let mut right = track.clips[index].clone();
    right.id = Uuid::new_v4();
    right.start = pos;
    right.src_in = left.src_out;
    right.fade_in = 0;
    right.clamp_fades();
    let right_id = right.id;

    track.clips[index] = left;
    track.clips.insert(index + 1, right);
    Some((left_id, right_id))
}

/// Removes or trims every clip inside the range `s..e`; does nothing if `s >= e`.
pub fn clear_range(track: &mut Track, s: u64, e: u64) {
    if s >= e {
        return;
    }
    let mut result: Vec<Clip> = Vec::with_capacity(track.clips.len());
    for clip in track.clips.drain(..) {
        let end = clip.end();
        if end <= s || clip.start >= e {
            // Entirely outside the range: keep unchanged.
            result.push(clip);
        } else if clip.start >= s && end <= e {
            // Entirely inside the range: remove.
        } else if clip.start < s && end > e {
            // Reaches past both edges: split into a left piece ending at s and
            // a right piece starting at e with a new id. (Strict on both
            // sides: a clip that starts exactly at s, or ends exactly at e,
            // has no left or right piece and falls through to the cases below
            // instead of leaving an empty clip behind.)
            let mut left = clip;
            let right_src_in = left.src_in + (e - left.start);
            let right_src_out = left.src_out;
            let right_fade_out = left.fade_out;
            left.src_out = left.src_in + (s - left.start);
            left.fade_out = 0;
            left.clamp_fades();
            let mut right = Clip {
                id: Uuid::new_v4(),
                media_id: left.media_id,
                start: e,
                src_in: right_src_in,
                src_out: right_src_out,
                gain_db: left.gain_db,
                fade_in: 0,
                fade_out: right_fade_out,
            };
            right.clamp_fades();
            result.push(left);
            result.push(right);
        } else if clip.start < s {
            // Starts before s and ends inside: cut its end back to s.
            let mut cut = clip;
            cut.src_out = cut.src_in + (s - cut.start);
            cut.fade_out = 0;
            cut.clamp_fades();
            result.push(cut);
        } else {
            // Starts inside and ends after e: move its start to e.
            let mut moved = clip;
            let shift = e - moved.start;
            moved.src_in += shift;
            moved.start = e;
            moved.fade_in = 0;
            moved.clamp_fades();
            result.push(moved);
        }
    }
    track.clips = result;
}

/// Overwrites the timeline range covered by `clip` with it, then keeps clips sorted.
pub fn place_overwrite(track: &mut Track, clip: Clip) {
    clear_range(track, clip.start, clip.end());
    track.clips.push(clip);
    track.sort_clips();
}

/// Moves every clip with `start >= pos` by `delta` samples.
///
/// Returns [`OpError::Overflow`] if any moved start would go below 0 or overflow `u64`.
pub fn shift_from(track: &mut Track, pos: u64, delta: i64) -> Result<(), OpError> {
    for clip in track.clips.iter_mut() {
        if clip.start >= pos {
            let new_start = clip.start as i128 + delta as i128;
            if new_start < 0 || new_start > u64::MAX as i128 {
                return Err(OpError::Overflow);
            }
            clip.start = new_start as u64;
        }
    }
    track.sort_clips();
    Ok(())
}

/// Splits clips at `pos` on the given tracks, or on all tracks when `None`.
///
/// Returns [`OpError::NotFound`] if a listed track id does not exist.
pub fn split_at(
    project: &mut Project,
    pos: u64,
    track_ids: Option<Vec<Uuid>>,
) -> Result<(), OpError> {
    match track_ids {
        Some(ids) => {
            for id in ids {
                let track = project
                    .tracks
                    .iter_mut()
                    .find(|track| track.id == id)
                    .ok_or(OpError::NotFound(id))?;
                split_clip_at(track, pos);
            }
            Ok(())
        }
        None => {
            for track in project.tracks.iter_mut() {
                split_clip_at(track, pos);
            }
            Ok(())
        }
    }
}

/// Trims one edge of the clip; keeps fades, then re-clamps them.
///
/// Start trims move `start` and `src_in` together; end trims move `src_out`.
/// Clamped so `src_in >= 0`, `src_out <= media length`, `len >= 1`, and the clip
/// never crosses a neighbor on the same track.
pub fn trim_clip(
    project: &mut Project,
    clip_id: Uuid,
    edge: Edge,
    new_pos: u64,
) -> Result<(), OpError> {
    // Locate the clip, its neighbors and its media length before mutating.
    let (track_index, clip_index) =
        locate_clip(project, clip_id).ok_or(OpError::NotFound(clip_id))?;
    let track = &project.tracks[track_index];
    let old = track.clips[clip_index].clone();
    let media_id = old.media_id;
    let media_len = project
        .media
        .iter()
        .find(|media| media.id == media_id)
        .ok_or(OpError::NotFound(media_id))?
        .length;
    let left_bound = track
        .clips
        .iter()
        .filter(|clip| clip.id != clip_id && clip.end() <= old.start)
        .map(|clip| clip.end())
        .max()
        .unwrap_or(0);
    let right_bound = track
        .clips
        .iter()
        .filter(|clip| clip.id != clip_id && clip.start >= old.end())
        .map(|clip| clip.start)
        .min();

    let clip = &mut project.tracks[track_index].clips[clip_index];
    match edge {
        Edge::Start => {
            let mut new_start = new_pos as i128;
            new_start = new_start.max(left_bound as i128);
            new_start = new_start.max((old.start - old.src_in) as i128); // src_in >= 0
            new_start = new_start.min(old.end() as i128 - 1); // len >= 1
            let start = new_start as u64;
            clip.start = start;
            clip.src_in = old.src_in + start - old.start;
        }
        Edge::End => {
            let mut new_end = new_pos as i128;
            if let Some(right) = right_bound {
                new_end = new_end.min(right as i128);
            }
            let media_end = old.start as i128 + media_len as i128 - old.src_in as i128;
            new_end = new_end.min(media_end); // src_out <= media length
            new_end = new_end.max(old.start as i128 + 1); // len >= 1
            clip.src_out = old.src_in + new_end as u64 - old.start;
        }
    }
    clip.clamp_fades();
    Ok(())
}

/// Removes the listed clips; with `ripple`, closes the gaps left to right.
///
/// Unknown ids return [`OpError::NotFound`] and nothing is changed.
pub fn delete_clips(project: &mut Project, clip_ids: &[Uuid], ripple: bool) -> Result<(), OpError> {
    // Validate first: every id must exist somewhere.
    for id in clip_ids {
        if locate_clip(project, *id).is_none() {
            return Err(OpError::NotFound(*id));
        }
    }

    if !ripple {
        for track in project.tracks.iter_mut() {
            track.clips.retain(|clip| !clip_ids.contains(&clip.id));
        }
        return Ok(());
    }

    // Ripple: per track, take deleted clips right to left and close each gap.
    for track_index in 0..project.tracks.len() {
        let mut deleted: Vec<Clip> = project.tracks[track_index]
            .clips
            .iter()
            .filter(|clip| clip_ids.contains(&clip.id))
            .cloned()
            .collect();
        deleted.sort_by_key(|clip| std::cmp::Reverse(clip.start));
        for clip in deleted {
            let end = clip.end();
            let len = clip.len() as i64;
            let track = &mut project.tracks[track_index];
            track.clips.retain(|other| other.id != clip.id);
            shift_from(track, end, -len)?;
        }
    }
    Ok(())
}

/// Sets the clip gain in dB, clamped to -60..=+24.
pub fn set_clip_gain(project: &mut Project, clip_id: Uuid, db: f32) -> Result<(), OpError> {
    let clip = find_clip_mut(project, clip_id).ok_or(OpError::NotFound(clip_id))?;
    clip.gain_db = db.clamp(-60.0, 24.0);
    Ok(())
}

/// Sets both fade lengths, then clamps them to the clip length.
pub fn set_clip_fades(
    project: &mut Project,
    clip_id: Uuid,
    fade_in: u64,
    fade_out: u64,
) -> Result<(), OpError> {
    let clip = find_clip_mut(project, clip_id).ok_or(OpError::NotFound(clip_id))?;
    clip.fade_in = fade_in;
    clip.fade_out = fade_out;
    clip.clamp_fades();
    Ok(())
}

/// Finds a clip by id anywhere in the project.
fn find_clip_mut(project: &mut Project, clip_id: Uuid) -> Option<&mut Clip> {
    project
        .tracks
        .iter_mut()
        .find_map(|track| track.clips.iter_mut().find(|clip| clip.id == clip_id))
}
/// Moves the listed clips by `delta_time` samples and `delta_tracks` tracks.
///
/// Validates every new start and track index first; on any violation returns
/// [`OpError::Invalid`] and changes nothing. Clip ids stay the same; the moved
/// range overwrites whatever it lands on. Unknown ids return [`OpError::NotFound`].
pub fn move_clips(
    project: &mut Project,
    clip_ids: &[Uuid],
    delta_time: i64,
    delta_tracks: i32,
) -> Result<(), OpError> {
    // Locate every clip and validate the destinations before mutating anything.
    let mut located: Vec<(Clip, usize)> = Vec::with_capacity(clip_ids.len());
    for id in clip_ids {
        let (track_index, clip_index) = locate_clip(project, *id).ok_or(OpError::NotFound(*id))?;
        let clip = project.tracks[track_index].clips[clip_index].clone();
        let new_start = clip.start as i128 + delta_time as i128;
        if new_start < 0 || new_start > u64::MAX as i128 {
            return Err(OpError::Invalid(
                "moved clip start out of range".to_string(),
            ));
        }
        let new_track = track_index as i64 + delta_tracks as i64;
        if new_track < 0 || new_track >= project.tracks.len() as i64 {
            return Err(OpError::Invalid(
                "moved clip track out of range".to_string(),
            ));
        }
        located.push((clip, new_track as usize));
    }

    // Remove all listed clips from their tracks, then overwrite into place.
    for track in project.tracks.iter_mut() {
        track.clips.retain(|clip| !clip_ids.contains(&clip.id));
    }
    for (mut clip, track_index) in located {
        clip.start = (clip.start as i128 + delta_time as i128) as u64;
        place_overwrite(&mut project.tracks[track_index], clip);
    }
    Ok(())
}
/// Which end of a clip a trim acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// The clip's start edge: moves `start` and `src_in` together.
    Start,
    /// The clip's end edge: moves `src_out` (the timeline end follows).
    End,
}

/// Places a new clip (gain 0, no fades) on the track, overwrite or insert mode.
///
/// Returns the new clip id. `insert` splits at `start`, shifts later clips right
/// by the clip length, then inserts; otherwise the covered range is overwritten.
/// Returns [`OpError::NotFound`] for an unknown track or media id.
pub fn place_clip(
    project: &mut Project,
    track_id: Uuid,
    media_id: Uuid,
    start: u64,
    src_in: u64,
    src_out: u64,
    insert: bool,
) -> Result<Uuid, OpError> {
    let track = project
        .tracks
        .iter_mut()
        .find(|track| track.id == track_id)
        .ok_or(OpError::NotFound(track_id))?;
    if !project.media.iter().any(|media| media.id == media_id) {
        return Err(OpError::NotFound(media_id));
    }
    let clip = Clip {
        id: Uuid::new_v4(),
        media_id,
        start,
        src_in,
        src_out,
        gain_db: 0.0,
        fade_in: 0,
        fade_out: 0,
    };
    let id = clip.id;
    if insert {
        split_clip_at(track, start);
        shift_from(track, start, clip.len() as i64)?;
        track.clips.push(clip);
        track.sort_clips();
    } else {
        place_overwrite(track, clip);
    }
    Ok(id)
}

/// Locates a clip by id anywhere in the project as `(track index, clip index)`.
fn locate_clip(project: &Project, clip_id: Uuid) -> Option<(usize, usize)> {
    for (track_index, track) in project.tracks.iter().enumerate() {
        if let Some(clip_index) = track.clips.iter().position(|clip| clip.id == clip_id) {
            return Some((track_index, clip_index));
        }
    }
    None
}

/// Appends a new track named "Track N" holding one new clip of `media_id` at `start`.
///
/// The clip spans the full source media (`src_in` 0, `src_out` = media length),
/// gain 0 and no fades. One undo step covers both the track and the clip.
/// Returns [`OpError::NotFound`] for an unknown media id.
pub fn place_clip_on_new_track(
    project: &mut Project,
    media_id: Uuid,
    start: u64,
) -> Result<(), OpError> {
    let media_len = project
        .media
        .iter()
        .find(|media| media.id == media_id)
        .ok_or(OpError::NotFound(media_id))?
        .length;
    let mut track = Track::new(format!("Track {}", project.tracks.len() + 1));
    track.clips.push(Clip {
        id: Uuid::new_v4(),
        media_id,
        start,
        src_in: 0,
        src_out: media_len,
        gain_db: 0.0,
        fade_in: 0,
        fade_out: 0,
    });
    project.tracks.push(track);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::{project_with_clips, spans, srcs};

    /// Builds one clip with custom fades, returning the project and the clip id.
    fn clip_with_fades(start: u64, len: u64, fade_in: u64, fade_out: u64) -> (Project, Uuid) {
        let mut project = project_with_clips(&[&[(start, len)]]);
        let clip = &mut project.tracks[0].clips[0];
        clip.fade_in = fade_in;
        clip.fade_out = fade_out;
        let id = clip.id;
        (project, id)
    }

    #[test]
    fn split_at_150_makes_two_clips_with_adjacent_sources() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        let (left_id, right_id) = split_clip_at(&mut project.tracks[0], 150).unwrap();
        assert_eq!(spans(&project, 0), [(100, 150), (150, 200)]);
        assert_eq!(srcs(&project, 0), [(0, 50), (50, 100)]);
        let clips = &project.tracks[0].clips;
        assert_eq!(clips[0].id, left_id);
        assert_eq!(clips[1].id, right_id);
        assert_ne!(left_id, right_id);
    }

    #[test]
    fn split_at_clip_boundaries_does_nothing() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        assert_eq!(split_clip_at(&mut project.tracks[0], 100), None);
        assert_eq!(split_clip_at(&mut project.tracks[0], 200), None);
        assert_eq!(spans(&project, 0), [(100, 200)]);
    }

    #[test]
    fn split_resets_fades_on_cut_edges_only() {
        let (mut project, _) = clip_with_fades(100, 100, 20, 30);
        split_clip_at(&mut project.tracks[0], 150).unwrap();
        let clips = &project.tracks[0].clips;
        assert_eq!(clips[0].fade_in, 20);
        assert_eq!(clips[0].fade_out, 0);
        assert_eq!(clips[1].fade_in, 0);
        assert_eq!(clips[1].fade_out, 30);
    }

    #[test]
    fn clear_range_middle_splits_around_the_gap() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 120, 180);
        assert_eq!(spans(&project, 0), [(100, 120), (180, 200)]);
        assert_eq!(srcs(&project, 0), [(0, 20), (80, 100)]);
    }

    #[test]
    fn clear_range_overlapping_start_cuts_the_head() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 50, 150);
        assert_eq!(spans(&project, 0), [(150, 200)]);
        assert_eq!(srcs(&project, 0), [(50, 100)]);
    }

    #[test]
    fn clear_range_overlapping_end_cuts_the_tail() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 150, 250);
        assert_eq!(spans(&project, 0), [(100, 150)]);
    }

    #[test]
    fn clear_range_full_removal_and_degenerate_ranges() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 50, 250);
        assert!(project.tracks[0].clips.is_empty());

        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 120, 120);
        assert_eq!(spans(&project, 0), [(100, 200)]);
    }

    #[test]
    fn clear_range_at_a_clip_edge_leaves_no_empty_clip() {
        // Starts exactly at the clip's start: only the tail piece remains.
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 100, 150);
        assert_eq!(spans(&project, 0), [(150, 200)]);
        assert_eq!(srcs(&project, 0), [(50, 100)]);
        assert!(crate::model::invariants::check(&project).is_ok());

        // Ends exactly at the clip's end: only the head piece remains.
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 150, 200);
        assert_eq!(spans(&project, 0), [(100, 150)]);
        assert_eq!(srcs(&project, 0), [(0, 50)]);
        assert!(crate::model::invariants::check(&project).is_ok());

        // Exactly the clip, and a range reaching past both its edges: it is gone.
        for (s, e) in [(100, 200), (50, 250), (100, 250), (50, 200)] {
            let mut project = project_with_clips(&[&[(100, 100)]]);
            clear_range(&mut project.tracks[0], s, e);
            assert!(project.tracks[0].clips.is_empty(), "{s}..{e}");
        }

        // Strictly inside still splits in two.
        let mut project = project_with_clips(&[&[(100, 100)]]);
        clear_range(&mut project.tracks[0], 101, 199);
        assert_eq!(spans(&project, 0), [(100, 101), (199, 200)]);
    }

    #[test]
    fn place_overwrite_at_the_exact_start_of_a_longer_clip() {
        // The re-take case: a new clip lands where an older one starts.
        let mut project = project_with_clips(&[&[(0, 100)]]);
        let media_id = project.tracks[0].clips[0].media_id;
        let clip = Clip {
            id: Uuid::new_v4(),
            media_id,
            start: 0,
            src_in: 0,
            src_out: 40,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
        };
        place_overwrite(&mut project.tracks[0], clip);
        assert_eq!(spans(&project, 0), [(0, 40), (40, 100)]);
        assert_eq!(srcs(&project, 0), [(0, 40), (40, 100)]);
        assert!(crate::model::invariants::check(&project).is_ok());
    }

    #[test]
    fn place_overwrite_replaces_the_covered_range() {
        let mut project = project_with_clips(&[&[(100, 100)]]);
        let media_id = project.tracks[0].clips[0].media_id;
        let clip = Clip {
            id: Uuid::new_v4(),
            media_id,
            start: 130,
            src_in: 0,
            src_out: 40,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
        };
        place_overwrite(&mut project.tracks[0], clip);
        assert_eq!(spans(&project, 0), [(100, 130), (130, 170), (170, 200)]);
    }

    #[test]
    fn shift_from_moves_clips_left_and_right() {
        let mut project = project_with_clips(&[&[(100, 10), (200, 10)]]);
        shift_from(&mut project.tracks[0], 150, -60).unwrap();
        assert_eq!(spans(&project, 0), [(100, 110), (140, 150)]);

        let mut project = project_with_clips(&[&[(100, 10), (200, 10)]]);
        shift_from(&mut project.tracks[0], 150, 30).unwrap();
        assert_eq!(spans(&project, 0), [(100, 110), (230, 240)]);
    }

    #[test]
    fn shift_from_below_zero_returns_overflow() {
        let mut project = project_with_clips(&[&[(100, 10)]]);
        let result = shift_from(&mut project.tracks[0], 0, -200);
        assert!(matches!(result, Err(OpError::Overflow)));
        assert_eq!(spans(&project, 0), [(100, 110)]);
    }

    #[test]
    fn split_at_project_level_on_all_tracks_and_one_track() {
        let mut project = project_with_clips(&[&[(100, 100)], &[(100, 100)]]);
        split_at(&mut project, 150, None).unwrap();
        assert_eq!(spans(&project, 0), [(100, 150), (150, 200)]);
        assert_eq!(spans(&project, 1), [(100, 150), (150, 200)]);

        let mut project = project_with_clips(&[&[(100, 100)], &[(100, 100)]]);
        let id = project.tracks[1].id;
        split_at(&mut project, 150, Some(vec![id])).unwrap();
        assert_eq!(spans(&project, 0), [(100, 200)]);
        assert_eq!(spans(&project, 1), [(100, 150), (150, 200)]);
    }

    #[test]
    fn split_at_unknown_track_returns_not_found() {
        let mut project = project_with_clips(&[&[]]);
        let missing = Uuid::new_v4();
        let result = split_at(&mut project, 150, Some(vec![missing]));
        assert!(matches!(result, Err(OpError::NotFound(id)) if id == missing));
    }
}
