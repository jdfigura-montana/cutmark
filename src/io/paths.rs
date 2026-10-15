//! io::paths: project folder layout (media/, cache/, autosave/, exports/).

use std::collections::HashSet;
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

/// Removes media and cache files that nothing in the project or its history references.
///
/// `keep_files` holds the media/ file names that must stay, `keep_ids` the media
/// ids whose `<id>.f32.wav` and `<id>.peaks` cache files must stay. Unreferenced
/// media/ files go to the Recycle Bin (or are deleted directly when `recycle` is
/// false, used by tests so they never touch the real bin), unreferenced cache
/// files are deleted. `.part` files, non-regular files and cache names that do
/// not match the two patterns are left alone, and autosave/ and exports/ are
/// never touched. Returns one message per removed file and, for individual
/// failures, an error message; failures never make the caller's Save fail.
pub fn cleanup_unused(
    paths: &ProjectPaths,
    keep_ids: &HashSet<Uuid>,
    keep_files: &HashSet<String>,
    recycle: bool,
) -> Vec<String> {
    let mut messages = Vec::new();
    cleanup_media(paths, keep_files, recycle, &mut messages);
    cleanup_cache(paths, keep_ids, &mut messages);
    messages
}

/// Removes unreferenced regular files from media/, skipping *.part names.
fn cleanup_media(
    paths: &ProjectPaths,
    keep_files: &HashSet<String>,
    recycle: bool,
    messages: &mut Vec<String>,
) {
    let Ok(entries) = fs::read_dir(paths.media()) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".part") || keep_files.contains(&name) {
            continue;
        }
        remove_unused(&entry.path(), &name, recycle, messages);
    }
}

/// Deletes unreferenced `<id>.f32.wav` and `<id>.peaks` files from cache/.
/// Names that do not match either pattern are left alone.
fn cleanup_cache(paths: &ProjectPaths, keep_ids: &HashSet<Uuid>, messages: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(paths.cache()) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = cache_id(&name) else {
            continue;
        };
        if keep_ids.contains(&id) {
            continue;
        }
        remove_unused(&entry.path(), &name, false, messages);
    }
}

/// The media id of a cache name `<uuid>.f32.wav` or `<uuid>.peaks`, if it matches.
fn cache_id(name: &str) -> Option<Uuid> {
    let stem = if let Some(stem) = name.strip_suffix(".f32.wav") {
        stem
    } else {
        name.strip_suffix(".peaks")?
    };
    Uuid::parse_str(stem).ok()
}

/// Removes one unused file, collecting the name or a failure message.
fn remove_unused(path: &Path, name: &str, recycle: bool, messages: &mut Vec<String>) {
    let result = if recycle {
        trash::delete(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    };
    match result {
        Ok(()) => messages.push(format!("Removed unused file: {name}")),
        Err(error) => messages.push(format!("Cleanup could not remove {name}: {error}")),
    }
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

    /// Writes `content` to `dir/name` and asserts nothing else.
    fn put(dir: &Path, name: &str, content: &[u8]) {
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn cleanup_unused_removes_only_unreferenced_files() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "C1").unwrap();
        let keep_id = Uuid::new_v4();
        let gone_id = Uuid::new_v4();
        let mut keep_files = HashSet::new();
        keep_files.insert("kept.wav".to_string());
        let mut keep_ids = HashSet::new();
        keep_ids.insert(keep_id);

        // media/: kept, unreferenced, a .part file and a directory.
        put(&paths.media(), "kept.wav", b"x");
        put(&paths.media(), "gone.wav", b"x");
        put(&paths.media(), "gone.wav.part", b"x");
        fs::create_dir(paths.media().join("subdir")).unwrap();

        // cache/: kept pair, unreferenced pair, stray notes.txt and junk.bin.
        put(&paths.cache(), &format!("{keep_id}.f32.wav"), b"x");
        put(&paths.cache(), &format!("{keep_id}.peaks"), b"x");
        put(&paths.cache(), &format!("{gone_id}.f32.wav"), b"x");
        put(&paths.cache(), &format!("{gone_id}.peaks"), b"x");
        put(&paths.cache(), "notes.txt", b"x");
        put(&paths.cache(), "junk.bin", b"x");

        let messages = cleanup_unused(&paths, &keep_ids, &keep_files, false);

        // Unreferenced media and cache files are gone; everything else stays.
        assert!(paths.media().join("kept.wav").is_file());
        assert!(!paths.media().join("gone.wav").exists());
        assert!(paths.media().join("gone.wav.part").is_file());
        assert!(paths.media().join("subdir").is_dir());
        assert!(paths.cache().join(format!("{keep_id}.f32.wav")).is_file());
        assert!(paths.cache().join(format!("{keep_id}.peaks")).is_file());
        assert!(!paths.cache().join(format!("{gone_id}.f32.wav")).exists());
        assert!(!paths.cache().join(format!("{gone_id}.peaks")).exists());
        assert!(paths.cache().join("notes.txt").is_file());
        assert!(paths.cache().join("junk.bin").is_file());

        // One message per removed file, none for files left alone.
        let joined = messages.join("\n");
        assert!(joined.contains("gone.wav"), "got: {joined}");
        assert!(joined.contains(&gone_id.to_string()), "got: {joined}");
        assert_eq!(messages.len(), 3);
    }

    #[test]
    fn cleanup_unused_collects_errors_without_failing() {
        let parent = tempdir().unwrap();
        let paths = create_layout(parent.path(), "C2").unwrap();
        put(&paths.media(), "locked.wav", b"x");
        // Holding the file open without share-delete makes removal fail.
        use std::os::windows::fs::OpenOptionsExt;
        let _handle = fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2) // read + write, no share-delete
            .open(paths.media().join("locked.wav"))
            .unwrap();

        let messages = cleanup_unused(&paths, &HashSet::new(), &HashSet::new(), false);

        // The failure is reported as a message, not an error of the call.
        assert!(paths.media().join("locked.wav").is_file());
        let joined = messages.join("\n");
        assert!(
            joined.contains("Cleanup could not remove locked.wav"),
            "got: {joined}"
        );
    }
}
