//! Profile files: one JSON file per profile in the Recorder's profiles folder, with the operations the window needs.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::{ConfigError, Profile, write_json};

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
    let profile = Profile { name: unique_name(NEW_NAME, &taken), ..Profile::tutorial_av1_1440p60() };
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

/// Writes the two shipped profiles when the folder holds no readable profile.
///
/// A broken file that is in the way of a shipped profile is renamed to .json.bad first, never overwritten.
pub fn init(dir: &Path) -> Result<(), ConfigError> {
    if !load_all(dir).profiles.is_empty() {
        return Ok(());
    }
    for profile in Profile::shipped() {
        let path = dir.join(file_name(&profile.name));
        if path.exists() {
            let bad = path.with_extension("json.bad");
            log::warn!("{} is not a usable profile, moving it to {}", path.display(), bad.display());
            fs::rename(&path, &bad).map_err(|source| ConfigError::Io { path: path.clone(), source })?;
        }
        save(dir, &profile)?;
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
    use crate::config::{Codec, RateControl};

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
    fn init_writes_the_two_shipped_profiles_once() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("init")?;
        init(&dir)?;
        let loaded = load_all(&dir);
        assert_eq!(loaded.profiles.len(), 2);
        assert_eq!(loaded.profiles[0].codec, Codec::Hevc);
        assert_eq!(loaded.profiles[1].codec, Codec::Av1);
        delete(&dir, "Premiere HEVC 1080p60")?;
        init(&dir)?;
        assert_eq!(load_all(&dir).profiles.len(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn init_moves_a_broken_file_out_of_the_way() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("init-broken")?;
        fs::write(dir.join("Tutorial AV1 1440p60.json"), "{ not json")?;
        init(&dir)?;
        assert_eq!(load_all(&dir).profiles.len(), 2);
        assert_eq!(fs::read_to_string(dir.join("Tutorial AV1 1440p60.json.bad"))?, "{ not json");
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
