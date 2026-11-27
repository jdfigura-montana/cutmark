//! The Cutmark Gallery binary: every ui-kit component in every state, inside
//! the real window chrome (spec: Design Language "Style gallery"; S1 phase
//! 9a). Run with `cargo run -p cutmark-gallery`.

mod app;
mod core_page;
mod help;
mod media;

use eframe::egui;

/// The main window's title, also the name eframe runs the app under.
const WINDOW_TITLE: &str = "CutMark Gallery";

/// The main window: its title and size on the suite's window chrome.
fn viewport() -> egui::ViewportBuilder {
    ui_kit::chrome::viewport(egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title(WINDOW_TITLE))
}

/// Starts the gallery window with the custom chrome viewport.
fn main() -> eframe::Result {
    let options = eframe::NativeOptions { viewport: viewport(), renderer: eframe::Renderer::Wgpu, ..Default::default() };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| {
            ui_kit::apply_for(&cc.egui_ctx, app::APPEARANCE);
            ui_kit::chrome::install(cc);
            let book = help::book().expect("the gallery help topics are valid");
            Ok(Box::new(app::Gallery::new(book)))
        }),
    )
}

#[cfg(test)]
mod tests {
    /// The product name's casing: a capital C and a capital M.
    #[test]
    fn the_main_window_title_starts_with_cutmark() {
        let title = super::viewport().title;
        assert!(title.as_deref().is_some_and(|title| title.starts_with("CutMark")), "{title:?}");
    }
}
