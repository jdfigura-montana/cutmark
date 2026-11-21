//! The Cutmark Gallery binary: every ui-kit component in every state, inside
//! the real window chrome (spec: Design Language "Style gallery"; S1 phase
//! 9a). Run with `cargo run -p cutmark-gallery`.

mod app;
mod core_page;
mod media;

use eframe::egui;

/// Starts the gallery window with the custom chrome viewport.
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: ui_kit::chrome::viewport(egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title("Cutmark Gallery")),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Cutmark Gallery",
        options,
        Box::new(|cc| {
            ui_kit::apply(&cc.egui_ctx);
            ui_kit::chrome::install(cc);
            Ok(Box::new(app::Gallery::new()))
        }),
    )
}
