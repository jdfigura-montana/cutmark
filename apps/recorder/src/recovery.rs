//! Marks recovery on launch: rebuilds the marks of takes that crashed and left a journal behind.

use std::fs;
use std::path::{Path, PathBuf};

use ::marks::export::{ExportOptions, markdown_with};
use ::marks::{SessionWriter, csv, load};

/// What a recovery scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryReport {
    /// The base names of the takes whose marks were recovered.
    pub recovered: Vec<String>,
    /// What went wrong, one message per failure; a failure on one take never stops the others.
    pub errors: Vec<String>,
}

/// Recovers the marks of every crashed take in `root` and its direct subfolders.
///
/// `is_recording` is asked for each journal with its folder and base name; a journal it accepts belongs to the running take and is left alone.
pub fn scan(root: &Path, is_recording: &dyn Fn(&Path, &str) -> bool) -> RecoveryReport {
    let mut report = RecoveryReport::default();
    let mut folders = vec![root.to_path_buf()];
    match fs::read_dir(root) {
        Ok(entries) => folders.extend(entries.flatten().map(|entry| entry.path()).filter(|path| path.is_dir())),
        Err(err) => {
            // A takes folder that does not exist yet has nothing to recover.
            if root.exists() {
                report.errors.push(format!("could not list {}: {err}", root.display()));
            }
            return report;
        }
    }
    for folder in folders {
        let journals = match ::marks::write::find_journals(&folder) {
            Ok(journals) => journals,
            Err(err) => {
                report.errors.push(format!("could not look for marks journals in {}: {err}", folder.display()));
                continue;
            }
        };
        for base_name in journals {
            if is_recording(&folder, &base_name) {
                continue;
            }
            match recover_take(&folder, &base_name, &mut report.errors) {
                Ok(true) => report.recovered.push(base_name),
                Ok(false) => {}
                Err(err) => report.errors.push(format!("could not recover the marks of {base_name}: {err}")),
            }
        }
    }
    report
}

/// Recovers one take and rewrites its Markdown and CSV logs; false when there was no journal after all.
///
/// A log that cannot be written is added to `errors`; the recovered sidecar still counts.
fn recover_take(folder: &Path, base_name: &str, errors: &mut Vec<String>) -> Result<bool, String> {
    let Some(path) = SessionWriter::recover(folder, base_name).map_err(|err| err.to_string())? else { return Ok(false) };
    let (sidecar, _warnings) = load(&path).map_err(|err| format!("the recovered marks file could not be read: {err}"))?;
    let markdown = markdown_with(&sidecar, &ExportOptions { include_system: true });
    if let Err(err) = write_atomic(&folder.join(format!("{base_name}_marks.md")), markdown.as_bytes()) {
        errors.push(format!("the Markdown marks log of {base_name} could not be written: {err}"));
    }
    if let Err(err) = write_atomic(&folder.join(format!("{base_name}_marks.csv")), &csv(&sidecar)) {
        errors.push(format!("the CSV marks log of {base_name} could not be written: {err}"));
    }
    Ok(true)
}

/// Writes a file through a temp file in the same folder, then renames it over the target.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    fs::write(&temp, bytes)?;
    fs::rename(&temp, path).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use ::marks::{MarkType, NewMark, Source, SystemClock, WriterInfo};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("cutmark_recovery_{name}_{}_{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    /// Starts a session with one mark and drops it without finishing, as a crash would.
    fn crashed_take(dir: &Path, base_name: &str) {
        let info = WriterInfo { app: "test".to_owned(), version: "0".to_owned() };
        let mut writer = SessionWriter::begin(dir, base_name, info, &dir.join(format!("{base_name}.mkv")), Box::new(SystemClock)).expect("writer");
        let mut mark = NewMark::point(MarkType::Note, 1500, Source::Hotkey);
        mark.label = Some("before the crash".to_owned());
        writer.add_mark(mark).expect("mark");
    }

    #[test]
    fn a_crashed_take_is_recovered_with_its_logs() {
        let root = scratch("crashed");
        let sub = root.join("project");
        fs::create_dir_all(&sub).expect("subfolder");
        crashed_take(&root, "top");
        crashed_take(&sub, "nested");
        let report = scan(&root, &|_, _| false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let mut recovered = report.recovered.clone();
        recovered.sort();
        assert_eq!(recovered, vec!["nested".to_owned(), "top".to_owned()]);
        assert!(root.join("top.marks.json").exists());
        assert!(!root.join("top.marks.journal").exists());
        let md = fs::read_to_string(sub.join("nested_marks.md")).expect("markdown log");
        assert!(md.contains("before the crash"), "{md}");
        let csv = fs::read_to_string(sub.join("nested_marks.csv")).expect("csv log");
        assert!(csv.contains("before the crash"), "{csv}");
        // A second scan finds nothing left to do.
        assert_eq!(scan(&root, &|_, _| false), RecoveryReport::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_running_take_is_skipped() {
        let root = scratch("running");
        crashed_take(&root, "live");
        crashed_take(&root, "old");
        let report = scan(&root, &|_, base| base == "live");
        assert_eq!(report.recovered, vec!["old".to_owned()]);
        assert!(root.join("live.marks.journal").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        let root = scratch("missing").join("nothing_here");
        assert_eq!(scan(&root, &|_, _| false), RecoveryReport::default());
    }
}
