//! The suite's shared dark palette. Every value is copied exactly from the
//! audio editor's `Theme`, so any app built on ui-kit reads as the same
//! product. App-specific colors (clips, lanes, waveforms, track colors) do
//! not belong here.
//!
//! Compatibility layer. New code uses ui_kit::tokens.

use egui::Color32;

// ---- Surfaces ----

/// The app background behind every panel.
/// Replaced by `ui_kit::tokens::color::BG_GROUND`; this name is removed after S1.
pub const APP_BG: Color32 = Color32::from_rgb(0x16, 0x18, 0x1C);

/// Panel, window and central-area background.
/// Replaced by `ui_kit::tokens::color::BG_PANEL`; this name is removed after S1.
pub const PANEL_BG: Color32 = Color32::from_rgb(0x1A, 0x1D, 0x22);

/// Menu bar and status bar background.
/// Replaced by `ui_kit::tokens::color::BG_GROUND`; this name is removed after S1.
pub const BAR_BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x18);

/// Recessed surface (the audio editor's unlit meter bars and fader slots);
/// egui's extreme background color.
/// Replaced by `ui_kit::tokens::color::BG_CONTROL`; this name is removed after S1.
pub const RECESSED: Color32 = Color32::from_rgb(0x0E, 0x10, 0x13);

/// Faint alternate shade for striped rows (the audio editor's odd lane
/// shade); egui's faint background color.
/// Replaced by `ui_kit::tokens::color::BG_TRACK_ALT`; this name is removed after S1.
pub const FAINT_BG: Color32 = Color32::from_rgb(0x1E, 0x21, 0x27);

/// General separator and window border color.
/// Replaced by `ui_kit::tokens::color::STROKE_HAIRLINE`; this name is removed after S1.
pub const BORDER: Color32 = Color32::from_rgb(0x23, 0x27, 0x2E);

// ---- Controls ----

/// Border color of controls.
/// Replaced by `ui_kit::tokens::color::STROKE_HAIRLINE`; this name is removed after S1.
pub const CONTROL_BORDER: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Fill of an idle control.
/// Replaced by `ui_kit::tokens::color::BG_CONTROL`; this name is removed after S1.
pub const CONTROL_FILL: Color32 = Color32::from_rgb(0x22, 0x26, 0x2D);

/// Fill of a hovered control.
/// Replaced by `ui_kit::tokens::color::BG_HOVER`; this name is removed after S1.
pub const CONTROL_HOVER: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Corner rounding of controls.
/// Replaced by `ui_kit::tokens::radius::SEGMENT`; this name is removed after S1.
pub const CONTROL_ROUNDING: f32 = 6.0;

/// Track color of an egui slider.
/// Replaced by `ui_kit::tokens::color::BG_SELECTED`; this name is removed after S1.
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x2B, 0x30, 0x38);

/// Filled part of an egui slider.
/// Replaced by `ui_kit::tokens::accent::ACCENT`; this name is removed after S1.
pub const SLIDER_FILL: Color32 = Color32::from_rgb(0x5A, 0x61, 0x6D);

/// Thumb color of an egui slider.
/// Replaced by `ui_kit::tokens::color::TEXT_PRIMARY`; this name is removed after S1.
pub const SLIDER_THUMB: Color32 = Color32::from_rgb(0xD7, 0xDA, 0xE0);

// ---- Text ----

/// Brightest text (strong text, headings, pressed controls).
/// Replaced by `ui_kit::tokens::color::TEXT_PRIMARY`; this name is removed after S1.
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xE6, 0xE9, 0xEE);

/// Default text color.
/// Replaced by `ui_kit::tokens::color::TEXT_SECONDARY`; this name is removed after S1.
pub const TEXT_NORMAL: Color32 = Color32::from_rgb(0xC9, 0xCE, 0xD6);

/// Muted secondary text.
/// Replaced by `ui_kit::tokens::color::TEXT_TERTIARY`; this name is removed after S1.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x8A, 0x90, 0x9A);

// ---- Accent and signal colors ----

/// The single accent color: selection, lit toggles, range lines and focus.
/// Replaced by `ui_kit::tokens::accent::ACCENT` (runtime Theme from phase 4); this name is removed after S1.
pub const ACCENT: Color32 = Color32::from_rgb(0x4C, 0x9B, 0xE8);

/// Red of an armed record button and of record state generally.
/// Replaced by `ui_kit::tokens::color::SEM_RECORD`; this name is removed after S1.
pub const RECORD_RED: Color32 = Color32::from_rgb(0xE5, 0x54, 0x4B);

/// Green of the transport play icon.
/// Replaced by `ui_kit::tokens::accent::ACCENT` (Play uses the accent); this name is removed after S1.
pub const PLAY_GREEN: Color32 = Color32::from_rgb(0x5C, 0xC4, 0x6E);

// ---- Meters ----

/// Meter bar color below -18 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_OK`; this name is removed after S1.
pub const METER_GREEN: Color32 = Color32::from_rgb(0x3D, 0xBE, 0x6B);

/// Meter bar color above -18 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_WARN`; this name is removed after S1.
pub const METER_YELLOW: Color32 = Color32::from_rgb(0xE3, 0xC0, 0x4A);

/// Meter bar color above -6 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_CLIP`; this name is removed after S1.
pub const METER_RED: Color32 = Color32::from_rgb(0xE5, 0x54, 0x4B);
