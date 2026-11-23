//! The Phosphor icon glyphs, re-exported per variant (spec "Icons").
//!
//! The two variants share the same codepoints, so a text family's fallback
//! list can only carry one of them and the variant that renders is chosen by
//! the FONT a widget lays the glyph out in, not by the constant. The rule:
//! line icons use [`regular`] with `fonts::icon_regular`, transport glyphs
//! use [`fill`] with `fonts::icon_fill`. Import the glyphs from here so a
//! call site never names a variant the widget will not render.

/// Line icon glyphs (the regular cut): toolbar actions, field icons,
/// category icons.
pub use egui_phosphor::variants::regular;

/// Filled glyphs: transport and media buttons (play, record, stop, loop).
pub use egui_phosphor::variants::fill;
