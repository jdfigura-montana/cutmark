//! io::project_file: read and write project.json.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::ProjectPaths;
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
}
