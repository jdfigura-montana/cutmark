#![allow(dead_code)] // TEMP: remove in phase 10

mod app;
mod audio;
mod editor;
mod io;
mod model;
mod ui;
mod util;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Audio Editor")
            .with_inner_size([1400.0, 850.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "Audio Editor",
        options,
        Box::new(|_cc| Ok(Box::new(crate::app::App::default()))),
    )
}
