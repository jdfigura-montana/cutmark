//! editor::snap: 8 px snapping to clip edges, markers and the playhead.

use uuid::Uuid;

use crate::editor::editor::Editor;

/// Returns the nearest candidate within `threshold_px * spp` samples of `pos`,
/// or `None`. Candidates may be in any order; duplicates are harmless.
pub fn snap(pos: u64, candidates: &[u64], spp: f64, threshold_px: f32) -> Option<u64> {
    let threshold = threshold_px as f64 * spp;
    let idx = candidates.partition_point(|&c| c <= pos);
    let before = if idx > 0 { Some(candidates[idx - 1]) } else { None };
    let best = before.into_iter().chain(candidates.get(idx).copied()).min_by_key(|&c| (c as i128 - pos as i128).abs())?;
    if (best as i128 - pos as i128).abs() as f64 <= threshold { Some(best) } else { None }
}

impl Editor {
    /// Sorted, duplicate-free snap positions: 0, the playhead, every marker
    /// except `exclude_marker`, and the start and end of every clip not in
    /// `exclude_clips`.
    pub fn snap_candidates(&self, exclude_clips: &[Uuid], exclude_marker: Option<Uuid>) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        out.push(0);
        out.push(self.view.playhead);
        out.extend(self.project.markers.iter().filter(|m| Some(m.id) != exclude_marker).map(|m| m.position));
        for track in &self.project.tracks {
            for clip in &track.clips {
                if !exclude_clips.contains(&clip.id) {
                    out.push(clip.start);
                    out.push(clip.end());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Applies snapping to `pos`: returns the final position and the snap point
    /// when it snapped. No snapping when the view's snap toggle is off or
    /// `bypass` is true (Alt during a drag). The threshold in points comes from
    /// the caller (the UI passes its theme snap distance).
    pub fn snap_pos(&self, pos: u64, exclude_clips: &[Uuid], exclude_marker: Option<Uuid>, bypass: bool, threshold_px: f32) -> (u64, Option<u64>) {
        if !self.view.snap || bypass {
            return (pos, None);
        }
        let candidates = self.snap_candidates(exclude_clips, exclude_marker);
        match snap(pos, &candidates, self.view.samples_per_pixel, threshold_px) {
            Some(snapped) => (snapped, Some(snapped)),
            None => (pos, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::fixtures::project_with_clips;
    use crate::model::project::ViewState;

    /// An in-memory editor: two tracks, clips (0..100) and (200..400) on track 1.
    fn editor() -> Editor {
        Editor::with_project(project_with_clips(&[&[(0, 100), (200, 200)], &[]]), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") })
    }

    #[test]
    fn snap_picks_the_nearest_candidate_within_threshold() {
        let candidates = [0, 1004, 2000];
        assert_eq!(snap(1000, &candidates, 1.0, 8.0), Some(1004));
        assert_eq!(snap(1020, &candidates, 1.0, 8.0), None);
        assert_eq!(snap(1070, &candidates, 10.0, 8.0), Some(1004));
    }

    #[test]
    fn snap_with_no_candidates_is_none() {
        assert_eq!(snap(100, &[], 1.0, 8.0), None);
    }

    #[test]
    fn snap_candidates_lists_zero_playhead_markers_and_clip_edges() {
        let mut editor = editor();
        editor.view.playhead = 500;
        let clip = editor.project.tracks[0].clips[0].id;
        let candidates = editor.snap_candidates(&[], None);
        assert_eq!(candidates, vec![0, 100, 200, 400, 500]);
        assert_eq!(editor.snap_candidates(&[clip], None), vec![0, 200, 400, 500]);
    }

    #[test]
    fn snap_candidates_leaves_out_the_excluded_marker() {
        let mut editor = editor();
        editor.view.playhead = 500;
        let mut project = (*editor.project).clone();
        crate::editor::ops::marker::add_marker(&mut project, 300).unwrap();
        let id = project.markers[0].id;
        editor.project = Arc::new(project);
        let candidates = editor.snap_candidates(&[], Some(id));
        assert_eq!(candidates, vec![0, 100, 200, 400, 500]);
        // With nothing excluded the marker comes back.
        assert!(editor.snap_candidates(&[], None).contains(&300));
    }

    #[test]
    fn snap_pos_is_the_input_when_snap_is_off_or_bypassed() {
        let mut editor = editor();
        editor.view.playhead = 1004;
        editor.view.snap = false;
        assert_eq!(editor.snap_pos(1000, &[], None, false, 8.0), (1000, None));
        editor.view.snap = true;
        assert_eq!(editor.snap_pos(1000, &[], None, true, 8.0), (1000, None));
        assert_eq!(editor.snap_pos(1000, &[], None, false, 8.0), (1004, Some(1004)));
    }
}
