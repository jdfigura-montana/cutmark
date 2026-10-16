//! model::marker: Marker, a position-only timeline marker.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A marker on the timeline ruler, position only, no name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    /// Unique marker id.
    pub id: Uuid,
    /// Position in samples.
    pub position: u64,
}
