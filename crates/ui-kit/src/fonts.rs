//! The suite's fonts: IBM Plex Sans (proportional), JetBrains Mono
//! (monospace) and the phosphor icon font as a glyph fallback in every family.
//!
//! Both typefaces are SIL OFL 1.1. Their license files sit in `assets/fonts`
//! and must ship with any installer that uses ui-kit.

/// Named egui font family holding the IBM Plex Sans Medium face.
pub const MEDIUM_FAMILY: &str = "PlexMedium";

/// Named egui font family holding the IBM Plex Sans SemiBold face.
pub const SEMIBOLD_FAMILY: &str = "PlexSemiBold";

/// The font-data key egui-phosphor registers its icon font under.
pub const PHOSPHOR_FONT: &str = "phosphor";

/// IBM Plex Sans Regular (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_REGULAR: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf");
/// IBM Plex Sans Medium (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_MEDIUM: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf");
/// IBM Plex Sans SemiBold (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf");
/// JetBrains Mono Regular (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
/// JetBrains Mono Medium (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");

/// Builds the font set: IBM Plex Sans (Proportional), JetBrains Mono
/// (Monospace), the named "PlexMedium"/"PlexSemiBold" families, and the
/// phosphor icon font as a glyph fallback for EVERY family, so an icon
/// renders wherever it is mixed into text.
pub fn definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Fill);
    for (key, bytes) in [
        ("IBMPlexSans-Regular", PLEX_REGULAR),
        ("IBMPlexSans-Medium", PLEX_MEDIUM),
        ("IBMPlexSans-SemiBold", PLEX_SEMIBOLD),
        ("JetBrainsMono-Regular", JB_MONO_REGULAR),
        ("JetBrainsMono-Medium", JB_MONO_MEDIUM),
    ] {
        fonts.font_data.insert(key.into(), egui::FontData::from_static(bytes).into());
    }
    // IBM Plex Sans is the proportional face, JetBrains Mono the monospace
    // face; phosphor stays in every family list as the glyph fallback.
    {
        let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
        proportional.insert(0, "IBMPlexSans-Medium".into());
        proportional.insert(0, "IBMPlexSans-Regular".into());
        let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
        monospace.insert(0, "JetBrainsMono-Medium".into());
        monospace.insert(0, "JetBrainsMono-Regular".into());
        monospace.push(PHOSPHOR_FONT.into());
    }
    fonts.families.insert(egui::FontFamily::Name(MEDIUM_FAMILY.into()), vec!["IBMPlexSans-Medium".into(), "IBMPlexSans-Regular".into(), PHOSPHOR_FONT.into()]);
    fonts
        .families
        .insert(egui::FontFamily::Name(SEMIBOLD_FAMILY.into()), vec!["IBMPlexSans-SemiBold".into(), "IBMPlexSans-Medium".into(), PHOSPHOR_FONT.into()]);
    fonts
}

/// Monospace font at `size`, used for every timecode readout.
pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Monospace)
}

/// Medium-weight text font at `size`, used for row names and buttons.
pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// SemiBold heading font at `size`, used for headings and track names.
pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into()))
}
