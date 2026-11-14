//! io::paths: project folder layout (media/, cache/, autosave/, exports/).

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Local};
use uuid::Uuid;

use super::IoError;
use crate::model::validate::{check_media_file_name, is_reserved_device_name};

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

    /// Path of the media file `file_name` inside media/. The only way to turn
    /// a name stored in a project into a path: an unsafe name (separators,
    /// drive or UNC prefix, `..`, a device name ...) is `IoError::UnsafePath`
    /// and is never joined, so the result is always directly inside media/.
    pub fn media_file(&self, file_name: &str) -> Result<PathBuf, IoError> {
        check_media_file_name(file_name).map_err(|error| IoError::UnsafePath(error.to_string()))?;
        Ok(self.media().join(file_name))
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

/// Validates a project name: not empty, not whitespace-only, no forbidden or
/// control characters, no trailing dot or space (Windows drops them, so the
/// folder would not match the name) and not a reserved device name (CON, NUL,
/// COM1 ...).
pub fn validate_project_name(name: &str) -> Result<(), IoError> {
    let bad = name.trim().is_empty()
        || name
            .chars()
            .any(|c| FORBIDDEN_NAME_CHARS.contains(&c) || c.is_control())
        || name.ends_with(['.', ' '])
        || is_reserved_device_name(name);
    if bad {
        return Err(IoError::InvalidProjectName(name.to_string()));
    }
    Ok(())
}

/// Longest stem `safe_file_stem` returns, in characters.
const MAX_STEM_CHARS: usize = 120;

/// A safe file-name stem made from untrusted text such as a project name:
/// only what follows the last `\`, `/` or `:`, with forbidden and control
/// characters removed and trailing dots and spaces trimmed. Falls back to
/// "export" when nothing usable is left or the result is a reserved device
/// name, so `<stem>.wav` always names a plain file in the folder it is joined to.
pub fn safe_file_stem(name: &str) -> String {
    let last = name.rsplit(['\\', '/', ':']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !FORBIDDEN_NAME_CHARS.contains(c) && !c.is_control())
        .take(MAX_STEM_CHARS)
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']);
    if cleaned.trim().is_empty() || is_reserved_device_name(cleaned) {
        return "export".to_string();
    }
    cleaned.to_string()
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

/// File name for a new take in media/: "rec-YYYYMMDD-HHMMSS.wav", with the
/// same " (2)"-style clash suffix as [`unique_media_name`] when taken.
// The take job names the WAV with it (12.2).
#[allow(dead_code)]
pub fn record_file_name(media_dir: &Path, now: DateTime<Local>) -> String {
    unique_media_name(
        media_dir,
        &format!("rec-{}.wav", now.format("%Y%m%d-%H%M%S")),
    )
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

/// The `/select,"<path>"` argument for Explorer. Windows file names cannot
/// contain `"`, so a path with one is refused: it would close the quotes early
/// and let the rest of the text become extra Explorer arguments.
fn reveal_arg(path: &Path) -> Result<String, IoError> {
    let text = path.display().to_string();
    if text.contains('"') {
        return Err(IoError::UnsafePath(
            "the path contains a quote character".to_string(),
        ));
    }
    Ok(format!("/select,\"{text}\""))
}

/// Absolute path of explorer.exe: `%SystemRoot%\explorer.exe`, or
/// `C:\Windows\explorer.exe` when SystemRoot is unset or empty. Never a bare
/// name, so no search order (application folder, PATH) decides what runs.
fn explorer_exe(system_root: Option<OsString>) -> PathBuf {
    let root = system_root
        .filter(|root| !root.is_empty())
        .unwrap_or_else(|| OsString::from("C:\\Windows"));
    PathBuf::from(root).join("explorer.exe")
}

/// Opens Windows Explorer with `path` pre-selected. Paths with spaces and
/// commas are passed safely via a raw argument; a path containing `"` is
/// refused with an error. A failure to start Explorer itself is ignored.
pub fn reveal_in_explorer(path: &Path) -> Result<(), IoError> {
    use std::os::windows::process::CommandExt;
    let arg = reveal_arg(path)?;
    let exe = explorer_exe(std::env::var_os("SystemRoot"));
    let _ = Command::new(exe).raw_arg(arg).spawn();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    #[test]
    fn record_file_name_has_the_spec_format() {
        let dir = tempdir().unwrap();
        let now = Local.with_ymd_and_hms(2026, 11, 14, 9, 5, 3).unwrap();
        assert_eq!(record_file_name(dir.path(), now), "rec-20261114-090503.wav");
    }

    #[test]
    fn record_file_name_suffixes_a_clash() {
        let dir = tempdir().unwrap();
        let now = Local.with_ymd_and_hms(2026, 11, 14, 9, 5, 3).unwrap();
        fs::write(dir.path().join("rec-20261114-090503.wav"), b"x").unwrap();
        assert_eq!(
            record_file_name(dir.path(), now),
            "rec-20261114-090503 (2).wav"
        );
    }

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
        for name in [
            "My Project",
            "CONSOLE",
            "Comet",
            "a.b",
            "Song 2",
            "r\u{e9}sum\u{e9}",
        ] {
            assert!(validate_project_name(name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn validate_project_name_rejects_reserved_names_trailing_dots_and_control_chars() {
        for bad in [
            "CON",
            "nul",
            "Com1",
            "LPT9",
            "aux.txt",
            "prn ", // reserved device names
            "name.",
            "name ",
            "name..",
            "name. ", // Windows drops trailing dots and spaces
            "a\u{7}b",
            "a\nb",
            "tab\tname",
            "\u{0}", // control characters
            "a<b",
            "a|b",
            "a?b",
            "a*b",
            "a\"b", // the old forbidden set still applies
        ] {
            assert!(
                matches!(
                    validate_project_name(bad),
                    Err(IoError::InvalidProjectName(_))
                ),
                "expected rejection of {bad:?}"
            );
        }
    }

    #[test]
    fn create_layout_rejects_a_reserved_name() {
        let parent = tempdir().unwrap();
        assert!(matches!(
            create_layout(parent.path(), "NUL"),
            Err(IoError::InvalidProjectName(_))
        ));
        assert_eq!(fs::read_dir(parent.path()).unwrap().count(), 0);
    }

    #[test]
    fn media_file_joins_only_plain_names() {
        let dir = tempdir().unwrap();
        let paths = ProjectPaths {
            root: dir.path().to_path_buf(),
        };
        assert_eq!(
            paths.media_file("boom.wav").unwrap(),
            dir.path().join("media").join("boom.wav")
        );
        for bad in [
            r"\\attacker.example\share\a.wav",
            r"C:\Windows\win.ini",
            r"\Windows\win.ini",
            r"..\..\a.wav",
            "a\"b.wav",
            "nul.wav",
            "",
        ] {
            assert!(
                matches!(paths.media_file(bad), Err(IoError::UnsafePath(_))),
                "{bad:?} must not be joined"
            );
        }
    }

    #[test]
    fn safe_file_stem_keeps_only_a_plain_file_name() {
        assert_eq!(safe_file_stem("My Project"), "My Project");
        assert_eq!(safe_file_stem("a.b"), "a.b");
        // Directories and drive prefixes are dropped, only the last part stays.
        assert_eq!(safe_file_stem(r"..\..\Documents\x"), "x");
        assert_eq!(safe_file_stem("../../etc/x"), "x");
        assert_eq!(safe_file_stem(r"C:\Users\me\song"), "song");
        assert_eq!(safe_file_stem(r"\\server\share\song"), "song");
        assert_eq!(safe_file_stem("C:song"), "song");
        // Forbidden and control characters are removed, trailing dots and spaces trimmed.
        assert_eq!(safe_file_stem("a<b>c|d?e*f\"g"), "abcdefg");
        assert_eq!(safe_file_stem("a\u{7}b\nc"), "abc");
        assert_eq!(safe_file_stem("name. . "), "name");
        // Nothing usable, or a device name, falls back.
        for name in [
            "", "   ", ".", "..", r"..\..", r"a\", "CON", "nul", "com1 ", "<>",
        ] {
            assert_eq!(safe_file_stem(name), "export", "{name:?}");
        }
        // The result is bounded.
        assert!(safe_file_stem(&"x".repeat(10_000)).chars().count() <= 120);
    }

    #[test]
    fn reveal_arg_refuses_quotes_and_quotes_the_path() {
        let ok = reveal_arg(Path::new(r"C:\My Projects\a, b\media\boom.wav")).unwrap();
        assert_eq!(ok, "/select,\"C:\\My Projects\\a, b\\media\\boom.wav\"");
        // The injection from the audit: a quote that closes the argument early.
        let injected = Path::new("C:\\p\\media\\boom.wav\" \"C:\\Windows\\System32\\notepad.exe");
        assert!(matches!(reveal_arg(injected), Err(IoError::UnsafePath(_))));
        assert!(matches!(
            reveal_in_explorer(injected),
            Err(IoError::UnsafePath(_))
        ));
    }

    #[test]
    fn explorer_is_started_by_absolute_path() {
        assert_eq!(
            explorer_exe(Some(OsString::from(r"D:\Win"))),
            PathBuf::from(r"D:\Win\explorer.exe")
        );
        // SystemRoot unset or empty falls back to C:\Windows.
        for missing in [None, Some(OsString::new())] {
            assert_eq!(
                explorer_exe(missing),
                PathBuf::from(r"C:\Windows\explorer.exe")
            );
        }
        assert!(explorer_exe(std::env::var_os("SystemRoot")).is_absolute());
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
