#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Recorder: screen, webcam and audio recorder for the Cutmark suite.

mod api;
mod app;
mod background_ui;
mod bar_ui;
mod compositor;
mod config;
mod countdown;
mod data_text;
mod engine;
mod hotkeys;
mod key_ui;
mod label_popup;
mod layout_ui;
mod marks_ui;
mod platform;
// The review playback engine; the review UI wires it in R9c.
#[allow(dead_code)]
mod playback;
mod preflight_ui;
mod preview_ui;
mod profile_ui;
mod profiles;
mod record_ui;
mod recovery;
mod remote;
mod replay_ui;
mod review;
mod review_ui;
mod settings_ui;
mod status_ui;
mod sync_flash;
mod takes;
mod tools_ui;
mod tracks_ui;
mod tray;
mod webcam;
mod webcam_ui;

/// The main window's title, also the name eframe runs the app under.
const WINDOW_TITLE: &str = "CutMark Recorder";

/// The brand art of this app: its mark and icon.
const BRAND: ui_kit::brand::App = ui_kit::brand::App::Recorder;

/// The main window: its title, size and icon on the suite's window chrome.
fn viewport() -> egui::ViewportBuilder {
    let window = egui::ViewportBuilder::default().with_title(WINDOW_TITLE).with_inner_size([1280.0, 800.0]).with_icon(ui_kit::brand::window_icon(BRAND));
    ui_kit::chrome::viewport(window)
        // The Recorder needs more width than the suite's minimum for its stage and two panels.
        .with_min_inner_size([config::MIN_WINDOW_WIDTH, ui_kit::chrome::MIN_SIZE.y])
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // The suite's undecorated window chrome: the native caption goes, resize borders and
        // Aero Snap stay, and the title bar is drawn by ui_kit::main_toolbar in the app.
        viewport: viewport(),
        ..Default::default()
    };
    eframe::run_native(
        WINDOW_TITLE,
        options,
        Box::new(|cc| {
            ui_kit::apply_for(&cc.egui_ctx, app::APPEARANCE);
            ui_kit::chrome::install(cc);
            Ok(Box::new(app::RecorderApp::new(cc)))
        }),
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
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
