//! Profile files: one JSON file per profile in the Recorder's profiles folder, with the operations the window needs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use thiserror::Error;

use media_io::encode::{Codec as EncoderCodec, aligned_1080_value, probe_amf};

use crate::config::{ConfigError, Container, Profile, write_json};

/// How long an edit waits for further edits before the profile is written to its file.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// The name given to a profile that is migrated out of an older settings file.
const MIGRATED_NAME: &str = "Default";

/// The name a new profile starts with.
const NEW_NAME: &str = "New profile";

/// Why a profile operation was refused or failed.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// A profile file could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// Another profile already has this name.
    #[error("a profile named \"{0}\" already exists")]
    Exists(String),
    /// No profile has this name.
    #[error("there is no profile named \"{0}\"")]
    Missing(String),
    /// The last profile was asked to be deleted.
    #[error("the last profile cannot be deleted")]
    Last,
    /// The name has no letter or digit to build a file name from.
    #[error("a profile name needs at least one letter or digit")]
    BadName,
}

/// The profiles that could be read from a folder, and a warning for each file that was skipped.
#[derive(Debug, Default)]
pub struct Loaded {
    /// The valid profiles, sorted by name.
    pub profiles: Vec<Profile>,
    /// One line per skipped file, for the warning banner.
    pub warnings: Vec<String>,
}

/// The Recorder's profiles folder in the user's config folder, or None when that folder cannot be found.
pub fn profiles_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("Cutmark").join("recorder").join("profiles"))
}

/// The file name of a profile: its name with characters Windows does not allow replaced, plus .json.
pub fn file_name(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    format!("{}.json", stem.trim_end_matches(['.', ' ']))
}

/// Whether two names map to the same file, which Windows compares without regard to case.
fn same_file(a: &str, b: &str) -> bool {
    file_name(a).to_lowercase() == file_name(b).to_lowercase()
}

/// Whether a name can be turned into a file name.
fn valid_name(name: &str) -> bool {
    name.chars().any(char::is_alphanumeric)
}

/// Reads one profile file.
fn read_profile(path: &Path) -> Result<Profile, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let profile: Profile = serde_json::from_str(text.strip_prefix('\u{feff}').unwrap_or(&text)).map_err(|err| err.to_string())?;
    if valid_name(&profile.name) { Ok(profile) } else { Err("the profile has no usable name".to_owned()) }
}

/// Reads every profile file in the folder; a file that is broken or repeats a name is skipped with a warning.
pub fn load_all(dir: &Path) -> Loaded {
    let mut loaded = Loaded::default();
    let Ok(entries) = fs::read_dir(dir) else { return loaded };
    let mut paths: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json"))).collect();
    paths.sort();
    for path in paths {
        let shown = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        match read_profile(&path) {
            Ok(profile) if loaded.profiles.iter().any(|known| same_file(&known.name, &profile.name)) => {
                log::warn!("profile file {shown} repeats the name {}, skipped", profile.name);
                loaded.warnings.push(format!("Skipped profile file {shown}: the name \"{}\" is already used", profile.name));
            }
            Ok(profile) => loaded.profiles.push(profile),
            Err(reason) => {
                log::warn!("profile file {shown} could not be read, skipped: {reason}");
                loaded.warnings.push(format!("Skipped profile file {shown}: {reason}"));
            }
        }
    }
    loaded.profiles.sort_by_key(|profile| profile.name.to_lowercase());
    loaded
}

/// Finds the active profile by name in the user's profiles folder.
pub fn find(name: &str) -> Option<Profile> {
    let dir = profiles_dir()?;
    load_all(&dir).profiles.into_iter().find(|profile| profile.name == name)
}

/// Writes the profile to its file, through a temp file in the same folder.
pub fn save(dir: &Path, profile: &Profile) -> Result<(), ConfigError> {
    fs::create_dir_all(dir).map_err(|source| ConfigError::Io { path: dir.to_path_buf(), source })?;
    write_json(&dir.join(file_name(&profile.name)), profile)
}

/// The profiles in memory: the one place the window edits them, and the one debounced save that writes them.
///
/// The Profile section and the Webcam section both edit the profiles here, never copies of their own.
#[derive(Debug, Default)]
pub struct ProfileStore {
    /// The profiles folder, or None when the config folder could not be found.
    dir: Option<PathBuf>,
    /// The valid profiles, sorted by name.
    profiles: Vec<Profile>,
    /// A line per profile file that was skipped.
    warnings: Vec<String>,
    /// When and for which profile the last edit happened, until it is written.
    dirty: Option<(Instant, String)>,
    /// The text of the last failed save, until a save works.
    save_error: Option<String>,
}

impl ProfileStore {
    /// Reads the profiles folder of the user's config folder.
    pub fn load() -> Self {
        Self::open(profiles_dir())
    }

    /// Reads the profiles of a folder; with none readable, the shipped profiles are used in memory.
    pub fn open(dir: Option<PathBuf>) -> Self {
        let mut store = Self { dir, ..Self::default() };
        store.reload();
        store
    }

    /// Reads the profile files again, dropping edits that were not written; call [`flush`](Self::flush) first to keep them.
    pub fn reload(&mut self) {
        if let Some(dir) = &self.dir {
            let loaded = load_all(dir);
            self.profiles = loaded.profiles;
            self.warnings = loaded.warnings;
        }
        if self.profiles.is_empty() {
            self.profiles = Profile::shipped().map(|profile| with_default_container(profile, &|| aligned_1080_value().is_some())).to_vec();
        }
        self.dirty = None;
    }

    /// The profiles folder, if it could be found.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The profiles, sorted by name.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// One line per profile file that was skipped.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The text of the last failed save, if the latest save failed.
    pub fn save_error(&self) -> Option<&str> {
        self.save_error.as_deref()
    }

    /// The profile with this name.
    pub fn get(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    /// Replaces the profile of the same name with an edited one and schedules the save; false when there is no such profile.
    pub fn update(&mut self, profile: Profile) -> bool {
        let name = profile.name.clone();
        let Some(slot) = self.profiles.iter_mut().find(|known| known.name == name) else { return false };
        *slot = profile;
        self.mark_dirty(name);
        true
    }

    /// Changes the named profile in place and schedules the save; None when there is no such profile.
    pub fn edit<R>(&mut self, name: &str, change: impl FnOnce(&mut Profile) -> R) -> Option<R> {
        let result = change(self.profiles.iter_mut().find(|profile| profile.name == name)?);
        self.mark_dirty(name.to_owned());
        Some(result)
    }

    /// Notes that a profile changed; an edit of another profile than the pending one writes the pending one first.
    fn mark_dirty(&mut self, name: String) {
        if self.dirty.as_ref().is_some_and(|(_, pending)| *pending != name) {
            self.flush();
        }
        self.dirty = Some((Instant::now(), name));
    }

    /// Writes the pending edit now.
    pub fn flush(&mut self) {
        let Some((_, name)) = self.dirty.take() else { return };
        let (Some(dir), Some(profile)) = (&self.dir, self.profiles.iter().find(|profile| profile.name == name)) else { return };
        match save(dir, profile) {
            Ok(()) => self.save_error = None,
            Err(err) => {
                log::error!("could not save profile {name}: {err}");
                self.save_error = Some(format!("Could not save the profile: {err}"));
            }
        }
    }

    /// Writes the pending edit once it has waited long enough, or asks for a repaint when it is due.
    pub fn save_when_due(&mut self, ctx: &egui::Context) {
        let Some((at, _)) = &self.dirty else { return };
        let waited = at.elapsed();
        if waited >= SAVE_DEBOUNCE {
            self.flush();
        } else {
            ctx.request_repaint_after(SAVE_DEBOUNCE - waited);
        }
    }
}

/// A name based on `base` that no profile in `taken` uses, adding " 2", " 3" and so on.
fn unique_name(base: &str, taken: &[Profile]) -> String {
    let free = |name: &str| !taken.iter().any(|profile| same_file(&profile.name, name));
    if free(base) {
        return base.to_owned();
    }
    (2..).map(|n| format!("{base} {n}")).find(|name| free(name)).unwrap_or_else(|| base.to_owned())
}

/// Saves a new profile with the shipped defaults under a name that is not taken, and returns it.
pub fn create(dir: &Path) -> Result<Profile, ProfileError> {
    let taken = load_all(dir).profiles;
    let base = with_default_container(Profile::tutorial_av1_1440p60(), &|| aligned_1080_value().is_some());
    let profile = Profile { name: unique_name(NEW_NAME, &taken), ..base };
    save(dir, &profile)?;
    Ok(profile)
}

/// Saves a copy of the profile under a name that is not taken, and returns it.
pub fn duplicate(dir: &Path, profile: &Profile) -> Result<Profile, ProfileError> {
    let taken = load_all(dir).profiles;
    let copy = Profile { name: unique_name(&format!("{} copy", profile.name), &taken), ..profile.clone() };
    save(dir, &copy)?;
    Ok(copy)
}

/// Moves the profile `old` to the name `new` and returns it; a name that another profile has is refused.
pub fn rename(dir: &Path, old: &str, new: &str) -> Result<Profile, ProfileError> {
    let new = new.trim();
    if !valid_name(new) {
        return Err(ProfileError::BadName);
    }
    let loaded = load_all(dir);
    let mut profile = loaded.profiles.iter().find(|profile| profile.name == old).cloned().ok_or_else(|| ProfileError::Missing(old.to_owned()))?;
    if loaded.profiles.iter().any(|other| other.name != old && same_file(&other.name, new)) {
        return Err(ProfileError::Exists(new.to_owned()));
    }
    profile.name = new.to_owned();
    save(dir, &profile)?;
    if file_name(old) != file_name(new) {
        let old_path = dir.join(file_name(old));
        fs::remove_file(&old_path).map_err(|source| ConfigError::Io { path: old_path, source })?;
    }
    Ok(profile)
}

/// Deletes the profile's file; the last profile is never deleted.
pub fn delete(dir: &Path, name: &str) -> Result<(), ProfileError> {
    let loaded = load_all(dir);
    if !loaded.profiles.iter().any(|profile| profile.name == name) {
        return Err(ProfileError::Missing(name.to_owned()));
    }
    if loaded.profiles.len() < 2 {
        return Err(ProfileError::Last);
    }
    let path = dir.join(file_name(name));
    fs::remove_file(&path).map_err(|source| ConfigError::Io { path, source })?;
    Ok(())
}

/// The name of the shipped profile this release adds on top of the ones the app always shipped.
const NEW_SHIPPED_NAME: &str = "Cam only AV1";

/// Writes any shipped profile this install has not written yet: all of them on a first run, only the newly shipped ones on an existing install.
///
/// `written` is the list of shipped names already written, kept in the settings. An existing install
/// whose list is empty gets the previously shipped names seeded into it, so a shipped profile the user
/// deleted long ago stays deleted and a missing newly shipped profile is added once. A user profile
/// that already uses a shipped name is recorded as written and never overwritten. The webcam settings
/// of a newly added [`Profile::cam_only_av1_1080p30`] are copied from the first existing profile that
/// has the webcam on, else they are None.
pub fn init(dir: &Path, written: &mut Vec<String>) -> Result<(), ConfigError> {
    init_with(dir, written, &gpu_codes_1080_exactly)
}

/// Whether this GPU codes AV1 at exactly 1080 lines. The encoder probe answers once per run; it runs here if it has not yet.
fn gpu_codes_1080_exactly() -> bool {
    aligned_1080_value().is_some() || probe_amf(1920, 1080, 60).iter().any(|row| row.codec == EncoderCodec::Av1 && row.aligned_1080.is_some())
}

/// Gives a shipped profile its default container for a new install: MP4 for Tutorial AV1, and for Cam only AV1
/// MP4 when `aligned_1080` says the GPU codes 1080 lines exactly (it is asked only then), else MKV.
/// Premiere HEVC already ships as MP4. Profile files that exist are never passed through here.
fn with_default_container(profile: Profile, aligned_1080: &dyn Fn() -> bool) -> Profile {
    let container = if profile.name == Profile::tutorial_av1_1440p60().name {
        Container::Mp4
    } else if profile.name == NEW_SHIPPED_NAME {
        if aligned_1080() { Container::Mp4 } else { Container::Mkv }
    } else {
        profile.container
    };
    Profile { container, ..profile }
}

/// Writes the shipped profiles that are missing like `init`, with `aligned_1080` deciding the container of Cam only AV1.
fn init_with(dir: &Path, written: &mut Vec<String>, aligned_1080: &dyn Fn() -> bool) -> Result<(), ConfigError> {
    let shipped = Profile::shipped();
    let loaded = load_all(dir);
    if loaded.profiles.is_empty() && written.is_empty() {
        for profile in &shipped {
            move_broken_file_out_of_the_way(dir, &profile.name)?;
            save(dir, &with_default_container(profile.clone(), aligned_1080))?;
        }
        written.extend(shipped.iter().map(|profile| profile.name.clone()));
        return Ok(());
    }
    if written.is_empty() {
        // An install from before the list existed: the previously shipped names count as written, so
        // only the newly shipped profile is added and old deletions stay deletions.
        written.extend(shipped.iter().filter(|profile| profile.name != NEW_SHIPPED_NAME).map(|profile| profile.name.clone()));
    }
    let webcam_source = loaded.profiles.iter().find(|profile| profile.webcam.is_some()).and_then(|profile| profile.webcam.clone());
    let mut webcam_taken = false;
    for profile in &shipped {
        if written.iter().any(|known| known == &profile.name) {
            continue;
        }
        if loaded.profiles.iter().any(|known| same_file(&known.name, &profile.name)) {
            // The user already has a profile of this name; record it as written and never touch it.
            written.push(profile.name.clone());
            continue;
        }
        let mut added = with_default_container(profile.clone(), aligned_1080);
        if profile.name == NEW_SHIPPED_NAME && !webcam_taken {
            added.webcam = webcam_source.clone();
            webcam_taken = true;
        }
        move_broken_file_out_of_the_way(dir, &added.name)?;
        save(dir, &added)?;
        written.push(added.name.clone());
    }
    Ok(())
}

/// Renames a broken profile file that is in the way of `name` to .json.bad, so the shipped profile can be written.
fn move_broken_file_out_of_the_way(dir: &Path, name: &str) -> Result<(), ConfigError> {
    let path = dir.join(file_name(name));
    if path.exists() {
        let bad = path.with_extension("json.bad");
        log::warn!("{} is not a usable profile, moving it to {}", path.display(), bad.display());
        fs::rename(&path, &bad).map_err(|source| ConfigError::Io { path: path.clone(), source })?;
    }
    Ok(())
}

/// Moves the single profile of an older settings file into a profile file named Default and returns that name.
///
/// Returns None when the settings hold no such profile, or when it cannot be read. An existing Default file is kept.
pub fn migrate_legacy(settings_text: &str, dir: &Path) -> Result<Option<String>, ConfigError> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(settings_text.strip_prefix('\u{feff}').unwrap_or(settings_text)) else { return Ok(None) };
    let single = value
        .get("profile")
        .cloned()
        .or_else(|| value.get("profiles").and_then(|list| list.as_array()).filter(|list| list.len() == 1).map(|list| list[0].clone()));
    let Some(old) = single else { return Ok(None) };
    match serde_json::from_value::<Profile>(old) {
        Ok(profile) => {
            let migrated = Profile { name: MIGRATED_NAME.to_owned(), ..profile };
            if !dir.join(file_name(MIGRATED_NAME)).exists() {
                save(dir, &migrated)?;
            }
            Ok(Some(MIGRATED_NAME.to_owned()))
        }
        Err(err) => {
            log::warn!("the profile in the old settings file could not be migrated: {err}");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::config::{Codec, RateControl, WebcamSettings};

    /// A fresh, empty folder under the system temp folder.
    fn fresh_dir(name: &str) -> Result<PathBuf, std::io::Error> {
        let dir = std::env::temp_dir().join(format!("cutmark-recorder-profiles-{}-{name}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn save_and_load_round_trip() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("round-trip")?;
        let profile = Profile { rate_control: RateControl::Cbr { mbps: 25.0 }, ..Profile::premiere_hevc_1080p60() };
        save(&dir, &profile)?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles, vec![profile]);
        assert!(loaded.warnings.is_empty());
        assert!(!dir.join("Premiere HEVC 1080p60.json.tmp").exists());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_broken_file_is_skipped_with_a_warning() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("broken")?;
        save(&dir, &Profile::tutorial_av1_1440p60())?;
        fs::write(dir.join("Bad.json"), "{ not json")?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("Bad.json"), "{:?}", loaded.warnings);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn file_names_replace_forbidden_characters() {
        assert_eq!(file_name("a/b:c"), "a_b_c.json");
        assert_eq!(file_name("Name. "), "Name.json");
    }

    #[test]
    fn duplicate_takes_a_free_name() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("duplicate")?;
        let base = Profile::tutorial_av1_1440p60();
        save(&dir, &base)?;
        let first = duplicate(&dir, &base)?;
        let second = duplicate(&dir, &base)?;
        assert_eq!(first.name, "Tutorial AV1 1440p60 copy");
        assert_eq!(second.name, "Tutorial AV1 1440p60 copy 2");
        assert_eq!(load_all(&dir).profiles.len(), 3);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn rename_moves_the_file_and_refuses_a_taken_name() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("rename")?;
        for profile in Profile::shipped() {
            save(&dir, &profile)?;
        }
        let renamed = rename(&dir, "Tutorial AV1 1440p60", "Mine")?;
        assert_eq!(renamed.name, "Mine");
        assert!(dir.join("Mine.json").exists());
        assert!(!dir.join("Tutorial AV1 1440p60.json").exists());
        assert!(matches!(rename(&dir, "Mine", "premiere hevc 1080p60"), Err(ProfileError::Exists(_))));
        assert!(matches!(rename(&dir, "Mine", "  "), Err(ProfileError::BadName)));
        assert!(matches!(rename(&dir, "Nope", "Other"), Err(ProfileError::Missing(_))));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_last_profile_is_never_deleted() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("delete")?;
        for profile in Profile::shipped() {
            save(&dir, &profile)?;
        }
        delete(&dir, "Premiere HEVC 1080p60")?;
        delete(&dir, "Cam only AV1")?;
        assert!(matches!(delete(&dir, "Tutorial AV1 1440p60"), Err(ProfileError::Last)));
        assert_eq!(load_all(&dir).profiles.len(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn create_makes_a_new_profile_with_a_free_name() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("create")?;
        assert_eq!(create(&dir)?.name, "New profile");
        assert_eq!(create(&dir)?.name, "New profile 2");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn init_writes_the_shipped_profiles_once() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("init")?;
        let mut written = Vec::new();
        init(&dir, &mut written)?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 3);
        assert_eq!(loaded.profiles[0].name, "Cam only AV1");
        assert_eq!(loaded.profiles[0].codec, Codec::Av1);
        assert_eq!(loaded.profiles[0].layout, crate::config::LayoutKind::CamOnly);
        assert_eq!(loaded.profiles[0].background.kind, crate::config::BackgroundKind::Color);
        assert_eq!(loaded.profiles[1].codec, Codec::Hevc);
        assert_eq!(loaded.profiles[2].codec, Codec::Av1);
        // A deleted shipped profile whose name is in the written list is never recreated.
        delete(&dir, "Cam only AV1")?;
        assert_eq!(written.iter().filter(|name| *name == "Cam only AV1").count(), 1);
        init(&dir, &mut written)?;
        let names: Vec<String> = load_all(&dir).profiles.into_iter().map(|profile| profile.name).collect();
        assert!(!names.contains(&"Cam only AV1".to_owned()), "{names:?}");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_missing_newly_shipped_profile_is_added_once_to_an_existing_install() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("existing-install")?;
        save(&dir, &Profile::premiere_hevc_1080p60())?;
        // An install from before the list existed: only the new profile is added.
        let mut written = Vec::new();
        init(&dir, &mut written)?;
        let names: Vec<String> = load_all(&dir).profiles.into_iter().map(|profile| profile.name).collect();
        assert_eq!(names, vec!["Cam only AV1".to_owned(), "Premiere HEVC 1080p60".to_owned()]);
        assert_eq!(written, vec!["Tutorial AV1 1440p60".to_owned(), "Premiere HEVC 1080p60".to_owned(), "Cam only AV1".to_owned()]);
        // The user deletes the added profile; with the written list kept, it stays deleted.
        delete(&dir, "Cam only AV1")?;
        init(&dir, &mut written)?;
        assert_eq!(load_all(&dir).profiles.len(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_new_shipped_profile_copies_the_webcam_of_the_first_profile_that_has_one() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("webcam-copy")?;
        save(&dir, &Profile::tutorial_av1_1440p60())?;
        let webcam = WebcamSettings { camera_name: "Facecam".to_owned(), offset_ms: 40, ..WebcamSettings::default() };
        let mut with_cam = Profile::premiere_hevc_1080p60();
        with_cam.webcam = Some(webcam.clone());
        save(&dir, &with_cam)?;
        let mut written = Vec::new();
        init(&dir, &mut written)?;
        let added = load_all(&dir).profiles.into_iter().find(|profile| profile.name == "Cam only AV1").ok_or("the profile is missing")?;
        assert_eq!(added.webcam, Some(webcam));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_user_profile_with_a_shipped_name_is_never_overwritten() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("name-taken")?;
        let mine = Profile { fps: 24, ..Profile::cam_only_av1_1080p30() };
        save(&dir, &mine)?;
        let mut written = Vec::new();
        init(&dir, &mut written)?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].fps, 24);
        assert!(written.contains(&"Cam only AV1".to_owned()));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn init_moves_a_broken_file_out_of_the_way() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("init-broken")?;
        fs::write(dir.join("Tutorial AV1 1440p60.json"), "{ not json")?;
        let mut written = Vec::new();
        init(&dir, &mut written)?;
        assert_eq!(load_all(&dir).profiles.len(), 3);
        assert_eq!(fs::read_to_string(dir.join("Tutorial AV1 1440p60.json.bad"))?, "{ not json");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn new_installs_get_mp4_and_cam_only_follows_the_exact_1080_probe() -> Result<(), Box<dyn Error>> {
        for (aligned, cam_container) in [(true, Container::Mp4), (false, Container::Mkv)] {
            let dir = fresh_dir(&format!("containers-{aligned}"))?;
            init_with(&dir, &mut Vec::new(), &|| aligned)?;
            let containers: Vec<(String, Container)> = load_all(&dir).profiles.into_iter().map(|profile| (profile.name, profile.container)).collect();
            assert_eq!(
                containers,
                vec![
                    ("Cam only AV1".to_owned(), cam_container),
                    ("Premiere HEVC 1080p60".to_owned(), Container::Mp4),
                    ("Tutorial AV1 1440p60".to_owned(), Container::Mp4),
                ],
                "aligned {aligned}"
            );
            fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    #[test]
    fn the_probe_is_only_asked_when_cam_only_is_written() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("probe-lazy")?;
        let mut written = Profile::shipped().iter().map(|profile| profile.name.clone()).collect::<Vec<_>>();
        save(&dir, &Profile::tutorial_av1_1440p60())?;
        init_with(&dir, &mut written, &|| panic!("the probe must not run when nothing is written"))?;
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn an_existing_profile_file_keeps_its_container() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("keeps-container")?;
        // Tutorial AV1 as an older install wrote it: MKV, and already in the written list.
        save(&dir, &Profile { container: Container::Mkv, ..Profile::tutorial_av1_1440p60() })?;
        let mut written = vec!["Tutorial AV1 1440p60".to_owned(), "Premiere HEVC 1080p60".to_owned(), "Cam only AV1".to_owned()];
        init_with(&dir, &mut written, &|| true)?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].container, Container::Mkv);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    /// A profile with its own name.
    fn named(name: &str) -> Profile {
        Profile { name: name.to_owned(), ..Profile::tutorial_av1_1440p60() }
    }

    #[test]
    fn an_old_profile_file_without_the_newer_fields_loads_with_the_webcam_off() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("old-file")?;
        let mut value = serde_json::to_value(named("Old file profile"))?;
        if let Some(object) = value.as_object_mut() {
            for key in ["webcam", "preset", "keyframe_secs", "container"] {
                object.remove(key);
            }
        }
        fs::write(dir.join("Old file profile.json"), serde_json::to_string_pretty(&value)?)?;
        let loaded = load_all(&dir);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].webcam, None);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn an_encoder_change_then_a_webcam_change_then_a_save_keeps_both() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("store-both")?;
        save(&dir, &named("Store test"))?;
        let mut store = ProfileStore::open(Some(dir.clone()));
        // The Profile section edits a copy of the active profile and hands it back.
        let mut encoder_edit = store.get("Store test").cloned().ok_or("the profile is missing")?;
        encoder_edit.rate_control = RateControl::Cbr { mbps: 25.0 };
        encoder_edit.fps = 30;
        assert!(store.update(encoder_edit));
        // The Webcam section then edits the same profile in the store.
        let webcam = WebcamSettings { camera_name: "Cam".to_owned(), offset_ms: 80, ..WebcamSettings::default() };
        assert!(store.edit("Store test", |profile| profile.webcam = Some(webcam.clone())).is_some());
        store.flush();
        assert_eq!(store.save_error(), None);
        let saved = load_all(&dir).profiles;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].rate_control, RateControl::Cbr { mbps: 25.0 });
        assert_eq!(saved[0].fps, 30);
        assert_eq!(saved[0].webcam, Some(webcam));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_webcam_change_then_an_encoder_change_keeps_both_too() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("store-both-reversed")?;
        save(&dir, &named("Store test"))?;
        let mut store = ProfileStore::open(Some(dir.clone()));
        let webcam = WebcamSettings { camera_name: "Cam".to_owned(), ..WebcamSettings::default() };
        store.edit("Store test", |profile| profile.webcam = Some(webcam.clone()));
        let mut encoder_edit = store.get("Store test").cloned().ok_or("the profile is missing")?;
        encoder_edit.fps = 24;
        store.update(encoder_edit);
        store.flush();
        let saved = load_all(&dir).profiles;
        assert_eq!((saved[0].fps, saved[0].webcam.clone()), (24, Some(webcam)));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn editing_a_second_profile_writes_the_first_one_and_unknown_names_change_nothing() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("store-two")?;
        save(&dir, &named("Store one"))?;
        save(&dir, &named("Store two"))?;
        let mut store = ProfileStore::open(Some(dir.clone()));
        store.edit("Store one", |profile| profile.fps = 30);
        store.edit("Store two", |profile| profile.fps = 24);
        let on_disk: Vec<(String, u32)> = load_all(&dir).profiles.into_iter().map(|profile| (profile.name, profile.fps)).collect();
        assert_eq!(on_disk, vec![("Store one".to_owned(), 30), ("Store two".to_owned(), 60)]);
        assert!(store.edit("Nope", |profile| profile.fps = 1).is_none());
        assert!(!store.update(named("Nope")));
        store.flush();
        assert_eq!(load_all(&dir).profiles[1].fps, 24);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_single_profile_in_old_settings_becomes_default() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("migrate")?;
        let text = serde_json::json!({ "profile": Profile::premiere_hevc_1080p60(), "countdown_secs": 5 }).to_string();
        assert_eq!(migrate_legacy(&text, &dir)?, Some("Default".to_owned()));
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.profiles[0].name, "Default");
        assert_eq!(loaded.profiles[0].codec, Codec::Hevc);
        assert_eq!(migrate_legacy("{}", &dir)?, None);
        assert_eq!(migrate_legacy("{ not json", &dir)?, None);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
