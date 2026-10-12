//! io::paths: project folder layout (media/, cache/, autosave/, exports/).

use std::fs;
use std::path::{Path, PathBuf};

use super::IoError;

/// Locations of a project folder and its standard subfolders.
#[derive(Debug, Clone)]
pub struct ProjectPaths {
    /// Project root folder.
    pub root: PathBuf,
}

impl ProjectPaths {
    /// Path of project.json inside the root.
    pub fn project_json(&self) -> PathBuf {
        self.root.join("project.json")
    }

    /// Path of the media/ subfolder.
    pub fn media(&self) -> PathBuf {
        self.root.join("media")
    }

    /// Path of the cache/ subfolder.
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Path of the autosave/ subfolder.
    pub fn autosave(&self) -> PathBuf {
        self.root.join("autosave")
    }

    /// Path of the exports/ subfolder.
    pub fn exports(&self) -> PathBuf {
        self.root.join("exports")
    }
}

/// Forbidden characters in a project name.
const FORBIDDEN_NAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Validates a project name: not empty, not whitespace-only, no forbidden characters.
pub fn validate_project_name(name: &str) -> Result<(), IoError> {
    if name.trim().is_empty() || name.chars().any(|c| FORBIDDEN_NAME_CHARS.contains(&c)) {
        return Err(IoError::InvalidProjectName(name.to_string()));
    }
    Ok(())
}

/// Creates a new project folder with media/, cache/, autosave/ and exports/ under parent.
/// Errors if the folder already exists or the name is invalid.
pub fn create_layout(parent: &Path, name: &str) -> Result<ProjectPaths, IoError> {
    validate_project_name(name)?;
    let root = parent.join(name);
    if root.exists() {
        return Err(IoError::FolderExists(root.display().to_string()));
    }
    fs::create_dir(&root)?;
    for sub in ["media", "cache", "autosave", "exports"] {
        fs::create_dir(root.join(sub))?;
    }
    Ok(ProjectPaths { root })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_layout_makes_all_five_folders() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "My Project").unwrap();
        assert!(paths.root.is_dir());
        assert!(paths.media().is_dir());
        assert!(paths.cache().is_dir());
        assert!(paths.autosave().is_dir());
        assert!(paths.exports().is_dir());
    }

    #[test]
    fn create_layout_errors_when_folder_exists() {
        let parent = tempdir().unwrap();
        create_layout(parent.path(), "Proj").unwrap();
        assert!(matches!(
            create_layout(parent.path(), "Proj"),
            Err(IoError::FolderExists(_))
        ));
    }

    #[test]
    fn validate_project_name_rejects_bad_names() {
        for bad in ["", "   ", "a/b", "a:b"] {
            assert!(
                validate_project_name(bad).is_err(),
                "expected rejection of {bad:?}"
            );
        }
    }

    #[test]
    fn validate_project_name_accepts_simple_names() {
        assert!(validate_project_name("My Project").is_ok());
    }
}
