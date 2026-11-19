//! The Recorder window: draws what the engine controller publishes and sends it commands and settings.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use capture::{AudioDevice, Gpu, Preview};

use crate::config::{self, Settings};
use crate::engine::controller::{Controller, ControllerState, Phase, lock};
use crate::engine::session::Summary;
use crate::hotkeys::Hotkeys;
use crate::label_popup::PopupHandle;
use crate::marks_ui;
use crate::platform;
use crate::preview_ui::PreviewView;
use crate::profile_ui::{ProfileOutcome, ProfileSection};
use crate::recovery::{self, RecoveryReport};
use crate::tracks_ui::{self, LiveTrackStats, TracksChanged};
use crate::tray::{Tray, TrayHandle, TrayState};

/// How often the window repaints while recording, so the level meters move smoothly.
const METER_REFRESH: Duration = Duration::from_millis(33);

/// How often the window looks for the recovery report until it has arrived.
const RECOVERY_POLL: Duration = Duration::from_millis(250);

/// The Recorder app: settings, hotkeys, the tray icon and the window over the engine controller.
pub struct RecorderApp {
    settings: Settings,
    /// The settings the controller was last given, to notice every change the window makes.
    sent_settings: Settings,
    /// The main window's handle, or None when it could not be read.
    hwnd: Option<isize>,
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
    /// The live monitor preview, or None while it is off or could not start.
    preview: Option<Preview>,
    /// Draws the preview picture.
    preview_view: PreviewView,
    /// The Preview toggle; on at every start and not saved.
    preview_enabled: bool,
    /// The monitor the preview shows; there is no source setting yet, so this is the primary monitor like the recording.
    preview_monitor: usize,
    /// Whether the preview shows the cursor, like the recording.
    preview_cursor: bool,
    /// The monitor and cursor setting the preview was last started for, or None while it is off.
    preview_started_with: Option<(usize, bool)>,
    /// The graphics device of the preview, kept apart from the recording session's.
    preview_gpu: Option<Gpu>,
    /// Why the preview could not start, for the yellow banner.
    preview_start_error: Option<String>,
    /// Whether the preview was last told to run, so the window only tells it about changes.
    preview_active: bool,
    /// Where the startup recovery scan delivers its report, until it has.
    recovery_rx: Option<std::sync::mpsc::Receiver<RecoveryReport>>,
    /// What the recovery scan found, until the banner is dismissed.
    recovery: Option<RecoveryReport>,
    /// Whether the recovery banner lists the recovered take names.
    recovery_details: bool,
    /// The profile dropdown, its buttons and every encoder option.
    profile_section: ProfileSection,
    /// The size of the captured monitor, which a profile with the Source output size records at.
    source_size: (u32, u32),
}

impl RecorderApp {
    /// Applies the suite theme, excludes the window from capture, loads settings, starts the controller, the hotkeys and the tray icon.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_kit::apply(&cc.egui_ctx);
        let mut settings = config::load_or_init().unwrap_or_else(|err| {
            log::error!("could not load the Recorder settings, using the defaults: {err}");
            Settings::default()
        });
        let hwnd = match platform::raw_hwnd(cc) {
            Ok(hwnd) => Some(hwnd),
            Err(err) => {
                log::warn!("could not get the window handle, tray Show and Quit will only send commands: {err}");
                None
            }
        };
        let capture_exclusion_failed = !apply_capture_exclusion(hwnd, settings.hide_from_capture);
        let source_size = monitor_size();
        // This also makes the active profile one that exists, before the controller reads the settings.
        let profile_section = ProfileSection::new(&cc.egui_ctx, &mut settings, source_size);
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
        let controller = Controller::start(settings.clone(), Box::new(move || repaint_ctx.request_repaint()), listener, PopupHandle::start());
        let hotkeys = Hotkeys::start(controller.sender());
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
        let recovery_rx = start_recovery(&settings, &controller, &cc.egui_ctx);
        let mut app = Self {
            sent_settings: settings.clone(),
            hwnd,
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
            preview: None,
            preview_view: PreviewView::default(),
            preview_enabled: true,
            preview_monitor: 0,
            preview_cursor: true,
            preview_started_with: None,
            preview_gpu: None,
            preview_start_error: None,
            preview_active: true,
            recovery_rx,
            recovery: None,
            recovery_details: false,
            profile_section,
            source_size,
        };
        app.sync_preview(&cc.egui_ctx);
        app
    }

    /// Takes in the recovery report once the background scan has delivered it.
    fn poll_recovery(&mut self) {
        let Some(rx) = &self.recovery_rx else { return };
        match rx.try_recv() {
            Ok(report) => {
                self.recovery = Some(report);
                self.recovery_rx = None;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.recovery_rx = None,
        }
    }

    /// Draws the recovery banners: an info banner for recovered takes and a warning for errors, with a dismiss button.
    fn recovery_banner(&mut self, ui: &mut egui::Ui) {
        let Some(report) = &self.recovery else { return };
        let warn = ui.visuals().warn_fg_color;
        let mut details = self.recovery_details;
        let mut dismiss = false;
        if !report.recovered.is_empty() {
            ui.horizontal(|ui| {
                ui.colored_label(ui_kit::palette::ACCENT, format!("Recovered marks for {} take(s)", report.recovered.len()));
                if ui.small_button(if details { "Hide details" } else { "Details" }).clicked() {
                    details = !details;
                }
                if ui.small_button("Dismiss").clicked() {
                    dismiss = true;
                }
            });
            if details {
                for name in &report.recovered {
                    ui.label(format!("    {name}"));
                }
            }
        }
        if !report.errors.is_empty() {
            for error in &report.errors {
                ui.colored_label(warn, format!("Marks recovery: {error}"));
            }
            if report.recovered.is_empty() && ui.small_button("Dismiss").clicked() {
                dismiss = true;
            }
        }
        self.recovery_details = details;
        if dismiss {
            self.recovery = None;
            self.recovery_details = false;
        }
    }

    /// Starts, restarts or stops the preview so it matches the toggle, the monitor and the cursor setting.
    fn sync_preview(&mut self, ctx: &egui::Context) {
        let wanted = self.preview_enabled.then_some((self.preview_monitor, self.preview_cursor));
        if wanted == self.preview_started_with {
            return;
        }
        // Dropping the old preview ends its thread and its capture before the new one starts.
        self.preview = None;
        self.preview_view.reset();
        self.preview_started_with = wanted;
        self.preview_start_error = None;
        let Some((monitor, cursor)) = wanted else { return };
        match self.open_preview(ctx, monitor, cursor) {
            Ok(preview) => {
                self.preview = Some(preview);
                self.preview_active = true;
            }
            Err(err) => {
                log::warn!("the preview could not start: {err:#}");
                self.preview_start_error = Some(format!("{err:#}"));
            }
        }
    }

    /// Starts a preview of one monitor on the preview's own graphics device.
    fn open_preview(&mut self, ctx: &egui::Context, monitor_index: usize, cursor: bool) -> anyhow::Result<Preview> {
        if self.preview_gpu.is_none() {
            self.preview_gpu = Some(Gpu::create(Some(0)).context("could not open the graphics adapter")?);
        }
        let gpu = self.preview_gpu.as_ref().ok_or_else(|| anyhow!("the graphics device is missing"))?;
        let monitors = capture::list_monitors().context("could not list the monitors")?;
        let monitor = monitors.get(monitor_index).ok_or_else(|| anyhow!("monitor {monitor_index} was not found"))?;
        let preview = Preview::start(gpu, monitor, cursor).context("could not start the capture")?;
        let repaint_ctx = ctx.clone();
        preview.set_frame_callback(Box::new(move || repaint_ctx.request_repaint()));
        Ok(preview)
    }

    /// Pauses the preview while the window is minimized or hidden to the tray, and resumes it when the window is back.
    fn update_preview_activity(&mut self, ctx: &egui::Context) {
        let shown = !ctx.input(|input| input.viewport().minimized).unwrap_or(false);
        if shown != self.preview_active {
            self.preview_active = shown;
            if let Some(preview) = &self.preview {
                preview.set_active(shown);
            }
        }
    }

    /// Draws the preview, or a plain placeholder when it is off or could not start.
    fn show_preview(&mut self, ui: &mut egui::Ui) {
        match (&self.preview, self.preview_enabled) {
            (Some(preview), true) => self.preview_view.show(ui, preview),
            (_, true) => PreviewView::placeholder(ui, "Preview unavailable"),
            (_, false) => PreviewView::placeholder(ui, "Preview off"),
        }
    }

    /// The text of the preview's warning, if it has one.
    fn preview_problem(&self) -> Option<String> {
        self.preview_start_error.clone().or_else(|| self.preview.as_ref().and_then(Preview::error))
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

    /// Gives the controller the settings whenever they differ from what it last got, and marks them to be saved.
    ///
    /// Every UI change (project, naming template, root folder, profile, tracks) reaches the controller through here,
    /// so the next take is built from the settings on screen.
    fn push_settings_if_changed(&mut self) {
        if self.settings != self.sent_settings {
            self.sent_settings.clone_from(&self.settings);
            self.controller.set_settings(self.settings.clone());
            self.settings_dirty = true;
        }
    }

    /// Applies the Hide from screen captures switch to the main window; shows the warning when hiding failed.
    fn apply_hide_from_capture(&mut self) {
        self.capture_exclusion_failed = !apply_capture_exclusion(self.hwnd, self.settings.hide_from_capture);
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

/// Hides the window from screen capture or shows it to capture again; false when that failed or there is no window handle.
fn apply_capture_exclusion(hwnd: Option<isize>, exclude: bool) -> bool {
    let Some(hwnd) = hwnd else { return !exclude };
    match platform::set_capture_exclusion(hwnd, exclude) {
        Ok(()) => true,
        Err(err) => {
            log::warn!("could not change whether the Recorder window is hidden from screen capture: {err}");
            false
        }
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

/// Starts the marks recovery scan of the takes folder on a background thread and returns where its report arrives.
///
/// Returns None when the thread could not start.
fn start_recovery(settings: &Settings, controller: &Controller, ctx: &egui::Context) -> Option<std::sync::mpsc::Receiver<RecoveryReport>> {
    let root = crate::config::takes_root(settings);
    let shared = controller.shared();
    let ctx = ctx.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("recovery".to_owned()).spawn(move || {
        let is_recording = |folder: &std::path::Path, base_name: &str| {
            let state = lock(&shared);
            state.phase != Phase::Idle && state.take.as_ref().is_some_and(|take| take.folder == folder && take.base_name == base_name)
        };
        let report = recovery::scan(&root, &is_recording);
        // The window may be gone by now; then nobody needs the report.
        let _ = tx.send(report);
        ctx.request_repaint();
    });
    match spawned {
        Ok(_) => Some(rx),
        Err(err) => {
            log::warn!("the marks recovery could not start: {err}");
            None
        }
    }
}

/// The size of the primary monitor, rounded down to even numbers; 1920x1080 when it cannot be found.
fn monitor_size() -> (u32, u32) {
    let monitors = capture::list_monitors().unwrap_or_else(|err| {
        log::warn!("could not list the monitors: {err}");
        Vec::new()
    });
    match monitors.first() {
        Some(monitor) => (u32::try_from(monitor.width).unwrap_or(1920) & !1, u32::try_from(monitor.height).unwrap_or(1080) & !1),
        None => (1920, 1080),
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
            self.profile_section.flush();
            if self.settings_dirty {
                self.save_settings();
            }
        }
        let shared = self.controller.shared();
        let mut state = lock(&shared);
        self.sync_from_controller(&mut state);
        self.poll_recovery();
        if self.recovery_rx.is_some() {
            // Do not depend on the scan thread's wake-up alone: look again soon until the report has arrived.
            ctx.request_repaint_after(RECOVERY_POLL);
        }
        let active = state.phase != Phase::Idle;
        self.hide_to_tray_when_minimized(&ctx, active);
        self.sync_preview(&ctx);
        self.update_preview_activity(&ctx);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Cutmark Recorder");
            ui.label(format!("Active profile: {}", self.settings.active_profile));
            ui.add_space(8.0);

            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
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
                if let Some(problem) = self.preview_problem() {
                    ui.colored_label(warn, format!("Preview: {problem}"));
                }
                self.recovery_banner(ui);
                ui.add_space(8.0);

                ui.checkbox(&mut self.preview_enabled, "Preview");
                if ui.checkbox(&mut self.settings.hide_from_capture, "Hide Recorder from screen captures").changed() {
                    self.apply_hide_from_capture();
                }
                self.show_preview(ui);
                ui.add_space(8.0);

                marks_ui::show(ui, &state.recent_marks);
                ui.add_space(8.0);

                if active {
                    self.recording_panel(ui, &state);
                    ctx.request_repaint_after(METER_REFRESH);
                } else {
                    if self.profile_section.blocked() {
                        ui.colored_label(error, "Idle. Starting is blocked until the profile errors below are fixed");
                    } else {
                        ui.label("Idle");
                    }
                    if let Some(summary) = &state.summary {
                        self.summary_panel(ui, summary);
                    }
                }
                ui.add_space(8.0);

                let mut profile_outcome = ProfileOutcome::default();
                self.profile_section.show(ui, &mut self.settings, self.source_size, active, &mut profile_outcome);
                if profile_outcome.active_changed {
                    self.controller.set_settings(self.settings.clone());
                    self.save_settings();
                }
                ui.add_space(8.0);

                let live_tracks = if active { Some(&self.live_tracks) } else { None };
                let changed = tracks_ui::tracks_panel(ui, &mut self.settings.audio, &self.devices, live_tracks, active);
                self.push_settings_if_changed();
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
        });
    }
}
