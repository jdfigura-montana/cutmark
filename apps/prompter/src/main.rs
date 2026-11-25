#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Prompter: teleprompter for the Cutmark suite.

mod app;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // The suite's undecorated window chrome: the native caption goes, resize borders and
        // Aero Snap stay.
        viewport: ui_kit::chrome::viewport(egui::ViewportBuilder::default().with_title("Cutmark Prompter").with_inner_size([1100.0, 720.0])),
        ..Default::default()
    };
    eframe::run_native(
        "Cutmark Prompter",
        options,
        Box::new(|cc| {
            ui_kit::apply_for(&cc.egui_ctx, app::APPEARANCE);
            ui_kit::chrome::install(cc);
            Ok(Box::new(app::PrompterApp::new(prompter_platform::os::check_windows_11())))
        }),
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
}
