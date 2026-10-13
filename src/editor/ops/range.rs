//! editor::ops::range: range edits such as trim to selection and delete range.

use uuid::Uuid;

use crate::editor::ops::OpError;
use crate::editor::ops::clip::{clear_range, shift_from};
use crate::model::project::Project;
use crate::model::track::Track;

/// Returns the mutable track with the given id, or NotFound.
fn track_mut(project: &mut Project, id: Uuid) -> Result<&mut Track, OpError> {
    project
        .tracks
        .iter_mut()
        .find(|track| track.id == id)
        .ok_or(OpError::NotFound(id))
}

/// Removes content from `s..e` on the listed tracks.
///
/// Ripple on: later clips on those tracks shift left by `e - s`. Ripple off:
/// leaves a gap. Returns [`OpError::NotFound`] for an unknown track id.
pub fn delete_range(
    project: &mut Project,
    s: u64,
    e: u64,
    track_ids: &[Uuid],
    ripple: bool,
) -> Result<(), OpError> {
    for &id in track_ids {
        let track = track_mut(project, id)?;
        clear_range(track, s, e);
        if ripple && e > s {
            shift_from(track, e, -((e - s).min(i64::MAX as u64) as i64))?;
        }
    }
    Ok(())
}

/// Removes everything outside `s..e` on the listed tracks. Nothing moves.
///
/// Returns [`OpError::NotFound`] for an unknown track id.
pub fn trim_to_selection(
    project: &mut Project,
    s: u64,
    e: u64,
    track_ids: &[Uuid],
) -> Result<(), OpError> {
    for &id in track_ids {
        let track = track_mut(project, id)?;
        clear_range(track, 0, s);
        clear_range(track, e, u64::MAX);
        // clear_range keeps a zero-length piece when a clip starts exactly at 0
        // or ends exactly at e; trim must not leave empty clips behind.
        track.clips.retain(|clip| clip.len() > 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::{project_with_clips, spans};

    #[test]
    fn delete_range_ripple_closes_the_gap() {
        let mut project = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let id = project.tracks[0].id;
        delete_range(&mut project, 50, 150, &[id], true).unwrap();
        assert_eq!(spans(&project, 0), [(0, 50), (100, 200)]);
    }

    #[test]
    fn delete_range_without_ripple_leaves_the_gap() {
        let mut project = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let id = project.tracks[0].id;
        delete_range(&mut project, 50, 150, &[id], false).unwrap();
        assert_eq!(spans(&project, 0), [(0, 50), (200, 300)]);
    }

    #[test]
    fn delete_range_only_touches_the_listed_tracks() {
        let mut project = project_with_clips(&[&[(0, 100), (200, 100)], &[(0, 100), (200, 100)]]);
        let id = project.tracks[0].id;
        delete_range(&mut project, 50, 150, &[id], true).unwrap();
        assert_eq!(spans(&project, 0), [(0, 50), (100, 200)]);
        assert_eq!(spans(&project, 1), [(0, 100), (200, 300)]);
    }

    #[test]
    fn delete_range_unknown_track_returns_not_found() {
        let mut project = project_with_clips(&[&[(0, 100)]]);
        let result = delete_range(&mut project, 0, 50, &[Uuid::new_v4()], false);
        assert!(matches!(result, Err(OpError::NotFound(_))));
        assert_eq!(spans(&project, 0), [(0, 100)]);
    }

    #[test]
    fn trim_to_selection_keeps_only_the_range() {
        let mut project = project_with_clips(&[&[(0, 100), (200, 100)]]);
        let id = project.tracks[0].id;
        trim_to_selection(&mut project, 50, 250, &[id]).unwrap();
        assert_eq!(spans(&project, 0), [(50, 100), (200, 250)]);
    }

    #[test]
    fn trim_to_selection_only_touches_the_listed_tracks() {
        let mut project = project_with_clips(&[&[(0, 100), (200, 100)], &[(0, 100), (200, 100)]]);
        let id = project.tracks[0].id;
        trim_to_selection(&mut project, 50, 250, &[id]).unwrap();
        assert_eq!(spans(&project, 0), [(50, 100), (200, 250)]);
        assert_eq!(spans(&project, 1), [(0, 100), (200, 300)]);
    }
}
