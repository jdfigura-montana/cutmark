//! The Pre-flight card of the Sources panel (spec "Setup mode"): the pass or fail result, the
//! run button with its progress bar, and a Details popover with today's full report.

use std::time::Duration;

use egui::{Color32, RichText, Ui};
use ui_kit::tokens::{color, space};
use ui_kit::{inspector_group, switch};

use crate::data_text::stat_block;
use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::{ControllerState, Phase, PreflightResult};
use crate::engine::preflight::{DND_LABEL, Status, dnd_line};
use crate::platform::open_notification_settings;

/// The colour of a passing check in the full report (`sem.success`).
const PASS_GREEN: Color32 = color::SEM_SUCCESS;

/// How often the window repaints while a pre-flight runs, so the progress bar moves.
const PROGRESS_REFRESH: Duration = Duration::from_millis(100);

/// The state of the Pre-flight card that the window keeps between frames.
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

    /// Draws the Sources panel's pre-flight card (spec "Setup mode"): the result, one summary
    /// line, the Do Not Disturb line when it is off, the "Run pre-flight (5 s)" button with its
    /// progress bar, and a Details link opening the full report as a popover.
    ///
    /// Sends a pre-flight command to the bus when the button is clicked.
    pub fn sources_card(&mut self, ui: &mut Ui, state: &ControllerState, bus: &BusSender) {
        let idle = state.phase == Phase::Idle && state.preflight.is_none();
        let result = state.preflight_result.as_ref();
        inspector_group(ui, |ui| {
            match result.map(|result| &result.report) {
                Some(report) => {
                    let (label, tint) = match report.overall {
                        Status::Fail => ("Fail", color::SEM_RECORD),
                        Status::Info | Status::Warn | Status::Pass => ("Pass", color::SEM_METER_OK),
                    };
                    ui.colored_label(tint, RichText::new(label).strong());
                    let total = report.lines.len();
                    let passed = report.lines.iter().filter(|line| matches!(line.status, Status::Pass)).count();
                    stat_block(ui, &format!("{passed}/{total}"), "Checks pass");
                }
                None => {
                    ui.weak("Not run");
                    ui.weak("Checks the devices, disk and notifications before you record.");
                }
            }
            ui.add_space(space::SCALE_4);
            let dnd = dnd_line();
            if !matches!(dnd.status, Status::Pass) {
                ui.colored_label(colour(ui, dnd.status), format!("{DND_LABEL}: {}", dnd.detail));
            }
            ui.add_space(space::SCALE_4);
            if ui.add_enabled(idle, egui::Button::new("Run pre-flight (5 s)")).clicked() {
                let command = Command::new(CommandKind::Preflight { keep_files: self.keep_files }, capture::qpc_now());
                if bus.send(command).is_err() {
                    log::warn!("the pre-flight was not started because the controller has ended");
                }
            }
            if let Some(running) = &state.preflight {
                ui.add(egui::ProgressBar::new(running.progress).text(running.status));
                ui.ctx().request_repaint_after(PROGRESS_REFRESH);
            }
            let details = ui.button("Details");
            egui::Popup::menu(&details).width(340.0).show(|ui| details_popover(ui, result, &mut self.keep_files));
        });
    }
}

/// The Details popover: today's full report, the Keep files switch, "Open notification settings"
/// and, when the test files were kept, "Open folder".
fn details_popover(ui: &mut Ui, result: Option<&PreflightResult>, keep_files: &mut bool) {
    match result.map(|result| &result.report) {
        Some(report) => {
            for line in &report.lines {
                check_heading(ui, &line.label, line.status);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = space::SCALE_16;
                    for stat in &line.stats {
                        stat_block(ui, &stat.value, &stat.label);
                    }
                });
                ui.add_space(space::SCALE_8);
            }
            let dnd = dnd_line();
            check_heading(ui, DND_LABEL, dnd.status);
            ui.label(&dnd.detail);
        }
        None => {
            ui.weak("No pre-flight has run yet.");
        }
    }
    ui.add_space(space::SCALE_4);
    ui.horizontal(|ui| {
        ui.label("Keep files");
        switch(ui, keep_files);
    });
    if ui.button("Open notification settings").clicked() {
        open_notification_settings();
    }
    if let Some(folder) = result.and_then(|result| result.kept_folder.as_ref()) {
        ui.label(format!("Files kept in {}", folder.display()));
        if ui.button("Open folder").clicked()
            && let Err(err) = std::process::Command::new("explorer").arg(folder).spawn()
        {
            log::warn!("could not open {}: {err}", folder.display());
        }
    }
}

/// One check's name and its result.
fn check_heading(ui: &mut Ui, label: &str, status: Status) {
    ui.horizontal(|ui| {
        ui.strong(label);
        ui.colored_label(colour(ui, status), status.label());
    });
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
