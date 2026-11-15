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
    // One-time copy of the recent list from the old AudioEditor config dir.
    crate::io::recent::migrate_old_config();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(crate::app::APP_NAME)
            .with_inner_size([1400.0, 850.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
        ..Default::default()
    };
    // The app id doubles as the eframe persistence key.
    eframe::run_native(
        crate::app::APP_NAME,
        options,
        Box::new(|cc| {
            crate::ui::theme::apply(&cc.egui_ctx);
            Ok(Box::new(crate::app::App::default()))
        }),
    )
}

/// The 256x256 app icon for the window/taskbar, decoded once at startup.
fn window_icon() -> eframe::egui::IconData {
    let png = include_bytes!("../assets/icon/app.png");
    let img = image::load_from_memory(png).expect("embedded assets/icon/app.png decodes");
    let rgba = img.to_rgba8();
    eframe::egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
}
