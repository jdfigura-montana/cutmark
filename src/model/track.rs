//! model::track: Track, one timeline lane holding clips.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::clip::Clip;

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
