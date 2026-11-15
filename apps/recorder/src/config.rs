//! Recorder settings and capture profiles, kept as JSON in the Recorder's folder of the Cutmark config directory.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Name of the profile that is active until the user picks another.
const DEFAULT_PROFILE: &str = "Tutorial AV1 1440p60";

/// Why the settings or profiles could not be read or written.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The user's config folder could not be found.
    #[error("the user's config folder could not be found")]
    NoConfigDir,
    /// A file or folder could not be read or written.
    #[error("{}: {source}", .path.display())]
    Io {
        /// The file or folder involved.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
    /// A value could not be turned into JSON.
    #[error("{}: {source}", .path.display())]
    Json {
        /// The file involved.
        path: PathBuf,
        /// The underlying error.
        source: serde_json::Error,
    },
}

/// Settings of the local control API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSettings {
    /// Whether the API server runs.
    pub enabled: bool,
    /// The port it listens on, on 127.0.0.1 only.
    pub port: u16,
}

impl Default for ApiSettings {
    fn default() -> Self {
        Self { enabled: true, port: 4466 }
    }
}

/// App settings, stored in settings.json. Missing keys take their defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder that recordings go into, once the user has picked one.
    pub root_folder: Option<PathBuf>,
    /// Name of the profile in use.
    pub active_profile: String,
    /// File name template with the tokens {project}, {date}, {time}, {take} and {profile}.
    pub naming_template: String,
    /// Countdown before recording starts: 0, 3 or 5 seconds.
    pub countdown_secs: u8,
    /// Hide the Recorder's countdown while a prompter shows its own.
    pub hide_countdown_when_prompter_linked: bool,
    /// The local control API.
    pub api: ApiSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            root_folder: None,
            active_profile: DEFAULT_PROFILE.to_owned(),
            naming_template: "{date}_{time}_take{take}".to_owned(),
            countdown_secs: 3,
            hide_countdown_when_prompter_linked: true,
            api: ApiSettings::default(),
        }
    }
}

/// The video codec of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// AV1.
    Av1,
    /// HEVC (H.265).
    Hevc,
    /// H.264.
    H264,
}

/// How the encoder spends bits, tagged by `mode` in the JSON.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RateControl {
    /// Constant quality with a bitrate ceiling.
    Qvbr {
        /// Quality level, lower is better.
        quality: u8,
        /// Bitrate ceiling in megabits per second.
        max_mbps: f32,
    },
    /// Variable bitrate around a target, with a peak.
    Vbr {
        /// Target bitrate in megabits per second.
        target_mbps: f32,
        /// Peak bitrate in megabits per second.
        max_mbps: f32,
    },
    /// Constant bitrate.
    Cbr {
        /// Bitrate in megabits per second.
        mbps: f32,
    },
    /// Constant quantizer.
    Cqp {
        /// Quantizer for I frames.
        qp_i: u8,
        /// Quantizer for P frames.
        qp_p: u8,
    },
}

/// The file container of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// Matroska.
    Mkv,
    /// MP4.
    Mp4,
    /// QuickTime.
    Mov,
}

/// One capture and encode profile, stored as profiles\{name}.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Display name, also the file name.
    pub name: String,
    /// Video codec.
    pub codec: Codec,
    /// Rate control mode and its numbers.
    pub rate_control: RateControl,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Output frames per second.
    pub fps: u32,
    /// File container.
    pub container: Container,
    /// Write each audio track as its own AAC track.
    pub aac_tracks: bool,
    /// Also write a mixdown track.
    pub mixdown: bool,
}

impl Profile {
    /// Shipped profile: AV1 constant quality, 2560x1440 at 60 fps, MKV.
    pub fn tutorial_av1_1440p60() -> Self {
        Self {
            name: DEFAULT_PROFILE.to_owned(),
            codec: Codec::Av1,
            rate_control: RateControl::Qvbr { quality: 23, max_mbps: 60.0 },
            width: 2560,
            height: 1440,
            fps: 60,
            container: Container::Mkv,
            aac_tracks: true,
            mixdown: true,
        }
    }

    /// Shipped profile: HEVC VBR with a peak, 1920x1080 at 60 fps, MP4.
    pub fn premiere_hevc_1080p60() -> Self {
        Self {
            name: "Premiere HEVC 1080p60".to_owned(),
            codec: Codec::Hevc,
            rate_control: RateControl::Vbr { target_mbps: 40.0, max_mbps: 60.0 },
            width: 1920,
            height: 1080,
            fps: 60,
            container: Container::Mp4,
            aac_tracks: true,
            mixdown: true,
        }
    }

    /// Shipped profile: AV1 constant quality, 1920x1080 at 30 fps, MKV, for the webcam alone.
    pub fn cam_only_av1() -> Self {
        Self {
            name: "Cam only AV1".to_owned(),
            codec: Codec::Av1,
            rate_control: RateControl::Qvbr { quality: 23, max_mbps: 20.0 },
            width: 1920,
            height: 1080,
            fps: 30,
            container: Container::Mkv,
            aac_tracks: true,
            mixdown: true,
        }
    }

    /// The three profiles that ship with the app.
    pub fn shipped() -> [Self; 3] {
        [Self::tutorial_av1_1440p60(), Self::premiere_hevc_1080p60(), Self::cam_only_av1()]
    }

    /// The profile's file name, with characters Windows does not allow in file names replaced.
    fn file_name(&self) -> String {
        let stem: String = self.name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
        format!("{stem}.json")
    }
}

/// Reads the settings, creating the Recorder's config folder, the settings file and the shipped profiles when they are missing.
///
/// A settings file that is not valid JSON is logged and left alone, and the defaults are used in memory.
pub fn load_or_init() -> Result<Settings, ConfigError> {
    let dirs = directories::BaseDirs::new().ok_or(ConfigError::NoConfigDir)?;
    load_or_init_in(&dirs.config_dir().join("Cutmark").join("recorder"))
}

/// Does the work of [`load_or_init`] in the given Recorder folder.
fn load_or_init_in(dir: &Path) -> Result<Settings, ConfigError> {
    let profiles = dir.join("profiles");
    fs::create_dir_all(&profiles).map_err(|source| ConfigError::Io { path: profiles.clone(), source })?;
    let settings = load_settings(&dir.join("settings.json"))?;
    write_shipped_profiles_if_none(&profiles)?;
    Ok(settings)
}

/// Reads settings.json, writing the defaults first if it does not exist.
fn load_settings(path: &Path) -> Result<Settings, ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            Ok(serde_json::from_str(text).unwrap_or_else(|err| {
                log::error!("settings file {} is not valid, using the defaults and leaving the file alone: {err}", path.display());
                Settings::default()
            }))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let settings = Settings::default();
            write_json(path, &settings)?;
            Ok(settings)
        }
        Err(source) => Err(ConfigError::Io { path: path.to_path_buf(), source }),
    }
}

/// Writes the three shipped profiles when the folder holds no profile file.
fn write_shipped_profiles_if_none(profiles: &Path) -> Result<(), ConfigError> {
    let entries = fs::read_dir(profiles).map_err(|source| ConfigError::Io { path: profiles.to_path_buf(), source })?;
    let has_profile = entries.filter_map(Result::ok).any(|entry| entry.path().extension().is_some_and(|ext| ext.eq_ignore_ascii_case("json")));
    if has_profile {
        return Ok(());
    }
    for profile in Profile::shipped() {
        write_json(&profiles.join(profile.file_name()), &profile)?;
    }
    Ok(())
}

/// Writes `value` as pretty JSON through a temp file in the same folder.
fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|source| ConfigError::Json { path: path.to_path_buf(), source })?;
    text.push('\n');
    write_atomic(path, text.as_bytes()).map_err(|source| ConfigError::Io { path: path.to_path_buf(), source })
}

/// Writes the bytes to a .tmp file beside `path`, flushes them to disk, then renames the file over `path`.
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp_name = path.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?.to_os_string();
    tmp_name.push(".tmp");
    let tmp_path = path.with_file_name(tmp_name);

    let result = write_then_rename(&tmp_path, path, bytes);
    if result.is_err() {
        // Best effort: do not leave the temp file behind.
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Writes `bytes` to `tmp_path`, flushes them to disk, then renames `tmp_path` over `path`.
fn write_then_rename(tmp_path: &Path, path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(tmp_path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp_path, path)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

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

    #[test]
    fn first_run_writes_defaults_and_the_shipped_profiles() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("first-run")?;
        let settings = load_or_init_in(&dir)?;
        assert_eq!(settings, Settings::default());

        let on_disk: Settings = serde_json::from_str(&fs::read_to_string(dir.join("settings.json"))?)?;
        assert_eq!(on_disk, Settings::default());

        let names: Vec<String> = fs::read_dir(dir.join("profiles"))?.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.iter().all(|name| name.ends_with(".json")), "{names:?}");

        let first: Profile = serde_json::from_str(&fs::read_to_string(dir.join("profiles").join("Tutorial AV1 1440p60.json"))?)?;
        assert_eq!(first, Profile::tutorial_av1_1440p60());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn bad_settings_json_gives_defaults_and_is_not_overwritten() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("bad-json")?;
        fs::write(dir.join("settings.json"), "{ not json")?;
        let settings = load_or_init_in(&dir)?;
        assert_eq!(settings, Settings::default());
        assert_eq!(fs::read_to_string(dir.join("settings.json"))?, "{ not json");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn missing_keys_take_their_defaults() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("missing-keys")?;
        fs::write(dir.join("settings.json"), r#"{ "countdown_secs": 5, "api": { "port": 5000 } }"#)?;
        let settings = load_or_init_in(&dir)?;
        assert_eq!(settings.countdown_secs, 5);
        assert_eq!(settings.api, ApiSettings { enabled: true, port: 5000 });
        assert_eq!(settings.naming_template, "{date}_{time}_take{take}");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn existing_profiles_are_left_alone() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("existing-profiles")?;
        fs::create_dir_all(dir.join("profiles"))?;
        write_json(&dir.join("profiles").join("Mine.json"), &Profile::cam_only_av1())?;
        load_or_init_in(&dir)?;
        assert_eq!(fs::read_dir(dir.join("profiles"))?.count(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
