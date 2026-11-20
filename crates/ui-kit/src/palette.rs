//! The suite's shared dark palette. Every value is copied exactly from the
//! audio editor's `Theme`, so any app built on ui-kit reads as the same
//! product. App-specific colors (clips, lanes, waveforms, track colors) do
//! not belong here.
//!
//! Compatibility layer. New code uses ui_kit::tokens.

use egui::Color32;

use crate::tokens;

// ---- Surfaces ----

/// The app background behind every panel.
/// Replaced by `ui_kit::tokens::color::BG_GROUND`; this name is removed after S1.
pub const APP_BG: Color32 = tokens::color::BG_GROUND;

/// Panel, window and central-area background.
/// Replaced by `ui_kit::tokens::color::BG_PANEL`; this name is removed after S1.
pub const PANEL_BG: Color32 = tokens::color::BG_PANEL;

/// Menu bar and status bar background.
/// Replaced by `ui_kit::tokens::color::BG_GROUND`; this name is removed after S1.
pub const BAR_BG: Color32 = tokens::color::BG_GROUND;

/// Recessed surface (the audio editor's unlit meter bars and fader slots);
/// egui's extreme background color.
/// Replaced by `ui_kit::tokens::color::BG_CONTROL`; this name is removed after S1.
pub const RECESSED: Color32 = tokens::color::BG_CONTROL;

/// Faint alternate shade for striped rows (the audio editor's odd lane
/// shade); egui's faint background color.
/// Replaced by `ui_kit::tokens::color::BG_TRACK_ALT`; this name is removed after S1.
pub const FAINT_BG: Color32 = tokens::color::BG_TRACK_ALT;

/// General separator and window border color.
/// Replaced by `ui_kit::tokens::color::STROKE_HAIRLINE`; this name is removed after S1.
pub const BORDER: Color32 = tokens::color::STROKE_HAIRLINE;

// ---- Controls ----

/// Border color of controls.
/// Replaced by `ui_kit::tokens::color::STROKE_HAIRLINE`; this name is removed after S1.
pub const CONTROL_BORDER: Color32 = tokens::color::STROKE_HAIRLINE;

/// Fill of an idle control.
/// Replaced by `ui_kit::tokens::color::BG_CONTROL`; this name is removed after S1.
pub const CONTROL_FILL: Color32 = tokens::color::BG_CONTROL;

/// Fill of a hovered control.
/// Replaced by `ui_kit::tokens::color::BG_HOVER`; this name is removed after S1.
pub const CONTROL_HOVER: Color32 = tokens::color::BG_HOVER;

/// Corner rounding of controls.
/// Replaced by `ui_kit::tokens::radius::SEGMENT`; this name is removed after S1.
pub const CONTROL_ROUNDING: f32 = tokens::radius::SEGMENT;

/// Track color of an egui slider.
/// Replaced by `ui_kit::tokens::color::BG_SELECTED`; this name is removed after S1.
pub const SLIDER_TRACK: Color32 = tokens::color::BG_SELECTED;

/// Filled part of an egui slider.
/// Replaced by `ui_kit::tokens::accent::ACCENT`; this name is removed after S1.
pub const SLIDER_FILL: Color32 = tokens::accent::ACCENT;

/// Thumb color of an egui slider.
/// Replaced by `ui_kit::tokens::color::TEXT_PRIMARY`; this name is removed after S1.
pub const SLIDER_THUMB: Color32 = tokens::color::TEXT_PRIMARY;

// ---- Text ----

/// Brightest text (strong text, headings, pressed controls).
/// Replaced by `ui_kit::tokens::color::TEXT_PRIMARY`; this name is removed after S1.
pub const TEXT_PRIMARY: Color32 = tokens::color::TEXT_PRIMARY;

/// Default text color.
/// Replaced by `ui_kit::tokens::color::TEXT_SECONDARY`; this name is removed after S1.
pub const TEXT_NORMAL: Color32 = tokens::color::TEXT_SECONDARY;

/// Muted secondary text.
/// Replaced by `ui_kit::tokens::color::TEXT_TERTIARY`; this name is removed after S1.
pub const TEXT_MUTED: Color32 = tokens::color::TEXT_TERTIARY;

// ---- Accent and signal colors ----

/// The single accent color: selection, lit toggles, range lines and focus.
/// Replaced by `ui_kit::tokens::accent::ACCENT` (runtime Theme from phase 4); this name is removed after S1.
pub const ACCENT: Color32 = tokens::accent::ACCENT;

/// Red of an armed record button and of record state generally.
/// Replaced by `ui_kit::tokens::color::SEM_RECORD`; this name is removed after S1.
pub const RECORD_RED: Color32 = tokens::color::SEM_RECORD;

/// Green of the transport play icon.
/// Replaced by `ui_kit::tokens::accent::ACCENT` (Play uses the accent); this name is removed after S1.
pub const PLAY_GREEN: Color32 = tokens::accent::ACCENT;

// ---- Meters ----

/// Meter bar color below -18 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_OK`; this name is removed after S1.
pub const METER_GREEN: Color32 = tokens::color::SEM_METER_OK;

/// Meter bar color above -18 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_WARN`; this name is removed after S1.
pub const METER_YELLOW: Color32 = tokens::color::SEM_METER_WARN;

/// Meter bar color above -6 dBFS.
/// Replaced by `ui_kit::tokens::color::SEM_METER_CLIP`; this name is removed after S1.
pub const METER_RED: Color32 = tokens::color::SEM_METER_CLIP;
