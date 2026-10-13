//! io::paths: project folder layout (media/, cache/, autosave/, exports/).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use uuid::Uuid;

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

/// Returns a unique file name for media/: "boom.wav", then "boom (2).wav",
/// "boom (3).wav", ... as long as the previous name is taken in `media_dir`.
pub fn unique_media_name(media_dir: &Path, file_name: &str) -> String {
    let is_taken = |name: &str| media_dir.join(name).exists();
    if !is_taken(file_name) {
        return file_name.to_string();
    }
    let stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".to_string());
    let ext = Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut n = 2u32;
    loop {
        let candidate = if ext.is_empty() {
            format!("{stem} ({n})")
        } else {
            format!("{stem} ({n}).{ext}")
        };
        if !is_taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Path of the conformed 32-bit float WAV for a media id: cache/<id>.f32.wav.
pub fn conformed_path(paths: &ProjectPaths, media_id: Uuid) -> PathBuf {
    paths.cache().join(format!("{}.f32.wav", media_id))
}

/// Path of the waveform peaks cache file for a media id: cache/<id>.peaks.
pub fn peaks_path(paths: &ProjectPaths, media_id: Uuid) -> PathBuf {
    paths.cache().join(format!("{}.peaks", media_id))
}

/// Opens Windows Explorer with `path` pre-selected. Paths with spaces and
/// commas are passed safely via a raw argument. Errors are ignored.
pub fn reveal_in_explorer(path: &Path) {
    use std::os::windows::process::CommandExt;
    let _ = Command::new("explorer.exe")
        .raw_arg(format!("/select,\"{}\"", path.display()))
        .spawn();
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

    #[test]
    fn unique_media_name_avoids_clashes() {
        let dir = tempdir().unwrap();
        let media = dir.path().join("media");
        fs::create_dir_all(&media).unwrap();
        assert_eq!(unique_media_name(&media, "boom.wav"), "boom.wav");
        fs::write(media.join("boom.wav"), b"x").unwrap();
        assert_eq!(unique_media_name(&media, "boom.wav"), "boom (2).wav");
        fs::write(media.join("boom (2).wav"), b"x").unwrap();
        assert_eq!(unique_media_name(&media, "boom.wav"), "boom (3).wav");
    }

    #[test]
    fn cache_paths_use_media_id() {
        let dir = tempdir().unwrap();
        let paths = create_layout(dir.path(), "P2").unwrap();
        let id = Uuid::new_v4();
        assert_eq!(
            conformed_path(&paths, id),
            paths.cache().join(format!("{}.f32.wav", id))
        );
        assert_eq!(
            peaks_path(&paths, id),
            paths.cache().join(format!("{}.peaks", id))
        );
    }
}
