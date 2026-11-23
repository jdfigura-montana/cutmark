//! The Recorder window: draws what the engine controller publishes and sends it commands and settings.

use std::cell::Cell;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use capture::{AudioDevice, CompositeInput, Preview};
use ui_kit::tokens::{color, size, space, typography};

use crate::api::{ApiServer, token};
use crate::compositor::{ComposeConfig, CompositorThread};
use crate::config::{self, Mode, ModeState, Settings};
use crate::engine::bus::{BusSender, Command, CommandKind, MarkKind};
use crate::engine::controller::{Controller, ControllerState, Phase, lock};
use crate::engine::session::Summary;
use crate::hotkeys::Hotkeys;
use crate::key_ui::{self, KeyUiState};
use crate::label_popup::PopupHandle;
use crate::layout_ui;
use crate::marks_ui;
use crate::platform;
use crate::preflight_ui::PreflightSection;
use crate::preview_ui::PreviewView;
use crate::profile_ui::{ProfileOutcome, ProfileSection};
use crate::profiles::ProfileStore;
use crate::recovery::{self, RecoveryReport};
use crate::status_ui;
use crate::tools_ui::{self, Tool};
use crate::tracks_ui::{self, LiveTrackStats, TracksChanged};
use crate::tray::{Tray, TrayHandle, TrayState};
use crate::webcam::WebcamHost;
use crate::webcam_ui;

/// How often the window repaints while recording, so the level meters move smoothly.
const METER_REFRESH: Duration = Duration::from_millis(33);

/// How often the window looks for the recovery report until it has arrived.
const RECOVERY_POLL: Duration = Duration::from_millis(250);

/// Side of the app mark square at the left edge of the title bar.
const APP_MARK: f32 = 14.0;

/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// Left padding of a panel header's title; with the panel's own 8px padding the title sits 14px from the panel edge.
const HEADER_PAD: f32 = 6.0;

/// Width of the left Sources panel (spec "Panels": default 270).
const LEFT_PANEL_W: f32 = 270.0;

/// Width of the right settings panel (spec "Panels": default 340).
const RIGHT_PANEL_W: f32 = 340.0;

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
    /// The live preview of the composite, or None while it is off or could not start.
    preview: Option<Preview>,
    /// Draws the preview picture.
    preview_view: PreviewView,
    /// The Preview toggle; on at every start and not saved.
    preview_enabled: bool,
    /// The monitor the composite shows; there is no source setting yet, so this is the primary monitor like the recording.
    preview_monitor: usize,
    /// Whether the composite shows the cursor, like the recording.
    preview_cursor: bool,
    /// The output size the preview was last started for, or None while it is off.
    preview_started_with: Option<(u32, u32)>,
    /// The compose thread that makes the preview picture and the recording's video; started with the preview.
    compositor: Option<CompositorThread>,
    /// Why the preview could not start, for the yellow banner.
    preview_start_error: Option<String>,
    /// Whether the preview was last told to run, so the window only tells it about changes.
    preview_active: bool,
    /// The webcam of the active profile: its settings, the open camera, its preview and its controls.
    webcam: WebcamHost,
    /// The matte view and the eyedropper of the Chroma key section; never saved, and off whenever a take starts.
    key_ui: KeyUiState,
    /// The Setup tool the right panel shows; persisted with the settings (spec "Mode rules").
    tool: Tool,
    /// Where the startup recovery scan delivers its report, until it has.
    recovery_rx: Option<std::sync::mpsc::Receiver<RecoveryReport>>,
    /// What the recovery scan found, until the banner is dismissed.
    recovery: Option<RecoveryReport>,
    /// Whether the recovery banner lists the recovered take names.
    recovery_details: bool,
    /// The profile dropdown, its buttons and every encoder option.
    profile_section: ProfileSection,
    /// The profiles in memory: the one place the Profile and Webcam sections edit, with one debounced save.
    profile_store: ProfileStore,
    /// The size of the captured monitor, which a profile with the Source output size records at.
    source_size: (u32, u32),
    /// The control API server, or None while it is off or could not start.
    api: Option<ApiServer>,
    /// The token clients must send, or None when Credential Manager could not give one.
    api_token: Option<String>,
    /// The port and token the API was last started for, so the window only restarts it when they change.
    api_started_with: Option<(u16, String)>,
    /// The text of the yellow banner when the API is on but not running.
    api_error: Option<String>,
    /// Whether the token is shown in full.
    api_show_token: bool,
    /// Set by the Regenerate button; done at the start of the next frame, when the state lock is free.
    api_regenerate: bool,
    /// The port as typed in the window, applied when the field is left.
    api_port_edit: u16,
    /// Set once the window is closing, so the API is not started again.
    shutting_down: bool,
    /// The Pre-flight section: its button and the Keep files choice.
    preflight_section: PreflightSection,
    /// The window mode and the mode the running take returns to (spec "Mode rules").
    modes: ModeState,
    /// Whether the controller ran a take, countdown included, on the previous frame.
    was_running: bool,
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
        let mut profile_store = ProfileStore::load();
        let profile_section = ProfileSection::new(&cc.egui_ctx, &mut profile_store, &mut settings, source_size);
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
        let (api_token, api_error) = match token::load_or_create() {
            Ok(token) => (Some(token), None),
            Err(err) => {
                log::warn!("the control API token is not available: {err}");
                (None, Some(format!("Control API is off: {err}")))
            }
        };
        let api_port_edit = settings.api.port;
        let tool = settings.tool;
        let modes = ModeState::new(settings.mode);
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
            compositor: None,
            preview_start_error: None,
            preview_active: true,
            webcam: WebcamHost::new(&cc.egui_ctx),
            key_ui: KeyUiState::default(),
            tool,
            recovery_rx,
            recovery: None,
            recovery_details: false,
            profile_section,
            profile_store,
            source_size,
            api: None,
            api_token,
            api_started_with: None,
            api_error,
            api_show_token: false,
            api_regenerate: false,
            api_port_edit,
            shutting_down: false,
            preflight_section: PreflightSection::default(),
            modes,
            was_running: false,
        };
        app.sync_preview(&cc.egui_ctx);
        app.sync_api(&cc.egui_ctx);
        app
    }

    /// Makes a new token when asked, then starts, restarts or stops the control API so it matches the switch, the port and the token.
    ///
    /// Stopping waits for the API's threads, which read the controller state, so this must run while the window does not hold the state lock.
    fn sync_api(&mut self, ctx: &egui::Context) {
        if self.api_regenerate {
            self.api_regenerate = false;
            match token::regenerate() {
                Ok(new_token) => self.api_token = Some(new_token),
                Err(err) => {
                    log::warn!("the control API token could not be replaced: {err}");
                    self.api_error = Some(format!("The API token could not be replaced: {err}"));
                }
            }
        }
        let wanted = match (&self.api_token, self.settings.api.enabled && !self.shutting_down) {
            (Some(token), true) => Some((self.settings.api.port, token.clone())),
            _ => None,
        };
        if wanted == self.api_started_with {
            return;
        }
        // Dropping the old server closes its connections and frees its port before the new one binds.
        self.api = None;
        self.api_error = None;
        self.api_started_with.clone_from(&wanted);
        let Some((port, token)) = wanted else { return };
        let repaint_ctx = ctx.clone();
        match ApiServer::start(port, token, self.controller.api_link(), Arc::new(move || repaint_ctx.request_repaint())) {
            Ok(server) => self.api = Some(server),
            Err(err) => {
                log::warn!("the control API could not start: {err}");
                self.api_error = Some(format!("Control API: {err}"));
            }
        }
    }

    /// Draws the Control API section: the switch, the port, the token and the number of connected clients.
    fn api_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Control API");
        ui.checkbox(&mut self.settings.api.enabled, "On");
        ui.horizontal(|ui| {
            ui.label("Port");
            let response = ui.add(egui::DragValue::new(&mut self.api_port_edit).range(1024..=65535).speed(0.0));
            if response.lost_focus() || response.drag_stopped() {
                self.settings.api.port = self.api_port_edit;
            }
        });
        match &self.api {
            Some(server) => {
                ui.label(format!("Listening on 127.0.0.1:{}", server.port()));
                ui.label(format!("Connected clients: {}", server.client_count()));
            }
            None => {
                ui.label("Not running");
            }
        }
        let Some(api_token) = self.api_token.clone() else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label("Token");
            let shown = if self.api_show_token { api_token.clone() } else { "*".repeat(api_token.chars().count().min(24)) };
            ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).wrap());
        });
        ui.horizontal(|ui| {
            if ui.button(if self.api_show_token { "Hide" } else { "Show" }).clicked() {
                self.api_show_token = !self.api_show_token;
            }
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(api_token);
            }
            if ui.button("Regenerate").on_hover_text("Makes a new token and disconnects every client").clicked() {
                self.api_regenerate = true;
                ui.ctx().request_repaint();
            }
        });
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

    /// The size of the composite: the active profile's output size, or the captured monitor's for the Source size.
    fn output_size(&self) -> (u32, u32) {
        match self.profile_store.get(&self.settings.active_profile) {
            Some(profile) if !profile.is_source_size() => (profile.width, profile.height),
            _ => (self.source_size.0 & !1, self.source_size.1 & !1),
        }
    }

    /// Starts, restarts or stops the preview so it matches the toggle and the profile's output size.
    fn sync_preview(&mut self, ctx: &egui::Context) {
        let wanted = self.preview_enabled.then(|| self.output_size());
        if wanted == self.preview_started_with {
            return;
        }
        // Dropping the old preview ends its thread before the new one starts.
        self.preview = None;
        self.preview_view.reset();
        self.preview_started_with = wanted;
        self.preview_start_error = None;
        let Some(size) = wanted else { return };
        match self.open_preview(ctx, size) {
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

    /// Starts the compose thread if it is not running, and a preview of its composite.
    fn open_preview(&mut self, ctx: &egui::Context, size: (u32, u32)) -> anyhow::Result<Preview> {
        if self.compositor.is_none() {
            let mut thread = CompositorThread::start(0).context("could not start the compositor")?;
            thread.register();
            self.compositor = Some(thread);
        }
        let handle = self.compositor.as_ref().ok_or_else(|| anyhow!("the compositor is missing"))?.handle().clone();
        let source = handle.clone();
        let preview = Preview::start_composite(
            &handle.gpu(),
            size,
            Box::new(move || {
                source.latest().map(|frame| CompositeInput { gpu: frame.gpu, texture: frame.texture, width: frame.width, height: frame.height, seq: frame.seq })
            }),
        )
        .context("could not start the preview")?;
        let repaint_ctx = ctx.clone();
        preview.set_frame_callback(Box::new(move || repaint_ctx.request_repaint()));
        Ok(preview)
    }

    /// Gives the compose thread the active profile's picture, and runs it while the window is shown with the preview on.
    ///
    /// A recording keeps the thread running and decides its picture for the length of the take.
    fn sync_compositor(&self, window_shown: bool) {
        let Some(thread) = &self.compositor else { return };
        let handle = thread.handle();
        if let Some(profile) = self.profile_store.get(&self.settings.active_profile) {
            let mut config = ComposeConfig::from_profile(profile, self.source_size, self.preview_monitor, self.preview_cursor);
            config.matte = self.key_ui.matte;
            handle.set_config(config);
        }
        handle.set_active(window_shown && self.preview_enabled);
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
        self.preview_start_error
            .clone()
            .or_else(|| self.compositor.as_ref().and_then(|thread| thread.handle().error()))
            .or_else(|| self.preview.as_ref().and_then(Preview::error))
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
        let active = state.phase.has_take();
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

    /// Draws the countdown settings: how long it counts, and whether a linked Prompter's own countdown replaces it.
    fn countdown_section(&mut self, ui: &mut egui::Ui) {
        ui.heading("Countdown");
        egui::ComboBox::from_label("Countdown before recording").selected_text(countdown_label(self.settings.countdown_secs)).show_ui(ui, |ui| {
            for secs in COUNTDOWN_CHOICES {
                ui.selectable_value(&mut self.settings.countdown_secs, secs, countdown_label(secs));
            }
        });
        ui.checkbox(&mut self.settings.hide_countdown_when_linked, "Hide my countdown when a prompter is linked");
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

    /// Follows the controller phase (spec "Mode rules"): a take starting, countdown included,
    /// switches to Record and remembers the mode that was open; the take ending returns to it.
    /// The mode is persisted with the settings.
    fn sync_mode(&mut self, state: &ControllerState) {
        let running = state.phase != Phase::Idle;
        if running && !self.was_running {
            self.modes.start();
        } else if !running && self.was_running {
            self.modes.stop();
        }
        if self.modes.mode != self.settings.mode {
            self.settings.mode = self.modes.mode;
            self.settings_dirty = true;
        }
        self.was_running = running;
    }

    /// Moves to `to` when the mode rules allow it; the mode is persisted with the settings.
    fn set_mode(&mut self, recording: bool, to: Mode) {
        if self.modes.can_switch(recording, to) && self.modes.mode != to {
            self.modes.mode = to;
            self.settings.mode = to;
            self.settings_dirty = true;
        }
    }

    /// Ctrl+1, Ctrl+2 and Ctrl+3 switch to Setup, Record and Review (spec "Menus": View).
    fn handle_mode_keys(&mut self, ctx: &egui::Context, recording: bool) {
        for (key, mode) in [(egui::Key::Num1, Mode::Setup), (egui::Key::Num2, Mode::Record), (egui::Key::Num3, Mode::Review)] {
            if ctx.input(|input| input.modifiers.ctrl && input.key_pressed(key)) {
                self.set_mode(recording, mode);
            }
        }
    }

    /// The caption next to the project name: the profile, its output size and frame rate (spec "Title bar").
    fn title_caption(&self) -> String {
        let profile = self.profile_store.get(&self.settings.active_profile);
        let size = match profile {
            Some(profile) if !profile.is_source_size() => format!("{}x{}", profile.width, profile.height),
            _ => format!("Source {}x{}", self.source_size.0 & !1, self.source_size.1 & !1),
        };
        let fps = profile.map_or(0, |profile| profile.fps);
        format!("{} \u{b7} {} \u{b7} {} fps", self.settings.active_profile, size, fps)
    }

    /// Draws the 44px title bar (spec "Title bar", rev 3): the app mark and the menus on the
    /// left, the project name with its caption in the center, and the mode switch on the
    /// right; the separator and the caption buttons are the toolbar's own. A mode the user
    /// picked in the View menu or the switch is reported through `mode_picked`.
    fn title_bar(&self, ui: &mut egui::Ui, running: bool, state: &ControllerState, mode_picked: &Cell<Option<Mode>>) {
        let bus = self.controller.sender();
        let project = self.settings.project.clone();
        let caption = self.title_caption();
        let chords = Chords::of(&self.hotkeys);
        let project_folder = project_folder(&self.settings);
        // The pre-flight button is enabled while idle and not already running; the sync test
        // also needs the camera file on, exactly like the buttons in the parked column.
        let idle = state.phase == Phase::Idle && state.preflight.is_none();
        let sync_ready = idle && state.sync_test.is_none() && self.webcam.settings(&self.profile_store).is_some_and(|settings| settings.iso);
        let keep_files = self.preflight_section.keep_files();
        let mode = self.modes.mode;
        let mode_labels = Mode::ALL.map(Mode::label);
        let selected = Cell::new(Mode::ALL.iter().position(|candidate| *candidate == mode).unwrap_or(0));

        ui_kit::main_toolbar(
            ui,
            |ui| {
                app_mark(ui);
                let file = ui_kit::Menu::new("File", |ui| {
                    if menu_item(ui, "Open project folder", "").clicked() {
                        open_folder(&project_folder);
                        ui.close();
                    }
                    ui.separator();
                    ui.add_enabled_ui(false, |ui| menu_item(ui, "Settings", "Ctrl+,").on_disabled_hover_text("Arrives with the Settings dialog (R6)"));
                    ui.separator();
                    if menu_item(ui, "Exit", "").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });
                let capture = ui_kit::Menu::new("Capture", |ui| {
                    if menu_item(ui, "Start or Stop recording", &chords.start_stop).clicked() {
                        send(&bus, CommandKind::StartStop);
                        ui.close();
                    }
                    if menu_item(ui, "Pause or Resume", &chords.pause_resume).clicked() {
                        send(&bus, CommandKind::PauseResume);
                        ui.close();
                    }
                    ui.separator();
                    if menu_item(ui, "Add mark", &chords.mark).clicked() {
                        send(&bus, CommandKind::Mark(MarkKind::Silent));
                        ui.close();
                    }
                    if menu_item(ui, "Add mark with label", &chords.mark_label).clicked() {
                        send(&bus, CommandKind::Mark(MarkKind::WithLabel));
                        ui.close();
                    }
                    if menu_item(ui, "Mute mic", &chords.mute_mic).clicked() {
                        // The mic is assumed to sit on track 0, like the hotkey does.
                        send(&bus, CommandKind::Mute { track: 0 });
                        ui.close();
                    }
                    ui.separator();
                    ui.add_enabled_ui(idle, |ui| {
                        if menu_item(ui, "Run pre-flight", "").clicked() {
                            send(&bus, CommandKind::Preflight { keep_files });
                            ui.close();
                        }
                    });
                    ui.add_enabled_ui(sync_ready, |ui| {
                        if menu_item(ui, "Camera sync test", "").clicked() {
                            send(&bus, CommandKind::SyncTest);
                            ui.close();
                        }
                    });
                });
                let view = ui_kit::Menu::new("View", |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        if menu_item(ui, "Setup", "Ctrl+1").clicked() {
                            mode_picked.set(Some(Mode::Setup));
                            ui.close();
                        }
                    });
                    if menu_item(ui, "Record", "Ctrl+2").clicked() {
                        mode_picked.set(Some(Mode::Record));
                        ui.close();
                    }
                    ui.add_enabled_ui(!running, |ui| {
                        if menu_item(ui, "Review", "Ctrl+3").clicked() {
                            mode_picked.set(Some(Mode::Review));
                            ui.close();
                        }
                    });
                });
                // Help gets its items with the Help phase; the menu itself is here already.
                let help = ui_kit::Menu::new("Help", |_ui| {});
                ui_kit::menu_bar(ui, &mut [file, capture, view, help]);
            },
            |ui| {
                ui_kit::title_block(ui, &ui_kit::Title::Document { name: &project, unsaved: false, caption: Some(&caption) });
            },
            |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    let mut chosen = selected.get();
                    ui_kit::segmented(ui, "mode-switch", &mut chosen, &mode_labels);
                    selected.set(chosen);
                });
            },
        );

        // The switch wrote its selection into the cell; report a change through `mode_picked`.
        let picked = Mode::ALL.get(selected.get()).copied().unwrap_or(mode);
        if picked != mode {
            mode_picked.set(Some(picked));
        }
    }

    /// The Setup layout (spec "Window layout" and "Setup mode"): the Sources panel on the
    /// left, the Preview in the center and the parked settings column on the right, with the
    /// 8px window padding and gaps.
    fn setup_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        let gap = space::WINDOW_GAP as i8;
        let active = state.phase.has_take();
        egui::Panel::left("setup-sources")
            .resizable(false)
            .exact_size(LEFT_PANEL_W)
            .frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin { left: gap, right: 0, top: gap, bottom: gap }))
            .show(ui, sources_panel);
        egui::Panel::right("setup-settings")
            .resizable(false)
            .exact_size(RIGHT_PANEL_W)
            .frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin { left: 0, right: gap, top: gap, bottom: gap }))
            .show(ui, |ui| self.settings_panel(ui, ctx, state));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin { left: gap, right: gap, top: gap, bottom: gap }))
            .show(ui, |ui| self.preview_panel(ui, active));
    }

    /// The right Setup panel (spec "Setup mode"): the tool strip over the open tool's panel in a
    /// vertical ScrollArea (the ui-kit overlay scrollbar comes from ui_kit::apply's global scroll style).
    fn settings_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        let active = state.phase.has_take();
        ui_kit::panel(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let items = tools_ui::strip_items();
            let mut tool = self.tool;
            let strip = ui_kit::tool_strip(ui, &mut tool, &items);
            // The hairline under the tool strip runs edge to edge of the panel like every
            // panel header's (spec "Panel header"); the strip's own cells may end short of it.
            ui.painter().hline(ui.max_rect().x_range().expand(space::PANEL_PADDING), strip.rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
            if tool != self.tool {
                self.tool = tool;
                self.settings.tool = tool;
                self.settings_dirty = true;
            }
            ui.add_space(space::SCALE_8);
            egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                match self.tool {
                    Tool::Camera => {
                        tools_ui::camera_panel(ui, &mut self.webcam, &mut self.profile_store, active, state, &self.controller.sender(), &mut self.key_ui);
                    }
                    Tool::Key => {
                        let picked = webcam_ui::camera_picture(ui, &mut self.webcam, self.key_ui.eyedropper);
                        key_ui::key_panel(ui, &self.webcam, &mut self.profile_store, state, &mut self.key_ui, picked.as_ref());
                    }
                    Tool::Layout => layout_ui::show(ui, &mut self.profile_store, &self.settings.active_profile, active),
                    Tool::Output => {
                        let mut outcome = ProfileOutcome::default();
                        self.profile_section.output_panel(ui, &mut self.profile_store, &mut self.settings, self.source_size, active, &mut outcome);
                        self.on_profile_outcome(&outcome);
                        ui.add_space(space::SCALE_8);
                        webcam_ui::iso_group(ui, &self.webcam, &mut self.profile_store, active, &state.active_profile);
                    }
                    Tool::Audio => self.parked_column(ui, ctx, state),
                }
            });
            // Every settings change, the open tool among them, saves once the pointer is up and no field has focus.
            let settled = !ctx.input(|input| input.pointer.any_down()) && ctx.memory(|memory| memory.focused().is_none());
            if self.settings_dirty && settled {
                self.save_settings();
            }
            self.profile_store.save_when_due(ctx);
        });
    }

    /// The center Preview panel (spec "Setup mode"): the panel toolbar (rev 3, 44 tall) over
    /// today's preview filling the rest of the panel.
    fn preview_panel(&mut self, ui: &mut egui::Ui, active: bool) {
        let mut outcome = ProfileOutcome::default();
        ui_kit::panel(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.preview_toolbar(ui, active, &mut outcome);
            ui.add_space(space::SCALE_8);
            self.show_preview(ui);
        });
        self.on_profile_outcome(&outcome);
    }

    /// The preview panel's toolbar: the Profile caption and picker, a separator, the Preview and
    /// Matte view chips, and the next take on the right with its full path as the tooltip.
    fn preview_toolbar(&mut self, ui: &mut egui::Ui, active: bool, outcome: &mut ProfileOutcome) {
        let key_on = self.webcam.settings(&self.profile_store).is_some_and(|settings| settings.key.enabled);
        let (take_label, take_path) = self.profile_section.next_take(&self.profile_store, &self.settings);
        let next_take = match take_label {
            Some(label) => format!("Next take {label}"),
            None => "Next take".to_owned(),
        };
        let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
        let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(HEADER_PAD);
            ui.add(
                egui::Label::new(egui::RichText::new("Profile").font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY))
                    .selectable(false),
            );
            ui.add_space(space::SCALE_8);
            ui.add_enabled_ui(!active, |ui| {
                self.profile_section.picker(ui, &mut self.profile_store, &mut self.settings, outcome);
            });
            ui.separator();
            ui.add_space(space::SCALE_4);
            ui_kit::chip(ui, &mut self.preview_enabled, "Preview");
            ui.add_space(space::SCALE_4);
            ui.add_enabled_ui(key_on, |ui| {
                ui_kit::chip(ui, &mut self.key_ui.matte, "Matte view");
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(next_take).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY))
                        .selectable(false),
                )
                .on_hover_text(take_path);
            });
        });
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    }

    /// Sends the settings to the controller and saves them when the active profile changed.
    fn on_profile_outcome(&mut self, outcome: &ProfileOutcome) {
        if outcome.active_changed {
            self.controller.set_settings(self.settings.clone());
            self.save_settings();
        }
    }

    /// The Record and Review placeholder: an empty panel with a centered caption (spec step R2).
    fn placeholder_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin::same(space::WINDOW_GAP as i8))).show(
            ui,
            |ui| {
                ui_kit::panel(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    let galley =
                        ui.painter().layout_no_wrap("Coming in Phase 13".to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), color::TEXT_TERTIARY);
                    let center = ui.max_rect().center();
                    ui.painter().galley(center - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
                });
            },
        );
    }

    /// PARKED: holds what R5 and R6 still take. The camera and key controls moved to their tools,
    /// and the Layout, Output and Camera ISO file groups to the Output tool; the rest waits for
    /// the Sources panel and the Settings dialog (R5 and R6).
    fn parked_column(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        let warn = ui.visuals().warn_fg_color;
        let error = ui.visuals().error_fg_color;
        ui.heading("Cutmark Recorder");
        ui.label(format!("Active profile: {}", self.settings.active_profile));
        ui.add_space(8.0);

        if self.capture_exclusion_failed {
            ui.colored_label(warn, "This window may appear in recordings");
        }
        if let Some(message) = &state.error {
            ui.colored_label(error, format!("Recording error: {message}"));
        }
        if let Some(notice) = &state.audio_notice {
            ui.colored_label(warn, notice);
        }
        for banner in &state.banners {
            ui.colored_label(warn, banner);
        }
        for problem in &self.hotkeys.problems {
            ui.colored_label(warn, problem);
        }
        if let Some(problem) = self.preview_problem() {
            ui.colored_label(warn, format!("Preview: {problem}"));
        }
        if let Some(problem) = self.webcam.warning() {
            ui.colored_label(warn, format!("Webcam: {problem}"));
        }
        if let Some(problem) = &self.api_error {
            ui.colored_label(warn, problem);
        }
        self.recovery_banner(ui);
        ui.add_space(8.0);

        if ui.checkbox(&mut self.settings.hide_from_capture, "Hide Recorder from screen captures").changed() {
            self.apply_hide_from_capture();
        }
        ui.add_space(8.0);

        let active = state.phase.has_take();
        marks_ui::show(ui, &state.recent_marks);
        ui.add_space(8.0);

        if active {
            self.recording_panel(ui, state);
            ctx.request_repaint_after(METER_REFRESH);
        } else if let Phase::Countdown(left) = state.phase {
            ui.heading(format!("Starting in {left}"));
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

        // PARKED: the Project and Naming template fields until the Settings dialog takes them in R6.
        let mut profile_outcome = ProfileOutcome::default();
        self.profile_section.naming_fields(ui, &mut self.settings, &mut profile_outcome);
        self.on_profile_outcome(&profile_outcome);
        ui.add_space(8.0);

        self.countdown_section(ui);
        ui.add_space(8.0);

        self.preflight_section.show(ui, state, &self.controller.sender());
        ui.add_space(8.0);

        self.api_section(ui);
        ui.add_space(8.0);

        let live_tracks = if active { Some(&self.live_tracks) } else { None };
        let changed = tracks_ui::tracks_panel(ui, &mut self.settings.audio, &self.devices, live_tracks, active);
        self.push_settings_if_changed();
        self.on_tracks_changed(ctx, changed);
        ui.add_space(8.0);

        ui.heading("Hotkeys");
        self.hotkey_grid(ui);
        ui.add_space(8.0);

        ui.heading("Recent commands");
        if state.recent.is_empty() {
            ui.label("None yet. Press a hotkey.");
        } else {
            for command in state.recent.iter().rev() {
                ui.label(format!("{:?}", command.kind));
            }
        }
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

/// The countdown lengths the settings offer, in seconds; 0 is off.
const COUNTDOWN_CHOICES: [u8; 3] = [0, 3, 5];

/// The text for a countdown length in the settings.
fn countdown_label(secs: u8) -> String {
    if secs == 0 { "Off".to_owned() } else { format!("{secs} seconds") }
}

/// The tray icon state that matches a controller phase.
fn tray_state(phase: Phase) -> TrayState {
    match phase {
        Phase::Idle | Phase::Countdown(_) => TrayState::Idle,
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

/// The hotkey chords the Capture menu shows as its shortcuts; empty when a chord is not registered.
struct Chords {
    start_stop: String,
    pause_resume: String,
    mark: String,
    mark_label: String,
    mute_mic: String,
}

impl Chords {
    /// Reads the registered chords for the Capture menu's actions.
    fn of(hotkeys: &Hotkeys) -> Self {
        let chord = |action: &str| hotkeys.registered.iter().find(|hotkey| hotkey.action == action).map_or_else(String::new, |hotkey| hotkey.chord.clone());
        Self {
            start_stop: chord("start_stop"),
            pause_resume: chord("pause_resume"),
            mark: chord("mark"),
            mark_label: chord("mark_label"),
            mute_mic: chord("mute_mic"),
        }
    }
}

/// The app mark at the left edge of the title bar: a small rounded square in the accent color.
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(APP_MARK, APP_MARK), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, ui_kit::tokens::radius::SEGMENT, ui_kit::tokens::accent::ACCENT);
    }
}

/// A panel header row (44 tall): the title in the semibold body font, with a hairline along
/// the bottom of the row that runs edge to edge of the panel (spec "Panel header").
fn panel_header(ui: &mut egui::Ui, title: &str) {
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.add(egui::Label::new(egui::RichText::new(title).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY)).selectable(false));
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// The left Sources panel: the header and an empty body for now; R5 fills it (spec "Setup mode").
fn sources_panel(ui: &mut egui::Ui) {
    ui_kit::panel(ui, |ui| {
        ui.set_min_size(ui.available_size());
        panel_header(ui, "Sources");
        ui.add_space(space::SCALE_8);
    });
}

/// One menu item: the label in the body font, then the hotkey in the mono value style
/// (spec "Menus": the items show the existing hotkeys as their shortcuts).
fn menu_item(ui: &mut egui::Ui, label: &str, chord: &str) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    job.append(label, 0.0, egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY));
    if !chord.is_empty() {
        job.append(chord, space::SCALE_12, egui::TextFormat::simple(ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY));
    }
    ui.add(egui::Button::new(job))
}

/// Sends one command to the controller bus with the current QPC time, logging when the bus has ended.
fn send(bus: &BusSender, kind: CommandKind) {
    let command = Command::new(kind, capture::qpc_now());
    if bus.send(command).is_err() {
        log::warn!("the command was not sent because the controller has ended");
    }
}

/// The project folder: the recording root joined with the project name made safe (the takes rule).
fn project_folder(settings: &Settings) -> std::path::PathBuf {
    let name = crate::takes::sanitize(&settings.project);
    let name = if name.is_empty() { crate::config::DEFAULT_PROJECT.to_owned() } else { name };
    config::takes_root(settings).join(name)
}

/// Opens a folder in Explorer; a failure is logged, not shown.
fn open_folder(folder: &std::path::Path) {
    if let Err(err) = std::process::Command::new("explorer").arg(folder).spawn() {
        log::warn!("could not open {}: {err}", folder.display());
    }
}

impl eframe::App for RecorderApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            // The X button or the tray's Quit: the API closes its connections, then the controller finishes the take's file.
            self.shutting_down = true;
            self.sync_api(&ctx);
            self.controller.stop();
            self.profile_store.flush();
            if self.settings_dirty {
                self.save_settings();
            }
        }
        self.sync_api(&ctx);
        let shared = self.controller.shared();
        let mut state = lock(&shared);
        self.sync_from_controller(&mut state);
        self.poll_recovery();
        if self.recovery_rx.is_some() {
            // Do not depend on the scan thread's wake-up alone: look again soon until the report has arrived.
            ctx.request_repaint_after(RECOVERY_POLL);
        }
        let active = state.phase.has_take();
        if !key_ui::is_idle(&state) {
            // The matte view is for tuning only: it goes off as soon as a countdown or a take starts, so a matte frame never reaches a take.
            self.key_ui.clear();
        }
        self.hide_to_tray_when_minimized(&ctx, active);
        self.sync_preview(&ctx);
        self.update_preview_activity(&ctx);
        let window_shown = !ctx.input(|input| input.viewport().minimized).unwrap_or(false);
        self.webcam.sync(&mut self.profile_store, &self.settings.active_profile, window_shown);
        self.sync_compositor(window_shown);
        // The mode rules (spec "Modes"): a take starting or ending moves the mode.
        self.sync_mode(&state);

        // The 26px status bar (spec "Status bar"): state, disk free, camera note, API clients.
        egui::Panel::bottom("status-bar").frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            let clients = self.api.as_ref().map(|server| server.client_count());
            status_ui::show(ui, &state, clients);
        });

        // The 44px title bar (spec "Title bar", rev 3); it reports a picked mode through the cell.
        let running = state.phase != Phase::Idle;
        let mode_picked = Cell::new(None::<Mode>);
        egui::Panel::top("title-bar").frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| self.title_bar(ui, running, &state, &mode_picked));
        if let Some(to) = mode_picked.get() {
            self.set_mode(running, to);
        }
        self.handle_mode_keys(&ctx, running);

        match self.modes.mode {
            Mode::Setup => self.setup_panels(ui, &ctx, &state),
            Mode::Record | Mode::Review => self.placeholder_panel(ui),
        }
    }
}
