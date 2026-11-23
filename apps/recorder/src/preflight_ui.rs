//! The Pre-flight section: a button that records 5 seconds with the active profile, a progress bar, and the pass or fail report.

use std::time::Duration;

use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::{ControllerState, Phase, PreflightResult};
use crate::engine::preflight::{DND_LABEL, Status, dnd_line};
use crate::platform::open_notification_settings;

/// The colour of a passing result; the theme has no green of its own.
const PASS_GREEN: egui::Color32 = egui::Color32::from_rgb(70, 190, 100);

/// How often the window repaints while a pre-flight runs, so the progress bar moves.
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);

/// The state of the Pre-flight section that the window keeps between frames.
#[derive(Debug, Default)]
pub struct PreflightSection {
    /// Whether the test take's files are kept; off at every start and not saved.
    keep_files: bool,
}

impl PreflightSection {
    /// Whether the test take's files are kept, so the Capture menu's Run pre-flight item sends the same command as the button.
    pub fn keep_files(&self) -> bool {
        self.keep_files
    }

    /// Draws the section and sends a pre-flight command to the bus when the button is clicked.
    pub fn show(&mut self, ui: &mut egui::Ui, state: &ControllerState, bus: &BusSender) {
        ui.heading("Pre-flight");
        let idle = state.phase == Phase::Idle && state.preflight.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(idle, egui::Button::new("Run pre-flight (5 s)")).clicked() {
                let command = Command::new(CommandKind::Preflight { keep_files: self.keep_files }, capture::qpc_now());
                if bus.send(command).is_err() {
                    log::warn!("the pre-flight was not started because the controller has ended");
                }
            }
            ui.add_enabled(idle, egui::Checkbox::new(&mut self.keep_files, "Keep files"));
        });
        if let Some(running) = &state.preflight {
            ui.add(egui::ProgressBar::new(running.progress).text(running.status));
            ui.ctx().request_repaint_after(PROGRESS_REFRESH);
        }
        if let Some(result) = &state.preflight_result {
            show_result(ui, result);
        }
    }
}

/// The colour that goes with a status: plain text, green, yellow or red.
fn colour(ui: &egui::Ui, status: Status) -> egui::Color32 {
    match status {
        Status::Info => ui.visuals().text_color(),
        Status::Pass => PASS_GREEN,
        Status::Warn => ui.visuals().warn_fg_color,
        Status::Fail => ui.visuals().error_fg_color,
    }
}

/// Draws the overall result, one row per check, and the Open folder button when the files were kept.
fn show_result(ui: &mut egui::Ui, result: &PreflightResult) {
    let report = &result.report;
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("Pre-flight result: {}", report.overall.label())).strong().color(colour(ui, report.overall)));
    egui::Grid::new("preflight-report").num_columns(3).show(ui, |ui| {
        for line in &report.lines {
            ui.strong(&line.label);
            ui.colored_label(colour(ui, line.status), line.status.label());
            ui.label(&line.detail);
            ui.end_row();
        }
        let dnd = dnd_line();
        ui.strong(DND_LABEL);
        ui.colored_label(colour(ui, dnd.status), dnd.status.label());
        ui.horizontal(|ui| {
            ui.label(&dnd.detail);
            if ui.button("Open notification settings").clicked() {
                open_notification_settings();
            }
        });
        ui.end_row();
    });
    if let Some(folder) = &result.kept_folder {
        ui.label(format!("Files kept in {}", folder.display()));
        if ui.button("Open folder").clicked()
            && let Err(err) = std::process::Command::new("explorer").arg(folder).spawn()
        {
            log::warn!("could not open {}: {err}", folder.display());
        }
    }
}
