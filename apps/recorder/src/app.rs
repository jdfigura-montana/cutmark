//! The Recorder window: the active profile, the hotkeys, the live recording status and the recent commands.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::config::{self, AudioCodecChoice, Profile, Settings};
use crate::engine::bus::{Bus, Command};
use crate::engine::session::{RecordingSession, SessionConfig, StatsSnapshot, Summary, TrackSpec};
use crate::hotkeys::Hotkeys;
use crate::platform;
use crate::takes::{self, TakePaths};
use crate::tray::Tray;

/// How many recent commands the window keeps.
const RECENT_LIMIT: usize = 20;

/// How often the status refreshes while recording.
const STATUS_REFRESH: Duration = Duration::from_millis(250);

/// The Recorder app: settings, hotkeys, the tray icon, the command bus, the running take and the recent commands.
pub struct RecorderApp {
    settings: Settings,
    hotkeys: Hotkeys,
    tray: Option<Tray>,
    bus: Bus,
    recent: VecDeque<Command>,
    /// The running take, when there is one.
    session: Option<RecordingSession>,
    /// The paths of the running take, for the status bar.
    take: Option<TakePaths>,
    /// Frames per second of the running take, for the elapsed clock.
    active_fps: u32,
    /// The bytes written at the previous stats sample, to work out the bitrate over the last second.
    last_poll: Option<(Instant, u64)>,
    /// The summary of the last finished take, shown until the next one starts.
    summary: Option<Summary>,
    /// The text of the last start or stop error, shown in a red banner until the next take.
    error: Option<String>,
    /// The text of the yellow banner naming audio devices that were missing when the take started.
    audio_notice: Option<String>,
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
        Self {
            settings,
            hotkeys,
            tray,
            bus,
            recent: VecDeque::with_capacity(RECENT_LIMIT),
            session: None,
            take: None,
            active_fps: 60,
            last_poll: None,
            summary: None,
            error: None,
            audio_notice: None,
            capture_exclusion_failed,
            was_minimized: false,
        }
    }

    /// Pulls hotkey presses and other queued commands off the bus and acts on them.
    fn handle_commands(&mut self) {
        self.hotkeys.pump();
        for command in self.bus.drain() {
            log::info!("command: {command:?}");
            if matches!(command, Command::StartStop) {
                if self.session.is_some() {
                    self.stop_take();
                } else {
                    self.start_take();
                }
            }
            if matches!(command, Command::PauseResume) {
                self.toggle_pause();
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
        if minimized && !self.was_minimized && self.session.is_some() && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.was_minimized = minimized;
    }

    /// Starts a new take; on failure shows a red banner and stays idle.
    fn start_take(&mut self) {
        self.error = None;
        self.audio_notice = None;
        self.summary = None;
        match self.start_take_inner() {
            Ok(()) => self.update_tray(true),
            Err(err) => {
                log::error!("could not start the recording: {err:#}");
                self.error = Some(format!("{err:#}"));
            }
        }
    }

    /// Picks the take folder, builds the session config from the active profile and starts the session.
    fn start_take_inner(&mut self) -> anyhow::Result<()> {
        let root = self.take_root()?;
        let take = takes::next_take(&root, chrono::Local::now())?;
        let profile = Profile::shipped().into_iter().find(|profile| profile.name == self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        self.active_fps = profile.fps;
        let (tracks, missing) = self.track_specs();
        if !missing.is_empty() {
            log::warn!("recording without these audio devices: {}", missing.join(", "));
            self.audio_notice =
                Some(format!("Audio device not found for {}. Recording anyway; those tracks stay silent until the device returns.", missing.join(", ")));
        }
        let (mixdown, track_streams) = (profile.mixdown, profile.aac_tracks);
        let cfg = SessionConfig {
            adapter: 0,
            monitor: 0,
            cursor: true,
            profile,
            output_file: take.video_file.clone(),
            tracks,
            audio_codec: audio_codec(self.settings.audio.codec),
            mixdown,
            track_streams,
            wav_sidecars: self.settings.audio.wav_sidecars,
            take_folder: take.video_file.parent().map(std::path::Path::to_path_buf).unwrap_or_default(),
            base_name: take.video_file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        };
        let session = RecordingSession::start(cfg).map_err(|err| anyhow!("could not start recording into {}: {err}", take.video_file.display()))?;
        self.session = Some(session);
        self.take = Some(take);
        self.last_poll = None;
        Ok(())
    }

    /// Builds a spec for every enabled track in the settings, resolving each device, and names the tracks whose device is missing.
    fn track_specs(&self) -> (Vec<TrackSpec>, Vec<String>) {
        let mut specs = Vec::new();
        let mut missing = Vec::new();
        for track in self.settings.audio.tracks.iter().filter(|track| track.enabled) {
            let device = capture::find_device(&track.device_id, &track.device_name);
            if device.is_none() {
                missing.push(if track.device_name.is_empty() { track.name.clone() } else { format!("{} ({})", track.name, track.device_name) });
            }
            specs.push(TrackSpec {
                name: track.name.clone(),
                device,
                device_id: track.device_id.clone(),
                device_name: track.device_name.clone(),
                gain_db: track.gain_db,
                muted: track.muted,
                offset_ms: track.offset_ms,
                in_mixdown: track.in_mixdown,
            });
        }
        (specs, missing)
    }

    /// Stops the running take, keeps its summary for the panel and returns the tray to idle.
    fn stop_take(&mut self) {
        let Some(session) = self.session.take() else { return };
        self.take = None;
        self.last_poll = None;
        self.update_tray(false);
        match session.stop() {
            Ok(summary) => {
                log::info!("take finished: {} frames, {} bytes", summary.frames, summary.bytes_written);
                self.summary = Some(summary);
            }
            Err(err) => {
                log::error!("the recording failed on stop: {err}");
                self.error = Some(format!("the recording failed on stop: {err}"));
            }
        }
    }

    /// Pauses the running take, or resumes it when it is paused.
    fn toggle_pause(&mut self) {
        let Some(session) = &self.session else { return };
        if session.stats().paused {
            session.resume();
            self.update_tray(true);
        } else {
            session.pause();
            self.update_tray(false);
        }
    }

    /// The folder takes go into: the settings root folder, or else the user's Videos folder joined with Cutmark.
    fn take_root(&self) -> anyhow::Result<PathBuf> {
        if let Some(root) = &self.settings.root_folder {
            return Ok(root.clone());
        }
        let dirs = directories::UserDirs::new().ok_or_else(|| anyhow!("the user's profile folders could not be found"))?;
        dirs.video_dir().map(|videos| videos.join("Cutmark")).ok_or_else(|| anyhow!("the user's Videos folder could not be found"))
    }

    /// Updates the tray icon to match the recording state.
    fn update_tray(&self, recording: bool) {
        if let Some(tray) = &self.tray {
            tray.set_recording(recording);
        }
    }

    /// Samples the running session's counters and the bitrate since the previous sample.
    fn live_stats(&mut self) -> Option<(StatsSnapshot, f64)> {
        let stats = self.session.as_ref()?.stats();
        let now = Instant::now();
        let mbps = match self.last_poll {
            Some((then, bytes_then)) => {
                let seconds = now.duration_since(then).as_secs_f64();
                if seconds > 0.0 { stats.bytes_written.saturating_sub(bytes_then) as f64 * 8.0 / 1_000_000.0 / seconds } else { 0.0 }
            }
            None => 0.0,
        };
        self.last_poll = Some((now, stats.bytes_written));
        Some((stats, mbps))
    }

    /// Draws the status while a take is running or paused.
    fn recording_panel(&self, ui: &mut egui::Ui, stats: &StatsSnapshot, current_mbps: f64) {
        let Some(take) = &self.take else { return };
        ui.heading(if stats.paused { "Paused" } else { "Recording" });
        let elapsed_seconds = stats.frames / u64::from(self.active_fps.max(1));
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
            ui.label(format!("{current_mbps:.1} Mbps"));
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

/// The audio codec the settings ask for, as the encoder takes it.
fn audio_codec(choice: AudioCodecChoice) -> media_io::AudioCodec {
    match choice {
        AudioCodecChoice::Aac320 => media_io::AudioCodec::Aac { kbps: 320 },
        AudioCodecChoice::Pcm24 => media_io::AudioCodec::Pcm24,
    }
}

/// Formats a number of seconds as HH:MM:SS.
fn format_hms(total_seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", total_seconds / 3600, total_seconds / 60 % 60, total_seconds % 60)
}

impl eframe::App for RecorderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            // The X button or the tray's Quit: finish the file before the app goes away, then let it close.
            self.stop_take();
        }
        self.handle_commands();
        self.hide_to_tray_when_minimized(&ctx);
        let live = self.live_stats();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutmark Recorder");
            ui.label(format!("Active profile: {}", self.settings.active_profile));
            ui.add_space(8.0);

            let warn = ui.visuals().warn_fg_color;
            let error = ui.visuals().error_fg_color;
            if self.capture_exclusion_failed {
                ui.colored_label(warn, "This window may appear in recordings");
            }
            if let Some(message) = &self.error {
                ui.colored_label(error, format!("Recording error: {message}"));
            }
            if let Some(notice) = &self.audio_notice {
                ui.colored_label(warn, notice);
            }
            for problem in &self.hotkeys.problems {
                ui.colored_label(warn, problem);
            }
            ui.add_space(8.0);

            if let Some((stats, current_mbps)) = live {
                self.recording_panel(ui, &stats, current_mbps);
                ctx.request_repaint_after(STATUS_REFRESH);
            } else {
                ui.label("Idle");
                if let Some(summary) = &self.summary {
                    self.summary_panel(ui, summary);
                }
            }
            ui.add_space(8.0);

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
