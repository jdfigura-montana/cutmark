//! Take folders: the naming template, take numbering and creation under the recording root.
//!
//! A take lives in `{root}/{project}/{name}/`, where `{name}` is the naming template expanded.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Datelike, Local, Timelike};

#[cfg(test)]
use crate::config::DEFAULT_NAMING_TEMPLATE;
use crate::config::{Container, DEFAULT_PROJECT, Settings, takes_root};

/// The paths of one take: its folder, its base name and its video file.
pub struct TakePaths {
    /// The folder that holds every file of the take.
    pub folder: PathBuf,
    /// The take's name without an extension, shared by every file of the take.
    pub base_name: String,
    /// The main video file inside the folder.
    pub video_file: PathBuf,
}

/// What the naming template needs besides the time and the take number.
#[derive(Debug, Clone, Copy)]
pub struct Naming<'a> {
    /// The project name, which is also the folder under the root that holds the project's takes.
    pub project: &'a str,
    /// The template of the take folder name and the file base name.
    pub template: &'a str,
    /// The name of the profile in use.
    pub profile: &'a str,
}

/// A token of the naming template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Project,
    Date,
    Time,
    Take,
    Profile,
}

/// The tokens a template may hold, as written.
const TOKENS: [(&str, Token); 5] =
    [("{project}", Token::Project), ("{date}", Token::Date), ("{time}", Token::Time), ("{take}", Token::Take), ("{profile}", Token::Profile)];

/// One piece of a parsed template.
#[derive(Debug, PartialEq, Eq)]
enum Segment {
    /// Text that goes into the name as it is, once the characters Windows forbids are gone.
    Literal(String),
    /// A token that is replaced by its value.
    Token(Token),
}

/// Removes the characters Windows forbids in file names: `< > : " / \ | ? *` and control characters.
fn strip_forbidden(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() && !"<>:\"/\\|?*".contains(*c)).collect()
}

/// Makes `text` fit as a file or folder name: removes the characters Windows forbids and trims trailing dots and spaces.
pub fn sanitize(text: &str) -> String {
    strip_forbidden(text).trim_end_matches(['.', ' ']).to_owned()
}

/// The project folder name: the project name made safe, or the default project when nothing is left.
fn project_dir(project: &str) -> String {
    let name = sanitize(project);
    if name.is_empty() { DEFAULT_PROJECT.to_owned() } else { name }
}

/// Splits a template into literals and tokens; a template without {take} gets `_{take}` appended so takes never share a name.
fn parse(template: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        literal.push_str(&strip_forbidden(&rest[..start]));
        let after = &rest[start..];
        if let Some((text, token)) = TOKENS.iter().find(|(text, _)| after.starts_with(text)) {
            if !literal.is_empty() {
                segments.push(Segment::Literal(std::mem::take(&mut literal)));
            }
            segments.push(Segment::Token(*token));
            rest = &after[text.len()..];
        } else {
            // A brace that opens no known token is plain text.
            literal.push('{');
            rest = &after[1..];
        }
    }
    literal.push_str(&strip_forbidden(rest));
    if !literal.is_empty() {
        segments.push(Segment::Literal(literal));
    }
    if !segments.contains(&Segment::Token(Token::Take)) {
        segments.push(Segment::Literal("_".to_owned()));
        segments.push(Segment::Token(Token::Take));
    }
    segments
}

/// The date as the {date} token writes it: `YYYY-MM-DD`.
fn date_text(now: DateTime<Local>) -> String {
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

/// The time as the {time} token writes it: `HH-MM-SS`.
fn time_text(now: DateTime<Local>) -> String {
    format!("{:02}-{:02}-{:02}", now.hour(), now.minute(), now.second())
}

/// Expands the naming template for take number `take` at `now` into a name that is safe on Windows.
///
/// {take} is at least two digits, and every template has one, so the name is never empty.
pub fn expand(naming: &Naming, now: DateTime<Local>, take: u32) -> String {
    let mut name = String::new();
    for segment in parse(naming.template) {
        match segment {
            Segment::Literal(text) => name.push_str(&text),
            Segment::Token(Token::Project) => name.push_str(&project_dir(naming.project)),
            Segment::Token(Token::Date) => name.push_str(&date_text(now)),
            Segment::Token(Token::Time) => name.push_str(&time_text(now)),
            Segment::Token(Token::Take) => name.push_str(&format!("{take:02}")),
            Segment::Token(Token::Profile) => name.push_str(&sanitize(naming.profile)),
        }
    }
    sanitize(&name)
}

/// The values a folder name must carry to count as an earlier take of the same project and day.
struct Today<'a> {
    /// The project folder name.
    project: &'a str,
    /// The date as the {date} token writes it.
    date: &'a str,
}

/// The rest of `text` after a {time} value, or None when `text` does not start with one.
fn strip_time(text: &str) -> Option<&str> {
    let head = text.as_bytes().get(..8)?;
    let fits = head.iter().enumerate().all(|(i, byte)| if i == 2 || i == 5 { *byte == b'-' } else { byte.is_ascii_digit() });
    if fits { text.get(8..) } else { None }
}

/// Matches `text` against the template and returns the take number it holds, or None when it is not a take of today.
fn match_from(segments: &[Segment], text: &str, today: &Today, take: Option<u32>) -> Option<u32> {
    let Some((first, rest)) = segments.split_first() else {
        return if text.is_empty() { take } else { None };
    };
    match first {
        Segment::Literal(literal) => match_from(rest, text.strip_prefix(literal.as_str())?, today, take),
        Segment::Token(Token::Project) => match_from(rest, text.strip_prefix(today.project)?, today, take),
        Segment::Token(Token::Date) => match_from(rest, text.strip_prefix(today.date)?, today, take),
        Segment::Token(Token::Time) => match_from(rest, strip_time(text)?, today, take),
        Segment::Token(Token::Take) => {
            let digits = text.bytes().take_while(u8::is_ascii_digit).count();
            (1..=digits).rev().find_map(|len| {
                let value = text[..len].parse::<u32>().ok()?;
                match_from(rest, &text[len..], today, take.or(Some(value)))
            })
        }
        Segment::Token(Token::Profile) => {
            let mut ends = std::iter::once(0).chain(text.char_indices().map(|(i, c)| i + c.len_utf8()));
            ends.find_map(|end| match_from(rest, &text[end..], today, take))
        }
    }
}

/// Adds the folder to an I/O error's message.
fn with_path(path: &Path, source: io::Error) -> io::Error {
    io::Error::new(source.kind(), format!("{}: {source}", path.display()))
}

/// The number of the next take: the highest take number among the project's folders for today, plus 1.
///
/// A deleted folder frees its number only when it was the highest. A project folder that does not exist yet starts at 1.
fn next_number(project_folder: &Path, naming: &Naming, now: DateTime<Local>) -> Result<u32, io::Error> {
    let entries = match fs::read_dir(project_folder) {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(1),
        Err(source) => return Err(with_path(project_folder, source)),
    };
    let segments = parse(naming.template);
    let project = project_dir(naming.project);
    let date = date_text(now);
    let today = Today { project: &project, date: &date };
    let mut highest = 0u32;
    for entry in entries {
        let entry = entry.map_err(|source| with_path(project_folder, source))?;
        let is_dir = entry.file_type().map_err(|source| with_path(&entry.path(), source))?.is_dir();
        if !is_dir {
            continue;
        }
        if let Some(number) = match_from(&segments, &entry.file_name().to_string_lossy(), &today, None) {
            highest = highest.max(number);
        }
    }
    Ok(highest.saturating_add(1))
}

/// Works out the next take's paths under `root` for `now` without creating anything.
///
/// The folder is `{root}/{project}/{name}` and the video file is `{name}.{ext}` inside it, with the extension of `container`.
pub fn plan_take(root: &Path, now: DateTime<Local>, container: Container, naming: &Naming) -> Result<TakePaths, io::Error> {
    let project_folder = root.join(project_dir(naming.project));
    let take = next_number(&project_folder, naming, now)?;
    let base_name = expand(naming, now, take);
    let folder = project_folder.join(&base_name);
    let video_file = folder.join(format!("{base_name}.{}", container.extension()));
    Ok(TakePaths { folder, base_name, video_file })
}

/// Picks the next take folder under `root` for `now` by the naming template, creates it and returns its paths.
pub fn next_take_with(root: &Path, now: DateTime<Local>, container: Container, naming: &Naming) -> Result<TakePaths, io::Error> {
    // Held until the folder exists, so a replay being saved and a recording that starts cannot pick the same number.
    static CLAIM: Mutex<()> = Mutex::new(());
    let _claim = CLAIM.lock().unwrap_or_else(PoisonError::into_inner);
    let take = plan_take(root, now, container, naming)?;
    fs::create_dir_all(&take.folder).map_err(|source| with_path(&take.folder, source))?;
    Ok(take)
}

/// Works out the next take's paths for the settings' root folder, project and naming template without creating anything.
///
/// It is the planning half of [`next_take_for`], so a plan always matches the folder a recording creates.
#[cfg(test)]
pub fn plan_take_for(settings: &Settings, now: DateTime<Local>, container: Container) -> Result<TakePaths, io::Error> {
    plan_take(&takes_root(settings), now, container, &naming_of(settings))
}

/// Picks the next take folder for the settings' root folder, project and naming template, creates it and returns its paths.
pub fn next_take_for(settings: &Settings, now: DateTime<Local>, container: Container) -> Result<TakePaths, io::Error> {
    next_take_with(&takes_root(settings), now, container, &naming_of(settings))
}

/// The naming the settings ask for: their project, template and active profile.
fn naming_of(settings: &Settings) -> Naming<'_> {
    Naming { project: &settings.project, template: &settings.naming_template, profile: &settings.active_profile }
}

/// Picks the next take folder with the default project, template and no profile name.
#[cfg(test)]
pub fn next_take(root: &Path, now: DateTime<Local>, container: Container) -> Result<TakePaths, io::Error> {
    next_take_with(root, now, container, &Naming { project: DEFAULT_PROJECT, template: DEFAULT_NAMING_TEMPLATE, profile: "" })
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

    /// The default naming for a project.
    fn default_naming(project: &str) -> Naming<'_> {
        Naming { project, template: DEFAULT_NAMING_TEMPLATE, profile: "Tutorial" }
    }

    #[test]
    fn every_token_is_expanded() -> Result<(), Box<dyn Error>> {
        let naming = Naming { project: "My Show", template: "{project}_{date}_{time}_{profile}_t{take}", profile: "Tutorial AV1 1440p60" };
        let name = expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 3);
        assert_eq!(name, "My Show_2026-11-17_14-30-05_Tutorial AV1 1440p60_t03");
        Ok(())
    }

    #[test]
    fn the_take_number_has_at_least_two_digits() -> Result<(), Box<dyn Error>> {
        let naming = default_naming("P");
        let now = at(2026, 11, 17, 14, 30, 5)?;
        assert!(expand(&naming, now, 7).ends_with("_take07"));
        assert!(expand(&naming, now, 123).ends_with("_take123"));
        Ok(())
    }

    #[test]
    fn a_template_without_take_gets_the_number_appended() -> Result<(), Box<dyn Error>> {
        let naming = Naming { project: "P", template: "{date}", profile: "" };
        assert_eq!(expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 2), "2026-11-17_02");
        Ok(())
    }

    #[test]
    fn unknown_braces_stay_as_text() -> Result<(), Box<dyn Error>> {
        let naming = Naming { project: "P", template: "{nope}_{take}", profile: "" };
        assert_eq!(expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 1), "{nope}_01");
        Ok(())
    }

    #[test]
    fn a_template_with_nothing_usable_leaves_only_the_take_number() -> Result<(), Box<dyn Error>> {
        let naming = Naming { project: "P", template: "", profile: "" };
        assert_eq!(expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 4), "_04");
        let naming = Naming { project: "P", template: "???", profile: "" };
        assert_eq!(expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 4), "_04");
        Ok(())
    }

    #[test]
    fn sanitizing_strips_forbidden_characters_and_trims_the_end() {
        assert_eq!(sanitize("a<b>c:d\"e/f\\g|h?i*j"), "abcdefghij");
        assert_eq!(sanitize("tab\there\nline"), "tabhereline");
        assert_eq!(sanitize("name. . "), "name");
        assert_eq!(sanitize("keep.inner dots"), "keep.inner dots");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize(""), "");
    }

    #[test]
    fn forbidden_characters_in_tokens_and_the_template_are_stripped() -> Result<(), Box<dyn Error>> {
        let naming = Naming { project: "Show: <Part 1>.", template: "{project}/{profile}_{take}", profile: "AV1 | 60?" };
        let name = expand(&naming, at(2026, 11, 17, 14, 30, 5)?, 1);
        assert_eq!(name, "Show Part 1AV1  60_01");
        let root = fresh_dir("sanitized-folder")?;
        let take = next_take_with(&root, at(2026, 11, 17, 14, 30, 5)?, Container::Mkv, &naming)?;
        assert_eq!(take.folder, root.join("Show Part 1").join("Show Part 1AV1  60_01"));
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_project_that_sanitizes_to_nothing_uses_the_default_project() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("empty-project")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let take = next_take_with(&root, now, Container::Mkv, &default_naming(".."))?;
        assert_eq!(take.folder.parent(), Some(root.join(DEFAULT_PROJECT).as_path()));
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn first_take_is_take01_and_creates_the_project_and_take_folders() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("first-take")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let take = next_take_with(&root, now, Container::Mkv, &default_naming("Show"))?;
        assert_eq!(take.base_name, "2026-11-17_14-30-05_take01");
        assert_eq!(take.folder, root.join("Show").join("2026-11-17_14-30-05_take01"));
        assert_eq!(take.video_file, take.folder.join("2026-11-17_14-30-05_take01.mkv"));
        assert!(take.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn the_next_take_the_same_day_is_take02() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("second-take")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let naming = default_naming("Show");
        next_take_with(&root, now, Container::Mkv, &naming)?;
        let second = next_take_with(&root, now, Container::Mkv, &naming)?;
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
        let take = next_take(&root, now, Container::Mkv)?;
        assert!(root.is_dir());
        assert_eq!(take.base_name, "2026-11-17_14-30-05_take01");
        assert_eq!(take.folder, root.join(DEFAULT_PROJECT).join("2026-11-17_14-30-05_take01"));
        assert!(take.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn planning_creates_nothing() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("plan-only")?;
        let now = at(2026, 11, 17, 14, 30, 5)?;
        let take = plan_take(&root, now, Container::Mp4, &default_naming("Show"))?;
        assert_eq!(take.video_file, take.folder.join("2026-11-17_14-30-05_take01.mp4"));
        assert!(!take.folder.exists());
        assert!(!root.join("Show").exists());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn a_new_day_starts_again_at_take01() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("new-day")?;
        let naming = default_naming("Show");
        let now = at(2026, 11, 17, 23, 59, 0)?;
        next_take_with(&root, now, Container::Mkv, &naming)?;
        next_take_with(&root, now, Container::Mkv, &naming)?;
        let next_day = at(2026, 11, 18, 0, 0, 30)?;
        let take = next_take_with(&root, next_day, Container::Mkv, &naming)?;
        assert_eq!(take.base_name, "2026-11-18_00-00-30_take01");
        let again = next_take_with(&root, next_day, Container::Mkv, &naming)?;
        assert_eq!(again.base_name, "2026-11-18_00-00-30_take02");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn numbering_continues_after_the_highest_folder_when_there_are_gaps() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("gaps")?;
        let project = root.join("Show");
        for name in ["2026-11-17_10-00-00_take01", "2026-11-17_11-00-00_take04", "2026-11-17_12-00-00_take05"] {
            fs::create_dir_all(project.join(name))?;
        }
        let now = at(2026, 11, 17, 15, 0, 0)?;
        let naming = default_naming("Show");
        assert_eq!(next_take_with(&root, now, Container::Mkv, &naming)?.base_name, "2026-11-17_15-00-00_take06");
        // Deleting a folder below the highest changes nothing.
        fs::remove_dir_all(project.join("2026-11-17_11-00-00_take04"))?;
        assert_eq!(next_take_with(&root, now, Container::Mkv, &naming)?.base_name, "2026-11-17_15-00-00_take07");
        // Deleting the highest ones frees their numbers: the next is the highest left, plus 1.
        fs::remove_dir_all(project.join("2026-11-17_15-00-00_take07"))?;
        fs::remove_dir_all(project.join("2026-11-17_15-00-00_take06"))?;
        fs::remove_dir_all(project.join("2026-11-17_12-00-00_take05"))?;
        assert_eq!(plan_take(&root, now, Container::Mkv, &naming)?.base_name, "2026-11-17_15-00-00_take02");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn other_days_projects_files_and_odd_names_are_not_counted() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("not-counted")?;
        fs::create_dir_all(root.join("Show").join("2026-11-16_10-00-00_take09"))?;
        fs::create_dir_all(root.join("Show").join("notes"))?;
        fs::create_dir_all(root.join("Show").join("2026-11-17_10-00-00_take"))?;
        fs::write(root.join("Show").join("2026-11-17_10-00-00_take08"), b"a file, not a folder")?;
        fs::create_dir_all(root.join("Other").join("2026-11-17_10-00-00_take07"))?;
        let now = at(2026, 11, 17, 15, 0, 0)?;
        assert_eq!(plan_take(&root, now, Container::Mkv, &default_naming("Show"))?.base_name, "2026-11-17_15-00-00_take01");
        assert_eq!(plan_take(&root, now, Container::Mkv, &default_naming("Other"))?.base_name, "2026-11-17_15-00-00_take08");
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn the_settings_pick_the_root_project_and_template() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("for-settings")?;
        let now = at(2026, 11, 17, 15, 0, 0)?;
        let mut settings = Settings { root_folder: Some(root.clone()), ..Settings::default() };
        let first = plan_take_for(&settings, now, Container::Mkv)?;
        assert_eq!(first.folder, root.join("Cutmark").join("2026-11-17_15-00-00_take01"));
        settings.project = "Test4a".to_owned();
        settings.naming_template = "{project}_{take}".to_owned();
        let second = next_take_for(&settings, now, Container::Mp4)?;
        assert_eq!(second.folder, root.join("Test4a").join("Test4a_01"));
        assert_eq!(second.video_file, second.folder.join("Test4a_01.mp4"));
        assert!(second.folder.is_dir());
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn numbering_follows_a_custom_template() -> Result<(), Box<dyn Error>> {
        let root = fresh_dir("custom-template")?;
        let naming = Naming { project: "Show", template: "{project} {profile} #{take}", profile: "AV1" };
        let now = at(2026, 11, 17, 15, 0, 0)?;
        fs::create_dir_all(root.join("Show").join("Show AV1 #03"))?;
        fs::create_dir_all(root.join("Show").join("Show Other profile #11"))?;
        assert_eq!(next_take_with(&root, now, Container::Mkv, &naming)?.base_name, "Show AV1 #12");
        fs::remove_dir_all(&root)?;
        Ok(())
    }
}
