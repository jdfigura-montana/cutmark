//! io::project_file: read and write project.json.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::{self, ProjectPaths};
use super::{IoError, is_project_json};
use crate::model::project::{Project, ViewState};

/// Current project.json format version.
pub const FORMAT_VERSION: u32 = 1;

/// The on-disk project.json document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Format version, currently 1.
    pub format_version: u32,
    /// The project data.
    pub project: Project,
    /// Saved UI view state.
    pub view: ViewState,
}

/// Saves the project pretty-printed to project.json: writes project.json.tmp
/// in the root, then renames it over project.json.
pub fn save(paths: &ProjectPaths, project: &Project, view: &ViewState) -> Result<(), IoError> {
    let file = ProjectFile {
        format_version: FORMAT_VERSION,
        project: project.clone(),
        view: view.clone(),
    };
    let json = serde_json::to_string_pretty(&file)?;
    let tmp = paths.root.join("project.json.tmp");
    let target = paths.project_json();
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

/// Loads a project from a project.json file path or a project folder path.
pub fn load(path: &Path) -> Result<(ProjectPaths, Project, ViewState), IoError> {
    let json_path: PathBuf = if path.is_dir() {
        let candidate = path.join("project.json");
        if !candidate.is_file() {
            return Err(IoError::NotAProject(path.display().to_string()));
        }
        candidate
    } else if is_project_json(path) {
        path.to_path_buf()
    } else {
        return Err(IoError::NotAProject(path.display().to_string()));
    };
    let root: PathBuf = match json_path.parent() {
        Some(parent) => parent.to_path_buf(),
        None => PathBuf::from("."),
    };
    let text = fs::read_to_string(&json_path)?;
    let file: ProjectFile = serde_json::from_str(&text)?;
    if file.format_version > FORMAT_VERSION {
        return Err(IoError::UnsupportedVersion {
            found: file.format_version,
            max: FORMAT_VERSION,
        });
    }
    Ok((ProjectPaths { root }, file.project, file.view))
}

/// Copies a whole project folder to parent/new_name: validates the name, errors
/// if the target exists, then copies every file and folder recursively.
/// Returns the paths of the new project folder.
pub fn copy_project(
    from: &ProjectPaths,
    parent: &Path,
    new_name: &str,
) -> Result<ProjectPaths, IoError> {
    paths::validate_project_name(new_name)?;
    let target_root = parent.join(new_name);
    if target_root.exists() {
        return Err(IoError::FolderExists(target_root.display().to_string()));
    }
    fs::create_dir(&target_root)?;
    copy_tree(&from.root, &target_root)?;
    Ok(ProjectPaths { root: target_root })
}

/// Recursively copies every entry of `source` into `target` (both are folders).
fn copy_tree(source: &Path, target: &Path) -> Result<(), IoError> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            fs::create_dir(&target_path)?;
            copy_tree(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::paths::create_layout;
    use tempfile::tempdir;

    fn sample() -> (Project, ViewState) {
        (
            Project::new(
                "My Project".to_string(),
                48_000,
                crate::model::project::BitDepth::Int24,
            ),
            ViewState {
                scroll_x: 4_096,
                playhead: 12_345,
                ..ViewState::default()
            },
        )
    }

    #[test]
    fn save_then_load_returns_equal_project_and_view() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        let (project, view) = sample();
        save(&paths, &project, &view).unwrap();
        let (loaded_paths, loaded_project, loaded_view) = load(&paths.root).unwrap();
        assert_eq!(loaded_paths.root, paths.root);
        assert_eq!(loaded_project, project);
        assert_eq!(loaded_view, view);
    }

    #[test]
    fn load_works_with_folder_and_json_paths() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        let (project, view) = sample();
        save(&paths, &project, &view).unwrap();
        let from_folder = load(&paths.root).unwrap();
        let from_json = load(&paths.project_json()).unwrap();
        assert_eq!(from_folder.1, from_json.1);
        assert_eq!(from_folder.2, from_json.2);
    }

    #[test]
    fn save_leaves_no_tmp_file_behind() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        let (project, view) = sample();
        save(&paths, &project, &view).unwrap();
        assert!(paths.project_json().is_file());
        assert!(!paths.root.join("project.json.tmp").exists());
    }

    #[test]
    fn load_errors_on_unsupported_format_version() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        let (project, view) = sample();
        save(&paths, &project, &view).unwrap();
        let text = fs::read_to_string(paths.project_json()).unwrap();
        let text = text.replace("\"format_version\": 1", "\"format_version\": 999");
        fs::write(paths.project_json(), text).unwrap();
        assert!(matches!(
            load(&paths.root),
            Err(IoError::UnsupportedVersion { found: 999, max: 1 })
        ));
    }

    #[test]
    fn copy_project_copies_nested_files_in_media_and_cache() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        fs::create_dir_all(paths.media().join("conformed")).unwrap();
        fs::write(paths.media().join("conformed").join("song.wav"), b"wav").unwrap();
        fs::write(paths.cache().join("peaks.bin"), b"peaks").unwrap();
        fs::write(paths.project_json(), b"{}").unwrap();

        let destination = tempdir().unwrap();
        let copied = copy_project(&paths, destination.path(), "Copy").unwrap();

        assert_eq!(copied.root, destination.path().join("Copy"));
        assert_eq!(
            fs::read(copied.media().join("conformed").join("song.wav")).unwrap(),
            b"wav"
        );
        assert_eq!(
            fs::read(copied.cache().join("peaks.bin")).unwrap(),
            b"peaks"
        );
        assert!(copied.project_json().is_file());
    }

    #[test]
    fn copy_project_errors_if_target_exists() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        assert!(matches!(
            copy_project(&paths, parent.path(), "Proj"),
            Err(IoError::FolderExists(_))
        ));
    }

    #[test]
    fn copy_project_errors_on_invalid_name() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        assert!(matches!(
            copy_project(&paths, parent.path(), "a/b"),
            Err(IoError::InvalidProjectName(_))
        ));
    }
}
