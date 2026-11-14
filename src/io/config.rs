//! io::config: app-wide settings (audio input) in %APPDATA%\CutMark\config.json.

// Nothing reads these settings until the Editor is wired up in task 11.2.
#![allow(dead_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::IoError;

/// Smallest latency offset in milliseconds.
pub const MIN_LATENCY_OFFSET_MS: i32 = -100;
/// Largest latency offset in milliseconds.
pub const MAX_LATENCY_OFFSET_MS: i32 = 100;

/// Which input channel(s) a take records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordChannels {
    /// Mono, from input channel 1.
    #[default]
    Mono1,
    /// Mono, from input channel 2.
    Mono2,
    /// Stereo, from input channels 1 and 2.
    Stereo,
}

/// App settings that belong to the machine, not to a project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Input device display name; None means the Windows default input.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Which input channel(s) to record.
    #[serde(default)]
    pub record_channels: RecordChannels,
    /// Extra latency compensation in milliseconds, -100..=100.
    #[serde(default)]
    pub latency_offset_ms: i32,
}

impl AppConfig {
    /// This config with `latency_offset_ms` forced into -100..=100.
    pub fn clamped(mut self) -> AppConfig {
        self.latency_offset_ms = self
            .latency_offset_ms
            .clamp(MIN_LATENCY_OFFSET_MS, MAX_LATENCY_OFFSET_MS);
        self
    }
}

/// Default location of config.json: `%APPDATA%\CutMark\config.json`.
///
/// Built from the same `ProjectDirs` as recent.json, whose `config_dir()` is
/// the `config` folder inside `%APPDATA%\CutMark`; config.json sits one level
/// up, next to that folder. None if the config folder cannot be found.
pub fn default_path() -> Option<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "CutMark")?;
    Some(dirs.config_dir().parent()?.join("config.json"))
}

/// Loads the config from the default location. A missing, unreadable or
/// corrupt file gives the defaults; the file is never written or repaired here.
pub fn load() -> AppConfig {
    match default_path() {
        Some(path) => load_from(&path),
        None => AppConfig::default(),
    }
}

/// Loads the config from `path` with the same rules as [`load`]. Fields
/// missing from the file take their defaults; the latency offset is clamped.
pub fn load_from(path: &Path) -> AppConfig {
    let Ok(text) = fs::read_to_string(path) else {
        return AppConfig::default();
    };
    serde_json::from_str::<AppConfig>(&text)
        .unwrap_or_default()
        .clamped()
}

/// Saves the config to the default location.
pub fn save(config: &AppConfig) -> Result<(), IoError> {
    let path = default_path()
        .ok_or_else(|| IoError::Io(io::Error::new(io::ErrorKind::NotFound, "no config folder")))?;
    save_to(config, &path)
}

/// Saves the config pretty-printed to `path`: writes `config.json.tmp` next
/// to it, creating the parent folder if needed, then renames it over the
/// target. The latency offset is clamped in what is written.
pub fn save_to(config: &AppConfig, path: &Path) -> Result<(), IoError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&config.clone().clamped())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_are_windows_default_mono1_zero_offset() {
        let config = AppConfig::default();
        assert_eq!(config.input_device, None);
        assert_eq!(config.record_channels, RecordChannels::Mono1);
        assert_eq!(config.latency_offset_ms, 0);
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempdir().unwrap();
        let config = load_from(&dir.path().join("nope.json"));
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sub").join("config.json");
        let config = AppConfig {
            input_device: Some("Microphone (USB Audio) \u{e9}".to_string()),
            record_channels: RecordChannels::Stereo,
            latency_offset_ms: -42,
        };
        save_to(&config, &path).unwrap();
        assert_eq!(load_from(&path), config);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn save_replaces_an_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let first = AppConfig {
            record_channels: RecordChannels::Mono2,
            ..AppConfig::default()
        };
        save_to(&first, &path).unwrap();
        save_to(&AppConfig::default(), &path).unwrap();
        assert_eq!(load_from(&path), AppConfig::default());
    }

    #[test]
    fn corrupt_file_gives_defaults_and_is_left_untouched() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let bytes: &[u8] = b"{ \"input_device\": \"Mic\", \"record_chan";
        fs::write(&path, bytes).unwrap();
        assert_eq!(load_from(&path), AppConfig::default());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn wrong_typed_or_unknown_values_give_defaults_without_a_rewrite() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        for text in [
            r#"{"record_channels":"Quad"}"#,
            r#"{"latency_offset_ms":"loud"}"#,
            r#"{"latency_offset_ms":99999999999}"#,
            "[]",
            "",
        ] {
            fs::write(&path, text).unwrap();
            assert_eq!(load_from(&path), AppConfig::default(), "{text:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), text);
        }
    }

    #[test]
    fn old_file_missing_fields_loads_with_defaults() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");

        fs::write(&path, r#"{"input_device":"Line In"}"#).unwrap();
        let config = load_from(&path);
        assert_eq!(config.input_device.as_deref(), Some("Line In"));
        assert_eq!(config.record_channels, RecordChannels::Mono1);
        assert_eq!(config.latency_offset_ms, 0);

        fs::write(&path, r#"{"record_channels":"Mono2"}"#).unwrap();
        let config = load_from(&path);
        assert_eq!(config.input_device, None);
        assert_eq!(config.record_channels, RecordChannels::Mono2);

        // An empty object and unknown extra fields are fine too.
        fs::write(&path, "{}").unwrap();
        assert_eq!(load_from(&path), AppConfig::default());
        fs::write(&path, r#"{"latency_offset_ms":7,"future_field":true}"#).unwrap();
        assert_eq!(load_from(&path).latency_offset_ms, 7);
    }

    #[test]
    fn latency_offset_is_clamped_on_load() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        for (text, want) in [
            (r#"{"latency_offset_ms":500}"#, 100),
            (r#"{"latency_offset_ms":-500}"#, -100),
            (r#"{"latency_offset_ms":100}"#, 100),
            (r#"{"latency_offset_ms":-100}"#, -100),
            (r#"{"latency_offset_ms":-3}"#, -3),
        ] {
            fs::write(&path, text).unwrap();
            assert_eq!(load_from(&path).latency_offset_ms, want, "{text}");
        }
    }

    #[test]
    fn latency_offset_is_clamped_on_save() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        for (offset, want) in [(1_000, 100), (i32::MIN, -100), (i32::MAX, 100), (25, 25)] {
            let config = AppConfig {
                latency_offset_ms: offset,
                ..AppConfig::default()
            };
            save_to(&config, &path).unwrap();
            // The clamped value is what is on disk, not just what load returns.
            let on_disk: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(on_disk["latency_offset_ms"], want, "offset {offset}");
        }
        // The caller's value is not changed by saving it.
        let config = AppConfig {
            latency_offset_ms: 1_000,
            ..AppConfig::default()
        };
        save_to(&config, &path).unwrap();
        assert_eq!(config.latency_offset_ms, 1_000);
    }

    #[test]
    fn default_path_is_config_json_next_to_the_config_folder() {
        let path = default_path().expect("a config folder");
        assert!(
            path.ends_with(Path::new("CutMark").join("config.json")),
            "{path:?}"
        );
        // recent.json lives in the `config` folder beside it.
        let recent = crate::io::recent::default_path().expect("a config folder");
        assert_eq!(recent.parent().unwrap().parent(), path.parent());
    }
}
