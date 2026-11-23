#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Recorder: screen, webcam and audio recorder for the Cutmark suite.

mod api;
mod app;
mod background_ui;
mod compositor;
mod config;
mod countdown;
mod engine;
mod hotkeys;
mod key_ui;
mod label_popup;
mod layout_ui;
mod marks_ui;
mod platform;
mod preflight_ui;
mod preview_ui;
mod profile_ui;
mod profiles;
mod record_ui;
mod recovery;
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

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // The suite's undecorated window chrome: the native caption goes, resize borders and
        // Aero Snap stay, and the title bar is drawn by ui_kit::main_toolbar in the app.
        viewport: ui_kit::chrome::viewport(egui::ViewportBuilder::default().with_title("Cutmark Recorder").with_inner_size([1280.0, 800.0])),
        ..Default::default()
    };
    eframe::run_native(
        "Cutmark Recorder",
        options,
        Box::new(|cc| {
            ui_kit::apply(&cc.egui_ctx);
            ui_kit::chrome::install(cc);
            Ok(Box::new(app::RecorderApp::new(cc)))
        }),
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
}
