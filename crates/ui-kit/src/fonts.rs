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

/// The font-data key the phosphor REGULAR variant is registered under, and
/// the name of the family that resolves glyphs from it alone.
pub const PHOSPHOR_REGULAR_FONT: &str = "phosphor-regular";

/// The font-data key the phosphor FILL variant is registered under, and the
/// name of the family that resolves glyphs from it alone.
pub const PHOSPHOR_FILL_FONT: &str = "phosphor-fill";

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
/// BOTH phosphor variants (regular and fill) under distinct keys as glyph
/// fallback for EVERY family, so an icon renders wherever it is mixed into
/// text. The variants share their codepoints, so the regular font leads the
/// fallback lists (line icons are the common case); a widget that needs the
/// filled cut lays its glyph out in the `phosphor-fill` family via
/// [`icon_fill`].
pub fn definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (key, bytes) in [
        (PHOSPHOR_REGULAR_FONT, egui_phosphor::variants::bytes::regular::FONT),
        (PHOSPHOR_FILL_FONT, egui_phosphor::variants::bytes::fill::FONT),
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
    // both phosphor cuts stay in every family list as the glyph fallback,
    // regular first.
    {
        let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
        proportional.insert(0, "Inter-Medium".into());
        proportional.insert(0, "Inter-Regular".into());
        proportional.push(PHOSPHOR_REGULAR_FONT.into());
        proportional.push(PHOSPHOR_FILL_FONT.into());
        let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
        monospace.insert(0, "JetBrainsMono-Medium".into());
        monospace.insert(0, "JetBrainsMono-Regular".into());
        monospace.push(PHOSPHOR_REGULAR_FONT.into());
        monospace.push(PHOSPHOR_FILL_FONT.into());
    }
    fonts.families.insert(
        egui::FontFamily::Name(MEDIUM_FAMILY.into()),
        vec!["Inter-Medium".into(), "Inter-Regular".into(), PHOSPHOR_REGULAR_FONT.into(), PHOSPHOR_FILL_FONT.into()],
    );
    fonts.families.insert(
        egui::FontFamily::Name(SEMIBOLD_FAMILY.into()),
        vec!["Inter-SemiBold".into(), "Inter-Medium".into(), PHOSPHOR_REGULAR_FONT.into(), PHOSPHOR_FILL_FONT.into()],
    );
    fonts.families.insert(
        egui::FontFamily::Name(MONO_SEMIBOLD_FAMILY.into()),
        vec![
            "JetBrainsMono-SemiBold".into(),
            "JetBrainsMono-Medium".into(),
            "JetBrainsMono-Regular".into(),
            PHOSPHOR_REGULAR_FONT.into(),
            PHOSPHOR_FILL_FONT.into(),
        ],
    );
    // One family per phosphor cut, holding that cut alone, so a widget picks
    // the variant it wants with `icon_regular` / `icon_fill`.
    fonts.families.insert(egui::FontFamily::Name(PHOSPHOR_REGULAR_FONT.into()), vec![PHOSPHOR_REGULAR_FONT.into()]);
    fonts.families.insert(egui::FontFamily::Name(PHOSPHOR_FILL_FONT.into()), vec![PHOSPHOR_FILL_FONT.into()]);
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

/// Phosphor regular-cut icon font at `size`, for line icons.
pub fn icon_regular(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(PHOSPHOR_REGULAR_FONT.into()))
}

/// Phosphor fill-cut icon font at `size`, for transport glyphs.
pub fn icon_fill(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(PHOSPHOR_FILL_FONT.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both phosphor cuts are registered under their own keys, each with a
    /// family of its own, and every text family falls back to both.
    #[test]
    fn both_phosphor_variants_are_registered() {
        let fonts = definitions();
        assert!(fonts.font_data.contains_key(PHOSPHOR_REGULAR_FONT), "the regular cut must be registered");
        assert!(fonts.font_data.contains_key(PHOSPHOR_FILL_FONT), "the fill cut must be registered");
        for key in [PHOSPHOR_REGULAR_FONT, PHOSPHOR_FILL_FONT] {
            let family = egui::FontFamily::Name(key.into());
            assert_eq!(fonts.families.get(&family), Some(&vec![key.to_string()]), "{key} must resolve from its own family");
        }
        let medium = fonts.families.get(&egui::FontFamily::Name(MEDIUM_FAMILY.into()));
        assert!(
            medium.is_some_and(|keys| keys.iter().any(|key| key == PHOSPHOR_REGULAR_FONT) && keys.iter().any(|key| key == PHOSPHOR_FILL_FONT)),
            "the named families must fall back to both cuts"
        );
    }
}
