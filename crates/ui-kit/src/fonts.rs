//! The suite's fonts: Inter (proportional), JetBrains Mono (monospace) and
//! the phosphor icon font as a glyph fallback in every family.
//!
//! Both typefaces are SIL OFL 1.1. Their license files sit in `assets/fonts`
//! and must ship with any installer that uses ui-kit. Each font file's source
//! URL, release version and SHA-256 are recorded in `assets/fonts/SOURCES.md`.

/// Named egui font family holding the Inter Medium face.
pub const MEDIUM_FAMILY: &str = "InterMedium";

/// Named egui font family holding the Inter SemiBold face.
pub const SEMIBOLD_FAMILY: &str = "InterSemiBold";

/// Named egui font family holding the JetBrains Mono SemiBold face.
pub const MONO_SEMIBOLD_FAMILY: &str = "JetBrainsMonoSemiBold";

/// The font-data key egui-phosphor registers its icon font under.
pub const PHOSPHOR_FONT: &str = "phosphor";

/// Inter Regular (SIL OFL 1.1, see assets/fonts/OFL-Inter.txt).
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
/// Inter Medium (SIL OFL 1.1, see assets/fonts/OFL-Inter.txt).
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
/// Inter SemiBold (SIL OFL 1.1, see assets/fonts/OFL-Inter.txt).
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
/// JetBrains Mono Regular (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_REGULAR: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
/// JetBrains Mono Medium (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_MEDIUM: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Medium.ttf");
/// JetBrains Mono SemiBold (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-SemiBold.ttf");

/// Builds the font set: Inter (Proportional), JetBrains Mono (Monospace), the
/// named "InterMedium"/"InterSemiBold"/"JetBrainsMonoSemiBold" families, and
/// the phosphor icon font as a glyph fallback for EVERY family, so an icon
/// renders wherever it is mixed into text.
pub fn definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Fill);
    for (key, bytes) in [
        ("Inter-Regular", INTER_REGULAR),
        ("Inter-Medium", INTER_MEDIUM),
        ("Inter-SemiBold", INTER_SEMIBOLD),
        ("JetBrainsMono-Regular", JB_MONO_REGULAR),
        ("JetBrainsMono-Medium", JB_MONO_MEDIUM),
        ("JetBrainsMono-SemiBold", JB_MONO_SEMIBOLD),
    ] {
        fonts.font_data.insert(key.into(), egui::FontData::from_static(bytes).into());
    }
    // Inter is the proportional face, JetBrains Mono the monospace face;
    // phosphor stays in every family list as the glyph fallback.
    {
        let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
        proportional.insert(0, "Inter-Medium".into());
        proportional.insert(0, "Inter-Regular".into());
        let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
        monospace.insert(0, "JetBrainsMono-Medium".into());
        monospace.insert(0, "JetBrainsMono-Regular".into());
        monospace.push(PHOSPHOR_FONT.into());
    }
    fonts.families.insert(egui::FontFamily::Name(MEDIUM_FAMILY.into()), vec!["Inter-Medium".into(), "Inter-Regular".into(), PHOSPHOR_FONT.into()]);
    fonts.families.insert(egui::FontFamily::Name(SEMIBOLD_FAMILY.into()), vec!["Inter-SemiBold".into(), "Inter-Medium".into(), PHOSPHOR_FONT.into()]);
    fonts.families.insert(
        egui::FontFamily::Name(MONO_SEMIBOLD_FAMILY.into()),
        vec!["JetBrainsMono-SemiBold".into(), "JetBrainsMono-Medium".into(), "JetBrainsMono-Regular".into(), PHOSPHOR_FONT.into()],
    );
    fonts
}

/// Monospace font at `size`, used for every timecode readout.
pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Monospace)
}

/// SemiBold monospace font at `size`, used for the large transport timecode.
pub fn mono_semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(MONO_SEMIBOLD_FAMILY.into()))
}

/// Medium-weight text font at `size`, used for row names and buttons.
pub fn medium(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(MEDIUM_FAMILY.into()))
}

/// SemiBold heading font at `size`, used for headings and track names.
pub fn semibold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into()))
}
