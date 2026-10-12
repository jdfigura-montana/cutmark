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
