#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod editor;
mod io;
mod model;
mod ui;
mod util;

// Detect allocations on the audio thread: debug builds print a warning on
// stderr instead of aborting (feature warn_debug); release builds are
// untouched because the disable_release default feature is enabled.
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

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
        Box::new(|cc| {
            crate::ui::theme::apply(&cc.egui_ctx);
            Ok(Box::new(crate::app::App::default()))
        }),
    )
}
