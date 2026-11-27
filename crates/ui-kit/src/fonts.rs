//! The suite's fonts: Instrument Sans (proportional), Martian Mono (monospace)
//! and the phosphor icon font as a glyph fallback in every family.
//!
//! Both typefaces are SIL OFL 1.1. Their license files sit in `assets/fonts`
//! and must ship with any installer that uses ui-kit. Each font file's source
//! URL, release version and SHA-256 are recorded in `assets/fonts/SOURCES.md`.

/// Named egui font family holding the Instrument Sans Medium face.
pub const MEDIUM_FAMILY: &str = "InstrumentSansMedium";

/// Named egui font family holding the Instrument Sans SemiBold face.
pub const SEMIBOLD_FAMILY: &str = "InstrumentSansSemiBold";

/// Named egui font family for the heavy monospace cut. It holds Martian Mono
/// Medium, the heaviest monospace weight the suite ships.
pub const MONO_SEMIBOLD_FAMILY: &str = "MartianMonoSemiBold";

/// Letter spacing of monospace numbers, as a fraction of the font size (spec
/// "Type": number style). egui text styles hold only an [`egui::FontId`], so
/// this applies per text run, through [`tracked`].
pub const MONO_LETTER_SPACING_EM: f32 = -0.01;

/// Letter spacing of the large timecode, as a fraction of the font size.
pub const TIMECODE_LETTER_SPACING_EM: f32 = -0.03;

/// The font-data key the phosphor REGULAR variant is registered under, and
/// the name of the family that resolves glyphs from it alone.
pub const PHOSPHOR_REGULAR_FONT: &str = "phosphor-regular";

/// The font-data key the phosphor FILL variant is registered under, and the
/// name of the family that resolves glyphs from it alone.
pub const PHOSPHOR_FILL_FONT: &str = "phosphor-fill";

/// Instrument Sans Regular (SIL OFL 1.1, see assets/fonts/OFL-InstrumentSans.txt).
const SANS_REGULAR: &[u8] = include_bytes!("../assets/fonts/InstrumentSans-Regular.ttf");
/// Instrument Sans Medium (SIL OFL 1.1, see assets/fonts/OFL-InstrumentSans.txt).
const SANS_MEDIUM: &[u8] = include_bytes!("../assets/fonts/InstrumentSans-Medium.ttf");
/// Instrument Sans SemiBold (SIL OFL 1.1, see assets/fonts/OFL-InstrumentSans.txt).
const SANS_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/InstrumentSans-SemiBold.ttf");
/// Martian Mono Regular (SIL OFL 1.1, see assets/fonts/OFL-MartianMono.txt).
const MARTIAN_REGULAR: &[u8] = include_bytes!("../assets/fonts/MartianMono-Regular.ttf");
/// Martian Mono Medium (SIL OFL 1.1, see assets/fonts/OFL-MartianMono.txt).
const MARTIAN_MEDIUM: &[u8] = include_bytes!("../assets/fonts/MartianMono-Medium.ttf");

/// egui's default proportional face (present with egui's `default_fonts`
/// feature), the last fallback for scripts Instrument Sans lacks.
const TEXT_FALLBACK_FONT: &str = "Ubuntu-Light";

/// Builds the font set: Instrument Sans (Proportional), Martian Mono
/// (Monospace), the named [`MEDIUM_FAMILY`], [`SEMIBOLD_FAMILY`] and
/// [`MONO_SEMIBOLD_FAMILY`] families, and
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
        ("InstrumentSans-Regular", SANS_REGULAR),
        ("InstrumentSans-Medium", SANS_MEDIUM),
        ("InstrumentSans-SemiBold", SANS_SEMIBOLD),
        ("MartianMono-Regular", MARTIAN_REGULAR),
        ("MartianMono-Medium", MARTIAN_MEDIUM),
    ] {
        fonts.font_data.insert(key.into(), egui::FontData::from_static(bytes).into());
    }
    // Instrument Sans is the proportional face, Martian Mono the monospace face;
    // both phosphor cuts stay in every family list as the glyph fallback,
    // regular first.
    {
        let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
        proportional.insert(0, "InstrumentSans-Medium".into());
        proportional.insert(0, "InstrumentSans-Regular".into());
        proportional.push(PHOSPHOR_REGULAR_FONT.into());
        proportional.push(PHOSPHOR_FILL_FONT.into());
        let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
        monospace.insert(0, "MartianMono-Medium".into());
        monospace.insert(0, "MartianMono-Regular".into());
        monospace.push(PHOSPHOR_REGULAR_FONT.into());
        monospace.push(PHOSPHOR_FILL_FONT.into());
    }
    fonts.families.insert(
        egui::FontFamily::Name(MEDIUM_FAMILY.into()),
        vec!["InstrumentSans-Medium".into(), "InstrumentSans-Regular".into(), PHOSPHOR_REGULAR_FONT.into(), PHOSPHOR_FILL_FONT.into()],
    );
    fonts.families.insert(
        egui::FontFamily::Name(SEMIBOLD_FAMILY.into()),
        vec!["InstrumentSans-SemiBold".into(), "InstrumentSans-Medium".into(), PHOSPHOR_REGULAR_FONT.into(), PHOSPHOR_FILL_FONT.into()],
    );
    fonts.families.insert(
        egui::FontFamily::Name(MONO_SEMIBOLD_FAMILY.into()),
        vec!["MartianMono-Medium".into(), "MartianMono-Regular".into(), PHOSPHOR_REGULAR_FONT.into(), PHOSPHOR_FILL_FONT.into()],
    );
    // Instrument Sans has no Greek or Cyrillic. `Proportional` already falls
    // back to egui's own text face; give the named sans families the same one,
    // so user text such as track and device names stays readable.
    if fonts.font_data.contains_key(TEXT_FALLBACK_FONT) {
        for name in [MEDIUM_FAMILY, SEMIBOLD_FAMILY] {
            fonts.families.entry(egui::FontFamily::Name(name.into())).or_default().push(TEXT_FALLBACK_FONT.into());
        }
    }
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

/// Heavy monospace font at `size` (see [`MONO_SEMIBOLD_FAMILY`]), used for
/// the large transport timecode.
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

/// One line of `text` with `em` letter spacing, a fraction of the font size
/// (see [`MONO_LETTER_SPACING_EM`] and [`TIMECODE_LETTER_SPACING_EM`]).
pub fn tracked(text: impl Into<String>, font: egui::FontId, color: egui::Color32, em: f32) -> egui::text::LayoutJob {
    let extra_letter_spacing = font.size * em;
    let format = egui::TextFormat { extra_letter_spacing, ..egui::TextFormat::simple(font, color) };
    egui::text::LayoutJob::single_section(text.into(), format)
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

    /// Every family lists at least one face, and every face it lists has data.
    #[test]
    fn every_font_family_is_non_empty() {
        let fonts = definitions();
        let named = [MEDIUM_FAMILY, SEMIBOLD_FAMILY, MONO_SEMIBOLD_FAMILY].map(|name| egui::FontFamily::Name(name.into()));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace].iter().chain(&named) {
            assert!(fonts.families.contains_key(family), "{family:?} must be defined");
        }
        for (family, keys) in &fonts.families {
            assert!(!keys.is_empty(), "{family:?} has no faces");
            for key in keys {
                assert!(fonts.font_data.get(key).is_some_and(|data| !data.font.is_empty()), "{family:?} lists {key}, which has no data");
            }
        }
    }

    /// The first face of both monospace helpers is Martian Mono, checked by
    /// the family name inside the font file's own `name` table (UTF-16BE).
    #[test]
    fn mono_resolves_to_martian_mono() {
        let fonts = definitions();
        let name: Vec<u8> = "Martian Mono".encode_utf16().flat_map(u16::to_be_bytes).collect();
        for (id, expected) in [(mono(12.0), "MartianMono-Regular"), (mono_semibold(12.0), "MartianMono-Medium")] {
            let first = fonts.families.get(&id.family).and_then(|keys| keys.first());
            assert_eq!(first.map(String::as_str), Some(expected));
            assert!(fonts.font_data.get(expected).is_some_and(|data| data.font.windows(name.len()).any(|window| window == name)));
        }
    }

    /// The sans families can draw Greek and Cyrillic through the fallback.
    #[test]
    fn sans_families_cover_greek_and_cyrillic() {
        use egui::epaint::text::{Fonts, TextOptions};
        let mut fonts = Fonts::new(TextOptions::default(), definitions());
        let mut view = fonts.with_pixels_per_point(1.0);
        for id in [medium(12.0), semibold(12.0)] {
            let keys = definitions().families.remove(&id.family).unwrap_or_default();
            assert!(keys.iter().any(|key| key == TEXT_FALLBACK_FONT), "{:?} has no text fallback", id.family);
            assert!(view.has_glyphs(&id, "\u{3b1}\u{416}"), "{:?} cannot draw Greek and Cyrillic", id.family);
            // No face here has CJK, so the probe can say no.
            assert!(!view.has_glyphs(&id, "\u{4e2d}"), "the glyph probe is blind");
        }
    }

    /// Letter spacing scales with the font size.
    #[test]
    fn tracked_text_spaces_by_a_fraction_of_the_size() {
        let job = tracked("00:00", mono(20.0), egui::Color32::WHITE, TIMECODE_LETTER_SPACING_EM);
        let spacing = job.sections.first().map(|section| section.format.extra_letter_spacing);
        assert!(spacing.is_some_and(|spacing| (spacing + 0.6).abs() < 1e-6), "{spacing:?}");
    }

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
