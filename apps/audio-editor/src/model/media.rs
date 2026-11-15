//! model::media: MediaItem, a bin entry pointing at conformed audio in media/.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// One imported media file in the bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaItem {
    /// Unique media id.
    pub id: Uuid,
    /// Name shown in the bin.
    pub display_name: String,
    /// File name inside the project's media/ folder.
    pub file_name: String,
    /// Original sample rate of the file.
    pub original_rate: u32,
    /// Channel count, 1 or 2.
    pub channels: u16,
    /// Length in samples at the project rate.
    pub length: u64,
}

/// Waveform min/max peaks at four block sizes, used for drawing (plain data, not serialized in project.json).
#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    /// Channel count the peaks were built from.
    pub channels: u16,
    /// Total frame count of the source audio.
    pub frames: u64,
    /// One level per block size (256, 1024, 4096, 16384).
    pub levels: Vec<PeakLevel>,
}

/// One peak level: min/max pairs per block of samples.
/// `data` holds [min, max] per block per channel, indexed `block * channels + channel`.
#[derive(Debug, Clone, PartialEq)]
pub struct PeakLevel {
    /// Samples per block at this level.
    pub block_size: u32,
    /// Flattened [min, max] pairs per block per channel.
    pub data: Vec<[f32; 2]>,
}
