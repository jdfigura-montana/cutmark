//! audio::mix_plan: MixPlan, the project plus media buffers handed to the audio thread.

use std::sync::Arc;

use uuid::Uuid;

use crate::model::project::Project;
use crate::util::db::db_to_linear;

/// One clip flattened for the mixer.
#[derive(Debug, Clone)]
pub struct ClipPlan {
    /// Timeline start position in samples.
    pub start: u64,
    /// Clip length in samples.
    pub len: u64,
    /// Offset into the source media in samples.
    pub src_in: u64,
    /// Source channel count, 1 or 2.
    pub channels: u16,
    /// Linear clip gain.
    pub gain: f32,
    /// Fade-in length in samples.
    pub fade_in: u64,
    /// Fade-out length in samples.
    pub fade_out: u64,
    /// Interleaved source samples, or None when the media is offline (plays silence).
    pub samples: Option<Arc<[f32]>>,
}

/// One track flattened for the mixer.
#[derive(Debug, Clone)]
pub struct TrackPlan {
    /// Whether the track takes part in mixing (mute/solo resolved).
    pub audible: bool,
    /// Linear gain applied to the left channel.
    pub gain_l: f32,
    /// Linear gain applied to the right channel.
    pub gain_r: f32,
    /// Clips sorted by start.
    pub clips: Vec<ClipPlan>,
}

/// The whole mix state handed to the audio thread: immutable snapshot, cheap to swap.
#[derive(Debug, Clone)]
pub struct MixPlan {
    /// Sample rate, matches the project.
    pub rate: u32,
    /// Project length in samples.
    pub length: u64,
    /// Linear master gain.
    pub master_gain: f32,
    /// One plan per project track, same order.
    pub tracks: Vec<TrackPlan>,
}

impl MixPlan {
    /// An empty plan: no tracks, no length, unity master gain.
    pub fn empty(rate: u32) -> MixPlan {
        MixPlan {
            rate,
            length: 0,
            master_gain: 1.0,
            tracks: Vec::new(),
        }
    }

    /// Builds a plan from the project, pulling each clip's samples through `media`.
    /// The closure returns the conformed interleaved samples for a media id, or None when offline.
    pub fn build(project: &Project, media: &dyn Fn(Uuid) -> Option<Arc<[f32]>>) -> MixPlan {
        let any_solo = project.tracks.iter().any(|t| t.solo);
        let mut tracks = Vec::with_capacity(project.tracks.len());
        for track in &project.tracks {
            let audible = !track.mute && (!any_solo || track.solo);
            let vol = db_to_linear(track.volume_db);
            let angle = (track.pan + 1.0) * std::f32::consts::PI / 4.0;
            let clips = track
                .clips
                .iter()
                .map(|clip| ClipPlan {
                    start: clip.start,
                    len: clip.len(),
                    src_in: clip.src_in,
                    channels: project
                        .media
                        .iter()
                        .find(|m| m.id == clip.media_id)
                        .map(|m| m.channels)
                        .unwrap_or(1),
                    gain: db_to_linear(clip.gain_db),
                    fade_in: clip.fade_in,
                    fade_out: clip.fade_out,
                    samples: media(clip.media_id),
                })
                .collect();
            tracks.push(TrackPlan {
                audible,
                gain_l: vol * angle.cos(),
                gain_r: vol * angle.sin(),
                clips,
            });
        }
        let length = project
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.end())
            .max()
            .unwrap_or(0);
        MixPlan {
            rate: project.sample_rate,
            length,
            master_gain: db_to_linear(project.master_volume_db),
            tracks,
        }
    }

    /// Builds a one-track, one-clip plan for previewing a bin item at unity gain.
    pub fn preview(
        media_len: u64,
        channels: u16,
        samples: Arc<[f32]>,
        rate: u32,
        master_volume_db: f32,
    ) -> MixPlan {
        MixPlan {
            rate,
            length: media_len,
            master_gain: db_to_linear(master_volume_db),
            tracks: vec![TrackPlan {
                audible: true,
                gain_l: 1.0,
                gain_r: 1.0,
                clips: vec![ClipPlan {
                    start: 0,
                    len: media_len,
                    src_in: 0,
                    channels,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                    samples: Some(samples),
                }],
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::clip::Clip;
    use crate::model::media::MediaItem;
    use crate::model::project::BitDepth;
    use crate::model::track::Track;

    fn media_item(id: Uuid, channels: u16, length: u64) -> MediaItem {
        MediaItem {
            id,
            display_name: "M".to_string(),
            file_name: "m.wav".to_string(),
            original_rate: 48_000,
            channels,
            length,
        }
    }

    #[test]
    fn empty_plan_is_silent_and_unity() {
        let plan = MixPlan::empty(48_000);
        assert_eq!(plan.rate, 48_000);
        assert_eq!(plan.length, 0);
        assert_eq!(plan.master_gain, 1.0);
        assert!(plan.tracks.is_empty());
    }

    #[test]
    fn build_resolves_mute_solo_pan_and_media() {
        let online = Uuid::new_v4();
        let offline = Uuid::new_v4();
        let mut project = Project::new("P".to_string(), 48_000, BitDepth::Int24);
        project.master_volume_db = -6.0206;
        let mut muted = Track::new("m".to_string());
        muted.mute = true;
        let mut soloed = Track::new("s".to_string());
        soloed.solo = true;
        let mut panned = Track::new("p".to_string());
        panned.pan = -1.0;
        let mut normal = Track::new("n".to_string());
        normal.clips = vec![Clip {
            id: Uuid::new_v4(),
            media_id: offline,
            start: 10,
            src_in: 0,
            src_out: 100,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
        }];
        let mut online_track = Track::new("o".to_string());
        online_track.clips = vec![Clip {
            id: Uuid::new_v4(),
            media_id: online,
            start: 0,
            src_in: 0,
            src_out: 1000,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
        }];
        project.media = vec![media_item(online, 2, 1000), media_item(offline, 1, 500)];
        project.tracks = vec![muted, soloed, panned, normal, online_track];

        let media_map: std::collections::HashMap<Uuid, Arc<[f32]>> =
            [(online, Arc::from(vec![0.0f32; 2000].into_boxed_slice()))]
                .into_iter()
                .collect();
        let lookup = |id: Uuid| media_map.get(&id).cloned();

        let plan = MixPlan::build(&project, &lookup);
        assert!((plan.master_gain - 0.5).abs() < 1e-3);
        assert_eq!(plan.length, 1000);
        assert!(!plan.tracks[0].audible, "muted track is not audible");
        assert!(plan.tracks[1].audible, "soloed track is audible");
        assert!(
            !plan.tracks[2].audible,
            "non-soloed track is silent while solo is active"
        );
        assert!(
            !plan.tracks[3].audible,
            "non-soloed track is silent while solo is active"
        );
        assert!(
            !plan.tracks[4].audible,
            "non-soloed track is silent while solo is active"
        );

        // Without any solo, every unmuted track is audible.
        let mut no_solo = project.clone();
        no_solo.tracks[1].solo = false;
        let plan2 = MixPlan::build(&no_solo, &lookup);
        for (i, t) in plan2.tracks.iter().enumerate() {
            assert_eq!(t.audible, i != 0, "track {i} audibility with no solo");
        }

        // Hard left pan: all gain in L, none in R.
        assert!(plan.tracks[2].gain_r.abs() < 1e-5);
        assert!((plan.tracks[2].gain_l - 1.0).abs() < 1e-5);

        // Offline media id -> samples None; online -> Some with the media's channel count.
        assert!(plan.tracks[3].clips[0].samples.is_none());
        assert_eq!(plan.tracks[3].clips[0].channels, 1);
        assert!(plan.tracks[4].clips[0].samples.is_some());
        assert_eq!(plan.tracks[4].clips[0].channels, 2);
    }

    #[test]
    fn center_pan_gives_minus_three_db_per_side() {
        let mut project = Project::new("P".to_string(), 48_000, BitDepth::Int24);
        project.tracks = vec![Track::new("c".to_string())];
        let plan = MixPlan::build(&project, &|_| None);
        assert!((plan.tracks[0].gain_l - 0.707_106_77).abs() < 1e-5);
        assert!((plan.tracks[0].gain_r - 0.707_106_77).abs() < 1e-5);
    }

    #[test]
    fn preview_is_one_unity_track_one_clip() {
        let samples: Arc<[f32]> = Arc::from(vec![0.5f32; 8].into_boxed_slice());
        let plan = MixPlan::preview(4, 2, samples, 48_000, -6.0206);
        assert_eq!(plan.length, 4);
        assert!((plan.master_gain - 0.5).abs() < 1e-3);
        assert_eq!(plan.tracks.len(), 1);
        assert!(plan.tracks[0].audible);
        assert_eq!(plan.tracks[0].gain_l, 1.0);
        assert_eq!(plan.tracks[0].gain_r, 1.0);
        assert_eq!(plan.tracks[0].clips.len(), 1);
        assert_eq!(plan.tracks[0].clips[0].len, 4);
        assert_eq!(plan.tracks[0].clips[0].channels, 2);
    }
}
