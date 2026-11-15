//! model::track: Track, one timeline lane holding clips.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::clip::Clip;

/// Smallest lane height in pixels (also the clamp applied to loaded projects).
pub const MIN_HEIGHT: f32 = 40.0;
/// Largest lane height in pixels.
pub const MAX_HEIGHT: f32 = 400.0;
/// Lane height of a new track, and the stand-in for a non-numeric loaded height.
pub const DEFAULT_HEIGHT: f32 = 96.0;

/// One timeline track, top to bottom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// Unique track id.
    pub id: Uuid,
    /// Track name.
    pub name: String,
    /// Track volume, -60.0 to +12.0 dB, default 0.0.
    pub volume_db: f32,
    /// Stereo pan, -1.0 (left) to +1.0 (right), default 0.0.
    pub pan: f32,
    /// Mute flag.
    pub mute: bool,
    /// Solo flag.
    pub solo: bool,
    /// Lane height in pixels, UI only.
    pub height: f32,
    /// Clips sorted by start, never overlapping.
    pub clips: Vec<Clip>,
}

impl Track {
    /// Creates an empty track: volume 0.0 dB, pan centered, not muted or soloed, 96 px high.
    pub fn new(name: String) -> Track {
        Track {
            id: Uuid::new_v4(),
            name,
            volume_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            height: DEFAULT_HEIGHT,
            clips: Vec::new(),
        }
    }

    /// Sorts the clips by start position.
    pub fn sort_clips(&mut self) {
        self.clips.sort_by_key(|clip| clip.start);
    }
}
