//! The Recorder window: the active profile, the hotkeys and the commands they produced.

use std::collections::VecDeque;

use crate::config::{self, Settings};
use crate::engine::bus::{Bus, Command};
use crate::hotkeys::Hotkeys;
use crate::platform;
use crate::tray::Tray;

/// How many recent commands the window keeps.
const RECENT_LIMIT: usize = 20;

/// The Recorder app: settings, hotkeys, the tray icon, the command bus and the recent-command list.
pub struct RecorderApp {
    settings: Settings,
    hotkeys: Hotkeys,
    tray: Option<Tray>,
    bus: Bus,
    recent: VecDeque<Command>,
    /// Placeholder until phase 2 adds real recording: toggled by `Command::StartStop`.
    recording: bool,
    /// True when the window could not be excluded from screen capture.
    capture_exclusion_failed: bool,
    /// Whether the window was minimized on the previous frame, to catch the moment it becomes minimized.
    was_minimized: bool,
}

impl RecorderApp {
    /// Applies the suite theme, excludes the window from capture, loads settings, hotkeys and the tray icon.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_kit::apply(&cc.egui_ctx);
        let capture_exclusion_failed = match platform::exclude_from_capture(cc) {
            Ok(()) => false,
            Err(err) => {
                log::warn!("could not exclude the Recorder window from screen capture: {err}");
                true
            }
        };
        let settings = config::load_or_init().unwrap_or_else(|err| {
            log::error!("could not load the Recorder settings, using the defaults: {err}");
            Settings::default()
        });
        let bus = Bus::default();
        let hotkeys = Hotkeys::start(&cc.egui_ctx, bus.sender());
        let hwnd = match platform::raw_hwnd(cc) {
            Ok(hwnd) => Some(hwnd),
            Err(err) => {
                log::warn!("could not get the window handle, tray Show and Quit will only send commands: {err}");
                None
            }
        };
        let tray = match Tray::new(&cc.egui_ctx, hwnd, bus.sender()) {
            Ok(tray) => Some(tray),
            Err(err) => {
                log::warn!("running without a tray icon: {err}");
                None
            }
        };
        Self { settings, hotkeys, tray, bus, recent: VecDeque::with_capacity(RECENT_LIMIT), recording: false, capture_exclusion_failed, was_minimized: false }
    }

    /// Pulls hotkey presses and other queued commands off the bus and records them.
    fn handle_commands(&mut self) {
        self.hotkeys.pump();
        for command in self.bus.drain() {
            log::info!("command: {command:?}");
            if matches!(command, Command::StartStop) {
                self.set_recording(!self.recording);
            }
            if self.recent.len() == RECENT_LIMIT {
                self.recent.pop_front();
            }
            self.recent.push_back(command);
        }
    }

    /// While recording, hides the window to the tray the moment it is minimized.
    fn hide_to_tray_when_minimized(&mut self, ctx: &egui::Context) {
        let minimized = ctx.input(|input| input.viewport().minimized).unwrap_or(false);
        if minimized && !self.was_minimized && self.recording && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.was_minimized = minimized;
    }

    /// Sets the placeholder recording state and updates the tray icon to match.
    fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
        if let Some(tray) = &self.tray {
            tray.set_recording(recording);
        }
    }

    /// Draws the hotkey list with each chord's status.
    fn hotkey_grid(&self, ui: &mut egui::Ui) {
        let warn = ui.visuals().warn_fg_color;
        let error = ui.visuals().error_fg_color;
        egui::Grid::new("hotkeys").num_columns(3).striped(true).show(ui, |ui| {
            ui.strong("Chord");
            ui.strong("Action");
            ui.strong("Status");
            ui.end_row();
            for hotkey in &self.hotkeys.registered {
                ui.monospace(&hotkey.chord);
                ui.label(&hotkey.action);
                match self.hotkeys.altgr.iter().find(|conflict| conflict.action == hotkey.action) {
                    Some(conflict) => ui.colored_label(warn, format!("AltGr types '{}'", conflict.typed)),
                    None => ui.label("OK"),
                };
                ui.end_row();
            }
            for hotkey in &self.hotkeys.refused {
                ui.monospace(&hotkey.chord);
                ui.label(&hotkey.action);
                ui.colored_label(error, "Taken by another program").on_hover_text(&hotkey.reason);
                ui.end_row();
            }
        });
    }
}

impl eframe::App for RecorderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_commands();
        self.hide_to_tray_when_minimized(&ctx);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutmark Recorder");
            ui.label(format!("Active profile: {}", self.settings.active_profile));
            ui.label(if self.recording { "Recording (placeholder)" } else { "Idle" });
            ui.add_space(8.0);

            let warn = ui.visuals().warn_fg_color;
            if self.capture_exclusion_failed {
                ui.colored_label(warn, "This window may appear in recordings");
            }
            for problem in &self.hotkeys.problems {
                ui.colored_label(warn, problem);
            }

            ui.heading("Hotkeys");
            self.hotkey_grid(ui);
            ui.add_space(8.0);

            ui.heading("Recent commands");
            if self.recent.is_empty() {
                ui.label("None yet. Press a hotkey.");
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for command in self.recent.iter().rev() {
                        ui.label(format!("{command:?}"));
                    }
                });
            }
        });
    }
}
