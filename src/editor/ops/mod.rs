//! editor::ops: pure edit operations on clips, ranges, paste, tracks, markers and media.

pub mod amplitude;
pub mod clip;
pub mod marker;
pub mod media;
pub mod paste;
pub mod range;
pub mod track;

use thiserror::Error;
use uuid::Uuid;

/// Errors returned by the pure ops functions in `editor::ops`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpError {
    /// The item with the given id does not exist.
    #[error("item not found: {0}")]
    NotFound(Uuid),
    /// The operation's arguments are invalid (for example an empty name).
    #[error("invalid operation: {0}")]
    Invalid(String),
    /// An index is past the end of the list it indexes into.
    #[error("index out of range")]
    Overflow,
}
