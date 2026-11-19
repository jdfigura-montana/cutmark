//! Recorder settings and capture profiles, kept as JSON in the Recorder's folder of the Cutmark config directory.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use capture::{AudioDevice, CaptureError, DeviceKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Name of the profile that is active until the user picks another.
const DEFAULT_PROFILE: &str = "Tutorial AV1 1440p60";

/// The most audio tracks a take can have.
pub const MAX_TRACKS: usize = 6;

/// The largest sync offset of a track, in milliseconds, in either direction.
pub const MAX_OFFSET_MS: i32 = 500;

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

/// Whether a track's device captures sound or renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// A capture device such as a microphone.
    Input,
    /// A render device, captured through loopback.
    Output,
}

impl From<DeviceKind> for TrackKind {
    fn from(kind: DeviceKind) -> Self {
        match kind {
            DeviceKind::Input => Self::Input,
            DeviceKind::Output => Self::Output,
        }
    }
}

/// One audio track as saved in the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrackSettings {
    /// The track name, used as the stream title and in the WAV file name.
    pub name: String,
    /// The Windows endpoint ID of the device, the stable identity.
    pub device_id: String,
    /// The device's friendly name, the fallback when the ID no longer matches.
    pub device_name: String,
    /// Whether the device is an input or an output.
    pub kind: TrackKind,
    /// The track's output gain in dB.
    pub gain_db: f32,
    /// Whether the track's output is muted.
    pub muted: bool,
    /// The track's sync offset in ms, clamped to -500..=500 on load.
    pub offset_ms: i32,
    /// Whether the track takes part in the mixdown stream.
    pub in_mixdown: bool,
    /// Whether the track is recorded at all.
    pub enabled: bool,
}

impl Default for TrackSettings {
    fn default() -> Self {
        Self {
            name: "Track".to_owned(),
            device_id: String::new(),
            device_name: String::new(),
            kind: TrackKind::Input,
            gain_db: 0.0,
            muted: false,
            offset_ms: 0,
            in_mixdown: true,
            enabled: true,
        }
    }
}

impl TrackSettings {
    /// A track on `device` with neutral gain, no mute and no offset.
    fn for_device(name: &str, device: &AudioDevice, in_mixdown: bool) -> Self {
        Self { name: name.to_owned(), device_id: device.id.clone(), device_name: device.name.clone(), kind: device.kind.into(), in_mixdown, ..Self::default() }
    }
}

/// The audio codec of the streams in the video file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodecChoice {
    /// AAC-LC at 320 kbps.
    #[default]
    #[serde(rename = "aac_320")]
    Aac320,
    /// PCM, 24-bit.
    #[serde(rename = "pcm_24")]
    Pcm24,
}

/// The audio part of the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// The audio tracks, in stream order; at most [`MAX_TRACKS`].
    pub tracks: Vec<TrackSettings>,
    /// Whether every track is also written as a WAV sidecar.
    pub wav_sidecars: bool,
    /// The codec of the audio streams in the video file.
    pub codec: AudioCodecChoice,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self { tracks: Vec::new(), wav_sidecars: true, codec: AudioCodecChoice::default() }
    }
}

impl AudioSettings {
    /// Ignores tracks beyond [`MAX_TRACKS`] with a warning and clamps every offset into range.
    fn normalize(&mut self) {
        if self.tracks.len() > MAX_TRACKS {
            log::warn!("the settings list {} audio tracks, ignoring all after the first {MAX_TRACKS}", self.tracks.len());
            self.tracks.truncate(MAX_TRACKS);
        }
        for track in &mut self.tracks {
            track.offset_ms = track.offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
        }
    }
}

/// The suite's default tracks for the devices present: Stream Mix and Mic when a Stream Mix input exists, else Desktop and Mic.
pub fn default_tracks(devices: &[AudioDevice]) -> Vec<TrackSettings> {
    let mic = devices.iter().find(|d| d.kind == DeviceKind::Input && d.is_default);
    let stream_mix = devices.iter().find(|d| d.kind == DeviceKind::Input && d.name.contains("Stream Mix"));
    let mut tracks = Vec::new();
    if let Some(mix) = stream_mix {
        tracks.push(TrackSettings::for_device("Stream Mix", mix, true));
        tracks.extend(mic.map(|mic| TrackSettings::for_device("Mic", mic, false)));
    } else {
        let desktop = devices.iter().find(|d| d.kind == DeviceKind::Output && d.is_default);
        tracks.extend(desktop.map(|desktop| TrackSettings::for_device("Desktop", desktop, true)));
        tracks.extend(mic.map(|mic| TrackSettings::for_device("Mic", mic, true)));
    }
    tracks
}

/// App settings, stored in settings.json. Missing keys take their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// The audio tracks, the WAV sidecar switch and the audio codec.
    pub audio: AudioSettings,
}

/// The folder takes go into: the settings root folder, or else the user's Videos folder joined with Cutmark.
///
/// Recording and the launch recovery scan both use this. If the user's Videos folder cannot be found it falls back to a
/// Cutmark folder relative to the working directory.
pub fn takes_root(settings: &Settings) -> PathBuf {
    if let Some(root) = &settings.root_folder {
        return root.clone();
    }
    match directories::UserDirs::new().and_then(|dirs| dirs.video_dir().map(|videos| videos.join("Cutmark"))) {
        Some(root) => root,
        None => {
            log::warn!("the user's Videos folder could not be found, using a Cutmark folder in the working directory");
            PathBuf::from("Cutmark")
        }
    }
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
            audio: AudioSettings::default(),
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
///
/// When the settings hold no audio tracks, they are filled from [`default_tracks`] and saved.
pub fn load_or_init() -> Result<Settings, ConfigError> {
    let dirs = directories::BaseDirs::new().ok_or(ConfigError::NoConfigDir)?;
    load_or_init_in(&dirs.config_dir().join("Cutmark").join("recorder"), capture::list_audio_devices)
}

/// Does the work of [`load_or_init`] in the given Recorder folder, listing audio devices through `list_devices`.
fn load_or_init_in(dir: &Path, list_devices: impl FnOnce() -> Result<Vec<AudioDevice>, CaptureError>) -> Result<Settings, ConfigError> {
    let profiles = dir.join("profiles");
    fs::create_dir_all(&profiles).map_err(|source| ConfigError::Io { path: profiles.clone(), source })?;
    let settings_path = dir.join("settings.json");
    let (mut settings, saveable) = load_settings(&settings_path)?;
    if settings.audio.tracks.is_empty() {
        match list_devices() {
            Ok(devices) => {
                settings.audio.tracks = default_tracks(&devices);
                // A settings file that could not be read stays as the user left it.
                if saveable && !settings.audio.tracks.is_empty() {
                    write_json(&settings_path, &settings)?;
                }
            }
            Err(err) => log::warn!("could not list the audio devices, so no default tracks were made: {err}"),
        }
    }
    write_shipped_profiles_if_none(&profiles)?;
    Ok(settings)
}

/// Reads settings.json, writing the defaults first if it does not exist.
///
/// The flag is false when the file exists but is not valid JSON, so it must not be written over.
fn load_settings(path: &Path) -> Result<(Settings, bool), ConfigError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            Ok(match serde_json::from_str::<Settings>(text) {
                Ok(mut settings) => {
                    settings.audio.normalize();
                    (settings, true)
                }
                Err(err) => {
                    log::error!("settings file {} is not valid, using the defaults and leaving the file alone: {err}", path.display());
                    (Settings::default(), false)
                }
            })
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let settings = Settings::default();
            write_json(path, &settings)?;
            Ok((settings, true))
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

    /// A device lister for tests that must not depend on the machine's audio devices.
    fn no_devices() -> Result<Vec<AudioDevice>, CaptureError> {
        Ok(Vec::new())
    }

    /// A fake audio endpoint.
    fn device(id: &str, name: &str, kind: DeviceKind, is_default: bool) -> AudioDevice {
        AudioDevice { id: id.to_owned(), name: name.to_owned(), kind, is_default, sample_rate: 48_000, channels: 2 }
    }

    #[test]
    fn first_run_writes_defaults_and_the_shipped_profiles() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("first-run")?;
        let settings = load_or_init_in(&dir, no_devices)?;
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
        let settings = load_or_init_in(&dir, no_devices)?;
        assert_eq!(settings, Settings::default());
        assert_eq!(fs::read_to_string(dir.join("settings.json"))?, "{ not json");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn missing_keys_take_their_defaults() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("missing-keys")?;
        fs::write(dir.join("settings.json"), r#"{ "countdown_secs": 5, "api": { "port": 5000 } }"#)?;
        let settings = load_or_init_in(&dir, no_devices)?;
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
        load_or_init_in(&dir, no_devices)?;
        assert_eq!(fs::read_dir(dir.join("profiles"))?.count(), 1);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn default_tracks_with_a_stream_mix_input() {
        let devices = [
            device("out-1", "Speakers (Realtek)", DeviceKind::Output, true),
            device("in-mix", "Stream Mix (Wave Link)", DeviceKind::Input, false),
            device("in-mic", "Microphone (Wave:3)", DeviceKind::Input, true),
        ];
        let tracks = default_tracks(&devices);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "Stream Mix");
        assert_eq!(tracks[0].device_id, "in-mix");
        assert_eq!(tracks[0].device_name, "Stream Mix (Wave Link)");
        assert_eq!(tracks[0].kind, TrackKind::Input);
        assert!(tracks[0].in_mixdown);
        assert_eq!(tracks[1].name, "Mic");
        assert_eq!(tracks[1].device_id, "in-mic");
        assert!(!tracks[1].in_mixdown);
        assert!(tracks.iter().all(|t| t.enabled && !t.muted && t.gain_db == 0.0 && t.offset_ms == 0));
    }

    #[test]
    fn default_tracks_without_a_stream_mix_input() {
        let devices = [
            device("out-2", "Headphones", DeviceKind::Output, false),
            device("out-1", "Speakers (Realtek)", DeviceKind::Output, true),
            device("in-mic", "Microphone (Wave:3)", DeviceKind::Input, true),
        ];
        let tracks = default_tracks(&devices);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "Desktop");
        assert_eq!(tracks[0].device_id, "out-1");
        assert_eq!(tracks[0].kind, TrackKind::Output);
        assert!(tracks[0].in_mixdown);
        assert_eq!(tracks[1].name, "Mic");
        assert_eq!(tracks[1].device_id, "in-mic");
        assert_eq!(tracks[1].kind, TrackKind::Input);
        assert!(tracks[1].in_mixdown);
    }

    #[test]
    fn default_tracks_skip_a_track_whose_device_does_not_exist() {
        let only_mic = [device("in-mic", "Microphone", DeviceKind::Input, true)];
        let tracks = default_tracks(&only_mic);
        assert_eq!(tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Mic"]);

        let only_mix = [device("in-mix", "Stream Mix", DeviceKind::Input, false)];
        let tracks = default_tracks(&only_mix);
        assert_eq!(tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["Stream Mix"]);

        assert!(default_tracks(&[]).is_empty());
    }

    #[test]
    fn audio_settings_defaults_and_codec_names() -> Result<(), Box<dyn Error>> {
        let audio = AudioSettings::default();
        assert!(audio.tracks.is_empty());
        assert!(audio.wav_sidecars);
        assert_eq!(audio.codec, AudioCodecChoice::Aac320);
        assert_eq!(serde_json::to_string(&AudioCodecChoice::Aac320)?, "\"aac_320\"");
        assert_eq!(serde_json::to_string(&AudioCodecChoice::Pcm24)?, "\"pcm_24\"");
        let track: TrackSettings = serde_json::from_str(r#"{ "name": "Mic", "kind": "output" }"#)?;
        assert_eq!(track.kind, TrackKind::Output);
        assert!(track.enabled);
        assert_eq!(track.gain_db, 0.0);
        Ok(())
    }

    #[test]
    fn load_clamps_offsets_and_ignores_tracks_past_six() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("track-limits")?;
        let tracks: Vec<_> = (1..=8)
            .map(|n| {
                let offset = match n {
                    1 => 900,
                    2 => -900,
                    3 => 250,
                    _ => 0,
                };
                serde_json::json!({ "name": format!("T{n}"), "offset_ms": offset })
            })
            .collect();
        fs::write(dir.join("settings.json"), serde_json::json!({ "audio": { "tracks": tracks } }).to_string())?;
        let settings = load_or_init_in(&dir, no_devices)?;
        assert_eq!(settings.audio.tracks.len(), MAX_TRACKS);
        assert_eq!(settings.audio.tracks[0].offset_ms, 500);
        assert_eq!(settings.audio.tracks[1].offset_ms, -500);
        assert_eq!(settings.audio.tracks[2].offset_ms, 250);
        assert_eq!(settings.audio.tracks[5].name, "T6");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn empty_tracks_are_filled_from_the_devices_and_saved() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("fill-tracks")?;
        let devices = vec![device("out-1", "Speakers", DeviceKind::Output, true), device("in-mic", "Microphone", DeviceKind::Input, true)];
        let settings = load_or_init_in(&dir, || Ok(devices))?;
        assert_eq!(settings.audio.tracks.len(), 2);
        let on_disk: Settings = serde_json::from_str(&fs::read_to_string(dir.join("settings.json"))?)?;
        assert_eq!(on_disk, settings);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn saved_tracks_are_not_replaced_by_the_defaults() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("keep-tracks")?;
        fs::write(dir.join("settings.json"), r#"{ "audio": { "tracks": [ { "name": "Mine" } ] } }"#)?;
        let devices = vec![device("in-mic", "Microphone", DeviceKind::Input, true)];
        let settings = load_or_init_in(&dir, || Ok(devices))?;
        assert_eq!(settings.audio.tracks.len(), 1);
        assert_eq!(settings.audio.tracks[0].name, "Mine");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_failed_device_listing_leaves_the_tracks_empty() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("list-fails")?;
        let settings = load_or_init_in(&dir, || Err(CaptureError::Missing("no audio in this test")))?;
        assert!(settings.audio.tracks.is_empty());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn default_tracks_are_not_saved_over_an_unreadable_settings_file() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("bad-json-tracks")?;
        fs::write(dir.join("settings.json"), "{ not json")?;
        let devices = vec![device("in-mic", "Microphone", DeviceKind::Input, true)];
        let settings = load_or_init_in(&dir, || Ok(devices))?;
        assert_eq!(settings.audio.tracks.len(), 1);
        assert_eq!(fs::read_to_string(dir.join("settings.json"))?, "{ not json");
        fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
