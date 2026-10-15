//! io::autosave: autosave snapshots and recovery on open.

use std::fs;
use std::path::PathBuf;

use super::IoError;
use super::paths::ProjectPaths;
use super::project_file::{FORMAT_VERSION, ProjectFile};
use crate::model::project::{Project, ViewState};

/// File name of the autosave snapshot inside autosave/.
const AUTOSAVE_NAME: &str = "project.autosave.json";

/// File name an autosave is renamed to when the user opens the last save.
const DISCARDED_NAME: &str = "project.autosave.old.json";

/// Temp file name used for the atomic autosave write.
const TMP_NAME: &str = "project.autosave.json.tmp";

/// Path of autosave/project.autosave.json.
pub fn autosave_path(paths: &ProjectPaths) -> PathBuf {
    paths.autosave().join(AUTOSAVE_NAME)
}

/// Writes the autosave snapshot (same JSON format as project.json) atomically:
/// writes autosave/project.autosave.json.tmp, then renames it. Never touches
/// project.json.
pub fn write(paths: &ProjectPaths, project: &Project, view: &ViewState) -> Result<(), IoError> {
    fs::create_dir_all(paths.autosave())?;
    let file = ProjectFile {
        format_version: FORMAT_VERSION,
        project: project.clone(),
        view: view.clone(),
    };
    let json = serde_json::to_string_pretty(&file)?;
    let tmp = paths.autosave().join(TMP_NAME);
    let target = autosave_path(paths);
    fs::write(&tmp, json)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

/// True when an autosave exists and its modified time is later than
/// project.json's. False when there is no autosave.
pub fn newer_than_saved(paths: &ProjectPaths) -> bool {
    let autosave = autosave_path(paths);
    if !autosave.is_file() {
        return false;
    }
    let autosave_time = fs::metadata(&autosave).and_then(|m| m.modified());
    let saved_time = fs::metadata(paths.project_json()).and_then(|m| m.modified());
    match (autosave_time, saved_time) {
        (Ok(a), Ok(s)) => a > s,
        // No project.json (or unreadable times): the autosave is all there is.
        (Ok(_), Err(_)) => true,
        // Unreadable autosave time: treat as no recoverable autosave.
        (Err(_), _) => false,
    }
}

/// Loads the autosave snapshot, returning the project and view state.
pub fn load(paths: &ProjectPaths) -> Result<(Project, ViewState), IoError> {
    let text = fs::read_to_string(autosave_path(paths))?;
    let file: ProjectFile = serde_json::from_str(&text)?;
    if file.format_version > FORMAT_VERSION {
        return Err(IoError::UnsupportedVersion {
            found: file.format_version,
            max: FORMAT_VERSION,
        });
    }
    Ok((file.project, file.view))
}

/// Renames the autosave to autosave/project.autosave.old.json, replacing any
/// older discarded autosave. Does nothing when there is no autosave.
pub fn discard(paths: &ProjectPaths) -> Result<(), IoError> {
    let autosave = autosave_path(paths);
    if !autosave.is_file() {
        return Ok(());
    }
    let old = paths.autosave().join(DISCARDED_NAME);
    if old.is_file() {
        fs::remove_file(&old)?;
    }
    fs::rename(&autosave, &old)?;
    Ok(())
}

/// Removes the autosave snapshot when present.
pub fn delete(paths: &ProjectPaths) -> Result<(), IoError> {
    let autosave = autosave_path(paths);
    if autosave.is_file() {
        fs::remove_file(&autosave)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::project_file;
    use crate::model::project::BitDepth;
    use tempfile::tempdir;

    /// A tiny project and view for round-trip tests.
    fn sample() -> (Project, ViewState) {
        (
            Project::new("Proj".to_string(), 48_000, BitDepth::Int16),
            ViewState::default(),
        )
    }

    /// Creates a project layout and a saved project.json inside a tempdir.
    fn saved_layout() -> (tempfile::TempDir, ProjectPaths) {
        let parent = tempdir().unwrap();
        let paths = super::super::paths::create_layout(parent.path(), "Proj").unwrap();
        let (project, view) = sample();
        project_file::save(&paths, &project, &view).unwrap();
        (parent, paths)
    }

    #[test]
    fn write_then_load_round_trips() {
        let (_parent, paths) = saved_layout();
        let (project, view) = sample();
        write(&paths, &project, &view).unwrap();
        let (loaded_project, loaded_view) = load(&paths).unwrap();
        assert_eq!(loaded_project, project);
        assert_eq!(loaded_view, view);
    }

    #[test]
    fn write_leaves_no_tmp_file_behind() {
        let (_parent, paths) = saved_layout();
        let (project, view) = sample();
        write(&paths, &project, &view).unwrap();
        assert!(autosave_path(&paths).is_file());
        assert!(!paths.autosave().join(TMP_NAME).exists());
    }

    #[test]
    fn newer_than_saved_is_false_without_autosave() {
        let (_parent, paths) = saved_layout();
        assert!(!newer_than_saved(&paths));
    }

    #[test]
    fn newer_than_saved_is_true_after_a_later_autosave() {
        let (_parent, paths) = saved_layout();
        let (project, view) = sample();
        // Push project.json into the past so the autosave is clearly newer.
        let earlier = filetime::FileTime::from_unix_time(1_000_000, 0);
        filetime::set_file_times(paths.project_json(), earlier, earlier).unwrap();
        write(&paths, &project, &view).unwrap();
        assert!(newer_than_saved(&paths));
    }

    #[test]
    fn delete_removes_the_autosave() {
        let (_parent, paths) = saved_layout();
        let (project, view) = sample();
        write(&paths, &project, &view).unwrap();
        delete(&paths).unwrap();
        assert!(!autosave_path(&paths).exists());
        delete(&paths).unwrap(); // absent: still ok
    }

    #[test]
    fn discard_moves_to_old_and_leaves_no_autosave() {
        let (_parent, paths) = saved_layout();
        let (project, view) = sample();
        write(&paths, &project, &view).unwrap();
        discard(&paths).unwrap();
        assert!(!autosave_path(&paths).exists());
        assert!(paths.autosave().join(DISCARDED_NAME).is_file());
        // A second discard replaces the older discarded file.
        write(&paths, &project, &view).unwrap();
        discard(&paths).unwrap();
        assert!(paths.autosave().join(DISCARDED_NAME).is_file());
        assert!(!autosave_path(&paths).exists());
    }
}
