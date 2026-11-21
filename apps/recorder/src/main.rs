#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Recorder: screen, webcam and audio recorder for the Cutmark suite.

mod api;
mod app;
mod config;
mod countdown;
mod engine;
mod hotkeys;
mod label_popup;
mod marks_ui;
mod platform;
mod preflight_ui;
mod preview_ui;
mod profile_ui;
mod profiles;
mod recovery;
mod status_ui;
mod sync_flash;
mod takes;
mod tracks_ui;
mod tray;
mod webcam;
mod webcam_ui;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_title("Cutmark Recorder").with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Cutmark Recorder", options, Box::new(|cc| Ok(Box::new(app::RecorderApp::new(cc))))).map_err(|err| anyhow::anyhow!("{err}"))
}
