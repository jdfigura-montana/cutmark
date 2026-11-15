//! The Recorder window: the active profile, the hotkeys and the commands they produced.

use std::collections::VecDeque;

use crate::config::{self, Settings};
use crate::engine::bus::{Bus, Command};
use crate::hotkeys::Hotkeys;

/// How many recent commands the window keeps.
const RECENT_LIMIT: usize = 20;

/// The Recorder app: settings, hotkeys, the command bus and the recent-command list.
pub struct RecorderApp {
    settings: Settings,
    hotkeys: Hotkeys,
    bus: Bus,
    recent: VecDeque<Command>,
}

impl RecorderApp {
    /// Applies the suite theme, loads settings and hotkeys and builds the command bus.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_kit::apply(&cc.egui_ctx);
        let settings = config::load_or_init().unwrap_or_else(|err| {
            log::error!("could not load the Recorder settings, using the defaults: {err}");
            Settings::default()
        });
        let bus = Bus::default();
        let hotkeys = Hotkeys::start(&cc.egui_ctx, bus.sender());
        Self { settings, hotkeys, bus, recent: VecDeque::with_capacity(RECENT_LIMIT) }
    }

    /// Pulls hotkey presses and other queued commands off the bus and records them.
    fn handle_commands(&mut self) {
        self.hotkeys.pump();
        for command in self.bus.drain() {
            log::info!("command: {command:?}");
            if self.recent.len() == RECENT_LIMIT {
                self.recent.pop_front();
            }
            self.recent.push_back(command);
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
        self.handle_commands();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutmark Recorder");
            ui.label(format!("Active profile: {}", self.settings.active_profile));
            ui.add_space(8.0);

            let warn = ui.visuals().warn_fg_color;
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
