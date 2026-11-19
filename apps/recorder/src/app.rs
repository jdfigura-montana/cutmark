//! The Recorder window: draws what the engine controller publishes and sends it commands and settings.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use capture::AudioDevice;

use crate::config::{self, Settings};
use crate::engine::controller::{Controller, ControllerState, Phase, lock};
use crate::engine::session::Summary;
use crate::hotkeys::Hotkeys;
use crate::platform;
use crate::tracks_ui::{self, LiveTrackStats, TracksChanged};
use crate::tray::{Tray, TrayHandle, TrayState};

/// How often the window repaints while recording, so the level meters move smoothly.
const METER_REFRESH: Duration = Duration::from_millis(33);

/// The Recorder app: settings, hotkeys, the tray icon and the window over the engine controller.
pub struct RecorderApp {
    settings: Settings,
    hotkeys: Hotkeys,
    tray: Option<Tray>,
    controller: Controller,
    /// The audio devices that were present at the last refresh.
    devices: Vec<AudioDevice>,
    /// The level meters of the running take.
    live_tracks: LiveTrackStats,
    /// Whether the track settings changed and are not saved yet.
    settings_dirty: bool,
    /// True when the window could not be excluded from screen capture.
    capture_exclusion_failed: bool,
    /// Whether the window was minimized on the previous frame, to catch the moment it becomes minimized.
    was_minimized: bool,
    /// The last `settings_rev` of the controller that was copied into the settings.
    seen_settings_rev: u64,
    /// Whether a take was running on the previous frame, to clear the meters when it starts or ends.
    was_active: bool,
}

impl RecorderApp {
    /// Applies the suite theme, excludes the window from capture, loads settings, starts the controller, the hotkeys and the tray icon.
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
        let repaint_ctx = cc.egui_ctx.clone();
        // The tray needs the controller's senders, and the controller's listener needs the tray handle, so the handle arrives later.
        let tray_slot: Arc<OnceLock<TrayHandle>> = Arc::default();
        let listener_slot = Arc::clone(&tray_slot);
        let listener_ctx = cc.egui_ctx.clone();
        let listener = Box::new(move |state: &ControllerState| {
            if let Some(handle) = listener_slot.get() {
                handle.set_state(tray_state(state.phase));
            }
            listener_ctx.request_repaint();
        });
        let controller = Controller::start(settings.clone(), Box::new(move || repaint_ctx.request_repaint()), listener);
        let hotkeys = Hotkeys::start(controller.sender());
        let hwnd = match platform::raw_hwnd(cc) {
            Ok(hwnd) => Some(hwnd),
            Err(err) => {
                log::warn!("could not get the window handle, tray Show and Quit will only send commands: {err}");
                None
            }
        };
        let tray = match Tray::new(&cc.egui_ctx, hwnd, controller.sender(), controller.control_sender()) {
            Ok(tray) => {
                if tray_slot.set(tray.handle()).is_err() {
                    log::warn!("the tray handle was already set");
                }
                Some(tray)
            }
            Err(err) => {
                log::warn!("running without a tray icon: {err}");
                None
            }
        };
        let devices = list_devices();
        Self {
            settings,
            hotkeys,
            tray,
            controller,
            devices,
            live_tracks: LiveTrackStats::default(),
            settings_dirty: false,
            capture_exclusion_failed,
            was_minimized: false,
            seen_settings_rev: 0,
            was_active: false,
        }
    }

    /// Takes in what the controller published: its settings changes and the level peaks.
    fn sync_from_controller(&mut self, state: &mut ControllerState) {
        if state.settings_rev != self.seen_settings_rev {
            // The controller changed the profile or a mute (a hotkey); the window keeps its own copy of the rest.
            self.seen_settings_rev = state.settings_rev;
            self.settings.active_profile.clone_from(&state.active_profile);
            if self.settings.audio.tracks.len() == state.track_muted.len() {
                for (track, muted) in self.settings.audio.tracks.iter_mut().zip(&state.track_muted) {
                    track.muted = *muted;
                }
            }
            self.settings_dirty = true;
        }
        let active = state.phase != Phase::Idle;
        if active != self.was_active {
            self.live_tracks.clear();
            self.was_active = active;
        }
        if state.stats_seq != state.ui_seen_seq {
            if let Some(stats) = &state.stats {
                self.live_tracks.update(&stats.tracks, Instant::now());
            }
            state.ui_seen_seq = state.stats_seq;
        }
    }

    /// Writes the settings now, unless the settings file on disk is one the Recorder could not read.
    fn save_settings(&mut self) {
        self.settings_dirty = false;
        if let Err(err) = save_settings_file(&self.settings) {
            log::error!("could not save the Recorder settings: {err:#}");
        }
    }

    /// Acts on what the track panel reported this frame.
    fn on_tracks_changed(&mut self, ctx: &egui::Context, changed: TracksChanged) {
        self.settings_dirty |= changed.settings;
        if changed.settings || changed.live {
            self.controller.set_settings(self.settings.clone());
        }
        if changed.refresh_devices {
            self.devices = list_devices();
        }
        // Save when an edit has ended, or when nothing is being dragged and no field has focus.
        let settled = !ctx.input(|input| input.pointer.any_down()) && ctx.memory(|memory| memory.focused().is_none());
        if self.settings_dirty && (changed.commit || settled) {
            self.save_settings();
        }
    }

    /// While recording, hides the window to the tray the moment it is minimized.
    fn hide_to_tray_when_minimized(&mut self, ctx: &egui::Context, active: bool) {
        let minimized = ctx.input(|input| input.viewport().minimized).unwrap_or(false);
        if minimized && !self.was_minimized && active && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.was_minimized = minimized;
    }

    /// Draws the status while a take is running or paused.
    fn recording_panel(&self, ui: &mut egui::Ui, state: &ControllerState) {
        ui.heading(if state.phase == Phase::Paused { "Paused" } else { "Recording" });
        let (Some(take), Some(stats)) = (&state.take, &state.stats) else { return };
        let elapsed_seconds = stats.frames / u64::from(state.fps.max(1));
        egui::Grid::new("recording-status").num_columns(2).show(ui, |ui| {
            ui.strong("Time");
            ui.label(format_hms(elapsed_seconds));
            ui.end_row();
            ui.strong("File");
            ui.label(&take.base_name);
            ui.end_row();
            ui.strong("Folder");
            ui.label(take.folder.display().to_string());
            ui.end_row();
            ui.strong("Size");
            ui.label(format!("{:.1} MB", stats.bytes_written as f64 / 1_000_000.0));
            ui.end_row();
            ui.strong("Bitrate");
            ui.label(format!("{:.1} Mbps", state.mbps));
            ui.end_row();
            ui.strong("Frames");
            ui.label(stats.frames.to_string());
            ui.end_row();
            ui.strong("Duplicated");
            ui.label(stats.duplicated.to_string());
            ui.end_row();
            ui.strong("Dropped");
            ui.label(stats.dropped.to_string());
            ui.end_row();
            ui.strong("Captured");
            ui.label(stats.frames_received.to_string());
            ui.end_row();
        });
        ui.add_space(8.0);
    }

    /// Draws the totals of the last finished take, until the next one starts.
    fn summary_panel(&self, ui: &mut egui::Ui, summary: &Summary) {
        ui.heading("Last take");
        egui::Grid::new("take-summary").num_columns(2).show(ui, |ui| {
            ui.strong("File");
            ui.label(summary.file.display().to_string());
            ui.end_row();
            ui.strong("Frames");
            ui.label(summary.frames.to_string());
            ui.end_row();
            ui.strong("Duplicated");
            ui.label(summary.duplicated.to_string());
            ui.end_row();
            ui.strong("Dropped");
            ui.label(summary.dropped.to_string());
            ui.end_row();
            ui.strong("Size");
            ui.label(format!("{:.1} MB", summary.bytes_written as f64 / 1_000_000.0));
            ui.end_row();
            ui.strong("Duration");
            ui.label(format_hms(summary.duration_ms / 1000));
            ui.end_row();
            ui.strong("Average bitrate");
            ui.label(format!("{:.2} Mbps", summary.average_mbps));
            ui.end_row();
        });
        ui.add_space(8.0);
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

/// The tray icon state that matches a controller phase.
fn tray_state(phase: Phase) -> TrayState {
    match phase {
        Phase::Idle => TrayState::Idle,
        Phase::Recording => TrayState::Recording,
        Phase::Paused => TrayState::Paused,
    }
}

/// Lists the audio devices that are present; on failure logs a warning and lists none.
fn list_devices() -> Vec<AudioDevice> {
    capture::list_audio_devices().unwrap_or_else(|err| {
        log::warn!("could not list the audio devices: {err}");
        Vec::new()
    })
}

/// Writes settings.json through a temp file in the same folder, leaving a file the Recorder could not read alone.
fn save_settings_file(settings: &Settings) -> anyhow::Result<()> {
    let dirs = directories::BaseDirs::new().ok_or_else(|| anyhow!("the user's config folder could not be found"))?;
    let path = dirs.config_dir().join("Cutmark").join("recorder").join("settings.json");
    if let Ok(text) = std::fs::read_to_string(&path)
        && serde_json::from_str::<Settings>(text.strip_prefix('\u{feff}').unwrap_or(&text)).is_err()
    {
        return Err(anyhow!("{} is not valid, so it was not overwritten", path.display()));
    }
    let mut text = serde_json::to_string_pretty(settings).context("could not turn the settings into JSON")?;
    text.push('\n');
    let tmp = path.with_extension("json.tmp");
    let written = write_synced(&tmp, text.as_bytes()).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        // Best effort: do not leave the temp file behind.
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("could not write {}", path.display()))
}

/// Writes the bytes to `path` and flushes them to disk.
fn write_synced(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()
}

/// Formats a number of seconds as HH:MM:SS.
fn format_hms(total_seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", total_seconds / 3600, total_seconds / 60 % 60, total_seconds % 60)
}

impl eframe::App for RecorderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            // The X button or the tray's Quit: the controller finishes the take's file before the app goes away.
            self.controller.stop();
            if self.settings_dirty {
                self.save_settings();
            }
        }
        let shared = self.controller.shared();
        let mut state = lock(&shared);
        self.sync_from_controller(&mut state);
        let active = state.phase != Phase::Idle;
        self.hide_to_tray_when_minimized(&ctx, active);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutmark Recorder");
            ui.label(format!("Active profile: {}", state.active_profile));
            ui.add_space(8.0);

            let warn = ui.visuals().warn_fg_color;
            let error = ui.visuals().error_fg_color;
            if self.capture_exclusion_failed {
                ui.colored_label(warn, "This window may appear in recordings");
            }
            if let Some(message) = &state.error {
                ui.colored_label(error, format!("Recording error: {message}"));
            }
            if let Some(notice) = &state.audio_notice {
                ui.colored_label(warn, notice);
            }
            for problem in &self.hotkeys.problems {
                ui.colored_label(warn, problem);
            }
            ui.add_space(8.0);

            if active {
                self.recording_panel(ui, &state);
                ctx.request_repaint_after(METER_REFRESH);
            } else {
                ui.label("Idle");
                if let Some(summary) = &state.summary {
                    self.summary_panel(ui, summary);
                }
            }
            ui.add_space(8.0);

            let live_tracks = if active { Some(&self.live_tracks) } else { None };
            let changed = tracks_ui::tracks_panel(ui, &mut self.settings.audio, &self.devices, live_tracks, active);
            self.on_tracks_changed(&ctx, changed);
            ui.add_space(8.0);

            ui.heading("Hotkeys");
            self.hotkey_grid(ui);
            ui.add_space(8.0);

            ui.heading("Recent commands");
            if state.recent.is_empty() {
                ui.label("None yet. Press a hotkey.");
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for command in state.recent.iter().rev() {
                        ui.label(format!("{:?}", command.kind));
                    }
                });
            }
        });
    }
}
