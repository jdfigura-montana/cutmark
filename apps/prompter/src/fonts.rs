//! ui-kit's fonts, with the one thing the Prompter adds to them.

use ui_kit::fonts::{PHOSPHOR_FILL_FONT, PHOSPHOR_REGULAR_FONT};

/// The text face ui-kit registers that stands behind the icon fonts.
const TEXT_FONT: &str = "Inter-Regular";

/// ui-kit's fonts, with a text face behind each family that holds an icon font alone. Every
/// family needs "\u{25FB}" or "?" to draw in place of a character it lacks. The icon fonts
/// have neither, and egui logs "Failed to find replacement characters". The icons still come
/// from the icon font, which stays first.
pub fn definitions() -> egui::FontDefinitions {
    let mut fonts = ui_kit::fonts::definitions();
    for icons in [PHOSPHOR_REGULAR_FONT, PHOSPHOR_FILL_FONT] {
        if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Name(icons.into())) {
            family.push(TEXT_FONT.to_owned());
        }
    }
    fonts
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        /// This thread's log, so that tests running beside this one add nothing to it.
        static LOGGED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            LOGGED.with_borrow_mut(|logged| logged.push(record.args().to_string()));
        }

        fn flush(&self) {}
    }

    #[test]
    fn every_family_has_a_replacement_character_and_icons_stay_with_the_icon_fonts() {
        // The one logger of the test program.
        log::set_logger(&Capture).unwrap();
        log::set_max_level(log::LevelFilter::Warn);
        let fonts = definitions();
        let ctx = egui::Context::default();
        ctx.set_fonts(fonts.clone());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for family in fonts.families.keys() {
                ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::new(12.0, family.clone())));
            }
        });
        output.textures_delta.clear();
        assert_eq!(LOGGED.take(), Vec::<String>::new());
        for icons in [PHOSPHOR_REGULAR_FONT, PHOSPHOR_FILL_FONT] {
            assert_eq!(fonts.families[&egui::FontFamily::Name(icons.into())], [icons, TEXT_FONT]);
        }
    }
}
