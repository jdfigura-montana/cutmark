//! editor::ops: pure edit operations on clips, ranges, paste, tracks, markers and media.

pub mod amplitude;
pub mod clip;
pub mod marker;
pub mod media;
pub mod paste;
pub mod range;
pub mod track;

use crate::io::paths;
use crate::model::project::{BitDepth, Project};

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

/// Sets the project name and default export bit depth.
///
/// The name is validated with [`paths::validate_project_name`]; the project
/// folder on disk is never renamed.
pub fn set_project_settings(
    project: &mut Project,
    name: &str,
    export_bit_depth: BitDepth,
) -> Result<(), OpError> {
    if let Err(error) = paths::validate_project_name(name) {
        return Err(OpError::Invalid(error.to_string()));
    }
    project.name = name.to_string();
    project.export_bit_depth = export_bit_depth;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_project_settings_sets_name_and_depth() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[]]);
        set_project_settings(&mut project, "Renamed", BitDepth::Float32).unwrap();
        assert_eq!(project.name, "Renamed");
        assert_eq!(project.export_bit_depth, BitDepth::Float32);
    }

    #[test]
    fn set_project_settings_invalid_name_errors_and_changes_nothing() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[]]);
        let before = project.clone();
        for bad in ["", "   ", "a/b", "a:b"] {
            assert!(matches!(
                set_project_settings(&mut project, bad, BitDepth::Int16),
                Err(OpError::Invalid(_))
            ));
            assert_eq!(project, before);
        }
    }
}
