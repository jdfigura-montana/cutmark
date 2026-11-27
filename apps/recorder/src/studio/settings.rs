//! The Studio settings file, `{stem}.studio.json` (spec "Studio settings file"). It is part of the
//! take's file group. A missing field takes its default, an unknown one is ignored, and a number
//! outside its range is brought back into it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::autozoom::{MAX_LEVEL, MIN_LEVEL, ZoomSegment};

/// The version this build writes.
pub const SETTINGS_VERSION: u32 = 1;

/// The end of the settings file's name, after the take's stem.
pub const SETTINGS_SUFFIX: &str = ".studio.json";

/// The end of the name of the take's Studio export, after the take's stem.
pub const EXPORT_SUFFIX: &str = ".studio.mp4";

const WHITE: [u8; 3] = [255, 255, 255];

/// Why a Studio settings file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The file is there but cannot be read.
    #[error("{} could not be read: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },
    /// The file is not a settings file. It is left as it is.
    #[error("{} is not a Studio settings file: {source}", path.display())]
    Parse { path: PathBuf, source: serde_json::Error },
    /// The settings could not be turned into JSON.
    #[error("the Studio settings could not be encoded: {0}")]
    Encode(serde_json::Error),
    /// The file could not be written; the message names it.
    #[error("{0}")]
    Write(String),
}

/// How much the drawn cursor is smoothed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Smoothing {
    /// The cursor as recorded.
    Off,
    #[default]
    Light,
    Strong,
}

/// What a click looks like.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickEffect {
    None,
    #[default]
    Ring,
    Ripple,
}

/// What marks the cursor's place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    #[default]
    Off,
    /// A soft circle.
    Spotlight,
}

/// The Cursor panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorSettings {
    /// Cursor size in percent, 100 to 300.
    pub size_percent: u32,
    pub smoothing: Smoothing,
    pub click_effect: ClickEffect,
    /// The click effect's color, RGB.
    pub click_color: [u8; 3],
    /// The click effect's size in percent of its normal size, 50 to 300.
    pub click_size_percent: u32,
    pub highlight: Highlight,
    /// The spotlight's radius in pixels at 1080p, 20 to 400.
    pub highlight_radius_px: u32,
    /// Hide the cursor after this many idle seconds, 1 to 10; None never hides it.
    pub hide_idle_s: Option<f32>,
    pub visible: bool,
}

impl Default for CursorSettings {
    fn default() -> Self {
        Self {
            size_percent: 150,
            smoothing: Smoothing::Light,
            click_effect: ClickEffect::Ring,
            click_color: WHITE,
            click_size_percent: 100,
            highlight: Highlight::Off,
            highlight_radius_px: 80,
            hide_idle_s: Some(3.0),
            visible: true,
        }
    }
}

/// The Zoom panel and the zoom lane.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomSettings {
    pub auto: bool,
    /// The level auto-zoom gives its segments, 1.25 to 4.
    pub amount: f32,
    /// How long a segment takes to ease in, and to ease out, in seconds, 0.3 to 1.2.
    pub speed_s: f32,
    /// The zoom lane, in time order.
    pub segments: Vec<ZoomSegment>,
}

impl Default for ZoomSettings {
    fn default() -> Self {
        Self { auto: true, amount: 2.0, speed_s: 0.6, segments: Vec::new() }
    }
}

/// Where the shortcut overlay sits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeysPosition {
    #[default]
    BottomCenter,
    BottomLeft,
    BottomRight,
    TopCenter,
}

/// How large the shortcut overlay is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeysSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// The Keys panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeysSettings {
    pub show: bool,
    pub position: KeysPosition,
    pub size: KeysSize,
    /// How long a shortcut stays, in seconds, 0.8 to 3.
    pub duration_s: f32,
}

impl Default for KeysSettings {
    fn default() -> Self {
        Self { show: true, position: KeysPosition::BottomCenter, size: KeysSize::Medium, duration_s: 1.5 }
    }
}

/// What fills the space around the framed picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundKind {
    Solid,
    #[default]
    Gradient,
    Image,
}

/// The shadow under the framed picture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shadow {
    Off,
    #[default]
    Soft,
    Strong,
}

/// The Frame panel. Each background kind keeps its own value, so switching kinds loses nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameSettings {
    pub enabled: bool,
    pub background: BackgroundKind,
    /// The solid background's color, RGB.
    pub solid_color: [u8; 3],
    /// The gradient preset, 1 to 6.
    pub gradient_preset: u8,
    /// The background image; None until one is picked.
    pub image: Option<PathBuf>,
    /// Padding in percent, 0 to 15.
    pub padding_percent: f32,
    /// Corner radius in pixels at 1080p, 0 to 32.
    pub corner_radius_px: u32,
    pub shadow: Shadow,
}

impl Default for FrameSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            background: BackgroundKind::Gradient,
            solid_color: [0, 0, 0],
            gradient_preset: 1,
            image: None,
            padding_percent: 6.0,
            corner_radius_px: 12,
            shadow: Shadow::Soft,
        }
    }
}

/// The export's picture size; the aspect is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputSize {
    #[default]
    #[serde(rename = "source")]
    Source,
    #[serde(rename = "2160p")]
    P2160,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "1080p")]
    P1080,
}

/// The Export panel. The codec, rate control and frame rate come from the take, and its audio is
/// copied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    pub output_size: OutputSize,
    /// Quality steps above the take's own, 0 or more.
    pub quality_steps: u8,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self { output_size: OutputSize::Source, quality_steps: 1 }
    }
}

/// Everything Studio keeps about one take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StudioSettings {
    pub version: u32,
    pub cursor: CursorSettings,
    pub zoom: ZoomSettings,
    pub keys: KeysSettings,
    pub frame: FrameSettings,
    pub export: ExportSettings,
}

impl Default for StudioSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            cursor: CursorSettings::default(),
            zoom: ZoomSettings::default(),
            keys: KeysSettings::default(),
            frame: FrameSettings::default(),
            export: ExportSettings::default(),
        }
    }
}

/// What [`load`] found.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: StudioSettings,
    /// True when the take had no settings file. This is its first time in Studio, so auto-zoom has
    /// yet to run. A saved file with no segments is the user's choice and stays empty.
    pub first_open: bool,
}

/// The settings file of the take whose main file is `video`.
pub fn settings_path(video: &Path) -> PathBuf {
    let mut name = video.file_stem().unwrap_or_default().to_os_string();
    name.push(SETTINGS_SUFFIX);
    video.with_file_name(name)
}

/// Reads a settings file. A missing one gives the defaults; a damaged one is an error, so that it
/// is not overwritten unnoticed.
pub fn load(path: &Path) -> Result<Loaded, SettingsError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Loaded { settings: StudioSettings::default(), first_open: true }),
        Err(source) => return Err(SettingsError::Read { path: path.to_owned(), source }),
    };
    let mut settings: StudioSettings = serde_json::from_slice(&bytes).map_err(|source| SettingsError::Parse { path: path.to_owned(), source })?;
    settings.sanitize();
    Ok(Loaded { settings, first_open: false })
}

/// Writes a settings file through a temp file that is renamed over it.
pub fn save(path: &Path, settings: &StudioSettings) -> Result<(), SettingsError> {
    let current = StudioSettings { version: SETTINGS_VERSION, ..settings.clone() };
    let mut text = serde_json::to_string_pretty(&current).map_err(SettingsError::Encode)?;
    text.push('\n');
    crate::review::write_file(path, text.as_bytes()).map_err(SettingsError::Write)
}

impl StudioSettings {
    /// Brings every number into its range, drops empty segments and puts the rest in time order.
    fn sanitize(&mut self) {
        let cursor = &mut self.cursor;
        cursor.size_percent = cursor.size_percent.clamp(100, 300);
        cursor.click_size_percent = cursor.click_size_percent.clamp(50, 300);
        cursor.highlight_radius_px = cursor.highlight_radius_px.clamp(20, 400);
        cursor.hide_idle_s = cursor.hide_idle_s.map(|seconds| seconds.clamp(1.0, 10.0));
        let zoom = &mut self.zoom;
        zoom.amount = zoom.amount.clamp(MIN_LEVEL, MAX_LEVEL);
        zoom.speed_s = zoom.speed_s.clamp(0.3, 1.2);
        zoom.segments.retain(|segment| segment.end_us > segment.start_us);
        for segment in &mut zoom.segments {
            segment.level = segment.level.clamp(MIN_LEVEL, MAX_LEVEL);
        }
        zoom.segments.sort_by_key(|segment| segment.start_us);
        self.keys.duration_s = self.keys.duration_s.clamp(0.8, 3.0);
        self.frame.gradient_preset = self.frame.gradient_preset.clamp(1, 6);
        self.frame.padding_percent = self.frame.padding_percent.clamp(0.0, 15.0);
        self.frame.corner_radius_px = self.frame.corner_radius_px.min(32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn fresh_dir(name: &str) -> io::Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("cutmark-studio-{name}-{}", std::process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn a_missing_file_gives_the_specs_defaults() -> TestResult {
        let dir = fresh_dir("missing")?;
        let loaded = load(&dir.join("take.studio.json"))?;
        assert!(loaded.first_open);
        let settings = loaded.settings;
        assert_eq!((settings.cursor.size_percent, settings.cursor.smoothing, settings.cursor.click_effect), (150, Smoothing::Light, ClickEffect::Ring));
        assert_eq!((settings.cursor.click_color, settings.cursor.highlight, settings.cursor.hide_idle_s), (WHITE, Highlight::Off, Some(3.0)));
        assert!(settings.cursor.visible && settings.zoom.auto && settings.zoom.segments.is_empty());
        assert_eq!((settings.zoom.amount, settings.zoom.speed_s), (2.0, 0.6));
        assert_eq!(
            (settings.keys.show, settings.keys.position, settings.keys.size, settings.keys.duration_s),
            (true, KeysPosition::BottomCenter, KeysSize::Medium, 1.5)
        );
        assert_eq!((settings.frame.enabled, settings.frame.background, settings.frame.gradient_preset), (false, BackgroundKind::Gradient, 1));
        assert_eq!((settings.frame.padding_percent, settings.frame.corner_radius_px, settings.frame.shadow), (6.0, 12, Shadow::Soft));
        assert_eq!((settings.export.output_size, settings.export.quality_steps), (OutputSize::Source, 1));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn settings_round_trip_through_the_file() -> TestResult {
        let dir = fresh_dir("round-trip")?;
        let path = dir.join("take.studio.json");
        for hide_idle_s in [Some(3.0), None] {
            let mut settings = StudioSettings::default();
            save(&path, &settings)?;
            assert_eq!(load(&path)?, Loaded { settings: settings.clone(), first_open: false });
            // "Off" has to survive, not fall back to the default of 3 s.
            settings.cursor.hide_idle_s = hide_idle_s;
            settings.export.output_size = OutputSize::P1080;
            settings.zoom.segments.push(ZoomSegment {
                start_us: 1,
                end_us: 2_000_000,
                level: 3.0,
                follow: false,
                fixed_center: Some((10.0, 20.0)),
                user_edited: true,
            });
            save(&path, &settings)?;
            assert_eq!(load(&path)?.settings, settings);
        }
        assert!(!dir.join("take.studio.json.tmp").exists());
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_or_wild_ones_are_repaired() -> TestResult {
        let dir = fresh_dir("tolerant")?;
        let path = dir.join("take.studio.json");
        let text = r#"{"version":9,"future":true,"cursor":{"size_percent":900,"trail":"long"},
            "zoom":{"segments":[{"start_us":5000000,"end_us":7000000,"level":9.0},{"start_us":3,"end_us":3},{"start_us":0,"end_us":2000000}]}}"#;
        fs::write(&path, text)?;
        let settings = load(&path)?.settings;
        assert_eq!((settings.cursor.size_percent, settings.cursor.smoothing), (300, Smoothing::Light));
        assert_eq!(settings.keys, KeysSettings::default());
        let lane: Vec<(i64, f32, bool)> = settings.zoom.segments.iter().map(|segment| (segment.start_us, segment.level, segment.follow)).collect();
        assert_eq!(lane, [(0, 2.0, true), (5_000_000, MAX_LEVEL, true)]);
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_damaged_file_is_an_error() -> TestResult {
        let dir = fresh_dir("damaged")?;
        let path = dir.join("take.studio.json");
        fs::write(&path, b"{\"version\":")?;
        assert!(matches!(load(&path), Err(SettingsError::Parse { .. })));
        fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn the_settings_file_sits_beside_the_video_under_its_stem() {
        assert_eq!(settings_path(Path::new("takes/show.v2_take01.mkv")), Path::new("takes/show.v2_take01.studio.json"));
    }
}
