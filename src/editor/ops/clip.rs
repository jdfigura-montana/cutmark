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
        } else if clip.start <= s && end >= e {
            // Covers both edges: split into a left piece ending at s and a
            // right piece starting at e with a new id.
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
