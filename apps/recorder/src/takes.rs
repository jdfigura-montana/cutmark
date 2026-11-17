//! Take folders: names, numbering and creation under the recording root.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, Timelike};

/// The paths of one take: its folder, its base name and its video file.
pub struct TakePaths {
    /// The folder that holds every file of the take.
    pub folder: PathBuf,
    /// The take's name without an extension, shared by every file of the take.
    pub base_name: String,
    /// The main video file inside the folder.
    pub video_file: PathBuf,
}

/// Picks the next take folder under `root` for `now`, creates it and returns its paths.
///
/// The folder is `{root}/{YYYY-MM-DD}_{HH-MM-SS}_take{nn}`, where `nn` is at two digits and counts the
/// take folders already under `root` for that date, plus 1. The video file is `{same name}.mkv` inside
/// it. The full naming template arrives in phase 4.
pub fn next_take(root: &Path, now: DateTime<Local>) -> Result<TakePaths, io::Error> {
    fs::create_dir_all(root).map_err(|source| io::Error::new(source.kind(), format!("{}: {source}", root.display())))?;
    let date = format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day());
    let time = format!("{:02}-{:02}-{:02}", now.hour(), now.minute(), now.second());
    let count = count_take_folders(root, &date)?;
    let base_name = format!("{date}_{time}_take{:02}", count + 1);
    let folder = root.join(&base_name);
    fs::create_dir_all(&folder).map_err(|source| io::Error::new(source.kind(), format!("{}: {source}", folder.display())))?;
    let video_file = folder.join(format!("{base_name}.mkv"));
    Ok(TakePaths { folder, base_name, video_file })
}

/// Counts the take folders under `root` that belong to `date`.
fn count_take_folders(root: &Path, date: &str) -> Result<u32, io::Error> {
    let mut count = 0u32;
    for entry in fs::read_dir(root).map_err(|source| io::Error::new(source.kind(), format!("{}: {source}", root.display())))? {
        let entry = entry.map_err(|source| io::Error::new(source.kind(), format!("{}: {source}", root.display())))?;
        let entry_path = entry.path();
        let is_dir = entry.file_type().map_err(|source| io::Error::new(source.kind(), format!("{}: {source}", entry_path.display())))?.is_dir();
        if is_dir && is_take_folder(&entry.file_name().to_string_lossy(), date) {
            count += 1;
        }
    }
    Ok(count)
}

/// Whether `name` looks like a take folder of `date`: `{date}_{time}_take{nn}` with digits only.
fn is_take_folder(name: &str, date: &str) -> bool {
    let Some(rest) = name.strip_prefix(date).and_then(|rest| rest.strip_prefix('_')) else {
        return false;
    };
    match rest.split_once("_take") {
        Some((_, nn)) => !nn.is_empty() && nn.bytes().all(|byte| byte.is_ascii_digit()),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use chrono::TimeZone;

    use super::*;

    /// A fresh, empty folder under the system temp folder.
    fn fresh_dir(name: &str) -> Result<PathBuf, io::Error> {
        let dir = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-{name}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// A fixed local date and time, so the tests do not depend on the wall clock.
    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> Result<DateTime<Local>, Box<dyn Error>> {
        Ok(Local
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{year}-{month}-{day} {hour}:{minute}:{second} does not exist")))?)
    }

    #[test]
    fn first_take_is_take01_and_creates_the_folder() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("first-take")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let take = next_take(&root, now)?;
        assert_eq!(take.base_name, "2026-11-17_14-30-05_take01");
        assert_eq!(take.folder, root.join("2026-11-17_14-30-05_take01"));
        assert_eq!(take.video_file, take.folder.join("2026-11-17_14-30-05_take01.mkv"));
        assert!(take.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn the_next_take_the_same_day_is_take02() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("second-take")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        next_take(&root, now)?;
        let second = next_take(&root, now)?;
        assert_eq!(second.base_name, "2026-11-17_14-30-05_take02");
        assert!(second.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_missing_root_is_created_and_the_first_take_is_take01() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("missing-root")?;
        fs::remove_dir_all(&root)?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let take = next_take(&root, now)?;
        assert!(root.is_dir());
        assert_eq!(take.base_name, "2026-11-17_14-30-05_take01");
        assert!(take.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_new_day_starts_again_at_take01() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("new-day")?;
        let now = at(2026, 11, 17, 23, 59, 0)?;
        next_take(&root, now)?;
        let next_day = at(2026, 11, 18, 0, 0, 30)?;
        let take = next_take(&root, next_day)?;
        assert_eq!(take.base_name, "2026-11-18_00-00-30_take01");
        fs::remove_dir_all(&root)?;
        Ok(())
    }
}
