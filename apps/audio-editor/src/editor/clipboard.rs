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
        let clips = if s >= e { Vec::new() } else { track.clips.iter().filter_map(|clip| copy_piece(clip, s, e)).collect() };
        tracks.push(clips);
    }
    Clipboard { length: e.saturating_sub(s), tracks }
}

/// Copies whole selected clips into a clipboard, keeping their track offsets.
///
/// Clipboard tracks cover the project's topmost to bottommost tracks holding a
/// selected clip, including empty tracks in between. Clip positions are
/// relative to the earliest selected start; the length is the latest selected
/// end minus that start.
pub fn copy_clips(project: &Project, clip_ids: &[Uuid]) -> Clipboard {
    let is_selected = |clip: &Clip| clip_ids.contains(&clip.id);
    let first = project.tracks.iter().position(|t| t.clips.iter().any(&is_selected));
    let Some(first) = first else {
        return Clipboard { length: 0, tracks: Vec::new() };
    };
    let last = project.tracks.iter().rposition(|t| t.clips.iter().any(&is_selected)).unwrap_or(first);
    let earliest = project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| is_selected(c)).map(|c| c.start).min().unwrap_or(0);
    let latest = project.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| is_selected(c)).map(|c| c.end()).max().unwrap_or(earliest);
    let tracks = project.tracks[first..=last]
        .iter()
        .map(|track| {
            track
                .clips
                .iter()
                .filter(|c| is_selected(c))
                .map(|c| {
                    let mut copy = c.clone();
                    copy.id = Uuid::new_v4();
                    copy.start -= earliest;
                    copy
                })
                .collect()
        })
        .collect();
    Clipboard { length: latest.saturating_sub(earliest), tracks }
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
        fade_in: if overlap_start == clip.start { clip.fade_in } else { 0 },
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
        c.tracks[index].iter().map(|clip| (clip.start, clip.start + clip.len())).collect()
    }

    /// Returns the `(src_in, src_out)` of each clip on clipboard track `index`.
    fn srcs(c: &Clipboard, index: usize) -> Vec<(u64, u64)> {
        c.tracks[index].iter().map(|clip| (clip.src_in, clip.src_out)).collect()
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
        assert_eq!((whole.tracks[0][0].fade_in, whole.tracks[0][0].fade_out), (20, 20));

        let middle = copy_range(&project, 30, 70, &[track_id]);
        assert_eq!((middle.tracks[0][0].fade_in, middle.tracks[0][0].fade_out), (0, 0));
    }

    #[test]
    fn copy_clips_keeps_track_offsets_with_an_empty_middle_track() {
        let project = project_with_clips(&[&[(100, 50)], &[], &[(200, 50)]]);
        let clip_ids = [project.tracks[0].clips[0].id, project.tracks[2].clips[0].id];
        let clip = copy_clips(&project, &clip_ids);
        assert_eq!(clip.tracks.len(), 3, "tracks 0, 1, 2 in the clipboard");
        assert!(clip.tracks[1].is_empty(), "the middle track stays empty");
        assert_eq!(clip.length, 150, "latest end 250 minus earliest start 100");
        assert_eq!(spans(&clip, 0), [(0, 50)]);
        assert_eq!(spans(&clip, 2), [(100, 150)], "relative to the earliest start");
    }

    #[test]
    fn copy_clips_is_empty_with_no_selected_clips() {
        let project = project_with_clips(&[&[(0, 50)]]);
        let clip = copy_clips(&project, &[Uuid::new_v4()]);
        assert_eq!(clip.length, 0);
        assert!(clip.tracks.is_empty());
    }
}
