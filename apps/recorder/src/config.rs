//! Recorder settings and capture profiles, kept as JSON in the Recorder's folder of the Cutmark config directory.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use capture::{AudioDevice, CaptureError, ControlProp, DeviceKind};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::profiles;
use crate::tools_ui::Tool;

/// Name of the profile that is active until the user picks another.
const DEFAULT_PROFILE: &str = "Tutorial AV1 1440p60";

/// The project name until the user picks another.
pub const DEFAULT_PROJECT: &str = "Cutmark";

/// The naming template until the user picks another.
pub const DEFAULT_NAMING_TEMPLATE: &str = "{date}_{time}_take{take}";

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

/// What a track records: a capture device, a render device, one app's audio, or the desktop without one app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// A capture device such as a microphone.
    Input,
    /// A render device, captured through loopback.
    Output,
    /// One app and its child processes, captured through process loopback. The track's device fields are unused.
    App {
        /// The executable's file name in lowercase, for example `discord.exe`.
        exe: String,
        /// The app's name for display.
        app_name: String,
    },
    /// All desktop audio except one app and its child processes; the default output as it is while the app
    /// is not running. The track's device fields are unused.
    DesktopExcept {
        /// The executable's file name in lowercase, for example `spotify.exe`.
        exe: String,
        /// The app's name for display.
        app_name: String,
    },
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

/// The window mode (spec "Modes"): Setup holds the settings; Record and Review are empty
/// panels until R7 and R8.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Sources, preview and the settings tools.
    #[default]
    Setup,
    /// The recording view, empty until R7.
    Record,
    /// The takes view, empty until R8.
    Review,
}

impl Mode {
    /// The modes in the order the switch shows them.
    pub const ALL: [Mode; 3] = [Mode::Setup, Mode::Record, Mode::Review];

    /// The label the mode switch and the View menu show.
    pub fn label(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Record => "Record",
            Self::Review => "Review",
        }
    }
}

/// The mode rules (spec "Mode rules"): starting a recording from anywhere switches to Record
/// and remembers the mode that was open; stopping returns to it; while a take runs, Setup
/// and Review are disabled in the mode switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeState {
    /// The mode the window shows.
    pub mode: Mode,
    /// The mode to return to when the take ends; none while idle.
    pre_record: Option<Mode>,
}

impl ModeState {
    /// The state the persisted mode gives: `mode` open, nothing to return to.
    pub fn new(mode: Mode) -> Self {
        Self { mode, pre_record: None }
    }

    /// A recording started: Record mode, remembering where the user was.
    pub fn start(&mut self) {
        self.pre_record = Some(self.mode);
        self.mode = Mode::Record;
    }

    /// The recording ended: back to the mode that was open before it.
    pub fn stop(&mut self) {
        if let Some(previous) = self.pre_record.take() {
            self.mode = previous;
        }
    }

    /// Whether the mode switch may move to `to` while `recording`: while a take runs, Setup
    /// and Review are disabled and Record stays, because it is already open.
    pub fn can_switch(self, recording: bool, to: Mode) -> bool {
        if recording {
            return to == Mode::Record;
        }
        true
    }
}

/// App settings, stored in settings.json. Missing keys take their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Folder that recordings go into, once the user has picked one.
    pub root_folder: Option<PathBuf>,
    /// Folder that replays go into, or none to save them inside each project.
    pub replays_root: Option<PathBuf>,
    /// Whether the first-launch dialog about the recordings folder has been answered.
    pub folders_confirmed: bool,
    /// Name of the profile in use.
    pub active_profile: String,
    /// The project name: the folder under the root that holds the project's takes.
    pub project: String,
    /// Template of the take folder name and the file base name, with the tokens {project}, {date}, {time}, {take} and {profile}.
    pub naming_template: String,
    /// Countdown before recording starts: 0 (off), 3 or 5 seconds.
    pub countdown_secs: u8,
    /// Hide the Recorder's countdown while a prompter is linked and shows its own.
    #[serde(alias = "hide_countdown_when_prompter_linked")]
    pub hide_countdown_when_linked: bool,
    /// How much the replay buffer keeps, in seconds: one of [`REPLAY_LENGTHS_S`]. Read when the buffer turns on.
    pub replay_length_s: u32,
    /// The local control API.
    pub api: ApiSettings,
    /// Whether the Stream Deck remote control server runs.
    pub stream_deck: bool,
    /// The audio tracks, the WAV sidecar switch and the audio codec.
    pub audio: AudioSettings,
    /// Whether the main window is hidden from screen captures; the label popup is always hidden.
    pub hide_from_capture: bool,
    /// The window mode the app reopens in.
    pub mode: Mode,
    /// The Setup tool the right panel shows (spec "Mode rules": the open tool is remembered between launches).
    pub tool: Tool,
    /// The panel widths of every mode, remembered between launches (spec "Panels").
    pub panel_widths: ModeWidths,
    /// The shipped profile names this install has already written, so a deleted shipped profile is never recreated.
    #[serde(default)]
    pub shipped_written: Vec<String>,
}

/// The replay lengths the settings offer, in seconds.
pub const REPLAY_LENGTHS_S: [u32; 4] = [30, 60, 90, 120];

/// The replay length until the user picks one, in seconds.
const DEFAULT_REPLAY_LENGTH_S: u32 = 60;

impl Settings {
    /// How much the replay buffer keeps: the length setting, or the default for a length that is not offered.
    pub fn replay_length(&self) -> Duration {
        let seconds = if REPLAY_LENGTHS_S.contains(&self.replay_length_s) { self.replay_length_s } else { DEFAULT_REPLAY_LENGTH_S };
        Duration::from_secs(u64::from(seconds))
    }
}

/// The narrowest the Recorder's window may be, in points; wider than the suite's own minimum.
pub const MIN_WINDOW_WIDTH: f32 = 960.0;

/// The narrowest the left column may be (spec "Panels").
pub const LEFT_PANEL_MIN: f32 = 220.0;

/// The widest the left column may be (spec "Panels").
pub const LEFT_PANEL_MAX: f32 = 360.0;

/// The left column's width until the user resizes it (spec "Panels").
pub const LEFT_PANEL_DEFAULT: f32 = 270.0;

/// The narrowest the right column may be (spec "Panels").
pub const RIGHT_PANEL_MIN: f32 = 300.0;

/// The widest the right column may be (spec "Panels").
pub const RIGHT_PANEL_MAX: f32 = 440.0;

/// The right column's width until the user resizes it (spec "Panels").
pub const RIGHT_PANEL_DEFAULT: f32 = 340.0;

/// The column widths of one mode (spec "Panels").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelWidths {
    /// The left column's width.
    pub left: f32,
    /// The right column's width.
    pub right: f32,
}

impl Default for PanelWidths {
    fn default() -> Self {
        Self { left: LEFT_PANEL_DEFAULT, right: RIGHT_PANEL_DEFAULT }
    }
}

/// The column widths of every mode, remembered between launches (spec "Panels").
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ModeWidths {
    /// Setup mode's column widths.
    pub setup: PanelWidths,
    /// Record mode's column widths; Record has no left column.
    pub record: PanelWidths,
    /// Review mode's column widths.
    pub review: PanelWidths,
}

impl ModeWidths {
    /// The widths of `mode`.
    pub fn get(self, mode: Mode) -> PanelWidths {
        match mode {
            Mode::Setup => self.setup,
            Mode::Record => self.record,
            Mode::Review => self.review,
        }
    }

    /// The saved widths of `mode`, to change in place.
    pub fn entry(&mut self, mode: Mode) -> &mut PanelWidths {
        match mode {
            Mode::Setup => &mut self.setup,
            Mode::Record => &mut self.record,
            Mode::Review => &mut self.review,
        }
    }

    /// Clamps every mode's widths into the columns' ranges, as they are loaded. What a window
    /// has room for is up to the layout, which keeps the stage's minimum at any window width.
    fn clamp_to_ranges(&mut self) {
        for widths in [&mut self.setup, &mut self.record, &mut self.review] {
            widths.left = widths.left.clamp(LEFT_PANEL_MIN, LEFT_PANEL_MAX);
            widths.right = widths.right.clamp(RIGHT_PANEL_MIN, RIGHT_PANEL_MAX);
        }
    }
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
            replays_root: None,
            folders_confirmed: false,
            active_profile: DEFAULT_PROFILE.to_owned(),
            project: DEFAULT_PROJECT.to_owned(),
            naming_template: DEFAULT_NAMING_TEMPLATE.to_owned(),
            countdown_secs: 0,
            hide_countdown_when_linked: true,
            replay_length_s: DEFAULT_REPLAY_LENGTH_S,
            api: ApiSettings::default(),
            stream_deck: true,
            audio: AudioSettings::default(),
            hide_from_capture: true,
            mode: Mode::default(),
            tool: Tool::Layout,
            panel_widths: ModeWidths::default(),
            shipped_written: Vec::new(),
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

impl From<Codec> for media_io::encode::Codec {
    fn from(codec: Codec) -> Self {
        match codec {
            Codec::Av1 => Self::Av1,
            Codec::Hevc => Self::Hevc,
            Codec::H264 => Self::H264,
        }
    }
}

/// How the encoder spends bits, and the speed preset and file container of a profile, shared with media-io.
pub use media_io::encode::{Container, Preset, RateControl};

/// The keyframe interval of a profile file that has none, in seconds.
const DEFAULT_KEYFRAME_SECS: f32 = 2.0;

/// The preset of a profile file that has none.
fn default_preset() -> Preset {
    Preset::Balanced
}

/// The keyframe interval of a profile file that has none.
fn default_keyframe_secs() -> f32 {
    DEFAULT_KEYFRAME_SECS
}

/// How one camera property is held in a profile: left to the camera, or fixed at a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlSetting {
    /// The camera adjusts the property by itself.
    Auto,
    /// The property is held at this value.
    Manual(i32),
}

/// The camera controls saved in a profile; a property that is None is left as the camera has it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebcamControls {
    /// Exposure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<ControlSetting>,
    /// Focus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus: Option<ControlSetting>,
    /// White balance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white_balance: Option<ControlSetting>,
    /// Gain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gain: Option<ControlSetting>,
    /// Brightness.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<ControlSetting>,
}

impl WebcamControls {
    /// The saved setting of a property.
    pub fn get(&self, prop: ControlProp) -> Option<ControlSetting> {
        match prop {
            ControlProp::Exposure => self.exposure,
            ControlProp::Focus => self.focus,
            ControlProp::WhiteBalance => self.white_balance,
            ControlProp::Gain => self.gain,
            ControlProp::Brightness => self.brightness,
        }
    }

    /// Replaces the saved setting of a property.
    pub fn set(&mut self, prop: ControlProp, setting: Option<ControlSetting>) {
        match prop {
            ControlProp::Exposure => self.exposure = setting,
            ControlProp::Focus => self.focus = setting,
            ControlProp::WhiteBalance => self.white_balance = setting,
            ControlProp::Gain => self.gain = setting,
            ControlProp::Brightness => self.brightness = setting,
        }
    }
}

/// The camera mode saved in a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebcamFormat {
    /// The pixel format as its FourCC text, for example MJPG, NV12 or YUY2.
    pub subtype: String,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: f32,
}

impl Default for WebcamFormat {
    fn default() -> Self {
        Self { subtype: "MJPG".to_owned(), width: 1280, height: 720, fps: 30.0 }
    }
}

/// The webcam settings of a profile; a profile without them has the webcam off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WebcamSettings {
    /// The symbolic link that identifies the camera; empty until one is picked.
    pub camera_id: String,
    /// The camera's name, shown when the camera is not plugged in.
    pub camera_name: String,
    /// The camera mode.
    pub format: WebcamFormat,
    /// The saved camera controls.
    pub controls: WebcamControls,
    /// Whether the raw camera is also written to its own file.
    pub iso: bool,
    /// Rate control of that file.
    pub iso_rc: RateControl,
    /// Delay of the camera frames against the screen, in milliseconds, from -500 to 500.
    pub offset_ms: i32,
    /// The chroma key of the camera in the composite; the raw camera file is never keyed.
    pub key: KeyConfig,
}

/// The chroma key of the camera, saved in a profile; it mirrors `capture::KeySettings`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyConfig {
    /// Whether the camera is keyed.
    pub enabled: bool,
    /// The key colour as red, green and blue.
    pub color: [u8; 3],
    /// How far from the key colour a pixel may be and still be fully transparent, 0 to 1.
    pub similarity: f32,
    /// How wide the soft edge beyond the similarity is, 0 to 1.
    pub smoothness: f32,
    /// How strongly the key colour is pulled out of the pixels that stay, 0 to 1.
    pub spill: f32,
    /// The part of the camera's left side that is thrown away, in percent.
    pub crop_left_pct: f32,
    /// The part of the camera's right side that is thrown away, in percent.
    pub crop_right_pct: f32,
    /// The part of the camera's top that is thrown away, in percent.
    pub crop_top_pct: f32,
    /// The part of the camera's bottom that is thrown away, in percent.
    pub crop_bottom_pct: f32,
    /// Whether the mask rectangle is cut out of the picture.
    pub mask_enabled: bool,
    /// Left edge of the mask rectangle, in percent of the camera frame's width.
    pub mask_x_pct: f32,
    /// Top edge of the mask rectangle, in percent of the camera frame's height.
    pub mask_y_pct: f32,
    /// Width of the mask rectangle, in percent of the camera frame's width.
    pub mask_w_pct: f32,
    /// Height of the mask rectangle, in percent of the camera frame's height.
    pub mask_h_pct: f32,
}

impl Default for KeyConfig {
    fn default() -> Self {
        let key = capture::KeySettings::default();
        Self {
            enabled: key.enabled,
            color: key.color,
            similarity: key.similarity,
            smoothness: key.smoothness,
            spill: key.spill,
            crop_left_pct: key.crop_left_pct,
            crop_right_pct: key.crop_right_pct,
            crop_top_pct: key.crop_top_pct,
            crop_bottom_pct: key.crop_bottom_pct,
            mask_enabled: key.mask_enabled,
            mask_x_pct: key.mask_x_pct,
            mask_y_pct: key.mask_y_pct,
            mask_w_pct: key.mask_w_pct,
            mask_h_pct: key.mask_h_pct,
        }
    }
}

impl From<&KeyConfig> for capture::KeySettings {
    fn from(config: &KeyConfig) -> Self {
        Self {
            enabled: config.enabled,
            color: config.color,
            similarity: config.similarity,
            smoothness: config.smoothness,
            spill: config.spill,
            crop_left_pct: config.crop_left_pct,
            crop_right_pct: config.crop_right_pct,
            crop_top_pct: config.crop_top_pct,
            crop_bottom_pct: config.crop_bottom_pct,
            mask_enabled: config.mask_enabled,
            mask_x_pct: config.mask_x_pct,
            mask_y_pct: config.mask_y_pct,
            mask_w_pct: config.mask_w_pct,
            mask_h_pct: config.mask_h_pct,
        }
    }
}

impl Default for WebcamSettings {
    fn default() -> Self {
        Self {
            camera_id: String::new(),
            camera_name: String::new(),
            format: WebcamFormat::default(),
            controls: WebcamControls::default(),
            iso: true,
            iso_rc: RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 30.0 },
            offset_ms: 0,
            key: KeyConfig::default(),
        }
    }
}

impl WebcamSettings {
    /// The offset limited to the range the window allows.
    pub fn offset_clamped(&self) -> i32 {
        self.offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS)
    }
}

/// The background colour of a profile file that has none: dark grey.
const DEFAULT_BACKGROUND_COLOR: [u8; 3] = [32, 32, 32];

/// Which pictures the composite of a profile shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    /// Only the screen.
    #[default]
    ScreenOnly,
    /// The camera over the screen.
    Pip,
    /// Only the camera, over a background colour.
    CamOnly,
}

impl From<LayoutKind> for capture::Layout {
    fn from(kind: LayoutKind) -> Self {
        match kind {
            LayoutKind::ScreenOnly => Self::ScreenOnly,
            LayoutKind::Pip => Self::Pip,
            LayoutKind::CamOnly => Self::CamOnly,
        }
    }
}

/// Whether the camera is drawn into the main video of a take, saved in a profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositeMode {
    /// The main video shows what the layout shows, the camera included.
    #[default]
    Baked,
    /// The main video of a picture in picture take is the screen only; the camera goes to its own file.
    Clean,
}

/// The corner of the frame the picture in picture camera sits in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorCorner {
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    #[default]
    BottomRight,
}

impl From<AnchorCorner> for capture::Corner {
    fn from(corner: AnchorCorner) -> Self {
        match corner {
            AnchorCorner::TopLeft => Self::TopLeft,
            AnchorCorner::TopRight => Self::TopRight,
            AnchorCorner::BottomLeft => Self::BottomLeft,
            AnchorCorner::BottomRight => Self::BottomRight,
        }
    }
}

/// The shape of the picture in picture camera.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipShape {
    /// A rectangle, with optional rounded corners.
    #[default]
    Rectangle,
    /// A circle cut from the middle of the cropped camera.
    Circle,
}

impl From<PipShape> for capture::PipShape {
    fn from(shape: PipShape) -> Self {
        match shape {
            PipShape::Rectangle => Self::Rectangle,
            PipShape::Circle => Self::Circle,
        }
    }
}

/// The camera's place and look in the picture in picture layout, saved in a profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PipConfig {
    /// The corner of the frame the camera is anchored to.
    pub anchor: AnchorCorner,
    /// Distance from the left or right edge, in percent of the output width.
    pub offset_x_pct: f32,
    /// Distance from the top or bottom edge, in percent of the output height.
    pub offset_y_pct: f32,
    /// Width of the camera picture, in percent of the output width.
    pub scale_pct: f32,
    /// How much of the camera's left side is cut off, in percent.
    pub crop_left_pct: f32,
    /// How much of the camera's right side is cut off, in percent.
    pub crop_right_pct: f32,
    /// How much of the camera's top is cut off, in percent.
    pub crop_top_pct: f32,
    /// How much of the camera's bottom is cut off, in percent.
    pub crop_bottom_pct: f32,
    /// Border width in output pixels; 0 for none.
    pub border_px: f32,
    /// Border colour as red, green and blue.
    pub border_color: [u8; 3],
    /// Corner radius in output pixels; 0 for square corners. Ignored for a circle.
    pub radius_px: f32,
    /// The shape of the camera picture; profile files without one use a rectangle.
    pub shape: PipShape,
}

impl Default for PipConfig {
    fn default() -> Self {
        let pip = capture::PipSettings::default();
        Self {
            anchor: AnchorCorner::BottomRight,
            offset_x_pct: pip.offset_x_pct,
            offset_y_pct: pip.offset_y_pct,
            scale_pct: pip.scale_pct,
            crop_left_pct: pip.crop_left_pct,
            crop_right_pct: pip.crop_right_pct,
            crop_top_pct: pip.crop_top_pct,
            crop_bottom_pct: pip.crop_bottom_pct,
            border_px: pip.border_px,
            border_color: pip.border_color,
            radius_px: pip.radius_px,
            shape: PipShape::Rectangle,
        }
    }
}

impl From<&PipConfig> for capture::PipSettings {
    fn from(config: &PipConfig) -> Self {
        Self {
            anchor: config.anchor.into(),
            offset_x_pct: config.offset_x_pct,
            offset_y_pct: config.offset_y_pct,
            scale_pct: config.scale_pct,
            crop_left_pct: config.crop_left_pct,
            crop_right_pct: config.crop_right_pct,
            crop_top_pct: config.crop_top_pct,
            crop_bottom_pct: config.crop_bottom_pct,
            border_px: config.border_px,
            border_color: config.border_color,
            radius_px: config.radius_px,
            shape: config.shape.into(),
        }
    }
}

/// What a layout draws behind the pictures; mirrors `capture::BackgroundKind`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    /// Nothing; a keyed camera floats over the screen, and the Cam only layout uses the colour.
    #[default]
    None,
    /// A solid colour.
    Color,
    /// A PNG or JPG image from a file.
    Image,
    /// A looping MP4 or MKV video from a file, decoded on the GPU.
    Video,
}

/// How an image background fills the rectangle it is given; mirrors `capture::BackgroundFit`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundFit {
    /// Fills the rectangle and crops whatever does not fit.
    #[default]
    Cover,
    /// Fits whole inside the rectangle, letterboxed with the background colour.
    Contain,
    /// Stretches over the rectangle, ignoring the aspect ratio.
    Stretch,
}

/// The background settings of a profile; mirrors `capture::BackgroundSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackgroundConfig {
    /// What the background shows.
    pub kind: BackgroundKind,
    /// The solid colour as red, green and blue.
    pub color: [u8; 3],
    /// The path of the image file, used when the kind is Image.
    pub image_path: String,
    /// The path of the video file, used when the kind is Video.
    pub video_path: String,
    /// How an image or a video fills its rectangle.
    pub fit: BackgroundFit,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self { kind: BackgroundKind::None, color: DEFAULT_BACKGROUND_COLOR, image_path: String::new(), video_path: String::new(), fit: BackgroundFit::Cover }
    }
}

impl From<&BackgroundConfig> for capture::BackgroundSettings {
    fn from(config: &BackgroundConfig) -> Self {
        Self {
            // The capture compositor knows no video kind; the video is handed to it frame by frame
            // and the colour is what shows behind or around it.
            kind: match config.kind {
                BackgroundKind::Video => capture::BackgroundKind::Color,
                kind => kind.into(),
            },
            color: config.color,
            image_path: config.image_path.clone(),
            fit: config.fit.into(),
        }
    }
}

impl From<BackgroundKind> for capture::BackgroundKind {
    fn from(kind: BackgroundKind) -> Self {
        match kind {
            BackgroundKind::None => Self::None,
            // The capture compositor draws a video background on top of the colour.
            BackgroundKind::Color | BackgroundKind::Video => Self::Color,
            BackgroundKind::Image => Self::Image,
        }
    }
}

impl From<BackgroundFit> for capture::BackgroundFit {
    fn from(fit: BackgroundFit) -> Self {
        match fit {
            BackgroundFit::Cover => Self::Cover,
            BackgroundFit::Contain => Self::Contain,
            BackgroundFit::Stretch => Self::Stretch,
        }
    }
}

/// One capture and encode profile, stored as profiles\{name}.json.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Profile {
    /// Display name, also the file name.
    pub name: String,
    /// Video codec.
    pub codec: Codec,
    /// Rate control mode and its numbers.
    pub rate_control: RateControl,
    /// Speed and quality preset; profile files without one are Balanced.
    #[serde(default = "default_preset")]
    pub preset: Preset,
    /// Seconds between keyframes; profile files without one use 2.
    #[serde(default = "default_keyframe_secs")]
    pub keyframe_secs: f32,
    /// Output width in pixels; 0 means the size of the captured monitor.
    pub width: u32,
    /// Output height in pixels; 0 means the size of the captured monitor.
    pub height: u32,
    /// Output frames per second.
    pub fps: u32,
    /// File container; profile files without one are MKV.
    #[serde(default)]
    pub container: Container,
    /// Write each audio track as its own AAC track.
    pub aac_tracks: bool,
    /// Also write a mixdown track.
    pub mixdown: bool,
    /// The webcam settings; profile files without them have the webcam off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webcam: Option<WebcamSettings>,
    /// Which pictures the composite shows; profile files without one use Screen only.
    #[serde(default)]
    pub layout: LayoutKind,
    /// The camera's place and look in the Pip layout.
    #[serde(default)]
    pub pip: PipConfig,
    /// The background of the Cam only layout and of the picture in picture camera.
    #[serde(default)]
    pub background: BackgroundConfig,
    /// The Cam only background of an old profile file, migrated into `background.color` on load;
    /// always None in a profile that lives in memory.
    #[serde(default, skip_serializing)]
    pub cam_only_bg: Option<[u8; 3]>,
    /// Whether the camera is baked into the main video; profile files without it are Baked. A take
    /// follows [`Profile::effective_composite_mode`], not this field alone.
    #[serde(default)]
    pub composite_mode: CompositeMode,
    /// What happens to the mouse cursor; profile files without one show it.
    pub cursor: CursorMode,
    /// With Track: whether mouse buttons and the wheel are logged.
    pub record_clicks: bool,
    /// With Track: whether keyboard shortcuts are logged. Opt-in.
    pub record_keys: bool,
    /// With Track: whether foreground app changes are logged.
    pub record_apps: bool,
}

/// What a recording does with the mouse cursor (input track spec, Settings).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorMode {
    /// Burned into the video.
    #[default]
    Show,
    /// Left out of the video.
    Hide,
    /// Left out of the video and written to the take's events file.
    Track,
}

/// The default of the switches that start on.
fn on() -> bool {
    true
}

/// The shape of a profile file, so the old `cam_only_bg` key can be read and migrated.
#[derive(Deserialize)]
struct ProfileFile {
    name: String,
    codec: Codec,
    rate_control: RateControl,
    #[serde(default = "default_preset")]
    preset: Preset,
    #[serde(default = "default_keyframe_secs")]
    keyframe_secs: f32,
    width: u32,
    height: u32,
    fps: u32,
    #[serde(default)]
    container: Container,
    aac_tracks: bool,
    mixdown: bool,
    #[serde(default)]
    webcam: Option<WebcamSettings>,
    #[serde(default)]
    layout: LayoutKind,
    #[serde(default)]
    pip: PipConfig,
    #[serde(default)]
    background: BackgroundConfig,
    #[serde(default)]
    cam_only_bg: Option<[u8; 3]>,
    #[serde(default)]
    composite_mode: CompositeMode,
    #[serde(default)]
    cursor: CursorMode,
    #[serde(default = "on")]
    record_clicks: bool,
    #[serde(default)]
    record_keys: bool,
    #[serde(default = "on")]
    record_apps: bool,
}

impl<'de> Deserialize<'de> for Profile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let file = ProfileFile::deserialize(deserializer)?;
        let mut profile = Profile {
            name: file.name,
            codec: file.codec,
            rate_control: file.rate_control,
            preset: file.preset,
            keyframe_secs: file.keyframe_secs,
            width: file.width,
            height: file.height,
            fps: file.fps,
            container: file.container,
            aac_tracks: file.aac_tracks,
            mixdown: file.mixdown,
            webcam: file.webcam,
            layout: file.layout,
            pip: file.pip,
            background: file.background,
            cam_only_bg: None,
            composite_mode: file.composite_mode,
            cursor: file.cursor,
            record_clicks: file.record_clicks,
            record_keys: file.record_keys,
            record_apps: file.record_apps,
        };
        // An old profile's Cam only colour moves into the new background block, so nothing is lost.
        if let Some(bg) = file.cam_only_bg {
            profile.background.color = bg;
        }
        Ok(profile)
    }
}

impl Profile {
    /// Shipped profile: AV1 constant quality, 2560x1440 at 60 fps, MKV.
    pub fn tutorial_av1_1440p60() -> Self {
        Self {
            name: DEFAULT_PROFILE.to_owned(),
            codec: Codec::Av1,
            rate_control: RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 },
            preset: Preset::Balanced,
            keyframe_secs: DEFAULT_KEYFRAME_SECS,
            width: 2560,
            height: 1440,
            fps: 60,
            container: Container::Mkv,
            aac_tracks: true,
            mixdown: true,
            webcam: None,
            layout: LayoutKind::default(),
            pip: PipConfig::default(),
            background: BackgroundConfig::default(),
            cam_only_bg: None,
            composite_mode: CompositeMode::Baked,
            cursor: CursorMode::Show,
            record_clicks: true,
            record_keys: false,
            record_apps: true,
        }
    }

    /// Shipped profile: HEVC VBR with a peak, 1920x1080 at 60 fps, MP4.
    pub fn premiere_hevc_1080p60() -> Self {
        Self {
            name: "Premiere HEVC 1080p60".to_owned(),
            codec: Codec::Hevc,
            rate_control: RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 },
            preset: Preset::Balanced,
            keyframe_secs: DEFAULT_KEYFRAME_SECS,
            width: 1920,
            height: 1080,
            fps: 60,
            container: Container::Mp4,
            aac_tracks: true,
            mixdown: true,
            webcam: None,
            layout: LayoutKind::default(),
            pip: PipConfig::default(),
            background: BackgroundConfig::default(),
            cam_only_bg: None,
            composite_mode: CompositeMode::Baked,
            cursor: CursorMode::Show,
            record_clicks: true,
            record_keys: false,
            record_apps: true,
        }
    }

    /// Shipped profile: AV1 constant quality, Cam only at 1920x1080 and 30 fps, MKV.
    pub fn cam_only_av1_1080p30() -> Self {
        Self {
            name: "Cam only AV1".to_owned(),
            codec: Codec::Av1,
            rate_control: RateControl::ConstantQuality { quality: 70, max_mbps: 30.0 },
            preset: Preset::Balanced,
            keyframe_secs: DEFAULT_KEYFRAME_SECS,
            width: 1920,
            height: 1080,
            fps: 30,
            container: Container::Mkv,
            aac_tracks: true,
            mixdown: true,
            webcam: None,
            layout: LayoutKind::CamOnly,
            pip: PipConfig::default(),
            background: BackgroundConfig { kind: BackgroundKind::Color, ..BackgroundConfig::default() },
            cam_only_bg: None,
            composite_mode: CompositeMode::Baked,
            cursor: CursorMode::Show,
            record_clicks: true,
            record_keys: false,
            record_apps: true,
        }
    }

    /// The three profiles that ship with the app.
    pub fn shipped() -> [Self; 3] {
        [Self::tutorial_av1_1440p60(), Self::premiere_hevc_1080p60(), Self::cam_only_av1_1080p30()]
    }

    /// The mode a take of this profile records in: Clean only when it is chosen, the layout is Picture in
    /// picture and the webcam is on; otherwise Baked.
    pub fn effective_composite_mode(&self) -> CompositeMode {
        match (self.composite_mode, self.layout, &self.webcam) {
            (CompositeMode::Clean, LayoutKind::Pip, Some(_)) => CompositeMode::Clean,
            _ => CompositeMode::Baked,
        }
    }

    /// Whether the output size is the size of the captured monitor.
    pub fn is_source_size(&self) -> bool {
        self.width == 0 || self.height == 0
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
    let shipped_written_before = settings.shipped_written.clone();
    if saveable {
        // An older settings file that held one profile gives it up to a profile file, once: the next write drops the key.
        let text = fs::read_to_string(&settings_path).unwrap_or_default();
        if let Some(name) = profiles::migrate_legacy(&text, &profiles)? {
            settings.active_profile = name;
            write_json(&settings_path, &settings)?;
        }
    }
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
    // A broken settings file cannot keep the written list, so its additions are not remembered.
    let mut written = std::mem::take(&mut settings.shipped_written);
    profiles::init(&profiles, &mut written)?;
    if saveable {
        settings.shipped_written = written;
        if settings.shipped_written != shipped_written_before {
            write_json(&settings_path, &settings)?;
        }
    }
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
                    settings.panel_widths.clamp_to_ranges();
                    let offered = u32::try_from(settings.replay_length().as_secs()).unwrap_or(DEFAULT_REPLAY_LENGTH_S);
                    if settings.replay_length_s != offered {
                        log::warn!("a replay length of {} s is not offered, using {offered} s", settings.replay_length_s);
                        settings.replay_length_s = offered;
                    }
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

/// Writes `value` as pretty JSON through a temp file in the same folder.
pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigError> {
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
mod mode_tests {
    use super::*;

    #[test]
    fn starting_a_recording_from_any_mode_switches_to_record() {
        let mut setup = ModeState::new(Mode::Setup);
        setup.start();
        assert_eq!(setup.mode, Mode::Record);
        let mut review = ModeState::new(Mode::Review);
        review.start();
        assert_eq!(review.mode, Mode::Record);
        let mut record = ModeState::new(Mode::Record);
        record.start();
        assert_eq!(record.mode, Mode::Record);
    }

    #[test]
    fn stopping_returns_to_the_mode_that_was_open_before() {
        let mut setup = ModeState::new(Mode::Setup);
        setup.start();
        setup.stop();
        assert_eq!(setup.mode, Mode::Setup);
        let mut review = ModeState::new(Mode::Review);
        review.start();
        review.stop();
        assert_eq!(review.mode, Mode::Review);
        // Stopping twice does nothing; the mode stays where it came back to.
        setup.stop();
        assert_eq!(setup.mode, Mode::Setup);
    }

    #[test]
    fn setup_and_review_are_disabled_while_a_take_runs() {
        let state = ModeState::new(Mode::Setup);
        assert!(!state.can_switch(true, Mode::Setup), "Setup is disabled while recording");
        assert!(!state.can_switch(true, Mode::Review), "Review is disabled while recording");
        assert!(state.can_switch(true, Mode::Record), "Record stays selectable");
        assert!(state.can_switch(false, Mode::Setup));
        assert!(state.can_switch(false, Mode::Record));
        assert!(state.can_switch(false, Mode::Review));
    }

    #[test]
    fn the_mode_is_persisted_with_the_settings() -> Result<(), Box<dyn std::error::Error>> {
        let mut settings = Settings::default();
        assert_eq!(settings.mode, Mode::Setup, "a fresh install opens in Setup");
        settings.mode = Mode::Review;
        let text = serde_json::to_string(&settings)?;
        let back: Settings = serde_json::from_str(&text)?;
        assert_eq!(back.mode, Mode::Review);
        // A settings file from before the mode existed opens in Setup.
        let old: Settings = serde_json::from_str("{}")?;
        assert_eq!(old.mode, Mode::Setup);
        Ok(())
    }

    #[test]
    fn the_panel_widths_are_persisted_per_mode_with_the_settings() -> Result<(), Box<dyn std::error::Error>> {
        let mut settings = Settings::default();
        assert_eq!(settings.panel_widths.setup, PanelWidths::default(), "a fresh install uses the default widths");
        *settings.panel_widths.entry(Mode::Setup) = PanelWidths { left: 300.0, right: 420.0 };
        *settings.panel_widths.entry(Mode::Record) = PanelWidths { left: 220.0, right: 320.0 };
        *settings.panel_widths.entry(Mode::Review) = PanelWidths { left: 260.0, right: 340.0 };
        let text = serde_json::to_string(&settings)?;
        let back: Settings = serde_json::from_str(&text)?;
        assert_eq!(back.panel_widths.get(Mode::Setup), PanelWidths { left: 300.0, right: 420.0 });
        assert_eq!(back.panel_widths.get(Mode::Record), PanelWidths { left: 220.0, right: 320.0 });
        assert_eq!(back.panel_widths.get(Mode::Review), PanelWidths { left: 260.0, right: 340.0 });
        // A settings file from before the widths existed gets the defaults.
        let old: Settings = serde_json::from_str("{}")?;
        assert_eq!(old.panel_widths, ModeWidths::default());
        Ok(())
    }
}

#[cfg(test)]
mod webcam_tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn empty_webcam_settings_get_the_defaults() -> Result<(), Box<dyn Error>> {
        let settings: WebcamSettings = serde_json::from_str("{}")?;
        assert_eq!(settings, WebcamSettings::default());
        assert!(settings.iso);
        assert_eq!(settings.iso_rc, RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 30.0 });
        assert_eq!(settings.offset_ms, 0);
        assert_eq!(settings.controls, WebcamControls::default());
        Ok(())
    }

    #[test]
    fn a_partial_webcam_block_keeps_the_other_defaults() -> Result<(), Box<dyn Error>> {
        let settings: WebcamSettings = serde_json::from_str(r#"{ "camera_id": "usb#1", "offset_ms": -40, "format": { "width": 1920 } }"#)?;
        assert_eq!(settings.camera_id, "usb#1");
        assert_eq!(settings.offset_ms, -40);
        assert_eq!(settings.format.width, 1920);
        assert_eq!(settings.format.height, 720);
        assert!(settings.iso);
        Ok(())
    }

    #[test]
    fn control_settings_serialize_as_auto_or_a_manual_value() -> Result<(), Box<dyn Error>> {
        assert_eq!(serde_json::to_string(&ControlSetting::Auto)?, "\"auto\"");
        assert_eq!(serde_json::to_string(&ControlSetting::Manual(-6))?, "{\"manual\":-6}");
        let controls = WebcamControls { exposure: Some(ControlSetting::Manual(-6)), gain: Some(ControlSetting::Auto), ..WebcamControls::default() };
        let text = serde_json::to_string(&controls)?;
        assert!(!text.contains("focus"), "{text}");
        assert_eq!(serde_json::from_str::<WebcamControls>(&text)?, controls);
        Ok(())
    }

    #[test]
    fn controls_are_read_and_written_by_property() {
        let mut controls = WebcamControls::default();
        for (index, prop) in ControlProp::ALL.into_iter().enumerate() {
            assert_eq!(controls.get(prop), None);
            controls.set(prop, Some(ControlSetting::Manual(index as i32)));
        }
        assert_eq!(controls.get(ControlProp::Brightness), Some(ControlSetting::Manual(4)));
        controls.set(ControlProp::Brightness, None);
        assert_eq!(controls.get(ControlProp::Brightness), None);
    }

    #[test]
    fn a_profile_file_from_before_the_webcam_still_loads() -> Result<(), Box<dyn Error>> {
        let mut value = serde_json::to_value(Profile::tutorial_av1_1440p60())?;
        assert!(value.as_object_mut().is_some_and(|object| object.remove("webcam").is_none()), "a profile without a webcam writes no webcam key");
        let profile: Profile = serde_json::from_value(value)?;
        assert_eq!(profile, Profile::tutorial_av1_1440p60());
        assert_eq!(profile.webcam, None);
        Ok(())
    }

    #[test]
    fn a_profile_with_webcam_settings_round_trips() -> Result<(), Box<dyn Error>> {
        let webcam = WebcamSettings { camera_name: "Cam".to_owned(), offset_ms: 120, ..WebcamSettings::default() };
        let profile = Profile { webcam: Some(webcam), ..Profile::premiere_hevc_1080p60() };
        let back: Profile = serde_json::from_str(&serde_json::to_string(&profile)?)?;
        assert_eq!(back, profile);
        Ok(())
    }

    #[test]
    fn the_offset_is_limited_to_half_a_second() {
        let late = WebcamSettings { offset_ms: 900, ..WebcamSettings::default() };
        let early = WebcamSettings { offset_ms: -900, ..WebcamSettings::default() };
        assert_eq!(late.offset_clamped(), 500);
        assert_eq!(early.offset_clamped(), -500);
    }
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
        let expected = ["Tutorial AV1 1440p60", "Premiere HEVC 1080p60", "Cam only AV1"];
        assert_eq!(settings.shipped_written, expected.map(str::to_owned));
        assert_eq!(settings, Settings { shipped_written: expected.map(str::to_owned).to_vec(), ..Settings::default() });

        let on_disk: Settings = serde_json::from_str(&fs::read_to_string(dir.join("settings.json"))?)?;
        assert_eq!(on_disk, settings);

        let names: Vec<String> = fs::read_dir(dir.join("profiles"))?.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.iter().all(|name| name.ends_with(".json")), "{names:?}");

        let first: Profile = serde_json::from_str(&fs::read_to_string(dir.join("profiles").join("Tutorial AV1 1440p60.json"))?)?;
        // New installs get MP4 for Tutorial AV1; Cam only AV1 gets MP4 or MKV depending on this GPU's probe.
        assert_eq!(first, Profile { container: Container::Mp4, ..Profile::tutorial_av1_1440p60() });
        let cam_only: Profile = serde_json::from_str(&fs::read_to_string(dir.join("profiles").join("Cam only AV1.json"))?)?;
        assert_eq!(Profile { container: Container::Mkv, ..cam_only }, Profile::cam_only_av1_1080p30());
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
        assert!(settings.hide_countdown_when_linked);
        assert_eq!(Settings::default().countdown_secs, 0);
        assert_eq!(settings.api, ApiSettings { enabled: true, port: 5000 });
        assert_eq!(settings.naming_template, "{date}_{time}_take{take}");
        assert_eq!(settings.project, "Cutmark");
        assert!(settings.hide_from_capture);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_replay_length_round_trips_and_only_offered_lengths_are_kept() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("replay-length")?;
        let path = dir.join("settings.json");
        assert_eq!(Settings::default().replay_length(), Duration::from_secs(60));
        for seconds in REPLAY_LENGTHS_S {
            write_json(&path, &Settings { replay_length_s: seconds, ..Settings::default() })?;
            let (loaded, _) = load_settings(&path)?;
            assert_eq!((loaded.replay_length_s, loaded.replay_length()), (seconds, Duration::from_secs(u64::from(seconds))));
        }
        for text in [r#"{}"#, r#"{ "replay_length_s": 45 }"#, r#"{ "replay_length_s": 0 }"#] {
            fs::write(&path, text)?;
            assert_eq!(load_settings(&path)?.0.replay_length_s, 60, "{text}");
        }
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_folder_settings_round_trip_and_old_files_load_with_the_defaults() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("folders")?;
        let path = dir.join("settings.json");
        // An old settings file without the folder keys loads with the defaults and keeps its other values.
        fs::write(&path, r#"{ "countdown_secs": 3 }"#)?;
        let (old, _) = load_settings(&path)?;
        assert_eq!(old.root_folder, None);
        assert_eq!(old.replays_root, None);
        assert!(!old.folders_confirmed);
        assert_eq!(old.countdown_secs, 3);
        // Both folders set and confirmed round-trip through a save and a load.
        let settings =
            Settings { root_folder: Some(dir.join("recordings")), replays_root: Some(dir.join("replays")), folders_confirmed: true, ..Settings::default() };
        write_json(&path, &settings)?;
        assert_eq!(load_settings(&path)?.0, settings);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_profile_file_without_a_container_loads_as_mkv() -> Result<(), Box<dyn Error>> {
        let mut value = serde_json::to_value(Profile::premiere_hevc_1080p60())?;
        value.as_object_mut().ok_or("a profile is a JSON object")?.remove("container");
        let profile: Profile = serde_json::from_value(value)?;
        assert_eq!(profile.container, Container::Mkv);
        Ok(())
    }

    #[test]
    fn existing_profiles_are_left_alone() -> Result<(), Box<dyn Error>> {
        let dir = fresh_dir("existing-profiles")?;
        fs::create_dir_all(dir.join("profiles"))?;
        write_json(&dir.join("profiles").join("Mine.json"), &Profile::premiere_hevc_1080p60())?;
        // An existing install gets the newly shipped profile once; the user's own file is untouched.
        load_or_init_in(&dir, no_devices)?;
        let names: Vec<String> = fs::read_dir(dir.join("profiles"))?.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"Mine.json".to_owned()), "{names:?}");
        assert!(names.contains(&"Cam only AV1.json".to_owned()), "{names:?}");
        // The written list is in the settings, so a second run adds nothing more.
        load_or_init_in(&dir, no_devices)?;
        assert_eq!(fs::read_dir(dir.join("profiles"))?.count(), 2);
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
    fn an_app_track_round_trips_and_device_kinds_keep_their_saved_names() -> Result<(), Box<dyn Error>> {
        let kind = TrackKind::App { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned() };
        let track = TrackSettings { name: "Discord".to_owned(), kind, ..TrackSettings::default() };
        let audio = AudioSettings { tracks: vec![track], ..AudioSettings::default() };
        let text = serde_json::to_string(&audio)?;
        assert_eq!(serde_json::from_str::<AudioSettings>(&text)?, audio);
        assert!(text.contains(r#""kind":{"app":{"exe":"discord.exe","app_name":"Discord"}}"#), "{text}");
        let except = TrackKind::DesktopExcept { exe: "spotify.exe".to_owned(), app_name: "Spotify".to_owned() };
        let text = serde_json::to_string(&except)?;
        assert_eq!(text, r#"{"desktop_except":{"exe":"spotify.exe","app_name":"Spotify"}}"#);
        assert_eq!(serde_json::from_str::<TrackKind>(&text)?, except);
        assert_eq!(serde_json::to_string(&TrackKind::Input)?, "\"input\"");
        assert_eq!(serde_json::to_string(&TrackKind::Output)?, "\"output\"");
        Ok(())
    }

    #[test]
    fn loaded_panel_widths_are_clamped_into_their_ranges() -> Result<(), Box<dyn std::error::Error>> {
        let dir = fresh_dir("panel-width-clamp")?;
        let path = dir.join("settings.json");
        let mut settings = Settings::default();
        *settings.panel_widths.entry(Mode::Setup) = PanelWidths { left: 5000.0, right: 10.0 };
        fs::write(&path, serde_json::to_string(&settings)?)?;
        let (loaded, _) = load_settings(&path)?;
        assert_eq!(loaded.panel_widths.get(Mode::Setup), PanelWidths { left: LEFT_PANEL_MAX, right: RIGHT_PANEL_MIN });
        assert_eq!(loaded.panel_widths.get(Mode::Record), PanelWidths::default());
        fs::remove_dir_all(&dir)?;
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

#[cfg(test)]
mod layout_tests {
    use std::error::Error;

    use super::*;

    #[test]
    fn a_profile_file_from_before_the_layouts_still_loads() -> Result<(), Box<dyn Error>> {
        let mut value = serde_json::to_value(Profile::tutorial_av1_1440p60())?;
        let object = value.as_object_mut().ok_or("a profile is a JSON object")?;
        for key in ["layout", "pip", "background"] {
            assert!(object.remove(key).is_some(), "a profile writes the {key} key");
        }
        // The old Cam only colour key of an even older profile file migrates into the background block.
        object.insert("cam_only_bg".to_owned(), serde_json::json!([10, 20, 30]));
        let profile: Profile = serde_json::from_value(value)?;
        assert_eq!(profile, Profile { background: BackgroundConfig { color: [10, 20, 30], ..BackgroundConfig::default() }, ..Profile::tutorial_av1_1440p60() });
        assert_eq!(profile.layout, LayoutKind::ScreenOnly);
        assert_eq!(profile.background.color, [10, 20, 30]);
        Ok(())
    }

    #[test]
    fn clean_applies_only_to_picture_in_picture_with_the_webcam_on() {
        for layout in [LayoutKind::ScreenOnly, LayoutKind::Pip, LayoutKind::CamOnly] {
            for webcam_on in [false, true] {
                for composite_mode in [CompositeMode::Baked, CompositeMode::Clean] {
                    let webcam = webcam_on.then(WebcamSettings::default);
                    let profile = Profile { layout, webcam, composite_mode, ..Profile::tutorial_av1_1440p60() };
                    let clean = layout == LayoutKind::Pip && webcam_on && composite_mode == CompositeMode::Clean;
                    let expected = if clean { CompositeMode::Clean } else { CompositeMode::Baked };
                    assert_eq!(profile.effective_composite_mode(), expected, "{layout:?}, webcam {webcam_on}, {composite_mode:?}");
                }
            }
        }
    }

    #[test]
    fn a_profile_file_without_a_composite_mode_is_baked_and_the_mode_round_trips() -> Result<(), Box<dyn Error>> {
        let mut value = serde_json::to_value(Profile::tutorial_av1_1440p60())?;
        value.as_object_mut().ok_or("a profile is an object")?.remove("composite_mode");
        assert_eq!(serde_json::from_value::<Profile>(value)?.composite_mode, CompositeMode::Baked);
        let profile = Profile { composite_mode: CompositeMode::Clean, ..Profile::tutorial_av1_1440p60() };
        let text = serde_json::to_string(&profile)?;
        assert!(text.contains("\"composite_mode\":\"clean\""), "{text}");
        assert_eq!(serde_json::from_str::<Profile>(&text)?, profile);
        Ok(())
    }

    #[test]
    fn the_default_pip_is_bottom_right_with_two_percent_offsets() {
        let pip = PipConfig::default();
        assert_eq!(pip.anchor, AnchorCorner::BottomRight);
        assert_eq!((pip.offset_x_pct, pip.offset_y_pct, pip.scale_pct), (2.0, 2.0, 25.0));
        assert_eq!((pip.crop_left_pct, pip.crop_right_pct, pip.crop_top_pct, pip.crop_bottom_pct), (0.0, 0.0, 0.0, 0.0));
        assert_eq!((pip.border_px, pip.radius_px), (0.0, 12.0));
    }

    #[test]
    fn a_partial_pip_block_keeps_the_other_defaults() -> Result<(), Box<dyn Error>> {
        let pip: PipConfig = serde_json::from_str(r#"{ "anchor": "top_left", "scale_pct": 40.0 }"#)?;
        assert_eq!(pip.anchor, AnchorCorner::TopLeft);
        assert_eq!(pip.scale_pct, 40.0);
        assert_eq!(pip.offset_x_pct, 2.0);
        assert_eq!(pip.radius_px, 12.0);
        Ok(())
    }

    #[test]
    fn a_profile_with_a_layout_round_trips() -> Result<(), Box<dyn Error>> {
        let profile = Profile {
            layout: LayoutKind::Pip,
            pip: PipConfig { anchor: AnchorCorner::TopRight, border_px: 6.0, border_color: [255, 0, 0], ..PipConfig::default() },
            background: BackgroundConfig {
                kind: BackgroundKind::Image,
                color: [10, 20, 30],
                image_path: "bg.png".to_owned(),
                video_path: String::new(),
                fit: BackgroundFit::Contain,
            },
            ..Profile::premiere_hevc_1080p60()
        };
        let text = serde_json::to_string(&profile)?;
        assert!(text.contains("\"layout\":\"pip\""), "{text}");
        assert!(text.contains("\"kind\":\"image\"") && text.contains("\"fit\":\"contain\""), "{text}");
        assert!(!text.contains("cam_only_bg"), "{text}");
        assert_eq!(serde_json::from_str::<Profile>(&text)?, profile);
        Ok(())
    }

    #[test]
    fn the_saved_pip_converts_to_the_compositor_settings() {
        let config = PipConfig { anchor: AnchorCorner::BottomLeft, crop_top_pct: 5.0, ..PipConfig::default() };
        let settings = capture::PipSettings::from(&config);
        assert_eq!(settings.anchor, capture::Corner::BottomLeft);
        assert_eq!(settings.crop_top_pct, 5.0);
        assert_eq!(capture::PipSettings::from(&PipConfig::default()), capture::PipSettings::default());
        assert_eq!(capture::Layout::from(LayoutKind::CamOnly), capture::Layout::CamOnly);
    }

    #[test]
    fn the_saved_background_converts_to_the_compositor_settings() {
        let config = BackgroundConfig {
            kind: BackgroundKind::Image,
            color: [1, 2, 3],
            image_path: "bg.png".to_owned(),
            video_path: String::new(),
            fit: BackgroundFit::Stretch,
        };
        let settings = capture::BackgroundSettings::from(&config);
        assert_eq!(settings.kind, capture::BackgroundKind::Image);
        assert_eq!(settings.color, [1, 2, 3]);
        assert_eq!(settings.image_path, "bg.png");
        assert_eq!(settings.fit, capture::BackgroundFit::Stretch);
        assert_eq!(capture::BackgroundSettings::from(&BackgroundConfig::default()), capture::BackgroundSettings::default());
    }

    #[test]
    fn a_video_background_reaches_the_compositor_as_a_colour() -> Result<(), Box<dyn Error>> {
        let config = BackgroundConfig { kind: BackgroundKind::Video, video_path: r"C:\clips\bg.mp4".to_owned(), ..BackgroundConfig::default() };
        let settings = capture::BackgroundSettings::from(&config);
        assert_eq!(settings.kind, capture::BackgroundKind::Color);
        let text = serde_json::to_string(&config)?;
        assert!(text.contains("\"kind\":\"video\"") && text.contains("bg.mp4"), "{text}");
        assert_eq!(serde_json::from_str::<BackgroundConfig>(&text)?, config);
        Ok(())
    }

    #[test]
    fn the_key_defaults_match_the_spec() -> Result<(), Box<dyn Error>> {
        let key: KeyConfig = serde_json::from_str("{}")?;
        assert_eq!(key, KeyConfig::default());
        assert!(!key.enabled && !key.mask_enabled);
        assert_eq!(key.color, [0, 177, 64]);
        assert_eq!((key.similarity, key.smoothness, key.spill), (0.40, 0.08, 0.10));
        assert_eq!((key.crop_left_pct, key.crop_right_pct, key.crop_top_pct, key.crop_bottom_pct), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(capture::KeySettings::from(&key), capture::KeySettings::default());
        Ok(())
    }

    #[test]
    fn a_partial_key_keeps_the_defaults_for_the_rest() -> Result<(), Box<dyn Error>> {
        let key: KeyConfig = serde_json::from_str(r#"{ "enabled": true, "similarity": 0.55, "mask_enabled": true, "mask_w_pct": 30.0 }"#)?;
        assert!(key.enabled && key.mask_enabled);
        assert_eq!((key.similarity, key.mask_w_pct), (0.55, 30.0));
        assert_eq!((key.smoothness, key.spill, key.color), (0.08, 0.10, [0, 177, 64]));
        Ok(())
    }

    #[test]
    fn an_old_webcam_profile_without_a_key_still_loads_with_the_key_off() -> Result<(), Box<dyn Error>> {
        let settings: WebcamSettings = serde_json::from_str(r#"{ "camera_id": "usb#1", "offset_ms": 40 }"#)?;
        assert_eq!(settings.key, KeyConfig::default());
        assert!(!settings.key.enabled);
        let text = serde_json::to_string(&WebcamSettings { key: KeyConfig { enabled: true, ..KeyConfig::default() }, ..WebcamSettings::default() })?;
        assert!(serde_json::from_str::<WebcamSettings>(&text)?.key.enabled);
        Ok(())
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn a_profile_from_before_the_cursor_settings_shows_the_cursor() -> Result<(), serde_json::Error> {
        let mut old = serde_json::to_value(Profile::tutorial_av1_1440p60())?;
        if let Some(fields) = old.as_object_mut() {
            for key in ["cursor", "record_clicks", "record_keys", "record_apps"] {
                assert!(fields.remove(key).is_some(), "{key} is saved");
            }
        }
        let profile: Profile = serde_json::from_value(old)?;
        assert_eq!(profile.cursor, CursorMode::Show);
        assert_eq!((profile.record_clicks, profile.record_keys, profile.record_apps), (true, false, true));
        Ok(())
    }
}
