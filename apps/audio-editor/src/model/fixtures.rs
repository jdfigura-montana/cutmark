//! model::fixtures: shared test fixtures for building small projects.
use uuid::Uuid;

use crate::model::clip::Clip;
use crate::model::media::MediaItem;
use crate::model::project::{BitDepth, Project};
use crate::model::track::Track;

/// Builds a 48 kHz project with one stereo media item (length 1,000,000) and the given clips.
///
/// Each inner slice is one track named "Track 1", "Track 2", ...; each `(start, len)` pair
/// becomes a clip with `src_in 0`, `src_out len`, gain 0 and no fades.
pub fn project_with_clips(tracks: &[&[(u64, u64)]]) -> Project {
    let media_id = Uuid::new_v4();
    let media = MediaItem {
        id: media_id,
        display_name: "Test Media".to_string(),
        file_name: "test.wav".to_string(),
        original_rate: 48_000,
        channels: 2,
        length: 1_000_000,
    };

    let track_vecs: Vec<Track> = tracks
        .iter()
        .enumerate()
        .map(|(index, placements)| {
            let mut track = Track::new(format!("Track {}", index + 1));
            track.clips = placements
                .iter()
                .map(|&(start, len)| Clip { id: Uuid::new_v4(), media_id, start, src_in: 0, src_out: len, gain_db: 0.0, fade_in: 0, fade_out: 0 })
                .collect();
            track
        })
        .collect();

    let mut project = Project::new("Test".to_string(), 48_000, BitDepth::Int24);
    project.media = vec![media];
    project.tracks = track_vecs;
    project
}

/// Returns `(start, end)` of each clip on the given track index.
pub fn spans(project: &Project, track: usize) -> Vec<(u64, u64)> {
    project.tracks[track].clips.iter().map(|clip| (clip.start, clip.end())).collect()
}

/// Returns `(src_in, src_out)` of each clip on the given track index.
pub fn srcs(project: &Project, track: usize) -> Vec<(u64, u64)> {
    project.tracks[track].clips.iter().map(|clip| (clip.src_in, clip.src_out)).collect()
}
