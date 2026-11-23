#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod editor;
mod help;
mod io;
mod model;
mod remote;
mod shortcuts;
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
        viewport: ui_kit::chrome::viewport(
            eframe::egui::ViewportBuilder::default()
                .with_title(crate::app::APP_NAME)
                .with_inner_size([1400.0, 850.0])
                .with_drag_and_drop(true)
                .with_icon(window_icon()),
        )
        // The app's own minimum beats the suite default chrome::viewport sets.
        .with_min_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    // The app id doubles as the eframe persistence key.
    eframe::run_native(
        crate::app::APP_NAME,
        options,
        Box::new(|cc| {
            ui_kit::apply(&cc.egui_ctx);
            ui_kit::chrome::install(cc);
            // The Stream Deck remote control: start the server at launch and
            // wake the UI when a command arrives. None when another instance
            // of the app runs it or it failed to start.
            let ctx = cc.egui_ctx.clone();
            let remote = crate::remote::start(Box::new(move || ctx.request_repaint()));
            Ok(Box::new(crate::app::App::with_remote(remote)))
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
