#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cutmark Prompter: teleprompter for the Cutmark suite.

mod app;
mod menu;
mod renderer;
mod view;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let alpha = renderer::AlphaInUse::default();
    let mut options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        // The suite's undecorated window chrome: the native caption goes, resize borders and
        // Aero Snap stay.
        // Transparent because egui-wgpu takes its alpha mode choice for every viewport from this one.
        viewport: ui_kit::chrome::viewport(
            egui::ViewportBuilder::default().with_title(app::ROOT_TITLE).with_inner_size([1100.0, 720.0]).with_min_inner_size([900.0, 480.0]),
        )
        .with_transparent(true),
        ..Default::default()
    };
    options.wgpu_options.wgpu_setup = renderer::wgpu_setup(alpha.clone());
    eframe::run_native(
        "Cutmark Prompter",
        options,
        Box::new(|cc| {
            ui_kit::apply_for(&cc.egui_ctx, app::APPEARANCE);
            ui_kit::chrome::install(cc);
            Ok(Box::new(app::PrompterApp::new(prompter_platform::os::check_windows_11(), alpha)))
        }),
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
}
