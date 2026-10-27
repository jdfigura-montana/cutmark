//! model::invariants: unit-tested rules the model enforces (overlap, bounds, fades).
use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::model::project::Project;

/// A violated model invariant, carrying the id of the offending item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum InvariantError {
    /// Clips on a track are unsorted or overlap (rule a).
    #[error("clips on a track are unsorted or overlap")]
    UnsortedOrOverlapping {
        /// Offending clip id.
        clip_id: Uuid,
    },
    /// A clip points at a media id that does not exist (rule b).
    #[error("clip references missing media")]
    MissingMedia {
        /// Offending clip id.
        clip_id: Uuid,
    },
    /// `src_in < src_out <= media.length` is violated (rule c).
    #[error("clip source range out of bounds")]
    SrcRange {
        /// Offending clip id.
        clip_id: Uuid,
    },
    /// `fade_in + fade_out` exceeds the clip length (rule d).
    #[error("fades exceed clip length")]
    FadeSum {
        /// Offending clip id.
        clip_id: Uuid,
    },
    /// A volume, pan or gain value is out of range (rule e).
    #[error("level value out of range")]
    LevelRange {
        /// Offending track, clip, or nil for the master.
        id: Uuid,
    },
    /// Markers are not sorted by position (rule f).
    #[error("markers not sorted by position")]
    MarkersUnsorted {
        /// Offending marker id.
        marker_id: Uuid,
    },
    /// The same id appears twice among media, tracks, clips or markers (rule g).
    #[error("duplicate id")]
    DuplicateId {
        /// The duplicated id.
        id: Uuid,
    },
}

/// Verifies every model invariant; returns the first violation found.
pub fn check(project: &Project) -> Result<(), InvariantError> {
    // Rule g: no duplicate ids anywhere (media, tracks, clips, markers in that
    // order; the first repeat is reported). A set keeps this linear: the
    // check also runs on projects loaded from disk with up to 100,000 clips.
    let mut seen: HashSet<Uuid> = HashSet::new();
    let ids = project
        .media
        .iter()
        .map(|m| m.id)
        .chain(project.tracks.iter().map(|t| t.id))
        .chain(
            project
                .tracks
                .iter()
                .flat_map(|t| t.clips.iter().map(|c| c.id)),
        )
        .chain(project.markers.iter().map(|m| m.id));
    for id in ids {
        if !seen.insert(id) {
            return Err(InvariantError::DuplicateId { id });
        }
    }
    // Duplicate media ids are rejected above, so each id maps to one length.
    let media_lengths: HashMap<Uuid, u64> =
        project.media.iter().map(|m| (m.id, m.length)).collect();

    for track in &project.tracks {
        // Rule a: clips sorted by start and never overlapping.
        for pair in track.clips.windows(2) {
            if pair[1].start < pair[0].end() {
                return Err(InvariantError::UnsortedOrOverlapping {
                    clip_id: pair[1].id,
                });
            }
        }

        // Rule e: track volume and pan.
        if !(-60.0..=12.0).contains(&track.volume_db) || !(-1.0..=1.0).contains(&track.pan) {
            return Err(InvariantError::LevelRange { id: track.id });
        }

        // Rules b, c, d and clip gain (rule e) per clip.
        for clip in &track.clips {
            let Some(&media_length) = media_lengths.get(&clip.media_id) else {
                return Err(InvariantError::MissingMedia { clip_id: clip.id });
            };
            if !(clip.src_in < clip.src_out && clip.src_out <= media_length) {
                return Err(InvariantError::SrcRange { clip_id: clip.id });
            }
            if clip.fade_in.saturating_add(clip.fade_out) > clip.len() {
                return Err(InvariantError::FadeSum { clip_id: clip.id });
            }
            if !(-60.0..=24.0).contains(&clip.gain_db) {
                return Err(InvariantError::LevelRange { id: clip.id });
            }
        }
    }

    // Rule e: master volume.
    if !(-60.0..=12.0).contains(&project.master_volume_db) {
        return Err(InvariantError::LevelRange { id: Uuid::nil() });
    }

    // Rule f: markers sorted by position.
    for pair in project.markers.windows(2) {
        if pair[1].position < pair[0].position {
            return Err(InvariantError::MarkersUnsorted {
                marker_id: pair[1].id,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::{project_with_clips, spans, srcs};
    use crate::model::marker::Marker;

    #[test]
    fn valid_fixture_project_passes_check() {
        let project = project_with_clips(&[&[(0, 1000), (2000, 500)], &[(0, 800)]]);
        assert_eq!(check(&project), Ok(()));
    }

    #[test]
    fn overlapping_clips_fail_rule_a() {
        let mut project = project_with_clips(&[&[(0, 2000), (1000, 1000)]]);
        project.tracks[0].sort_clips();
        assert_eq!(
            check(&project),
            Err(InvariantError::UnsortedOrOverlapping {
                clip_id: project.tracks[0].clips[1].id
            })
        );
    }

    #[test]
    fn missing_media_fails_rule_b() {
        let mut project = project_with_clips(&[&[(0, 1000)]]);
        project.media.clear();
        assert_eq!(
            check(&project),
            Err(InvariantError::MissingMedia {
                clip_id: project.tracks[0].clips[0].id
            })
        );
    }

    #[test]
    fn src_out_beyond_media_length_fails_rule_c() {
        let mut project = project_with_clips(&[&[(0, 1_000_000)]]);
        project.tracks[0].clips[0].src_out += 1;
        assert_eq!(
            check(&project),
            Err(InvariantError::SrcRange {
                clip_id: project.tracks[0].clips[0].id
            })
        );
    }

    #[test]
    fn empty_source_range_fails_rule_c() {
        let mut project = project_with_clips(&[&[(0, 1000)]]);
        project.tracks[0].clips[0].src_in = 500;
        project.tracks[0].clips[0].src_out = 500;
        assert_eq!(
            check(&project),
            Err(InvariantError::SrcRange {
                clip_id: project.tracks[0].clips[0].id
            })
        );
    }

    #[test]
    fn fade_sum_beyond_length_fails_rule_d() {
        let mut project = project_with_clips(&[&[(0, 1000)]]);
        project.tracks[0].clips[0].fade_in = 600;
        project.tracks[0].clips[0].fade_out = 600;
        assert_eq!(
            check(&project),
            Err(InvariantError::FadeSum {
                clip_id: project.tracks[0].clips[0].id
            })
        );
    }

    #[test]
    fn out_of_range_track_volume_fails_rule_e() {
        let mut project = project_with_clips(&[&[]]);
        project.tracks[0].volume_db = 13.0;
        assert_eq!(
            check(&project),
            Err(InvariantError::LevelRange {
                id: project.tracks[0].id
            })
        );
    }

    #[test]
    fn out_of_range_clip_gain_fails_rule_e() {
        let mut project = project_with_clips(&[&[(0, 1000)]]);
        project.tracks[0].clips[0].gain_db = 25.0;
        assert_eq!(
            check(&project),
            Err(InvariantError::LevelRange {
                id: project.tracks[0].clips[0].id
            })
        );
    }

    #[test]
    fn out_of_range_master_volume_fails_rule_e() {
        let mut project = project_with_clips(&[&[]]);
        project.master_volume_db = -61.0;
        assert_eq!(
            check(&project),
            Err(InvariantError::LevelRange { id: Uuid::nil() })
        );
    }

    #[test]
    fn unsorted_markers_fail_rule_f() {
        let mut project = project_with_clips(&[&[]]);
        let late = Uuid::new_v4();
        let early = Uuid::new_v4();
        project.markers = vec![
            Marker {
                id: late,
                position: 500,
            },
            Marker {
                id: early,
                position: 100,
            },
        ];
        assert_eq!(
            check(&project),
            Err(InvariantError::MarkersUnsorted { marker_id: early })
        );
    }

    #[test]
    fn duplicate_track_id_fails_rule_g() {
        let mut project = project_with_clips(&[&[], &[]]);
        let duplicate = project.tracks[0].id;
        project.tracks[1].id = duplicate;
        assert_eq!(
            check(&project),
            Err(InvariantError::DuplicateId { id: duplicate })
        );
    }

    #[test]
    fn duplicate_clip_id_across_tracks_fails_rule_g() {
        let mut project = project_with_clips(&[&[(0, 1000)], &[(0, 1000)]]);
        let duplicate = project.tracks[0].clips[0].id;
        project.tracks[1].clips[0].id = duplicate;
        assert_eq!(
            check(&project),
            Err(InvariantError::DuplicateId { id: duplicate })
        );
    }

    #[test]
    fn clamp_fades_trims_fade_in_to_fit() {
        let mut project = project_with_clips(&[&[(0, 100)]]);
        let clip = &mut project.tracks[0].clips[0];
        clip.fade_in = 80;
        clip.fade_out = 50;
        clip.clamp_fades();
        assert_eq!((clip.fade_in, clip.fade_out), (50, 50));
    }

    #[test]
    fn clamp_fades_trims_fade_out_to_len_first() {
        let mut project = project_with_clips(&[&[(0, 100)]]);
        let clip = &mut project.tracks[0].clips[0];
        clip.fade_out = 150;
        clip.clamp_fades();
        assert_eq!((clip.fade_in, clip.fade_out), (0, 100));
    }

    #[test]
    fn spans_and_srcs_read_the_fixture() {
        let project = project_with_clips(&[&[(100, 400), (600, 200)]]);
        assert_eq!(spans(&project, 0), vec![(100, 500), (600, 800)]);
        assert_eq!(srcs(&project, 0), vec![(0, 400), (0, 200)]);
        assert_eq!(project.tracks[0].clips[0].len(), 400);
        assert_eq!(project.tracks[0].clips[0].end(), 500);
    }
}
