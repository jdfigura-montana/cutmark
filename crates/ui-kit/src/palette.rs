//! The suite's shared dark palette. Every value is copied exactly from the
//! audio editor's `Theme`, so any app built on ui-kit reads as the same
//! product. App-specific colors (clips, lanes, waveforms, track colors) do
//! not belong here.

use egui::Color32;

// ---- Surfaces ----

/// The app background behind every panel.
pub const APP_BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1C);

/// Panel, window and central-area background.
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x22);

/// Menu bar and status bar background.
pub const BAR_BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x18);

/// Recessed surface (the audio editor's unlit meter bars and fader slots);
/// egui's extreme background color.
pub const RECESSED: Color32 = Color32::from_rgb(0x0E, 0x10, 0x13);

/// Faint alternate shade for striped rows (the audio editor's odd lane
/// shade); egui's faint background color.
pub const FAINT_BG: Color32 = Color32::from_rgb(0x1E, 0x21, 0x27);

/// General separator and window border color.
pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x27, 0x2E);

// ---- Controls ----

/// Border color of controls.
pub const CONTROL_BORDER: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Fill of an idle control.
pub const CONTROL_FILL: Color32 = Color32::from_rgb(0x22, 0x26, 0x2D);

/// Fill of a hovered control.
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Corner rounding of controls.
pub const CONTROL_ROUNDING: f32 = 6.0;

/// Track color of an egui slider.
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Filled part of an egui slider.
pub const SLIDER_FILL: Color32 = Color32::from_rgb(0x5A, 0x61, 0x6D);

/// Thumb color of an egui slider.
pub const SLIDER_THUMB: Color32 = Color32::from_rgb(0xD7, 0xDA, 0xE0);

// ---- Text ----

/// Brightest text (strong text, headings, pressed controls).
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE6, 0xE9, 0xEE);

/// Default text color.
pub const TEXT_NORMAL: Color32 = Color32::from_rgb(0xC9, 0xCE, 0xD6);

/// Muted secondary text.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x90, 0x9A);

// ---- Accent and signal colors ----

/// The single accent color: selection, lit toggles, range lines and focus.
pub const ACCENT: Color32 = Color32::from_rgb(0x4C, 0x9B, 0xE8);

/// Red of an armed record button and of record state generally.
pub const RECORD_RED: Color32 = Color32::from_rgb(0xE5, 0x54, 0x4B);

/// Green of the transport play icon.
pub const PLAY_GREEN: Color32 = Color32::from_rgb(0x5C, 0xC4, 0x6E);

// ---- Meters ----

/// Meter bar color below -18 dBFS.
pub const METER_GREEN: Color32 = Color32::from_rgb(0x3D, 0xBE, 0x6B);

/// Meter bar color above -18 dBFS.
pub const METER_YELLOW: Color32 = Color32::from_rgb(0xE3, 0xC0, 0x4A);

/// Meter bar color above -6 dBFS.
pub const METER_RED: Color32 = Color32::from_rgb(0xE5, 0x54, 0x4B);
