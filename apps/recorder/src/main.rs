#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Recorder: screen, webcam and audio recorder for the Cutmark suite.

mod app;
mod config;
mod engine;
mod hotkeys;
mod platform;
mod takes;
mod tray;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_title("Cutmark Recorder").with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Cutmark Recorder", options, Box::new(|cc| Ok(Box::new(app::RecorderApp::new(cc))))).map_err(|err| anyhow::anyhow!("{err}"))
}
