//! editor::clipboard: cut/copy/paste clipboard holding clips and ranges.

use uuid::Uuid;

use crate::model::clip::Clip;
use crate::model::project::Project;

/// Clipboard content copied from a range: clips with starts relative to the range start.
#[derive(Debug, Clone, PartialEq)]
pub struct Clipboard {
    /// Total copied length in samples (range end minus start).
    pub length: u64,
    /// One clip list per selected track, in top-to-bottom project order.
    pub tracks: Vec<Vec<Clip>>,
}

/// Copies the clip fragments inside `s..e` on the listed tracks into a clipboard.
///
/// Selected tracks are packed top-to-bottom in project order, so non-contiguous
/// tracks end up adjacent in the clipboard.
pub fn copy_range(project: &Project, s: u64, e: u64, track_ids: &[Uuid]) -> Clipboard {
    let mut tracks = Vec::new();
    for track in &project.tracks {
        if !track_ids.contains(&track.id) {
            continue;
        }
        let clips = if s >= e {
            Vec::new()
        } else {
            track
                .clips
                .iter()
                .filter_map(|clip| copy_piece(clip, s, e))
                .collect()
        };
        tracks.push(clips);
    }
    Clipboard {
        length: e.saturating_sub(s),
        tracks,
    }
}

/// Returns the part of `clip` inside `s..e`, or `None` when it does not overlap.
fn copy_piece(clip: &Clip, s: u64, e: u64) -> Option<Clip> {
    let end = clip.end();
    if clip.start >= e || end <= s {
        return None;
    }
    let overlap_start = clip.start.max(s);
    let overlap_end = end.min(e);
    let mut piece = Clip {
        id: Uuid::new_v4(),
        media_id: clip.media_id,
        start: overlap_start - s,
        src_in: clip.src_in + (overlap_start - clip.start),
        src_out: clip.src_in + (overlap_end - clip.start),
        gain_db: clip.gain_db,
        fade_in: if overlap_start == clip.start {
            clip.fade_in
        } else {
            0
        },
        fade_out: if overlap_end == end { clip.fade_out } else { 0 },
    };
    piece.clamp_fades();
    Some(piece)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::project_with_clips;

    /// Returns the `(start, end)` of each clip on clipboard track `index`.
    fn spans(c: &Clipboard, index: usize) -> Vec<(u64, u64)> {
        c.tracks[index]
            .iter()
            .map(|clip| (clip.start, clip.start + clip.len()))
            .collect()
    }

    /// Returns the `(src_in, src_out)` of each clip on clipboard track `index`.
    fn srcs(c: &Clipboard, index: usize) -> Vec<(u64, u64)> {
        c.tracks[index]
            .iter()
            .map(|clip| (clip.src_in, clip.src_out))
            .collect()
    }

    #[test]
    fn copy_range_takes_the_part_inside_each_clip() {
        let project = project_with_clips(&[&[(100, 100), (300, 100)]]);
        let clip = copy_range(&project, 150, 350, &[project.tracks[0].id]);
        assert_eq!(clip.length, 200);
        assert_eq!(spans(&clip, 0), [(0, 50), (150, 200)]);
        assert_eq!(srcs(&clip, 0), [(50, 100), (0, 50)]);
    }

    #[test]
    fn copy_range_packs_non_contiguous_tracks_together() {
        let project = project_with_clips(&[&[(0, 100)], &[(0, 100)], &[(0, 100)]]);
        let track_ids = [project.tracks[0].id, project.tracks[2].id];
        let clip = copy_range(&project, 0, 100, &track_ids);
        assert_eq!(clip.tracks.len(), 2);
        assert_eq!(spans(&clip, 0), [(0, 100)]);
        assert_eq!(spans(&clip, 1), [(0, 100)]);
    }

    #[test]
    fn copy_range_skips_clips_outside_the_range() {
        let project = project_with_clips(&[&[(0, 50), (200, 100)]]);
        let clip = copy_range(&project, 100, 150, &[project.tracks[0].id]);
        assert_eq!(clip.length, 50);
        assert!(clip.tracks[0].is_empty());
    }

    #[test]
    fn copy_range_keeps_fades_only_at_untouched_edges() {
        let mut project = project_with_clips(&[&[(0, 100)]]);
        project.tracks[0].clips[0].fade_in = 20;
        project.tracks[0].clips[0].fade_out = 20;
        let track_id = project.tracks[0].id;

        let whole = copy_range(&project, 0, 100, &[track_id]);
        assert_eq!(
            (whole.tracks[0][0].fade_in, whole.tracks[0][0].fade_out),
            (20, 20)
        );

        let middle = copy_range(&project, 30, 70, &[track_id]);
        assert_eq!(
            (middle.tracks[0][0].fade_in, middle.tracks[0][0].fade_out),
            (0, 0)
        );
    }
}
