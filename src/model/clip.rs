//! model::clip: Clip, a reference into a media item with in/out points and fades.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A placed reference to a range of source media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Unique clip id.
    pub id: Uuid,
    /// Id of the media item this clip plays.
    pub media_id: Uuid,
    /// Timeline start position in samples.
    pub start: u64,
    /// Offset into the conformed media in samples.
    pub src_in: u64,
    /// Exclusive end offset into the conformed media in samples.
    pub src_out: u64,
    /// Clip gain, -60.0 to +24.0 dB, default 0.0.
    pub gain_db: f32,
    /// Fade-in length in samples.
    pub fade_in: u64,
    /// Fade-out length in samples.
    pub fade_out: u64,
}
