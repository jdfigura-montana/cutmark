//! model::project: the Project struct, the source of truth for the open project.

use serde::{Deserialize, Serialize};

use crate::model::marker::Marker;
use crate::model::media::MediaItem;
use crate::model::track::Track;

/// Export bit depth for WAV files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BitDepth {
    /// 16-bit integer PCM.
    Int16,
    /// 24-bit integer PCM.
    Int24,
    /// 32-bit float PCM.
    Float32,
}

/// The whole project: plain data, cheap to clone, no audio samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// Project name.
    pub name: String,
    /// Sample rate, fixed at creation.
    pub sample_rate: u32,
    /// Default export bit depth.
    pub export_bit_depth: BitDepth,
    /// Master volume, -60.0 to +12.0 dB, default 0.0.
    pub master_volume_db: f32,
    /// Media bin entries.
    pub media: Vec<MediaItem>,
    /// Tracks, top to bottom.
    pub tracks: Vec<Track>,
    /// Markers, sorted by position.
    pub markers: Vec<Marker>,
}

impl Project {
    /// Creates an empty project with the given name, sample rate and export bit depth.
    pub fn new(name: String, sample_rate: u32, export_bit_depth: BitDepth) -> Project {
        Project { name, sample_rate, export_bit_depth, master_volume_db: 0.0, media: Vec::new(), tracks: Vec::new(), markers: Vec::new() }
    }
}

/// UI view state saved in project.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// Horizontal zoom in samples per pixel.
    #[serde(default = "default_samples_per_pixel")]
    pub samples_per_pixel: f64,
    /// Horizontal scroll position in samples.
    #[serde(default)]
    pub scroll_x: u64,
    /// Vertical scroll position in pixels.
    #[serde(default)]
    pub scroll_y: f32,
    /// Playhead position in samples.
    #[serde(default)]
    pub playhead: u64,
    /// Snapping toggle.
    #[serde(default = "default_true")]
    pub snap: bool,
    /// Ripple edit toggle.
    #[serde(default)]
    pub ripple: bool,
}

/// Default zoom: 1024 samples per pixel.
fn default_samples_per_pixel() -> f64 {
    1024.0
}

/// Serde default helper for `true`.
fn default_true() -> bool {
    true
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState { samples_per_pixel: 1024.0, scroll_x: 0, scroll_y: 0.0, playhead: 0, snap: true, ripple: false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::clip::Clip;
    use uuid::Uuid;

    #[test]
    fn new_project_sets_fields_and_leaves_lists_empty() {
        let project = Project::new("Test".to_string(), 48_000, BitDepth::Int24);
        assert_eq!(project.name, "Test");
        assert_eq!(project.sample_rate, 48_000);
        assert_eq!(project.export_bit_depth, BitDepth::Int24);
        assert_eq!(project.master_volume_db, 0.0);
        assert!(project.media.is_empty());
        assert!(project.tracks.is_empty());
        assert!(project.markers.is_empty());
    }

    #[test]
    fn project_survives_serde_json_round_trip() {
        let media_id = Uuid::new_v4();
        let track_id = Uuid::new_v4();
        let clip_id = Uuid::new_v4();
        let marker_id = Uuid::new_v4();

        let project = Project {
            name: "Round".to_string(),
            sample_rate: 44_100,
            export_bit_depth: BitDepth::Float32,
            master_volume_db: -1.5,
            media: vec![MediaItem {
                id: media_id,
                display_name: "Song".to_string(),
                file_name: "song.wav".to_string(),
                original_rate: 44_100,
                channels: 2,
                length: 123_456,
            }],
            tracks: vec![Track {
                id: track_id,
                name: "Track 1".to_string(),
                volume_db: 0.0,
                pan: 0.0,
                mute: false,
                solo: true,
                height: 80.0,
                clips: vec![Clip { id: clip_id, media_id, start: 1000, src_in: 0, src_out: 5000, gain_db: 2.0, fade_in: 10, fade_out: 20 }],
            }],
            markers: vec![Marker { id: marker_id, position: 999 }],
        };

        let json = serde_json::to_string(&project).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back, project);
    }
}
