//! The Recorder window: draws what the engine controller publishes and sends it commands and settings.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context as _, anyhow};
use capture::{AudioDevice, CompositeInput, Preview};
use ui_kit::tokens::{color, size, space, typography};

use crate::api::{ApiServer, token};
use crate::bar_ui;
use crate::compositor::{ComposeConfig, CompositorThread};
use crate::config::{self, CompositeMode, Mode, ModeState, PanelWidths, Settings};
use crate::data_text;
use crate::engine::bus::{BusSender, Command, CommandKind, MarkKind};
use crate::engine::controller::{Controller, ControllerState, Phase, lock};
use crate::hotkeys::Hotkeys;
use crate::key_ui::{self, KeyUiState};
use crate::label_popup::PopupHandle;
use crate::layout_ui;
use crate::platform;
use crate::playback::Playback;
use crate::preflight_ui::PreflightSection;
use crate::preview_ui::PreviewView;
use crate::profile_ui::{self, Control, Lock, ProfileOutcome, ProfileSection};
use crate::profiles::ProfileStore;
use crate::record_ui;
use crate::recovery::{self, RecoveryReport};
use crate::replay_ui::{self, Notice};
use crate::review;
use crate::review_ui;
use crate::settings_ui::{self, ApiUi, NextTake, Section, SettingsDialog};
use crate::status_ui;
use crate::tools_ui::{self, Tool};
use crate::tracks_ui::{self, AppSources, LiveTrackStats, TracksChanged};

/// The Recorder's own appearance file folder and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::RECORDER;
use crate::tray::{Tray, TrayHandle, TrayState};
use crate::webcam::WebcamHost;
use crate::webcam_ui;

/// How often the window repaints while recording, so the level meters move smoothly.
const METER_REFRESH: Duration = Duration::from_millis(33);

/// How often the window looks for the recovery report until it has arrived.
const RECOVERY_POLL: Duration = Duration::from_millis(250);

/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// Half the height of the toast's Review button, so the button centers on the
/// toast capsule.
const TOAST_BUTTON_HALF_HEIGHT: f32 = 14.0;

/// Left padding of a panel header's title; with the panel's own 8px padding the title sits 14px from the panel edge.
const HEADER_PAD: f32 = 6.0;

/// Width the center column keeps while the side columns give way (spec "Panels").
const CENTER_MIN_W: f32 = 400.0;

/// Width of a column divider's accent line.
const DIVIDER_LINE_W: f32 = 2.0;

/// How far the arrow keys move a focused column divider.
const DIVIDER_KEY_STEP: f32 = 16.0;

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
    /// What the window keeps about app and desktop-except tracks.
    app_sources: AppSources,
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
    /// The press time of the newest mark command seen, so each press adds one Review mark.
    seen_mark_press: i64,
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
    /// The measured height of the Sources panel's pre-flight card, from the previous frame.
    sources_card_height: f32,
    /// The window mode and the mode the running take returns to (spec "Mode rules").
    modes: ModeState,
    /// Whether the controller ran a take, countdown included, on the previous frame.
    was_running: bool,
    /// The base name of the take that is running, for the stop toast.
    running_take: Option<String>,
    /// The Settings dialog: whether it is open and the section its right side shows.
    settings_dialog: SettingsDialog,
    /// The Review scan: the project's takes grouped by day, newest first.
    review_days: Vec<review::Day>,
    /// The selected take's name in Review, remembered while the app runs.
    review_selected: Option<String>,
    /// Whether the Review scan must run again before the Review panels draw.
    review_dirty: bool,
    /// The take the toast on screen offers a Review action for, until the toast is gone.
    review_take_pending: Option<String>,
    /// The toasts not shown yet, oldest first; each waits until the one before it is gone.
    notices: VecDeque<Notice>,
    /// Whether a toast was on screen on the previous frame.
    toast_up: bool,
    /// The number of the newest replay event already turned into a toast.
    seen_replay_seq: u64,
    /// The Review take panel's editing state: the rename field and the delete dialog.
    take_panel: review_ui::TakePanelState,
    /// The Review playback of the selected take, closed when Review is left or a take starts.
    review_playback: Option<Playback>,
    /// The main file the playback is opened for, so selecting another take closes it first.
    review_playback_for: Option<std::path::PathBuf>,
    /// Why the selected take's playback could not open, shown in the review center.
    review_playback_error: Option<String>,
    /// The review center's state: the frame texture, the scrub bar's drag state.
    review_center: review_ui::CenterState,
}

impl RecorderApp {
    /// Applies the suite theme, excludes the window from capture, loads settings, starts the controller, the hotkeys and the tray icon.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        ui_kit::apply_for(&cc.egui_ctx, APPEARANCE);
        data_text::apply(&cc.egui_ctx);
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
                handle.set_state(tray_state(state.phase, state.replay_on));
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
            app_sources: AppSources::default(),
            settings_dirty: false,
            capture_exclusion_failed,
            was_minimized: false,
            seen_settings_rev: 0,
            was_active: false,
            seen_mark_press: 0,
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
            sources_card_height: 0.0,
            modes,
            was_running: false,
            running_take: None,
            settings_dialog: SettingsDialog::default(),
            review_days: Vec::new(),
            review_selected: None,
            review_dirty: true,
            review_take_pending: None,
            notices: VecDeque::new(),
            toast_up: false,
            seen_replay_seq: 0,
            take_panel: review_ui::TakePanelState::default(),
            review_playback: None,
            review_playback_for: None,
            review_playback_error: None,
            review_center: review_ui::CenterState::default(),
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

    /// The banner strip at the top of the center panel (spec "Mapping"): one row per active
    /// warning banner, the recovered-marks banner with its Details popover and Dismiss button,
    /// and the "Starting is blocked" reason. Returns whether any row was drawn.
    fn banner_strip(&mut self, ui: &mut egui::Ui, state: &ControllerState) -> bool {
        let mut rows: Vec<BannerRow> = Vec::new();
        if self.capture_exclusion_failed {
            rows.push(BannerRow::Warn("This window may appear in recordings".to_owned()));
        }
        if let Some(message) = &state.error {
            rows.push(BannerRow::Error(format!("Recording error: {message}")));
        }
        if let Some(notice) = &state.audio_notice {
            rows.push(BannerRow::Warn(notice.clone()));
        }
        for banner in &state.banners {
            rows.push(BannerRow::Warn(banner.clone()));
        }
        for problem in &self.hotkeys.problems {
            rows.push(BannerRow::Warn(problem.clone()));
        }
        if let Some(problem) = self.preview_problem() {
            rows.push(BannerRow::Warn(format!("Preview: {problem}")));
        }
        if let Some(problem) = self.webcam.warning() {
            rows.push(BannerRow::Warn(format!("Webcam: {problem}")));
        }
        if let Some(problem) = &self.api_error {
            rows.push(BannerRow::Warn(problem.clone()));
        }
        if state.phase == Phase::Idle && self.profile_section.blocked() {
            rows.push(BannerRow::Error("Starting is blocked until the profile errors are fixed".to_owned()));
        }
        let recovered: Vec<String> = self.recovery.as_ref().map(|report| report.recovered.clone()).unwrap_or_default();
        let recovery_errors: Vec<String> =
            self.recovery.as_ref().map(|report| report.errors.iter().map(|error| format!("Marks recovery: {error}")).collect()).unwrap_or_default();
        if rows.is_empty() && recovered.is_empty() && recovery_errors.is_empty() {
            return false;
        }
        for row in &rows {
            match row {
                BannerRow::Warn(message) => {
                    banner_row(ui, color::SEM_METER_WARN, message, None, false);
                }
                BannerRow::Error(message) => {
                    banner_row(ui, color::SEM_RECORD, message, None, false);
                }
            }
        }
        if !recovered.is_empty() {
            let (_, details, dismiss) =
                banner_row(ui, color::SEM_METER_WARN, &format!("Recovered marks for {} take(s)", recovered.len()), Some("Details"), true);
            if let Some(details) = details {
                egui::Popup::menu(&details).show(|ui| {
                    for name in &recovered {
                        ui.label(name);
                    }
                });
            }
            if dismiss.is_some_and(|response| response.clicked()) {
                self.recovery = None;
            }
        }
        if !recovery_errors.is_empty() {
            let dismissable = recovered.is_empty();
            for message in &recovery_errors {
                let (_, _, dismiss) = banner_row(ui, color::SEM_METER_WARN, message, None, dismissable);
                if dismissable && dismiss.is_some_and(|response| response.clicked()) {
                    self.recovery = None;
                }
            }
        }
        true
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
        for notice in replay_ui::new_notices(state, &mut self.seen_replay_seq) {
            // A saved replay is a new take for the Review list.
            self.review_dirty |= notice.review.is_some();
            self.notices.push_back(notice);
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
        self.app_sources.observe(&self.settings.audio.tracks, &state.idle_apps, active.then_some(&self.live_tracks));
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

    /// Follows the controller phase (spec "Mode rules"): a take starting, countdown included,
    /// switches to Record and remembers the mode that was open; the take ending returns to it,
    /// shows the "takeNN saved" toast with its Review action, and flags the Review scan.
    fn sync_mode(&mut self, state: &ControllerState) {
        let running = state.phase != Phase::Idle;
        if running && !self.was_running {
            self.modes.start();
        } else if !running && self.was_running {
            self.modes.stop();
            if let Some(base_name) = self.running_take.take() {
                // The finished take joins the Review list, and the toast offers to open it there.
                self.review_dirty = true;
                let text = format!("{} saved", crate::profile_ui::take_label(&base_name));
                self.notices.push_back(Notice { text, review: Some(base_name) });
            }
        }
        self.running_take = state.take.as_ref().filter(|_| running).map(|take| take.base_name.clone());
        if self.modes.mode != self.settings.mode {
            self.settings.mode = self.modes.mode;
            self.settings_dirty = true;
        }
        self.was_running = running;
    }

    /// Moves to `to` when the mode rules allow it; the mode is persisted with the settings,
    /// and opening Review flags its scan.
    fn set_mode(&mut self, recording: bool, to: Mode) {
        if self.modes.can_switch(recording, to) && self.modes.mode != to {
            self.modes.mode = to;
            self.settings.mode = to;
            self.settings_dirty = true;
            if to == Mode::Review {
                self.review_dirty = true;
            }
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

    /// Shows the next waiting toast once no other is on screen, with its Review action.
    fn show_notices(&mut self, ctx: &egui::Context, running: bool) {
        if !self.toast_up
            && let Some(notice) = self.notices.pop_front()
        {
            ui_kit::toast(ctx, notice.text);
            self.review_take_pending = notice.review;
        }
        let shown = ui_kit::show_toasts(ctx);
        self.toast_up = shown.is_some();
        if !self.toast_up && !self.notices.is_empty() {
            ctx.request_repaint();
        }
        self.toast_review_action(ctx, shown, running);
    }

    /// Draws a toast's Review button beside the toast capsule and opens
    /// Review with that take selected when it is clicked; the button lives only
    /// as long as the toast shows (spec "Mode rules" 2). While a take runs the mode
    /// rules keep Review closed, so the button is disabled. `ui_kit::toast` carries
    /// no action, so the button is drawn in its own area next to the capsule.
    fn toast_review_action(&mut self, ctx: &egui::Context, shown: Option<egui::Response>, running: bool) {
        let pending = self.review_take_pending.clone();
        match (pending, shown) {
            (Some(take), Some(toast)) => {
                let clicked = Cell::new(false);
                egui::Area::new(egui::Id::new("toast-review-action"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(egui::pos2(toast.rect.right() + space::SCALE_8, toast.rect.center().y - TOAST_BUTTON_HALF_HEIGHT))
                    .show(ctx, |ui| {
                        let button = ui.add_enabled(!running, egui::Button::new("Review").small());
                        if button.on_disabled_hover_text("Review opens after the recording stops").clicked() {
                            clicked.set(true);
                        }
                    });
                if clicked.get() {
                    self.review_take_pending = None;
                    self.review_selected = Some(take);
                    // Also when Review is already open, so a take saved there is in the list.
                    self.review_dirty = true;
                    self.set_mode(running, Mode::Review);
                }
            }
            // The toast is gone; the Review action goes with it.
            (_, None) => self.review_take_pending = None,
            _ => {}
        }
    }

    /// Draws the 44px title bar (spec "Title bar", rev 3): the app mark and the menus on the
    /// left, the project name with its caption in the center, and the mode switch on the
    /// right; the separator and the caption buttons are the toolbar's own. Clicking the project
    /// name opens the rename popover; a mode the user picked in the View menu or the switch is
    /// reported through `mode_picked`.
    fn title_bar(&mut self, ui: &mut egui::Ui, running: bool, state: &ControllerState, mode_picked: &Cell<Option<Mode>>) {
        let bus = self.controller.sender();
        let project = self.settings.project.clone();
        // The rename popover edits a buffer; the settings keep it after the toolbar returns.
        let edited_project = RefCell::new(self.settings.project.clone());
        let project_changed = Cell::new(false);
        let open_settings = Cell::new(false);
        let open_appearance = Cell::new(false);
        // The caption is one fact, the profile (docs/suite/Recorder-E3.md, section 5).
        let caption = self.settings.active_profile.clone();
        let chords = Chords::of(&self.hotkeys);
        let project_folder = project_folder(&self.settings);
        // The pre-flight button is enabled while idle and not already running; the sync test
        // also needs the camera file on, exactly like the buttons in the parked column.
        let idle = state.phase == Phase::Idle && state.preflight.is_none();
        let sync_ready = idle && state.sync_test.is_none() && self.webcam.settings(&self.profile_store).is_some_and(|settings| settings.iso);
        let keep_files = self.preflight_section.keep_files();
        let replay_on = state.replay_on;
        let mode = self.modes.mode;
        // In Review the menu's Add mark goes to the selected take, which needs its playback.
        let can_mark = mode != Mode::Review || self.review_playback.is_some();
        let review_mark = Cell::new(false);
        let mode_labels = Mode::ALL.map(Mode::label);
        let selected = Cell::new(Mode::ALL.iter().position(|candidate| *candidate == mode).unwrap_or(0));

        ui_kit::main_toolbar(
            ui,
            |ui| {
                ui_kit::brand::mark(ui, crate::BRAND, ui_kit::brand::TITLE_BAR_MARK);
                let file = ui_kit::Menu::new("File", |ui| {
                    if menu_item(ui, "Open project folder", "").clicked() {
                        open_folder(&project_folder);
                        ui.close();
                    }
                    ui.separator();
                    if menu_item(ui, "Settings", "Ctrl+,").clicked() {
                        open_settings.set(true);
                        ui.close();
                    }
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
                    ui.add_enabled_ui(can_mark, |ui| {
                        if menu_item(ui, "Add mark", &chords.mark).clicked() {
                            if mode == Mode::Review {
                                review_mark.set(true);
                            } else {
                                send(&bus, CommandKind::Mark(MarkKind::Silent));
                            }
                            ui.close();
                        }
                    });
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
                    if let Some(command) = replay_ui::menu_items(ui, replay_on, &chords) {
                        send(&bus, command);
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
                    ui.separator();
                    if menu_item(ui, "Appearance", "").clicked() {
                        open_appearance.set(true);
                        ui.close();
                    }
                });
                // Help gets its items with the Help phase; the menu itself is here already.
                let help = ui_kit::Menu::new("Help", |_ui| {});
                ui_kit::menu_bar(ui, &mut [file, capture, view, help]);
            },
            |ui| {
                let rename = ui.interact(ui.max_rect(), egui::Id::new("project-rename"), egui::Sense::click());
                ui_kit::title_block(ui, &ui_kit::Title::Document { name: &project, unsaved: false, caption: Some(&caption) });
                egui::Popup::menu(&rename).width(320.0).show(|ui| {
                    ui.set_min_width(280.0);
                    ui.label("Project");
                    let edit = ui.add(egui::TextEdit::singleline(&mut *edited_project.borrow_mut()).desired_width(f32::INFINITY));
                    if edit.changed() {
                        project_changed.set(true);
                    }
                    // Enter confirms the rename and closes the popover.
                    if edit.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        ui.close();
                    }
                });
            },
            |ui| {
                ui.add_enabled_ui(!running, |ui| {
                    let mut chosen = selected.get();
                    ui_kit::segmented(ui, "mode-switch", &mut chosen, &mode_labels);
                    selected.set(chosen);
                });
            },
        );

        if review_mark.get() {
            self.add_mark_at_playhead(ui.ctx());
        }

        // The rename popover wrote the project like the old Project field did.
        if project_changed.get() {
            self.settings.project = edited_project.into_inner();
            self.push_settings_if_changed();
            self.save_settings();
        }
        if open_settings.get() {
            self.settings_dialog.open = true;
        }
        if open_appearance.get() {
            self.settings_dialog.open = true;
            self.settings_dialog.section = Section::Appearance;
        }

        // The switch wrote its selection into the cell; report a change through `mode_picked`.
        let picked = Mode::ALL.get(selected.get()).copied().unwrap_or(mode);
        if picked != mode {
            mode_picked.set(Some(picked));
        }
    }

    /// The Setup layout (spec "Window layout" and "Setup mode"): the Sources panel on the
    /// left, the Preview in the center and the tool panels column on the right, with the
    /// 8px window padding and gaps.
    fn setup_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        egui::CentralPanel::default().frame(window_frame(ctx)).show(ui, |ui| {
            let [left, center, right] = three_columns(ui.max_rect(), self.settings.panel_widths.get(Mode::Setup));
            let active = state.phase.has_take();
            column(ui, left, |ui| self.sources_panel(ui, state));
            column(ui, right, |ui| self.settings_panel(ui, ctx, state));
            stage_column(ui, center, |ui| self.preview_panel(ui, state, active));
            self.settings_dirty |= resize_dividers(ui, [left, center, right], self.settings.panel_widths.entry(Mode::Setup));
        });
        // The idle meters of app tracks move without any other news from the controller.
        if !state.idle_apps.is_empty() {
            ctx.request_repaint_after(METER_REFRESH);
        }
    }

    /// The left Sources panel (spec "Setup mode"): the video group with the display and camera
    /// rows, the audio tracks, and the pre-flight card pinned to the panel's bottom.
    fn sources_panel(&mut self, ui: &mut egui::Ui, state: &ControllerState) {
        panel_header(ui, "Sources");
        ui.add_space(space::SCALE_8);
        let panel = ui.max_rect();
        let top = ui.cursor().top();
        // The card is pinned to the bottom; the space above it is laid out with the card height
        // measured on the previous frame, so the first frame reserves MIN_CARD_HEIGHT.
        let card_height = self.sources_card_height.max(MIN_CARD_HEIGHT);
        let content_rect = egui::Rect::from_min_max(
            egui::Pos2::new(panel.left(), top),
            egui::Pos2::new(panel.right(), (panel.bottom() - card_height - space::SCALE_8).max(top)),
        );
        let card_rect = egui::Rect::from_min_max(egui::Pos2::new(panel.left(), (panel.bottom() - card_height).max(top)), panel.right_bottom());
        let mut content = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        content.set_clip_rect(content_rect);
        self.sources_content(&mut content, state);
        let mut card = ui.new_child(egui::UiBuilder::new().max_rect(card_rect));
        card.set_clip_rect(card_rect);
        self.preflight_section.sources_card(&mut card, state, &self.controller.sender());
        self.sources_card_height = card.min_rect().height();
    }

    /// The Sources panel's video and audio groups (spec "Setup mode").
    fn sources_content(&mut self, ui: &mut egui::Ui, state: &ControllerState) {
        group_caption(ui, "Video");
        let fps = self.profile_store.get(&self.settings.active_profile).map(|profile| profile.fps);
        display_source_row(ui, self.source_size, fps);
        self.camera_source_row(ui, Lock { recording: state.phase.has_take(), replay_on: state.replay_on });
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Audio");
        let live = if state.phase.has_take() { Some(&self.live_tracks) } else { None };
        tracks_ui::sources_rows(ui, &self.settings.audio, &self.app_sources, live);
    }

    /// The camera row of the video group: the camera's name, its format and a status dot;
    /// clicking it opens the Camera tool's Camera and Format pickers as a popover.
    fn camera_source_row(&mut self, ui: &mut egui::Ui, lock: Lock) {
        let (name, values, ok) = match self.webcam.settings(&self.profile_store) {
            Some(settings) => {
                let present = self.webcam.cameras().iter().any(|camera| camera.id == settings.camera_id);
                let name = if settings.camera_name.is_empty() { "No camera".to_owned() } else { settings.camera_name.clone() };
                // The least needed value is last, so it is the one a narrow panel leaves out.
                let values = vec![
                    data_text::size_text(settings.format.width, settings.format.height),
                    format!("{} fps", webcam_ui::fps_text(f64::from(settings.format.fps))),
                    settings.format.subtype.clone(),
                ];
                (name, values, present)
            }
            None => ("Camera off".to_owned(), Vec::new(), false),
        };
        let response = source_row(ui, ui_kit::icons::regular::CAMERA, &name, &values, ok);
        egui::Popup::menu(&response).width(320.0).show(|ui| {
            lock.scope(ui, Control::Profile, true, |ui| {
                tools_ui::camera_row(ui, &mut self.webcam, &mut self.profile_store);
                tools_ui::format_row(ui, &mut self.webcam, &mut self.profile_store);
            });
        });
    }

    /// The right Setup panel (spec "Setup mode"): the tool strip over the open tool's panel in a
    /// vertical ScrollArea (the ui-kit overlay scrollbar comes from ui_kit::apply's global scroll style).
    fn settings_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        let active = state.phase.has_take();
        let lock = Lock { recording: active, replay_on: state.replay_on };
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
        egui::ScrollArea::vertical().id_salt("setup-tool").auto_shrink(false).show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            match self.tool {
                Tool::Camera => {
                    tools_ui::camera_panel(
                        ui,
                        &mut self.webcam,
                        &mut self.profile_store,
                        lock.locks(Control::Profile),
                        state,
                        &self.controller.sender(),
                        &mut self.key_ui,
                    );
                }
                Tool::Key => {
                    let picked = webcam_ui::camera_picture(ui, &mut self.webcam, self.key_ui.eyedropper);
                    key_ui::key_panel(ui, &self.webcam, &mut self.profile_store, state, &mut self.key_ui, picked.as_ref());
                }
                Tool::Layout => layout_ui::show(ui, &mut self.profile_store, &self.settings.active_profile, lock),
                Tool::Output => {
                    let mut outcome = ProfileOutcome::default();
                    self.profile_section.output_panel(ui, &mut self.profile_store, &mut self.settings, self.source_size, lock, &mut outcome);
                    self.on_profile_outcome(&outcome);
                    ui.add_space(space::SCALE_8);
                    profile_ui::composite_group(ui, &mut self.profile_store, &self.settings.active_profile, lock);
                    ui.add_space(space::SCALE_8);
                    webcam_ui::iso_group(ui, &self.webcam, &mut self.profile_store, lock, &state.active_profile);
                }
                Tool::Audio => {
                    let live_tracks = if active { Some(&self.live_tracks) } else { None };
                    let changed = tracks_ui::audio_panel(ui, &mut self.settings.audio, &self.devices, &mut self.app_sources, live_tracks, lock);
                    self.push_settings_if_changed();
                    self.on_tracks_changed(ctx, changed);
                    if active {
                        ctx.request_repaint_after(METER_REFRESH);
                    }
                }
            }
        });
        // Every settings change, the open tool among them, saves once the pointer is up and no field has focus.
        let settled = !ctx.input(|input| input.pointer.any_down()) && ctx.memory(|memory| memory.focused().is_none());
        if self.settings_dirty && settled {
            self.save_settings();
        }
        self.profile_store.save_when_due(ctx);
    }

    /// The center Preview stage (spec "Setup mode"): the banner strip, the panel toolbar
    /// (rev 3, 44 tall) and today's preview filling the rest of the panel, over the take
    /// controls in the bottom bar.
    fn preview_panel(&mut self, ui: &mut egui::Ui, state: &ControllerState, active: bool) {
        self.take_stage(ui, state, |app, ui| {
            if app.banner_strip(ui, state) {
                ui.add_space(space::SCALE_8);
            }
            let mut outcome = ProfileOutcome::default();
            app.preview_toolbar(ui, Lock { recording: active, replay_on: state.replay_on }, &mut outcome);
            ui.add_space(space::SCALE_8);
            app.show_preview(ui);
            app.on_profile_outcome(&outcome);
        });
    }

    /// The Setup and Record stage around `add_content`: the bottom bar and the Record button
    /// in its notch (docs/suite/Recorder-E3.md, sections 2 and 3).
    fn take_stage(&mut self, ui: &mut egui::Ui, state: &ControllerState, add_content: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let chords = Chords::of(&self.hotkeys);
        // Only the idle bar shows the next take, so a running take never reads the takes folder.
        let (next_take, next_take_path) =
            if state.phase.has_take() { (None, String::new()) } else { self.profile_section.next_take(&self.profile_store, &self.settings) };
        let bar = bar_ui::TakeBar {
            phase: state.phase,
            fps: state.fps,
            frames: state.stats.as_ref().map(|stats| stats.frames),
            countdown_secs: self.settings.countdown_secs,
            replay_on: state.replay_on,
            chords: &chords,
            next_take: next_take.as_deref(),
            next_take_path: &next_take_path,
        };
        let bus = self.controller.sender();
        bar_ui::take_stage(ui, &bar, &bus, |ui| add_content(self, ui));
    }

    /// The preview panel's toolbar: the Profile caption and picker, a separator, and the
    /// Preview and Matte view chips.
    fn preview_toolbar(&mut self, ui: &mut egui::Ui, lock: Lock, outcome: &mut ProfileOutcome) {
        let key_on = self.webcam.settings(&self.profile_store).is_some_and(|settings| settings.key.enabled);
        let profile = self.profile_store.get(&self.settings.active_profile);
        let clean = profile.is_some_and(|profile| profile.effective_composite_mode() == CompositeMode::Clean);
        let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
        let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(HEADER_PAD);
            ui.add(
                egui::Label::new(egui::RichText::new("Profile").font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY))
                    .selectable(false),
            );
            ui.add_space(space::SCALE_8);
            lock.scope(ui, Control::Profile, true, |ui| {
                self.profile_section.picker(ui, &mut self.profile_store, &mut self.settings, outcome);
            });
            ui.separator();
            ui.add_space(space::SCALE_4);
            ui_kit::chip(ui, &mut self.preview_enabled, "Preview");
            ui.add_space(space::SCALE_4);
            ui.add_enabled_ui(key_on, |ui| {
                ui_kit::chip(ui, &mut self.key_ui.matte, "Matte view");
            });
            if clean {
                ui.add_space(space::SCALE_4);
                // Read-only: the value is made new every frame, so a click changes nothing.
                let mut on = true;
                ui_kit::chip(ui, &mut on, "Clean");
            }
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

    /// The Review layout (spec "Review mode"): the takes list on the left, the
    /// take's playback in the center and the take info with its Actions group on
    /// the right. The scan runs when Review opens, when a take finishes and on
    /// the header's refresh button.
    fn review_panels(&mut self, ui: &mut egui::Ui, state: &ControllerState) {
        if self.review_dirty {
            self.review_dirty = false;
            self.rescan_review();
        }
        egui::CentralPanel::default().frame(window_frame(ui.ctx())).show(ui, |ui| {
            let [left, center, right] = three_columns(ui.max_rect(), self.settings.panel_widths.get(Mode::Review));
            column(ui, left, |ui| match review_ui::takes_panel(ui, &self.review_days, &mut self.review_selected) {
                review_ui::TakesAction::OpenFolder => open_folder(&project_folder(&self.settings)),
                review_ui::TakesAction::Refresh => self.review_dirty = true,
                review_ui::TakesAction::None => {}
            });
            stage_column(ui, center, |ui| {
                let transport = self.review_playback.as_ref().map(review_ui::ReviewTransport::of);
                let (action, _) = review_ui::stage(ui, transport, |ui| {
                    if self.banner_strip(ui, state) {
                        ui.add_space(space::SCALE_8);
                    }
                    review_ui::center(
                        ui,
                        &self.review_days,
                        self.review_selected.as_deref(),
                        self.review_playback.as_mut(),
                        self.review_playback_error.as_deref(),
                        &mut self.review_center,
                    );
                });
                if let Some(action) = action {
                    self.run_review_bar_action(ui.ctx(), action);
                }
            });
            let recording = state.phase != Phase::Idle;
            let playback_info = self.review_playback.as_ref().map(|playback| (playback.position_ms(), playback.is_playing()));
            column(ui, right, |ui| {
                match review_ui::take_panel(ui, &self.review_days, self.review_selected.as_deref(), &mut self.take_panel, recording, playback_info) {
                    review_ui::TakeAction::ShowInFolder => {
                        if let Some(video) = review::find(&self.review_days, self.review_selected.as_deref()).map(|take| take.video.clone()) {
                            platform::show_in_folder(&video);
                        }
                    }
                    review_ui::TakeAction::Rename(new_name) => self.rename_selected_take(new_name),
                    review_ui::TakeAction::Seek(position) => {
                        if let Some(playback) = &mut self.review_playback {
                            playback.seek(position);
                            self.review_center.settle();
                        }
                    }
                    review_ui::TakeAction::None => {}
                }
            });
            self.settings_dirty |= resize_dividers(ui, [left, center, right], self.settings.panel_widths.entry(Mode::Review));
        });
        // The delete confirmation dialog (spec "Review mode"): every file of the
        // take with its size, Cancel and Move to Recycle Bin.
        if self.take_panel.confirm_delete {
            let take = review::find(&self.review_days, self.review_selected.as_deref());
            if review_ui::delete_dialog(ui.ctx(), take, &mut self.take_panel.confirm_delete) {
                self.delete_selected_take();
            }
        }
    }

    /// Carries out a click on the Review bar or its notch on the selected take's playback.
    fn run_review_bar_action(&mut self, ctx: &egui::Context, action: review_ui::BarAction) {
        let take = review::find(&self.review_days, self.review_selected.as_deref());
        let (Some(playback), Some(take)) = (self.review_playback.as_mut(), take) else { return };
        let outcome = review_ui::run_bar_action(action, playback, &take.marks, &mut self.review_center);
        if let review_ui::CenterAction::AddMark(position) = outcome {
            self.add_review_mark(position);
        }
        // The bar was drawn before the click took effect.
        ctx.request_repaint();
    }

    /// Adds a mark to the selected take where its playback is; without a playback it does nothing.
    fn add_mark_at_playhead(&mut self, ctx: &egui::Context) {
        let Some(position) = self.review_playback.as_ref().map(Playback::position_ms) else { return };
        self.add_review_mark(position);
        // The marks were drawn before this one was added.
        ctx.request_repaint();
    }

    /// The mark hotkey in Review. It is a global hotkey, so Windows keeps the key from the
    /// window and sends it to the controller, which ignores a mark while no take runs; the
    /// window sees the press among the controller's recent commands. It acts only while the
    /// window has the focus, so a press meant for another program never changes a take, and
    /// with no dialog over Review.
    fn review_mark_hotkey(&mut self, ctx: &egui::Context, state: &ControllerState) {
        // Every frame, so a press made in another mode is never applied later.
        let pressed = new_mark_press(&state.recent, &mut self.seen_mark_press);
        let focused = ctx.input(|input| input.viewport().focused) == Some(true);
        let in_review = self.modes.mode == Mode::Review && state.phase == Phase::Idle;
        if pressed && focused && in_review && !self.settings_dialog.open && !self.take_panel.confirm_delete {
            self.add_mark_at_playhead(ctx);
        }
    }

    /// The review playback lifecycle: opened on the selected take's main file
    /// while Review is open and no take runs, closed the moment Review is left
    /// or a take starts, and closed and reopened when another take is selected.
    fn sync_review_playback(&mut self, running: bool) {
        if self.modes.mode != Mode::Review || running {
            if self.review_playback.is_some() || self.review_playback_for.is_some() || self.review_playback_error.is_some() {
                if let Some(mut playback) = self.review_playback.take() {
                    playback.close();
                }
                self.review_playback_for = None;
                self.review_playback_error = None;
            }
            return;
        }
        let wanted = review::find(&self.review_days, self.review_selected.as_deref()).map(|take| take.video.clone());
        if self.review_playback_for == wanted && (self.review_playback.is_some() || self.review_playback_error.is_some()) {
            return;
        }
        // A new take: the previous playback's decoder and audio device are freed first.
        if let Some(mut playback) = self.review_playback.take() {
            playback.close();
        }
        self.review_playback_for = wanted.clone();
        self.review_playback_error = None;
        let Some(path) = wanted else { return };
        match Playback::open(&path) {
            Ok(playback) => self.review_playback = Some(playback),
            Err(err) => {
                log::warn!("the playback of {} could not open: {err}", path.display());
                self.review_playback_error = Some(err.to_string());
            }
        }
    }

    /// Adds a Note mark at `position` to the selected take's sidecar and
    /// rescans, so the list and the scrub bar's dots update at once.
    fn add_review_mark(&mut self, position: u64) {
        let Some(take) = review::find(&self.review_days, self.review_selected.as_deref()) else { return };
        let name = take.name.clone();
        match review::add_mark(take, position) {
            Ok(()) => {
                log::info!("added a note mark at {} to {name}", marks::format_tc(position));
                self.rescan_review();
            }
            Err(reason) => log::warn!("could not add the mark to {name}: {reason}"),
        }
    }

    /// Rescans the project folder for takes and keeps the selected take when
    /// it still exists, falling back to the newest one.
    fn rescan_review(&mut self) {
        self.review_days = review::scan(&project_folder(&self.settings));
        let names: Vec<String> = self.review_days.iter().flat_map(|day| day.takes.iter().map(|take| take.name.clone())).collect();
        if !names.iter().any(|name| Some(name) == self.review_selected.as_ref()) {
            self.review_selected = names.first().cloned();
        }
    }

    /// Renames every file of the selected take after checking the new name;
    /// a refused name keeps the field open with its reason under it.
    fn rename_selected_take(&mut self, new_name: String) {
        let Some(take) = review::find(&self.review_days, self.review_selected.as_deref()) else {
            self.take_panel.close_rename();
            return;
        };
        let old_name = take.name.clone();
        let others: Vec<String> =
            self.review_days.iter().flat_map(|day| day.takes.iter().map(|take| take.name.clone())).filter(|name| name != &old_name).collect();
        let outcome = review::check_take_name(&new_name, &others).and_then(|()| review::rename_take(take, &new_name));
        if let Err(reason) = outcome {
            self.take_panel.rename_error = Some(reason);
            return;
        }
        log::info!("renamed the take {old_name} to {new_name}");
        self.review_selected = Some(new_name);
        self.take_panel.close_rename();
        self.rescan_review();
    }

    /// Moves every file of the selected take to the Recycle Bin and selects
    /// the take that takes its place in the list (the newest when there is
    /// no next one).
    fn delete_selected_take(&mut self) {
        let Some(take) = review::find(&self.review_days, self.review_selected.as_deref()) else {
            return;
        };
        let name = take.name.clone();
        let files: Vec<std::path::PathBuf> = take.files.iter().map(|file| file.path.clone()).collect();
        let index = self
            .review_days
            .iter()
            .flat_map(|day| day.takes.iter().map(|take| take.name.clone()))
            .position(|name| Some(name) == self.review_selected)
            .unwrap_or(0);
        if let Err(err) = trash::delete_all(&files) {
            log::warn!("could not move {name} to the Recycle Bin: {err}");
            return;
        }
        log::info!("moved the take {name} to the Recycle Bin");
        self.review_dirty = true;
        self.rescan_review();
        let names: Vec<String> = self.review_days.iter().flat_map(|day| day.takes.iter().map(|take| take.name.clone())).collect();
        self.review_selected = names.get(index).or_else(|| names.first()).cloned();
    }

    /// The Review keys (spec "Review mode"): Space toggles play, Left and
    /// Right step 5 s, Home and End jump to the ends and [ and ] jump to the
    /// previous and next mark. They act only while Review is open, no dialog
    /// is over it and no widget (a text field above all) holds the focus.
    fn review_keys(&mut self, ctx: &egui::Context) {
        if self.modes.mode != Mode::Review || self.settings_dialog.open || self.take_panel.confirm_delete {
            return;
        }
        if ctx.memory(|memory| memory.focused().is_some()) {
            return;
        }
        let pressed = |key: egui::Key| ctx.input(|input| input.key_pressed(key));
        let Some(playback) = &mut self.review_playback else { return };
        let position = playback.position_ms();
        let duration = playback.duration_ms();
        let positions: Vec<u64> =
            review::find(&self.review_days, self.review_selected.as_deref()).map_or_else(Vec::new, |take| take.marks.iter().map(|mark| mark.t_ms).collect());
        let mut acted = false;
        if pressed(egui::Key::Space) {
            playback.toggle();
            acted = true;
        }
        if pressed(egui::Key::ArrowLeft) {
            playback.seek(position.saturating_sub(5_000));
            acted = true;
        }
        if pressed(egui::Key::ArrowRight) {
            playback.seek(position + 5_000);
            acted = true;
        }
        if pressed(egui::Key::Home) {
            playback.seek(0);
            acted = true;
        }
        if pressed(egui::Key::End) {
            playback.seek(duration);
            acted = true;
        }
        if pressed(egui::Key::OpenBracket)
            && let Some(t_ms) = review::previous_mark(&positions, position)
        {
            playback.seek(t_ms);
            acted = true;
        }
        if pressed(egui::Key::CloseBracket)
            && let Some(t_ms) = review::next_mark(&positions, position)
        {
            playback.seek(t_ms);
            acted = true;
        }
        if acted {
            self.review_center.settle();
        }
    }

    /// The Record layout (spec "Modes" and "Record mode"): the recording preview in a center
    /// that fills the window and the Live panel on the right; there is no left column.
    fn record_panels(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        egui::CentralPanel::default().frame(window_frame(ctx)).show(ui, |ui| {
            let [center, right] = two_columns(ui.max_rect(), self.settings.panel_widths.get(Mode::Record));
            column(ui, right, |ui| self.live_panel(ui, ctx, state));
            stage_column(ui, center, |ui| self.record_center(ui, state));
            self.settings_dirty |= resize_right_divider(ui, [center, right], self.settings.panel_widths.entry(Mode::Record));
        });
    }

    /// The Live panel (spec "Record mode"): read-only, so only the meters may
    /// ask for a repaint, while a take runs or an app track is watched.
    fn live_panel(&self, ui: &mut egui::Ui, ctx: &egui::Context, state: &ControllerState) {
        let live = if state.phase.has_take() { Some(&self.live_tracks) } else { None };
        record_ui::live_panel(ui, state, &self.settings.audio, &self.app_sources, live);
        if state.phase.has_take() || !state.idle_apps.is_empty() {
            ctx.request_repaint_after(METER_REFRESH);
        }
    }

    /// The Record mode's center stage: the banner strip over the recording preview, over
    /// the take controls in the bottom bar.
    fn record_center(&mut self, ui: &mut egui::Ui, state: &ControllerState) {
        self.take_stage(ui, state, |app, ui| {
            if app.banner_strip(ui, state) {
                ui.add_space(space::SCALE_8);
            }
            let preview = app.preview.as_ref();
            app.preview_view.show_recording(ui, preview, app.preview_enabled, state.phase == Phase::Paused);
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

/// The tray icon state that matches a controller phase and the replay buffer.
fn tray_state(phase: Phase, replay_on: bool) -> TrayState {
    match phase {
        Phase::Idle | Phase::Countdown(_) if replay_on => TrayState::Replay,
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

/// Whether `recent` holds a plain mark command (the mark hotkey's kind) pressed after `seen`,
/// which moves on to the newest one.
fn new_mark_press(recent: &VecDeque<Command>, seen: &mut i64) -> bool {
    let newest = recent.iter().filter(|command| command.kind == CommandKind::Mark(MarkKind::Silent)).map(|command| command.press_qpc).max();
    match newest {
        Some(press) if press > *seen => {
            *seen = press;
            true
        }
        _ => false,
    }
}

/// The hotkey chords the Capture menu and the bar's tooltips show as their shortcuts; empty
/// when a chord is not registered.
#[derive(Default)]
pub(crate) struct Chords {
    /// The start or stop recording chord.
    pub(crate) start_stop: String,
    /// The pause or resume chord.
    pub(crate) pause_resume: String,
    /// The add mark chord.
    pub(crate) mark: String,
    /// The add mark with label chord.
    pub(crate) mark_label: String,
    /// The mute mic chord.
    pub(crate) mute_mic: String,
    /// The replay buffer on or off chord.
    pub(crate) replay_toggle: String,
    /// The save replay chord.
    pub(crate) replay_save: String,
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
            replay_toggle: chord("replay_toggle"),
            replay_save: chord("replay_save"),
        }
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

/// The height the Sources panel reserves for the pre-flight card before it has been measured.
const MIN_CARD_HEIGHT: f32 = 150.0;

/// Height of a source row in the Sources panel.
const SOURCE_ROW_HEIGHT: f32 = 36.0;

/// Diameter of a source row's status dot.
const DOT_DIAMETER: f32 = 8.0;

/// The display row of the Sources panel's video group: the primary monitor, its size with the
/// frame rate it is captured at, and a status dot. There is no display picker today, so the row
/// is info only (spec "Mapping").
fn display_source_row(ui: &mut egui::Ui, size: (u32, u32), fps: Option<u32>) {
    let mut values = vec![data_text::size_text(size.0, size.1)];
    if let Some(fps) = fps.filter(|fps| *fps > 0) {
        values.push(format!("{fps} fps"));
    }
    source_row(ui, ui_kit::icons::regular::MONITOR, "Primary monitor", &values, true);
}

/// One source row of the video group: the icon, the name over its values and a status
/// dot. Returns the row's response so the camera row can open its picker popover on click.
fn source_row(ui: &mut egui::Ui, icon: &'static str, name: &str, values: &[String], ok: bool) -> egui::Response {
    let row = egui::vec2(ui.available_width(), SOURCE_ROW_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(icon).font(ui_kit::fonts::icon_regular(typography::BODY_SIZE)).color(color::TEXT_SECONDARY)).selectable(false),
        );
        ui.add_space(space::SCALE_4);
        let text_width = (ui.available_width() - DOT_DIAMETER - ui.spacing().item_spacing.x).max(0.0);
        ui.vertical(|ui| {
            ui.set_max_width(text_width);
            ui.add(
                egui::Label::new(egui::RichText::new(name).font(ui_kit::fonts::medium(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                    .selectable(false)
                    .truncate(),
            );
            if !values.is_empty() {
                source_values(ui, values, text_width);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            status_dot(ui, ok);
        });
    });
    ui.interact(inner.response.rect, ui.id().with(("source-row", name)), egui::Sense::click())
}

/// A source row's values as separate items in Martian Mono, `width` wide at most. A value that
/// does not fit whole is left out, with every value after it.
fn source_values(ui: &mut egui::Ui, values: &[String], width: f32) {
    let font = ui_kit::fonts::mono(typography::VALUE_SIZE);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SCALE_8;
        let mut used = 0.0;
        for value in values {
            let job = ui_kit::fonts::tracked(value, font.clone(), color::TEXT_TERTIARY, ui_kit::fonts::MONO_LETTER_SPACING_EM);
            let galley = ui.painter().layout_job(job);
            used += galley.size().x;
            if used > width {
                break;
            }
            used += space::SCALE_8;
            ui.add(egui::Label::new(galley).selectable(false));
        }
    });
}

/// The small status dot at a source row's right edge.
fn status_dot(ui: &mut egui::Ui, ok: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(DOT_DIAMETER, DOT_DIAMETER), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), DOT_DIAMETER / 2.0, if ok { color::SEM_METER_OK } else { color::SEM_METER_WARN });
}

/// The small caption naming a Sources group ("Video", "Audio").
fn group_caption(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false));
}

/// One row of the banner strip: the kind decides the dot color (spec "Mapping").
enum BannerRow {
    /// A warning with a `sem.meter.warn` dot.
    Warn(String),
    /// An error with a `sem.record` dot.
    Error(String),
}

/// One banner row: a colored dot, the message, and the optional Details and Dismiss buttons
/// on the right, reading left to right. Returns the row, Details and Dismiss responses.
fn banner_row(
    ui: &mut egui::Ui,
    dot: egui::Color32,
    message: &str,
    details_label: Option<&str>,
    dismiss: bool,
) -> (egui::Response, Option<egui::Response>, Option<egui::Response>) {
    let mut details = None;
    let mut dismiss_response = None;
    let row = ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(DOT_DIAMETER, DOT_DIAMETER), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), DOT_DIAMETER / 2.0, dot);
        ui.add(
            egui::Label::new(egui::RichText::new(message).font(ui_kit::fonts::medium(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                .selectable(false)
                .truncate(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if dismiss {
                dismiss_response = Some(ui.small_button("Dismiss"));
            }
            if let Some(label) = details_label {
                details = Some(ui.small_button(label));
            }
        });
    });
    (row.response, details, dismiss_response)
}

/// The window-body frame all three modes share: the ground fill with the 8px window gap on
/// every side (spec "Window layout"). The fill is the theme's tinted ground, the color the
/// stage cuts its notch in.
fn window_frame(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new().fill(ground(ctx)).inner_margin(egui::Margin::same(space::WINDOW_GAP as i8))
}

/// The window ground, tinted toward the accent.
fn ground(ctx: &egui::Context) -> egui::Color32 {
    ui_kit::Theme::get(ctx).neutrals.bg_ground
}

/// Where the mode's stage, and so its notch button, is centered in a `window` that wide.
pub(crate) fn stage_center_x(window: egui::Rect, mode: Mode, widths: PanelWidths) -> f32 {
    let body = window.shrink(space::WINDOW_GAP);
    let center = match mode {
        Mode::Record => two_columns(body, widths)[0],
        Mode::Setup | Mode::Review => three_columns(body, widths)[1],
    };
    center.center().x
}

/// The three columns inside the window body's content rect: the Sources panel on the left,
/// the settings panel on the right and what is left between them as the center, separated
/// by the window gap. The widths come from the mode's saved [`PanelWidths`], clamped into
/// the spec ranges, and when the window cannot keep the center's [`CENTER_MIN_W`] the side
/// columns give way first (spec "Panels").
pub(crate) fn three_columns(content: egui::Rect, widths: PanelWidths) -> [egui::Rect; 3] {
    let gap = space::WINDOW_GAP;
    let (left_w, right_w) = fitted_widths(widths.left, widths.right, content.width(), gap);
    let left = egui::Rect::from_min_max(content.left_top(), egui::Pos2::new(content.left() + left_w, content.bottom()));
    let right = egui::Rect::from_min_max(egui::Pos2::new((content.right() - right_w).max(left.right() + gap), content.top()), content.right_bottom());
    // On a window too narrow for both side panels the center collapses to zero width instead of inverting.
    let center_min_x = left.right() + gap;
    let center_max_x = (right.left() - gap).max(center_min_x);
    let center = egui::Rect::from_min_max(egui::Pos2::new(center_min_x, content.top()), egui::Pos2::new(center_max_x, content.bottom()));
    [left, center, right]
}

/// The Record columns inside the window body's content rect: the recording preview filling
/// everything left of the Live panel, separated by the window gap; there is no left column
/// in Record mode (spec "Modes" and "Record mode"). The right width comes from the mode's
/// saved [`PanelWidths`], clamped into its range and giving way to the center's
/// [`CENTER_MIN_W`] on a narrow window (spec "Panels").
pub(crate) fn two_columns(content: egui::Rect, widths: PanelWidths) -> [egui::Rect; 2] {
    let gap = space::WINDOW_GAP;
    let right_w = fitted_right(widths.right, content.width(), gap);
    let right = egui::Rect::from_min_max(egui::Pos2::new((content.right() - right_w).max(content.left()), content.top()), content.right_bottom());
    let center = egui::Rect::from_min_max(content.left_top(), egui::Pos2::new((right.left() - gap).max(content.left()), content.bottom()));
    [center, right]
}

/// The left and right column widths clamped into their spec ranges and, when the window is
/// too narrow to keep the center's [`CENTER_MIN_W`], shrunk toward their minimums in
/// proportion to how much room each has to give (spec "Panels": the side columns give way
/// first). A window narrower than the minimums leaves the center below [`CENTER_MIN_W`].
fn fitted_widths(left: f32, right: f32, content_width: f32, gap: f32) -> (f32, f32) {
    let left = left.clamp(config::LEFT_PANEL_MIN, config::LEFT_PANEL_MAX);
    let right = right.clamp(config::RIGHT_PANEL_MIN, config::RIGHT_PANEL_MAX);
    let overflow = left + right + 2.0 * gap + CENTER_MIN_W - content_width;
    if overflow <= 0.0 {
        return (left, right);
    }
    let left_room = left - config::LEFT_PANEL_MIN;
    let right_room = right - config::RIGHT_PANEL_MIN;
    let room = left_room + right_room;
    if room <= 0.0 {
        return (left, right);
    }
    let give = overflow.min(room);
    (left - give * left_room / room, right - give * right_room / room)
}

/// The right column width clamped into its spec range and, when the window is too narrow to
/// keep the center's [`CENTER_MIN_W`], shrunk toward its minimum (spec "Panels"); used by
/// Record mode, which has no left column.
fn fitted_right(right: f32, content_width: f32, gap: f32) -> f32 {
    let right = right.clamp(config::RIGHT_PANEL_MIN, config::RIGHT_PANEL_MAX);
    let overflow = right + gap + CENTER_MIN_W - content_width;
    if overflow <= 0.0 {
        return right;
    }
    let room = right - config::RIGHT_PANEL_MIN;
    if room <= 0.0 {
        return right;
    }
    right - overflow.min(room)
}

/// The window gap between two columns, as the divider's drag rect.
fn gap_between(a: &egui::Rect, b: &egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(egui::Pos2::new(a.right(), a.top()), egui::Pos2::new(b.left(), a.bottom()))
}

/// One column of the window body: the [`ui_kit::side_panel`] drawn exactly in `rect` and clipped to
/// it, so content wider than the column is clipped and never widens the column (spec "Panels").
fn column<R>(parent: &mut egui::Ui, rect: egui::Rect, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let mut ui = parent.new_child(egui::UiBuilder::new().max_rect(rect));
    ui.set_clip_rect(rect);
    ui_kit::side_panel(&mut ui, |ui| {
        ui.set_min_size(ui.available_size());
        add_contents(ui)
    })
    .inner
}

/// The center column, which a stage panel fills.
fn stage_column<R>(parent: &mut egui::Ui, rect: egui::Rect, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let mut ui = parent.new_child(egui::UiBuilder::new().max_rect(rect));
    ui.set_clip_rect(rect);
    add_contents(&mut ui)
}

/// Which column one divider resizes: the side fixes the drag direction, the width range and
/// the default a double-click restores (spec "Panels").
#[derive(Clone, Copy, PartialEq, Eq)]
enum DividerSide {
    /// The gap between the left column and the center: dragging right widens the left column.
    Left,
    /// The gap between the center and the right column: dragging left widens the right column.
    Right,
}

impl DividerSide {
    /// The width change per pointer pixel of drag, so the divider follows the pointer.
    fn sign(self) -> f32 {
        match self {
            Self::Left => 1.0,
            Self::Right => -1.0,
        }
    }

    /// The width clamped into the side's range.
    fn clamp(self, width: f32) -> f32 {
        match self {
            Self::Left => width.clamp(config::LEFT_PANEL_MIN, config::LEFT_PANEL_MAX),
            Self::Right => width.clamp(config::RIGHT_PANEL_MIN, config::RIGHT_PANEL_MAX),
        }
    }

    /// The width a double-click restores.
    fn default_width(self) -> f32 {
        match self {
            Self::Left => config::LEFT_PANEL_DEFAULT,
            Self::Right => config::RIGHT_PANEL_DEFAULT,
        }
    }
}

/// Handles the two column dividers of a three-column mode: a drag, the arrow keys or a
/// double-click writes the new width into the mode's saved `widths` (spec "Panels"). A
/// divider stops where the stage would get narrower than [`CENTER_MIN_W`]. A change saves
/// both widths as they are on screen, so a width the window had shrunk does not take the
/// room back from the column being dragged. Returns whether the widths changed.
fn resize_dividers(ui: &mut egui::Ui, columns: [egui::Rect; 3], widths: &mut PanelWidths) -> bool {
    let [left, center, right] = columns;
    // What the two side columns can share while the stage keeps its minimum.
    let room = right.right() - left.left() - 2.0 * space::WINDOW_GAP - CENTER_MIN_W;
    let mut changed = false;
    if let Some(width) = column_divider(ui, gap_between(&left, &center), "left", left.width(), DividerSide::Left, room - right.width()) {
        *widths = PanelWidths { left: width, right: right.width() };
        changed = true;
    }
    if let Some(width) = column_divider(ui, gap_between(&center, &right), "right", right.width(), DividerSide::Right, room - left.width()) {
        *widths = PanelWidths { left: left.width(), right: width };
        changed = true;
    }
    changed
}

/// Handles the Record mode's single divider between the center and the Live panel, like
/// [`resize_dividers`]; the unused left width is left as it is.
fn resize_right_divider(ui: &mut egui::Ui, columns: [egui::Rect; 2], widths: &mut PanelWidths) -> bool {
    let [center, right] = columns;
    let room = right.right() - center.left() - space::WINDOW_GAP - CENTER_MIN_W;
    let Some(width) = column_divider(ui, gap_between(&center, &right), "right", right.width(), DividerSide::Right, room) else {
        return false;
    };
    widths.right = width;
    true
}

/// One column divider in the window gap beside a column (spec "Panels"): dragging it resizes
/// the column, the arrow keys move 16 px when it is focused, and double-clicking restores
/// the default width. The cursor becomes the resize cursor on hover, and a thin accent line
/// shows on hover and while dragging. `width` is the column's width on screen and `max` the
/// widest the stage leaves room for. Returns the column's new width when this frame changed it.
fn column_divider(ui: &mut egui::Ui, rect: egui::Rect, id_salt: &'static str, width: f32, side: DividerSide, max: f32) -> Option<f32> {
    // The side's own minimum wins over `max` in a window too narrow for both.
    let limit = |wanted: f32| side.clamp(side.clamp(wanted).min(max));
    let response = ui.interact(rect, egui::Id::new("column_divider").with(id_salt), egui::Sense::click_and_drag());
    if response.clicked() {
        response.request_focus();
    }
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        let theme = ui_kit::Theme::get(ui.ctx());
        ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(DIVIDER_LINE_W, theme.accent.accent));
    }
    if response.double_clicked() {
        return Some(limit(side.default_width()));
    }
    let mut moved = 0.0;
    if response.dragged() {
        moved += side.sign() * response.drag_delta().x;
    }
    if response.has_focus() {
        let step = match ui.input(|input| (input.key_pressed(egui::Key::ArrowLeft), input.key_pressed(egui::Key::ArrowRight))) {
            (true, _) => -1.0,
            (_, true) => 1.0,
            (false, false) => 0.0,
        };
        moved += side.sign() * step * DIVIDER_KEY_STEP;
    }
    if moved == 0.0 {
        return None;
    }
    let new_width = limit(width + moved);
    (new_width != width).then_some(new_width)
}

/// One menu item: the label in the body font, then the hotkey in the mono value style
/// (spec "Menus": the items show the existing hotkeys as their shortcuts).
fn menu_item(ui: &mut egui::Ui, label: &str, chord: &str) -> egui::Response {
    ui.add(menu_button(label, chord))
}

/// [`menu_item`] that draws faded and reports no clicks unless `enabled`.
pub(crate) fn menu_item_enabled(ui: &mut egui::Ui, enabled: bool, label: &str, chord: &str) -> egui::Response {
    ui.add_enabled(enabled, menu_button(label, chord))
}

/// A menu item's button: the label, then its chord if it has one.
pub(crate) fn menu_button(label: &str, chord: &str) -> egui::Button<'static> {
    let mut job = egui::text::LayoutJob::default();
    job.append(label, 0.0, egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY));
    if !chord.is_empty() {
        job.append(chord, space::SCALE_12, egui::TextFormat::simple(ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY));
    }
    egui::Button::new(job)
}

/// Whether Ctrl+, must open the Settings dialog right now: the chord is pressed this frame,
/// the dialog is closed and no widget (a text field above all) holds the keyboard focus.
fn settings_key_pressed(ctx: &egui::Context, dialog_open: bool) -> bool {
    !dialog_open && ctx.memory(|memory| memory.focused().is_none()) && ctx.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Comma))
}

/// Sends one command to the controller bus with the current QPC time, logging when the bus has ended.
pub(crate) fn send(bus: &BusSender, kind: CommandKind) {
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
        // Before the mode rules, so the mode is still the one the press was made in.
        self.review_mark_hotkey(&ctx, &state);
        // The mode rules (spec "Modes"): a take starting or ending moves the mode; ending
        // also queues the "takeNN saved" toast.
        self.sync_mode(&state);

        // The status bar (docs/suite/Recorder-E3.md, section 4): state, disk free, camera note, API clients.
        // The notch button reaches into the status bar, so its text stays clear of it.
        let notch_x = stage_center_x(ui.max_rect(), self.modes.mode, self.settings.panel_widths.get(self.modes.mode));
        egui::Panel::bottom("status-bar").frame(egui::Frame::new().fill(ground(&ctx))).show(ui, |ui| {
            let clients = self.api.as_ref().map(|server| server.client_count());
            let blocked = state.phase == Phase::Idle && self.profile_section.blocked();
            status_ui::show(ui, &state, clients, blocked, crate::engine::cam::info_line().as_deref(), notch_x);
        });

        // The 44px title bar (spec "Title bar", rev 3); it reports a picked mode through the cell.
        let running = state.phase != Phase::Idle;
        let mode_picked = Cell::new(None::<Mode>);
        egui::Panel::top("title-bar").frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| self.title_bar(ui, running, &state, &mode_picked));
        if let Some(to) = mode_picked.get() {
            self.set_mode(running, to);
        }
        self.handle_mode_keys(&ctx, running);
        // The review playback (spec "Review mode"): opened for the selected take
        // while Review is open and idle, closed the moment Review is left or a
        // take starts (the decoder and the audio device are freed).
        self.sync_review_playback(running);
        // File > Settings (Ctrl+,) opens the Settings dialog (spec "Menus"): only on the
        // press itself, never while the dialog is open or a text field holds the focus.
        if settings_key_pressed(&ctx, self.settings_dialog.open) {
            self.settings_dialog.open = true;
        }

        match self.modes.mode {
            Mode::Setup => self.setup_panels(ui, &ctx, &state),
            Mode::Record => self.record_panels(ui, &ctx, &state),
            Mode::Review => self.review_panels(ui, &state),
        }
        // The Review keys (spec "Review mode"): play, seek steps and mark jumps.
        self.review_keys(&ctx);

        // The toasts (spec "Mode rules"): one glass capsule at a time; a saved
        // take's toast carries a Review action that opens Review on that take.
        self.show_notices(&ctx, running);

        // The Settings dialog (spec "Settings dialog"): File > Settings (Ctrl+,) and
        // View > Appearance open it, Esc closes it.
        let next_take = if self.settings_dialog.open {
            let (_label, path) = self.profile_section.next_take(&self.profile_store, &self.settings);
            NextTake { path }
        } else {
            NextTake::default()
        };
        let outcome = settings_ui::show(
            &ctx,
            &mut self.settings_dialog,
            &mut self.settings,
            &next_take,
            &self.hotkeys,
            &state.recent,
            ApiUi { server: self.api.as_ref(), token: self.api_token.as_deref(), show_token: &mut self.api_show_token, port_edit: &mut self.api_port_edit },
        );
        if outcome.settings_changed {
            self.push_settings_if_changed();
            self.save_settings();
        }
        if outcome.hide_from_capture_changed {
            self.apply_hide_from_capture();
        }
        if outcome.regenerate_token {
            self.api_regenerate = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;

    /// The app saves its accent in its own folder and resets to its own default.
    #[test]
    fn the_app_has_its_own_appearance_identity() {
        assert_eq!(APPEARANCE.id, "recorder");
        assert_eq!(APPEARANCE.default_preset, ui_kit::Preset::Teal);
    }

    /// The Setup layout, headless at 1300 x 840: the right settings column ends at the window
    /// width minus the window gap (it never grows past the window edge) and starts after the
    /// center column, with the window gap between them.
    #[test]
    fn the_setup_columns_fill_the_window_and_keep_the_right_panel_its_width() {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1300.0, 840.0))), ..Default::default() };
        let ctx = egui::Context::default();
        let columns = std::cell::Cell::new((egui::Rect::NOTHING, egui::Rect::NOTHING, egui::Rect::NOTHING));
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().frame(window_frame(ui.ctx())).show(ui, |ui| {
                let [left, center, right] = three_columns(ui.max_rect(), PanelWidths::default());
                columns.set((left, center, right));
            });
        });
        output.textures_delta.clear();
        let (left, center, right) = columns.get();
        let gap = space::WINDOW_GAP;
        assert_eq!(right.right(), 1300.0 - gap, "the right column ends at the window width minus the window gap");
        assert_eq!(right.left(), center.right() + gap, "the right column starts after the center column");
        assert_eq!(right.width(), config::RIGHT_PANEL_DEFAULT, "the right column keeps the settings panel width");
        assert_eq!(left.width(), config::LEFT_PANEL_DEFAULT, "the left column keeps the Sources panel width");
        assert_eq!(center.left(), left.right() + gap, "the center column starts after the left column");
    }

    /// The saved widths clamp into the spec ranges: below the minimum up to it, above the
    /// maximum down to it (spec "Panels": left 220 to 360, right 300 to 440).
    #[test]
    fn the_saved_widths_clamp_into_the_panel_ranges() {
        let gap = space::WINDOW_GAP;
        let (left, right) = fitted_widths(0.0, 0.0, 4000.0, gap);
        assert_eq!(left, config::LEFT_PANEL_MIN, "a left width under the minimum clamps up to it");
        assert_eq!(right, config::RIGHT_PANEL_MIN, "a right width under the minimum clamps up to it");
        let (left, right) = fitted_widths(1000.0, 1000.0, 4000.0, gap);
        assert_eq!(left, config::LEFT_PANEL_MAX, "a left width over the maximum clamps down to it");
        assert_eq!(right, config::RIGHT_PANEL_MAX, "a right width over the maximum clamps down to it");
    }

    /// On a window too narrow for the wanted widths, the side columns give way first and the
    /// center keeps its 400 px minimum; they shrink in proportion to the room each has left
    /// down to its minimum (spec "Panels").
    #[test]
    fn the_center_keeps_its_minimum_while_the_side_columns_give_way() {
        let gap = space::WINDOW_GAP;
        // 270 + 340 + 2 gaps + 400 = 1026 is 26 more than the 1000 the window offers.
        let (left, right) = fitted_widths(270.0, 340.0, 1000.0, gap);
        let left_room = 270.0 - config::LEFT_PANEL_MIN;
        let right_room = 340.0 - config::RIGHT_PANEL_MIN;
        assert_eq!(left, 270.0 - 26.0 * left_room / (left_room + right_room), "the left column gives way in proportion");
        assert_eq!(right, 340.0 - 26.0 * right_room / (left_room + right_room), "the right column gives way in proportion");
        assert!((left + right + 2.0 * gap + CENTER_MIN_W - 1000.0).abs() < 1e-3, "the center keeps its 400 px minimum");
    }

    /// A window narrower than the minimums cannot keep the center's minimum: the side
    /// columns stay at their minimums and the center goes below 400 px (spec "Panels").
    #[test]
    fn a_window_too_narrow_for_the_minimums_leaves_the_center_below_its_minimum() {
        let gap = space::WINDOW_GAP;
        let (left, right) = fitted_widths(270.0, 340.0, 900.0, gap);
        assert_eq!(left, config::LEFT_PANEL_MIN, "the left column stops at its minimum");
        assert_eq!(right, config::RIGHT_PANEL_MIN, "the right column stops at its minimum");
        assert_eq!(left + right + 2.0 * gap + CENTER_MIN_W, 936.0, "which is more than the 900 the window offers");
        let [left, center, right] = three_columns(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0)), PanelWidths::default());
        assert_eq!(left.width(), config::LEFT_PANEL_MIN);
        assert_eq!(right.width(), config::RIGHT_PANEL_MIN);
        assert_eq!(center.width(), 900.0 - 2.0 * gap - left.width() - right.width(), "the center takes what is left");
    }

    /// Record mode has one divider: the Live panel's width clamps into its range and gives
    /// way to the center's 400 px minimum on a narrow window (spec "Panels").
    #[test]
    fn the_record_columns_keep_the_center_at_its_minimum_on_a_narrow_window() {
        let gap = space::WINDOW_GAP;
        assert_eq!(fitted_right(340.0, 2000.0, gap), config::RIGHT_PANEL_DEFAULT, "a wide window keeps the saved width");
        // 340 + gap + 400 = 748 is 28 more than the 720 the window offers.
        let [center, right] = two_columns(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(720.0, 600.0)), PanelWidths::default());
        assert_eq!(right.width(), 340.0 - 28.0, "the Live panel gives way to the center");
        assert_eq!(center.width(), CENTER_MIN_W, "the center keeps its 400 px minimum");
    }

    /// Ctrl+, opens the Settings dialog only on the press itself, while the dialog is
    /// closed and no text field holds the keyboard focus.
    #[test]
    fn ctrl_comma_opens_the_dialog_only_on_a_press_while_it_is_closed_and_unfocused() {
        let pressed = |dialog_open: bool, focused: bool, ctrl: bool| {
            let ctx = egui::Context::default();
            ui_kit::apply(&ctx);
            let modifiers = if ctrl { egui::Modifiers::CTRL } else { egui::Modifiers::NONE };
            let mut events = Vec::new();
            if ctrl {
                events.push(egui::Event::ModifiersChanged(egui::Modifiers::CTRL));
            }
            events.push(egui::Event::Key { key: egui::Key::Comma, physical_key: None, pressed: true, repeat: false, modifiers });
            let raw = egui::RawInput { events, ..Default::default() };
            let seen = std::cell::Cell::new(false);
            let mut buffer = String::new();
            let ctx_ref = &ctx;
            let mut output = ctx.run_ui(raw, |ui| {
                if focused {
                    egui::TextEdit::singleline(&mut buffer).show(ui).response.request_focus();
                }
                seen.set(settings_key_pressed(ctx_ref, dialog_open));
            });
            output.textures_delta.clear();
            seen.get()
        };
        assert!(pressed(false, false, true), "Ctrl+, opens the dialog while it is closed and nothing is focused");
        assert!(!pressed(true, false, true), "Ctrl+, is ignored while the dialog is already open");
        assert!(!pressed(false, true, true), "Ctrl+, is ignored while a text field has the focus");
        assert!(!pressed(false, false, false), "a bare comma does not open the dialog");
    }

    /// A harness that draws a three-column mode `width` wide with live dividers over the
    /// shared `widths`, and reports the columns of the last frame.
    fn divider_harness(width: f32, widths: Rc<Cell<PanelWidths>>) -> (egui_kittest::Harness<'static>, Rc<Cell<[egui::Rect; 3]>>) {
        let columns = Rc::new(Cell::new([egui::Rect::NOTHING; 3]));
        let drawn = Rc::clone(&columns);
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(width, 600.0)).build_ui(move |ui| {
            // The harness's own panel has a margin; the columns are laid out on the whole window.
            let window = ui.ctx().content_rect();
            let mut body = ui.new_child(egui::UiBuilder::new().max_rect(window.shrink(space::WINDOW_GAP)));
            body.set_clip_rect(window);
            let mut saved = widths.get();
            let laid_out = three_columns(body.max_rect(), saved);
            drawn.set(laid_out);
            if resize_dividers(&mut body, laid_out, &mut saved) {
                widths.set(saved);
            }
        });
        harness.run();
        (harness, columns)
    }

    /// Drags from `from` by `by` points in a few steps and lets go.
    fn drag(harness: &mut egui_kittest::Harness<'_>, from: egui::Pos2, by: f32) {
        harness.hover_at(from);
        harness.run();
        harness.drag_at(from);
        harness.run();
        let steps = 8_u8;
        for step in 1..=steps {
            harness.hover_at(from + egui::vec2(by * f32::from(step) / f32::from(steps), 0.0));
            harness.run();
        }
        harness.drop_at(from + egui::vec2(by, 0.0));
        harness.run();
    }

    /// The middle of the divider between two columns.
    fn divider_at(a: egui::Rect, b: egui::Rect) -> egui::Pos2 {
        gap_between(&a, &b).center()
    }

    /// Dragging a divider resizes its column, and the new width comes back after a save and a load.
    #[test]
    fn a_divider_drag_resizes_the_column_and_the_width_is_remembered() -> Result<(), Box<dyn std::error::Error>> {
        let widths = Rc::new(Cell::new(PanelWidths::default()));
        let (mut harness, columns) = divider_harness(1280.0, Rc::clone(&widths));
        let [left, center, _] = columns.get();
        drag(&mut harness, divider_at(left, center), 50.0);
        let dragged = widths.get();
        assert!(dragged.left > config::LEFT_PANEL_DEFAULT + 30.0, "the drag widened the left column: {dragged:?}");
        assert_eq!(columns.get()[0].width(), dragged.left, "the column is drawn at the saved width");
        assert_eq!(dragged.right, config::RIGHT_PANEL_DEFAULT, "the other column keeps its width");

        let [_, center, right] = columns.get();
        drag(&mut harness, divider_at(center, right), -50.0);
        let both = widths.get();
        assert!(both.right > config::RIGHT_PANEL_DEFAULT + 30.0, "the drag widened the right column: {both:?}");
        assert_eq!(both.left, dragged.left);

        let mut settings = Settings::default();
        *settings.panel_widths.entry(Mode::Setup) = both;
        let loaded: Settings = serde_json::from_str(&serde_json::to_string(&settings)?)?;
        assert_eq!(loaded.panel_widths.get(Mode::Setup), both);
        Ok(())
    }

    /// In the narrowest window, dragging either divider as far as it goes, in either order,
    /// leaves the stage at least 400 wide, and what is saved is what is drawn.
    #[test]
    fn the_stage_keeps_400_in_the_narrowest_window_with_the_dividers_dragged_to_the_maximum() {
        let widest = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MAX };
        for (saved, left_first) in [(PanelWidths::default(), true), (PanelWidths::default(), false), (widest, true)] {
            let widths = Rc::new(Cell::new(saved));
            let (mut harness, columns) = divider_harness(config::MIN_WINDOW_WIDTH, Rc::clone(&widths));
            assert!(columns.get()[1].width() >= CENTER_MIN_W - 0.01, "remembered widths are clamped on the first frame");
            for dragging_left in [left_first, !left_first] {
                let [left, center, right] = columns.get();
                if dragging_left {
                    drag(&mut harness, divider_at(left, center), 600.0);
                } else {
                    drag(&mut harness, divider_at(center, right), -600.0);
                }
                let [left, center, right] = columns.get();
                assert!(center.width() >= CENTER_MIN_W - 0.01, "the stage is {} wide", center.width());
                assert!(left.width() >= config::LEFT_PANEL_MIN && right.width() >= config::RIGHT_PANEL_MIN);
                let now = widths.get();
                if now != saved {
                    assert!((now.left - left.width()).abs() < 0.01 && (now.right - right.width()).abs() < 0.01, "{now:?} is not what is drawn");
                }
            }
        }
    }

    /// Record mode's stage keeps 400 in the narrowest window with the widest saved panel.
    #[test]
    fn the_record_stage_keeps_400_in_the_narrowest_window() {
        let widths = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MAX };
        let body = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(config::MIN_WINDOW_WIDTH, 600.0)).shrink(space::WINDOW_GAP);
        assert!(two_columns(body, widths)[0].width() >= CENTER_MIN_W);
    }

    /// Each mark press counts once; other commands do not count.
    #[test]
    fn the_tray_shows_replay_only_while_nothing_records() {
        for phase in [Phase::Idle, Phase::Countdown(3)] {
            assert_eq!((tray_state(phase, true), tray_state(phase, false)), (TrayState::Replay, TrayState::Idle));
        }
        for replay_on in [false, true] {
            assert_eq!((tray_state(Phase::Recording, replay_on), tray_state(Phase::Paused, replay_on)), (TrayState::Recording, TrayState::Paused));
        }
    }

    #[test]
    fn a_mark_press_is_seen_once() {
        let mut seen = 0;
        let mut recent = VecDeque::new();
        assert!(!new_mark_press(&recent, &mut seen));
        recent.push_back(Command::new(CommandKind::StartStop, 5));
        recent.push_back(Command::new(CommandKind::Mark(MarkKind::WithLabel), 6));
        assert!(!new_mark_press(&recent, &mut seen), "only the plain mark counts");
        recent.push_back(Command::new(CommandKind::Mark(MarkKind::Silent), 10));
        assert!(new_mark_press(&recent, &mut seen));
        assert!(!new_mark_press(&recent, &mut seen), "the same press does not count again");
        recent.push_back(Command::new(CommandKind::Mark(MarkKind::Silent), 11));
        assert!(new_mark_press(&recent, &mut seen));
    }
}
