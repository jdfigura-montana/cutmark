//! io::project_file: read and write project.json.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::paths::{self, ProjectPaths};
use super::{IoError, is_project_json};
use crate::model::project::{Project, ViewState};
use crate::model::validate;

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
    let file = ProjectFile { format_version: FORMAT_VERSION, project: project.clone(), view: view.clone() };
    let json = serde_json::to_string_pretty(&file)?;
    let tmp = paths.root.join("project.json.tmp");
    let target = paths.project_json();
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

/// Repairs and validates a project that was just read from disk (project.json
/// or an autosave); shared by both loaders. Track heights are clamped to the
/// lane limits; everything else is checked and a failure is
/// `IoError::InvalidProject` with the reason, so the project does not open:
/// the project name (`paths::validate_project_name`) and then
/// `validate::validate_loaded` (sample rate, counts, channels, media file
/// names, level ranges, 24 hour limit, model invariants).
pub(super) fn finish_load(mut project: Project) -> Result<Project, IoError> {
    validate::clamp_track_heights(&mut project);
    if paths::validate_project_name(&project.name).is_err() {
        return Err(IoError::InvalidProject(format!("the project name {:?} is not allowed", project.name.chars().take(48).collect::<String>())));
    }
    validate::validate_loaded(&project).map_err(|error| IoError::InvalidProject(error.to_string()))?;
    Ok(project)
}

/// Loads a project from a project.json file path or a project folder path.
/// The project is validated (`finish_load`); an invalid one is an error.
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
        return Err(IoError::UnsupportedVersion { found: file.format_version, max: FORMAT_VERSION });
    }
    let project = finish_load(file.project)?;
    Ok((ProjectPaths { root }, project, file.view))
}

/// Copies a whole project folder to parent/new_name: validates the name, errors
/// if the target exists, then copies every file and folder recursively.
/// Returns the paths of the new project folder.
pub fn copy_project(from: &ProjectPaths, parent: &Path, new_name: &str) -> Result<ProjectPaths, IoError> {
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
            Project::new("My Project".to_string(), 48_000, crate::model::project::BitDepth::Int24),
            ViewState { scroll_x: 4_096, playhead: 12_345, ..ViewState::default() },
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

    /// Saves `project` into a fresh project folder (`save` does not validate,
    /// so this can write a project that `load` must refuse).
    fn saved(project: &Project) -> (tempfile::TempDir, ProjectPaths) {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        save(&paths, project, &ViewState::default()).unwrap();
        (parent, paths)
    }

    /// The text of the error `load` gives for `project`.
    fn load_error(project: &Project) -> String {
        let (_parent, paths) = saved(project);
        match load(&paths.root) {
            Err(error @ IoError::InvalidProject(_)) => error.to_string(),
            other => panic!("expected InvalidProject, got {other:?}"),
        }
    }

    /// A project with media, a track and two clips.
    fn full_project() -> Project {
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 1_000), (2_000, 500)]]);
        project.tracks[0].volume_db = -3.5;
        project.tracks[0].pan = 0.25;
        project.tracks[0].clips[0].gain_db = 2.0;
        project.tracks[0].clips[0].fade_in = 100;
        project.master_volume_db = -1.0;
        project
    }

    #[test]
    fn a_valid_project_round_trips_unchanged() {
        let project = full_project();
        let (_parent, paths) = saved(&project);
        let (_, loaded, view) = load(&paths.root).unwrap();
        assert_eq!(loaded, project);
        assert_eq!(view, ViewState::default());
    }

    #[test]
    fn a_44000_hz_project_still_loads() {
        for rate in [44_000, 88_000, 8_000, 192_000] {
            let mut project = full_project();
            project.sample_rate = rate;
            let (_parent, paths) = saved(&project);
            let (_, loaded, _) = load(&paths.root).unwrap();
            assert_eq!(loaded.sample_rate, rate);
        }
    }

    #[test]
    fn track_heights_are_clamped_on_load() {
        let mut project = full_project();
        project.tracks[0].height = 5.0;
        let (_parent, paths) = saved(&project);
        let (_, loaded, _) = load(&paths.root).unwrap();
        assert_eq!(loaded.tracks[0].height, 40.0);
        // Nothing else changed.
        project.tracks[0].height = 40.0;
        assert_eq!(loaded, project);
    }

    #[test]
    fn invalid_projects_do_not_open_and_say_why() {
        type Break = fn(&mut Project);
        let cases: [(&str, Break, &str); 12] = [
            ("sample rate 0", |p| p.sample_rate = 0, "sample rate 0 Hz"),
            ("sample rate too high", |p| p.sample_rate = 384_000, "384000"),
            ("3 channels", |p| p.media[0].channels = 3, "3 channels"),
            ("UNC file name", |p| p.media[0].file_name = r"\\attacker.example\share\a.wav".to_string(), "path separator"),
            ("absolute file name", |p| p.media[0].file_name = r"C:\Windows\win.ini".to_string(), "path separator"),
            ("parent file name", |p| p.media[0].file_name = "..".to_string(), "not a plain file name"),
            ("reserved file name", |p| p.media[0].file_name = "con.wav".to_string(), "reserved Windows device name"),
            (
                "src_out before src_in",
                |p| {
                    p.tracks[0].clips[0].src_in = 900;
                    p.tracks[0].clips[0].src_out = 10;
                },
                "source end before",
            ),
            (
                "clip past 24 hours",
                |p| {
                    p.media[0].length = u64::MAX / 2;
                    p.tracks[0].clips[1].start = 24 * 3600 * 48_000;
                },
                "24 hour limit",
            ),
            ("track volume out of range", |p| p.tracks[0].volume_db = 40.0, "track 1 volume"),
            ("clip gain out of range", |p| p.tracks[0].clips[0].gain_db = -200.0, "clip on track 1"),
            ("bad project name", |p| p.name = "CON".to_string(), "project name"),
        ];
        for (what, break_it, expected) in cases {
            let mut project = full_project();
            break_it(&mut project);
            let text = load_error(&project);
            assert!(text.starts_with("This project file is invalid: "), "{what}: {text}");
            assert!(text.contains(expected), "{what}: {text}");
        }
    }

    #[test]
    fn too_many_tracks_do_not_open() {
        let mut project = full_project();
        project.tracks = (0..1_001).map(|i| crate::model::track::Track::new(format!("T{i}"))).collect();
        assert!(load_error(&project).contains("too many tracks"));
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
        assert!(matches!(load(&paths.root), Err(IoError::UnsupportedVersion { found: 999, max: 1 })));
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
        assert_eq!(fs::read(copied.media().join("conformed").join("song.wav")).unwrap(), b"wav");
        assert_eq!(fs::read(copied.cache().join("peaks.bin")).unwrap(), b"peaks");
        assert!(copied.project_json().is_file());
    }

    #[test]
    fn copy_project_errors_if_target_exists() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        assert!(matches!(copy_project(&paths, parent.path(), "Proj"), Err(IoError::FolderExists(_))));
    }

    #[test]
    fn copy_project_errors_on_invalid_name() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "Proj").unwrap();
        assert!(matches!(copy_project(&paths, parent.path(), "a/b"), Err(IoError::InvalidProjectName(_))));
    }
}
