#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod editor;
mod help;
mod io;
mod model;
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

    let options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, viewport: viewport(), ..Default::default() };
    // The app id doubles as the eframe persistence key.
    eframe::run_native(
        crate::app::APP_NAME,
        options,
        Box::new(|cc| {
            ui_kit::apply_for(&cc.egui_ctx, crate::app::APPEARANCE);
            ui_kit::chrome::install(cc);
            Ok(Box::new(crate::app::App::default()))
        }),
    )
}

/// The main window: its title, size and icon on the suite's window chrome.
fn viewport() -> eframe::egui::ViewportBuilder {
    ui_kit::chrome::viewport(
        eframe::egui::ViewportBuilder::default()
            .with_title(crate::app::APP_NAME)
            .with_inner_size([1400.0, 850.0])
            .with_drag_and_drop(true)
            .with_icon(window_icon()),
    )
    // The app's own minimum beats the suite default chrome::viewport sets.
    .with_min_inner_size([1100.0, 700.0])
}

/// The 256x256 app icon for the window/taskbar, decoded once at startup.
fn window_icon() -> eframe::egui::IconData {
    let png = include_bytes!("../assets/icon/app.png");
    let img = image::load_from_memory(png).expect("embedded assets/icon/app.png decodes");
    let rgba = img.to_rgba8();
    eframe::egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
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
