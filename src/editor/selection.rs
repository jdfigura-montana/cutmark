//! editor::selection: range and clip selection state, not undoable.

use uuid::Uuid;

use crate::model::project::Project;

/// What is currently selected; only one kind is active at a time.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Selection {
    /// Nothing selected.
    #[default]
    None,
    /// A time range on a set of tracks, always `start < end`, tracks in
    /// top-to-bottom project order with no duplicates.
    Range {
        /// Range start in samples, always less than `end`.
        start: u64,
        /// Range end in samples, always greater than `start`.
        end: u64,
        /// Selected track ids in top-to-bottom project order, no duplicates.
        tracks: Vec<Uuid>,
    },
    /// A set of selected clip ids.
    Clips(Vec<Uuid>),
}

impl Selection {
    /// Drops clip and track ids that no longer exist; `None` when nothing is left.
    pub fn tidy(self, project: &Project) -> Selection {
        match self {
            Selection::None => Selection::None,
            Selection::Range { start, end, tracks } => {
                let tracks: Vec<Uuid> = tracks
                    .into_iter()
                    .filter(|id| project.tracks.iter().any(|t| t.id == *id))
                    .collect();
                if tracks.is_empty() {
                    Selection::None
                } else {
                    Selection::Range { start, end, tracks }
                }
            }
            Selection::Clips(ids) => {
                let ids: Vec<Uuid> = ids
                    .into_iter()
                    .filter(|id| {
                        project
                            .tracks
                            .iter()
                            .any(|t| t.clips.iter().any(|c| c.id == *id))
                    })
                    .collect();
                if ids.is_empty() {
                    Selection::None
                } else {
                    Selection::Clips(ids)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::project_with_clips;

    #[test]
    fn tidy_drops_gone_track_and_clip_ids() {
        let project = project_with_clips(&[&[(0, 100)], &[(0, 100)]]);
        let track0 = project.tracks[0].id;
        let track1 = project.tracks[1].id;
        let clip = project.tracks[0].clips[0].id;

        let range = Selection::Range {
            start: 0,
            end: 100,
            tracks: vec![track0, track1],
        }
        .tidy(&project);
        assert_eq!(
            range,
            Selection::Range {
                start: 0,
                end: 100,
                tracks: vec![track0, track1]
            }
        );

        let gone = Uuid::new_v4();
        let kept = Selection::Range {
            start: 0,
            end: 100,
            tracks: vec![track0, gone],
        }
        .tidy(&project);
        assert_eq!(
            kept,
            Selection::Range {
                start: 0,
                end: 100,
                tracks: vec![track0]
            }
        );

        let clips = Selection::Clips(vec![clip, gone]).tidy(&project);
        assert_eq!(clips, Selection::Clips(vec![clip]));
    }

    #[test]
    fn tidy_returns_none_when_nothing_is_left() {
        let project = project_with_clips(&[&[(0, 100)]]);
        let gone = Uuid::new_v4();
        assert_eq!(Selection::None.tidy(&project), Selection::None);
        assert_eq!(
            Selection::Range {
                start: 0,
                end: 10,
                tracks: vec![gone]
            }
            .tidy(&project),
            Selection::None
        );
        assert_eq!(Selection::Clips(vec![gone]).tidy(&project), Selection::None);
    }
}
