//! ui: egui panels and views; talks only to the editor.

pub mod bin_panel;
pub mod dialogs;
pub mod master_strip;
pub mod menu;
pub mod start_screen;
pub mod status_bar;
#[cfg(test)]
pub mod test_util;
pub mod timeline;
pub mod widgets;

/// The surfaces, strokes and text colors tinted toward the current accent
/// (Design Language E3, "Tinted neutrals"). The fixed `color` tokens hold only
/// the default accent's values, so the ui reads neutrals through this.
pub(crate) fn neutrals(ctx: &eframe::egui::Context) -> ui_kit::Neutrals {
    ui_kit::Theme::get(ctx).neutrals
}

#[cfg(test)]
mod tests {
    use eframe::egui;

    /// The phosphor icon glyphs resolve, and both cuts sit in the fallback
    /// list of every font family the app draws with, under the ui-kit font set
    /// `ui_kit::apply` installs.
    ///
    /// `Fonts::has_glyph` only checks the FIRST face of a family, so the
    /// fallback membership is asserted structurally; real text layout walks
    /// the fallbacks and renders the icons.
    #[test]
    fn icons_resolve_in_every_app_font_family() {
        use eframe::egui::epaint::text::{Fonts, TextOptions};
        let defs = ui_kit::fonts::definitions();
        let families = [
            egui::FontFamily::Proportional,
            egui::FontFamily::Monospace,
            egui::FontFamily::Name(ui_kit::fonts::MEDIUM_FAMILY.into()),
            egui::FontFamily::Name(ui_kit::fonts::SEMIBOLD_FAMILY.into()),
        ];
        // Both phosphor cuts are in every family's fallback list (the
        // tofu-icons regression this test guards).
        for family in &families {
            let keys = defs.families.get(family).expect("every app font family is defined");
            for cut in [ui_kit::fonts::PHOSPHOR_REGULAR_FONT, ui_kit::fonts::PHOSPHOR_FILL_FONT] {
                assert!(keys.iter().any(|key| key == cut), "phosphor cut {cut} missing from {family:?}");
            }
        }
        let mut fonts = Fonts::new(TextOptions::default(), defs);
        // Strip every icon-capable face: the phosphor cuts AND egui's own
        // icon font defaults. What is left holds text only, so the icons
        // must NOT resolve: this proves the probe is not blind.
        let mut bare = ui_kit::fonts::definitions();
        for keys in bare.families.values_mut() {
            keys.retain(|key| key.starts_with("InstrumentSans-") || key.starts_with("MartianMono-") || key == "Ubuntu-Light" || key == "NotoSans-Regular");
        }
        let mut bare_fonts = Fonts::new(TextOptions::default(), bare);
        for (name, icon) in [
            ("FOLDER_OPEN", ui_kit::icons::fill::FOLDER_OPEN),
            ("FILE_AUDIO", ui_kit::icons::fill::FILE_AUDIO),
            ("PLUS", ui_kit::icons::fill::PLUS),
            ("X", ui_kit::icons::fill::X),
            ("WAVEFORM", ui_kit::icons::fill::WAVEFORM),
        ] {
            let c = icon.chars().next().unwrap();
            // The glyphs live in the phosphor font: both dedicated cut
            // families (holding that cut alone) resolve them.
            for cut in [ui_kit::fonts::PHOSPHOR_REGULAR_FONT, ui_kit::fonts::PHOSPHOR_FILL_FONT] {
                let id = egui::FontId::new(ui_kit::tokens::typography::BODY_SIZE, egui::FontFamily::Name(cut.into()));
                assert!(fonts.has_glyph(&id, c), "{name} (U+{:04X}) missing from the {cut} family", c as u32);
            }
            // The text-only set has none of them.
            for family in &families {
                let id = egui::FontId::new(ui_kit::tokens::typography::BODY_SIZE, family.clone());
                assert!(!bare_fonts.has_glyph(&id, c), "{name} (U+{:04X}) resolves WITHOUT an icon font in {family:?}: the probe is blind", c as u32);
            }
        }
    }
}
