//! The engine controller: one thread that handles every command and owns the recording session.
//!
//! The window only draws [`ControllerState`] and sends commands, so hotkeys and tray clicks work
//! while the window is hidden and no frames run.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use thiserror::Error;

use super::bus::{Bus, BusSender, Command, CommandKind, MarkKind};
use super::cam::CamSummary;
use super::marks::{MarksWarning, RecentMark, TakeMarks};
use super::preflight::{self, PreflightReport};
use super::script::{PositionMsg, ScriptMsg, ScriptSnapshot, TakeScript};
use super::session::{RecordingSession, SessionConfig, StatsSnapshot, Summary, TrackSpec};
use super::stats::{self, GIB, StatsTracker, TakeStats};
use crate::config::{AudioCodecChoice, CompositeMode, Profile, Settings, TrackKind};
use crate::countdown::CountdownHandle;
use crate::label_popup::PopupHandle;
use crate::profiles;
use crate::sync_flash::SyncFlashHandle;
use crate::takes::{self, TakePaths};

/// How many recent commands the state keeps.
const RECENT_LIMIT: usize = 20;

/// How often a running take's counters are sampled.
const ACTIVE_POLL: Duration = Duration::from_millis(33);

/// How long the thread waits for a command while idle.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// How long the bitrate is averaged over while recording.
const BITRATE_WINDOW: Duration = Duration::from_millis(250);

/// The name of the track the mute hotkey prefers.
const MIC_TRACK_NAME: &str = "Mic";

/// How many API command tags are kept before the oldest are forgotten; a tag lives only until its command is handled.
const TAG_LIMIT: usize = 1024;

/// How many scripts the controller keeps a snapshot of before the oldest are forgotten.
const SNAPSHOT_LIMIT: usize = 32;

/// How often the free space is read while no take runs.
const IDLE_DISK_INTERVAL: Duration = Duration::from_secs(2);

/// Below this much free space a warning banner and sound are raised.
const LOW_FREE_BYTES: u64 = 10 * GIB;

/// Below this much free space the take is stopped.
const STOP_FREE_BYTES: u64 = 2 * GIB;

/// Below this many seconds of recording time left a warning banner and sound are raised.
const LOW_REMAINING_SECS: u64 = 15 * 60;

/// How far back the dropped-frame share is measured.
const OVERLOAD_WINDOW: Duration = Duration::from_secs(10);

/// The share of frames that may be dropped within the window before the encoder counts as overloaded.
const OVERLOAD_RATIO: f64 = 0.005;

/// How many warning banners the state keeps.
const BANNER_LIMIT: usize = 6;

/// How many raised warnings the state keeps for the API.
const WARNING_LOG_LIMIT: usize = 32;

/// The time between two numbers of the countdown.
const COUNTDOWN_STEP: Duration = Duration::from_secs(1);

/// How much media a pre-flight records before it stops itself, in milliseconds.
const PREFLIGHT_MEDIA_MS: u64 = 5000;

/// The longest a pre-flight may run on the wall clock before it is stopped anyway, in case no frames arrive.
const PREFLIGHT_TIMEOUT: Duration = Duration::from_secs(30);

/// How much media a sync test records before it stops itself, in milliseconds.
const SYNC_MEDIA_MS: u64 = 12_000;

/// The longest a sync test may run on the wall clock before it is stopped anyway, in case no frames arrive.
const SYNC_TIMEOUT: Duration = Duration::from_secs(40);

/// When the screen flashes during a sync test, in milliseconds after it starts.
const SYNC_FLASH_TIMES_MS: [u64; 5] = [2000, 4000, 6000, 8000, 10_000];

/// How long each flash of a sync test lasts, in milliseconds.
const SYNC_FLASH_MS: u32 = 100;

/// What the window shows while the sync test is measuring the files it recorded.
pub const SYNC_MEASURING: &str = "Measuring";

/// What the window shows while the sync test records and flashes.
pub const SYNC_RECORDING: &str = "Recording and flashing";

/// How far a running sync test has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncProgress {
    /// What the window shows next to the progress bar.
    pub status: &'static str,
    /// The share of the 12 seconds recorded so far, from 0 to 1.
    pub progress: f32,
}

/// What the window shows while a finished take's MP4 or MOV files are rewritten as standard files.
pub const FINALIZING: &str = "Finalizing";

/// How far the rewrite of a finished take's files has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinalizeProgress {
    /// What the window shows next to the progress bar.
    pub status: &'static str,
    /// The share of the take's files rewritten so far, from 0 to 1.
    pub progress: f32,
}

/// How far a running pre-flight has got.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreflightProgress {
    /// What the window shows next to the progress bar.
    pub status: &'static str,
    /// The share of the 5 seconds recorded so far, from 0 to 1.
    pub progress: f32,
}

/// A finished pre-flight: its report, and where its files are when they were kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightResult {
    /// The verdict on the test take.
    pub report: PreflightReport,
    /// The temp folder holding the test take's files, or none when they were deleted.
    pub kept_folder: Option<PathBuf>,
}

/// What the Recorder is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// No take is running.
    #[default]
    Idle,
    /// The countdown before a take is running; the number is the seconds left.
    Countdown(u8),
    /// A take is running.
    Recording,
    /// A take is running but paused.
    Paused,
}

impl Phase {
    /// Whether a take is running or paused, which is neither idle nor counting down.
    pub fn has_take(self) -> bool {
        matches!(self, Self::Recording | Self::Paused)
    }
}

/// The numbers of a take that has just ended, kept for the `recordingStopped` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastTake {
    /// The path of the take's video file.
    pub take: String,
    /// How much of the take was recorded, in milliseconds.
    pub elapsed_ms: u64,
    /// Frames repeated because no new one arrived in time.
    pub duplicated: u64,
    /// Frames dropped because the encoder was behind.
    pub dropped: u64,
}

/// One warning that was raised, kept so the control API can send it as a `warning` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningNote {
    /// The number of the warning, counting from 1 across all takes.
    pub seq: u64,
    /// A short machine-readable code such as `disk_low`.
    pub code: String,
    /// The text for people.
    pub message: String,
}

/// Everything the window draws, published by the controller thread.
#[derive(Default)]
pub struct ControllerState {
    /// Idle, counting down, recording or paused.
    pub phase: Phase,
    /// The paths of the running take.
    pub take: Option<TakePaths>,
    /// Frames per second of the running take, for the elapsed clock.
    pub fps: u32,
    /// The summary of the last finished take, kept until the next one starts.
    pub summary: Option<Summary>,
    /// The totals of the last finished take's camera file, kept until the next take starts; none when it had no camera file.
    pub cam_summary: Option<CamSummary>,
    /// The latest counters of the running take.
    pub stats: Option<StatsSnapshot>,
    /// Counts the stats samples published, so the window can tell a new one.
    pub stats_seq: u64,
    /// The `stats_seq` the window last used; the window sets it, so unread level peaks are kept.
    pub ui_seen_seq: u64,
    /// The bitrate of the running take over the last sample window, in megabits per second.
    pub mbps: f64,
    /// The text of the last start or stop error, for a red banner until the next take.
    pub error: Option<String>,
    /// The text of the yellow banner naming audio devices that were missing when the take started.
    pub audio_notice: Option<String>,
    /// The active profile name.
    pub active_profile: String,
    /// The effective composite mode of the running take, or of the active profile while idle.
    pub composite_mode: CompositeMode,
    /// Whether each track in the settings is muted, in settings order.
    pub track_muted: Vec<bool>,
    /// The name of each track in the settings, in settings order.
    pub track_names: Vec<String>,
    /// Counts the changes the controller made to the settings, so the window can copy them.
    pub settings_rev: u64,
    /// The commands handled most recently, oldest first.
    pub recent: VecDeque<Command>,
    /// How many marks the running or last take has.
    pub marks_count: usize,
    /// The last 10 marks of the running or last take, oldest first, as timecode, category and label.
    pub recent_marks: VecDeque<RecentMark>,
    /// The text of the warning banner for a marks problem; the recording carries on.
    pub marks_warning: Option<String>,
    /// The final numbers of the take that just ended, kept until the next take starts.
    pub last_take: Option<LastTake>,
    /// The live numbers of the running take, sampled twice a second.
    pub take_stats: Option<TakeStats>,
    /// Free space on the volume the next take goes to, read every 2 seconds while idle.
    pub idle_free_bytes: Option<u64>,
    /// The warning banners of the running or last take, oldest first.
    pub banners: Vec<String>,
    /// The warnings raised most recently, newest last.
    pub warnings: VecDeque<WarningNote>,
    /// How many warnings were raised so far; the newest note carries this number.
    pub warning_seq: u64,
    /// How many countdowns were cancelled so far, so the API can announce each one.
    pub countdown_cancels: u64,
    /// How many open API connections signed in as the Prompter.
    pub prompters: usize,
    /// How far the running pre-flight has got, or none while no pre-flight runs; the phase stays idle.
    pub preflight: Option<PreflightProgress>,
    /// The result of the last finished pre-flight, kept until the next one starts.
    pub preflight_result: Option<PreflightResult>,
    /// How far the running sync test has got, or none while no sync test runs; the phase stays idle.
    pub sync_test: Option<SyncProgress>,
    /// The result of the last finished sync test, or why it failed, kept until the next one starts.
    pub sync_result: Option<Result<media_io::SyncReport, String>>,
    /// How far the rewrite of the last take's MP4 or MOV files has got, or none while no rewrite runs; the phase stays idle.
    pub finalizing: Option<FinalizeProgress>,
}

/// A function the controller calls with the state after every change, while it holds the state lock.
///
/// It must be quick and must not add or remove listeners.
pub type Listener = Box<dyn Fn(&ControllerState) + Send>;

/// The functions that follow every state change, such as the tray and the control API.
#[derive(Default)]
struct Listeners {
    list: Mutex<Vec<(u64, Listener)>>,
    next_id: AtomicU64,
}

impl Listeners {
    /// Adds a listener and returns the id that removes it.
    fn add(&self, listener: Listener) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.list.lock().unwrap_or_else(PoisonError::into_inner).push((id, listener));
        id
    }

    /// Removes the listener with this id.
    fn remove(&self, id: u64) {
        self.list.lock().unwrap_or_else(PoisonError::into_inner).retain(|(listener_id, _)| *listener_id != id);
    }

    /// Calls every listener with the state.
    fn notify(&self, state: &ControllerState) {
        for (_, listener) in self.list.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            listener(state);
        }
    }
}

/// What an API client meant by a command, for the cases where the bus command alone is ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiIntent {
    /// Start a take, and do nothing if one is already running.
    Start,
    /// Stop the take, and do nothing if none is running.
    Stop,
    /// The mark came from the API, so it is stored with source api.
    Mark,
    /// Toggle mute on the named track, or on the mic track when none is named.
    Mute(Option<String>),
    /// Esc was pressed: cancel a running countdown, and do nothing otherwise.
    Escape,
}

/// The intents of API commands that are queued on the bus, found again by their press time.
#[derive(Debug, Clone, Default)]
struct ApiTags(Arc<Mutex<HashMap<i64, ApiIntent>>>);

impl ApiTags {
    /// Remembers the intent of the command issued at `press_qpc`.
    fn insert(&self, press_qpc: i64, intent: ApiIntent) {
        let mut tags = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if tags.len() >= TAG_LIMIT {
            tags.clear();
        }
        tags.insert(press_qpc, intent);
    }

    /// Takes the intent of the command issued at `press_qpc`, if it came from the API.
    fn remove(&self, press_qpc: i64) -> Option<ApiIntent> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&press_qpc)
    }
}

/// What the control API needs from the controller: the bus, the state and the listener list.
#[derive(Clone)]
pub struct ApiLink {
    bus: BusSender,
    tags: ApiTags,
    shared: Arc<Mutex<ControllerState>>,
    listeners: Arc<Listeners>,
    script: Sender<ScriptMsg>,
}

impl ApiLink {
    /// Queues a script message from an API client for the controller.
    pub fn send_script(&self, message: ScriptMsg) -> Result<(), String> {
        self.script.send(message).map_err(|_| "the Recorder is shutting down".to_owned())
    }

    /// Queues a command from an API client, issued at `press_qpc`, with what the client meant by it.
    pub fn send(&self, kind: CommandKind, intent: Option<ApiIntent>, press_qpc: i64) -> Result<(), String> {
        if let Some(intent) = intent {
            self.tags.insert(press_qpc, intent);
        }
        self.bus.send(Command::new(kind, press_qpc)).map_err(|_| {
            self.tags.remove(press_qpc);
            "the Recorder is shutting down".to_owned()
        })
    }

    /// A link with no controller behind it, for tests: it starts from `state`, and the returned bus receives its commands.
    #[cfg(test)]
    pub fn detached(state: ControllerState) -> (Self, Bus) {
        let (link, bus, _script) = Self::detached_with_script(state);
        (link, bus)
    }

    /// Like [`ApiLink::detached`], and the returned receiver gets the script messages.
    #[cfg(test)]
    pub fn detached_with_script(state: ControllerState) -> (Self, Bus, Receiver<ScriptMsg>) {
        let bus = Bus::default();
        let (script, script_rx) = mpsc::channel();
        let link = Self { bus: bus.sender(), tags: ApiTags::default(), shared: Arc::new(Mutex::new(state)), listeners: Arc::new(Listeners::default()), script };
        (link, bus, script_rx)
    }

    /// Counts a connection that signed in as the Prompter, until [`ApiLink::prompter_left`].
    pub fn prompter_joined(&self) {
        lock(&self.shared).prompters += 1;
    }

    /// Stops counting a Prompter connection that has closed.
    pub fn prompter_left(&self) {
        let mut state = lock(&self.shared);
        state.prompters = state.prompters.saturating_sub(1);
    }

    /// Reads the controller state through `read` while holding its lock; keep `read` short.
    pub fn read_state<T>(&self, read: impl FnOnce(&ControllerState) -> T) -> T {
        read(&lock(&self.shared))
    }

    /// Adds a listener that is called after every state change, and returns the id that removes it.
    pub fn add_listener(&self, listener: Listener) -> u64 {
        self.listeners.add(listener)
    }

    /// Removes a listener added by [`ApiLink::add_listener`].
    pub fn remove_listener(&self, id: u64) {
        self.listeners.remove(id);
    }
}

/// A message to the controller thread other than a command.
enum Control {
    /// The UI's current settings.
    Settings(Box<Settings>),
    /// Finish any running take and end the thread.
    Stop,
}

/// A cloneable handle that sends settings and the stop request to the controller thread.
#[derive(Clone)]
pub struct ControlSender {
    tx: Sender<Control>,
}

impl ControlSender {
    /// Gives the controller the settings as the window has them now.
    pub fn set_settings(&self, settings: Settings) {
        if self.tx.send(Control::Settings(Box::new(settings))).is_err() {
            log::warn!("settings were not sent because the controller has ended");
        }
    }

    /// Asks the controller to finish any running take and end; it does not wait.
    pub fn stop(&self) {
        // A controller that has already ended has nothing left to stop.
        let _ = self.tx.send(Control::Stop);
    }
}

/// The handle the app keeps: starts the controller thread and stops it on exit.
pub struct Controller {
    bus: BusSender,
    control: ControlSender,
    shared: Arc<Mutex<ControllerState>>,
    listeners: Arc<Listeners>,
    tags: ApiTags,
    script: Sender<ScriptMsg>,
    handle: Option<JoinHandle<()>>,
}

impl Controller {
    /// Starts the "controller" thread with a settings snapshot, a callback that asks the window to repaint and a
    /// first listener that is called with the state after every change, so the tray can follow while no frames run.
    /// More listeners can be added later through [`Controller::api_link`].
    /// The label popup sends its saved marks to this controller's bus.
    pub fn start(settings: Settings, repaint: Box<dyn Fn() + Send>, listener: Listener, popup: PopupHandle) -> Self {
        let bus = Bus::default();
        let bus_sender = bus.sender();
        popup.connect(bus.sender());
        let (control_tx, control_rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(ControllerState {
            fps: 60,
            active_profile: settings.active_profile.clone(),
            track_muted: settings.audio.tracks.iter().map(|track| track.muted).collect(),
            track_names: settings.audio.tracks.iter().map(|track| track.name.clone()).collect(),
            ..ControllerState::default()
        }));
        let listeners = Arc::new(Listeners::default());
        listeners.add(listener);
        let tags = ApiTags::default();
        let overlay = CountdownHandle::start(escape_callback(bus_sender.clone(), tags.clone()));
        let (script_tx, script_rx) = mpsc::channel();
        let worker = Worker {
            bus,
            control: control_rx,
            script: script_rx,
            snapshots: HashMap::new(),
            shared: Arc::clone(&shared),
            repaint,
            listeners: Arc::clone(&listeners),
            tags: tags.clone(),
            settings,
            active: None,
            popup,
            deferred: VecDeque::new(),
            last_idle_check: None,
            countdown: None,
            overlay,
            flash: SyncFlashHandle::start(),
            measuring: None,
        };
        let handle = match std::thread::Builder::new().name("controller".to_owned()).spawn(move || worker.run()) {
            Ok(handle) => Some(handle),
            Err(err) => {
                log::error!("the controller thread could not be started: {err}");
                lock(&shared).error = Some(format!("the recording engine could not start: {err}"));
                None
            }
        };
        Self { bus: bus_sender, control: ControlSender { tx: control_tx }, shared, listeners, tags, script: script_tx, handle }
    }

    /// The handle the control API uses: the bus with intents, the state, the listener list and the script messages.
    pub fn api_link(&self) -> ApiLink {
        ApiLink {
            bus: self.bus.clone(),
            tags: self.tags.clone(),
            shared: Arc::clone(&self.shared),
            listeners: Arc::clone(&self.listeners),
            script: self.script.clone(),
        }
    }

    /// A handle that hotkeys, the tray and buttons send commands through.
    pub fn sender(&self) -> BusSender {
        self.bus.clone()
    }

    /// A handle that sends settings and the stop request.
    pub fn control_sender(&self) -> ControlSender {
        self.control.clone()
    }

    /// The state the window draws.
    pub fn shared(&self) -> Arc<Mutex<ControllerState>> {
        Arc::clone(&self.shared)
    }

    /// Gives the controller the settings as the window has them now.
    pub fn set_settings(&self, settings: Settings) {
        self.control.set_settings(settings);
    }

    /// Stops a running take and finishes its file, ends the thread and waits for it.
    pub fn stop(&mut self) {
        self.control.stop();
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!("the controller thread panicked");
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Locks the shared state, carrying on with the data if a thread panicked while holding it.
pub fn lock(shared: &Mutex<ControllerState>) -> MutexGuard<'_, ControllerState> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A take that is running or paused.
struct ActiveTake {
    session: RecordingSession,
    /// The take's last marks error, read after every mark and at stop.
    marks_warning: MarksWarning,
    /// The take's script files and the script line being read.
    script: TakeScript,
    paused: bool,
    /// The time and byte count of the last bitrate sample.
    last_poll: Option<(Instant, u64)>,
    /// The bitrate of the last sample, in megabits per second.
    last_mbps: f64,
    /// When the take started.
    started: Instant,
    /// Frames per second of the take.
    fps: u32,
    /// Builds the live numbers twice a second.
    tracker: StatsTracker,
    /// Watches the live numbers for the conditions that raise warnings.
    monitor: Monitor,
    /// Set when this take is a pre-flight test, which has its own folder, progress and verdict.
    preflight: Option<PreflightRun>,
    /// Set when this take is a camera sync test, which has its own folder, flashes and measurement.
    sync: Option<SyncRun>,
}

/// What a running sync test needs: its temp folder, its main file, and how many flashes it has fired.
struct SyncRun {
    folder: PathBuf,
    main_file: PathBuf,
    /// When the session had started, for the flash times and the wall-clock limit.
    began: Instant,
    /// How many of the flashes were already shown.
    flashed: usize,
}

/// What a running pre-flight needs to finish: its temp folder and whether to keep it.
struct PreflightRun {
    folder: PathBuf,
    keep_files: bool,
    /// When the session had started, for the wall-clock limit.
    began: Instant,
}

/// Why a take could not start.
#[derive(Debug, Error)]
enum StartError {
    /// The take folder could not be chosen.
    #[error("{0}")]
    Take(String),
    /// The recording session refused to start.
    #[error("could not start recording into {file}: {reason}")]
    Session {
        /// The file the take would have written.
        file: String,
        /// Why the session failed.
        reason: String,
    },
}

/// The state of the controller thread.
struct Worker {
    bus: Bus,
    control: Receiver<Control>,
    /// Script messages from API clients.
    script: Receiver<ScriptMsg>,
    /// The latest script snapshot for each script id, kept across takes.
    snapshots: HashMap<String, ScriptSnapshot>,
    shared: Arc<Mutex<ControllerState>>,
    repaint: Box<dyn Fn() + Send>,
    listeners: Arc<Listeners>,
    tags: ApiTags,
    settings: Settings,
    active: Option<ActiveTake>,
    popup: PopupHandle,
    /// Commands that were taken off the bus while a stop collected the popup's mark; handled before new ones.
    deferred: VecDeque<Command>,
    /// When the free space was last read while idle.
    last_idle_check: Option<Instant>,
    /// The countdown that runs before the take, if one does.
    countdown: Option<CountdownRun>,
    /// The overlay that shows the countdown, and the Esc hotkey that cancels it.
    overlay: CountdownHandle,
    /// The white window the sync test flashes the screen with.
    flash: SyncFlashHandle,
    /// Receives the sync test's report from the thread that measures it.
    measuring: Option<Receiver<Result<media_io::SyncReport, String>>>,
}

/// The callback the countdown overlay calls on Esc: it queues a start/stop command that carries the Escape intent.
fn escape_callback(bus: BusSender, tags: ApiTags) -> Box<dyn Fn() + Send + Sync> {
    Box::new(move || {
        let press_qpc = capture::qpc_now();
        tags.insert(press_qpc, ApiIntent::Escape);
        if bus.send(Command::new(CommandKind::StartStop, press_qpc)).is_err() {
            tags.remove(press_qpc);
        }
    })
}

/// A countdown that is running: the seconds left, and when the next one is due.
#[derive(Debug, Clone, Copy)]
struct CountdownRun {
    remaining: u8,
    next_tick: Instant,
    /// Whether the overlay shows the numbers.
    visible: bool,
}

/// What one second of countdown did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tick {
    /// The countdown goes on with this many seconds left.
    Running(u8),
    /// The countdown reached zero, so the take starts.
    Done,
}

impl CountdownRun {
    /// A countdown of `secs` seconds that begins at `now`.
    fn new(secs: u8, now: Instant, visible: bool) -> Self {
        Self { remaining: secs, next_tick: now + COUNTDOWN_STEP, visible }
    }

    /// Moves on by one second when it is due at `now`; `None` while the current second is still running.
    fn tick(&mut self, now: Instant) -> Option<Tick> {
        if now < self.next_tick {
            return None;
        }
        self.next_tick += COUNTDOWN_STEP;
        self.remaining = self.remaining.saturating_sub(1);
        Some(if self.remaining == 0 { Tick::Done } else { Tick::Running(self.remaining) })
    }
}

/// The length of the countdown the settings ask for, or none when it is off.
fn countdown_length(secs: u8) -> Option<u8> {
    (secs > 0).then_some(secs)
}

/// What a command does when it reaches the controller, given whether a countdown is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    /// Cancel the countdown; no take starts.
    Cancel,
    /// Do nothing with the command.
    Ignore,
    /// Handle the command as usual.
    Pass,
}

/// Decides what a command does while a countdown may be running.
///
/// Esc or a second start/stop cancels it; pause, marks and profile changes are ignored until it is over.
fn countdown_verdict(counting: bool, kind: &CommandKind, intent: Option<&ApiIntent>) -> Verdict {
    if intent == Some(&ApiIntent::Escape) {
        return if counting { Verdict::Cancel } else { Verdict::Ignore };
    }
    if !counting {
        return Verdict::Pass;
    }
    match kind {
        // An API start is "make sure a take runs", which the countdown already does.
        CommandKind::StartStop if intent == Some(&ApiIntent::Start) => Verdict::Ignore,
        CommandKind::StartStop => Verdict::Cancel,
        CommandKind::Pause
        | CommandKind::Resume
        | CommandKind::PauseResume
        | CommandKind::Mark(_)
        | CommandKind::LabeledMark { .. }
        | CommandKind::SetProfile(_)
        | CommandKind::Preflight { .. }
        | CommandKind::SyncTest => Verdict::Ignore,
        CommandKind::Mute { .. } | CommandKind::GetStatus => Verdict::Pass,
    }
}

impl Worker {
    /// Handles settings, commands and stats samples until told to stop, then finishes any running take.
    fn run(mut self) {
        while self.drain_control() {
            self.drain_script();
            let wait = if self.active.is_some() || self.countdown.is_some() { ACTIVE_POLL } else { IDLE_POLL };
            if let Some(command) = self.deferred.pop_front().or_else(|| self.bus.recv_timeout(wait)) {
                self.handle(command);
            }
            self.poll_countdown();
            if self.is_preflight() {
                self.poll_preflight();
            } else if self.is_sync() {
                self.poll_sync();
            } else {
                self.poll_stats();
            }
            self.poll_measuring();
            self.poll_idle_disk();
        }
        self.cancel_countdown();
        self.stop_take();
    }

    /// Applies the queued settings; false once the stop request came or the app dropped its handles.
    fn drain_control(&mut self) -> bool {
        let profile_before = self.settings.active_profile.clone();
        let (keep_running, changed) = apply_control(&self.control, &mut self.settings);
        if self.settings.active_profile != profile_before {
            // The idle check then reads the new profile's composite mode right away.
            self.last_idle_check = None;
        }
        if changed {
            self.forward_live_controls();
            let names: Vec<String> = self.settings.audio.tracks.iter().map(|track| track.name.clone()).collect();
            self.update_quietly(|state| state.track_names = names);
        }
        keep_running
    }

    /// Handles every script message that is waiting.
    fn drain_script(&mut self) {
        while let Ok(message) = self.script.try_recv() {
            match message {
                ScriptMsg::Snapshot { script_id, title, markdown } => {
                    if self.snapshots.len() >= SNAPSHOT_LIMIT && !self.snapshots.contains_key(&script_id) {
                        self.snapshots.clear();
                    }
                    self.snapshots.insert(script_id, ScriptSnapshot { title, markdown });
                }
                ScriptMsg::Position(position) => self.script_position(&position),
            }
        }
    }

    /// Logs a script position in the running take and makes its line the one that marks carry; with no take it is ignored.
    ///
    /// A file error becomes a warning banner and the recording carries on.
    fn script_position(&mut self, position: &PositionMsg) {
        let Some(active) = &mut self.active else { return };
        if active.preflight.is_some() || active.sync.is_some() {
            return;
        }
        let ms = active.session.marks_mut().map_or(0, |marks| marks.clock_ms(position.qpc));
        let result = active.script.position(self.snapshots.get(&position.script_id), position, ms);
        if let (Some((line, label)), Some(marks)) = (active.script.current(), active.session.marks_mut()) {
            marks.set_script(line, label);
        }
        if let Err(err) = result {
            log::warn!("script: {err}");
            self.update(|state| state.marks_warning = Some(format!("Script problem: {err}")));
        }
    }

    /// Changes the shared state, tells the listener about it and asks the window to repaint.
    fn update(&self, change: impl FnOnce(&mut ControllerState)) {
        {
            let mut state = lock(&self.shared);
            change(&mut state);
            self.listeners.notify(&state);
        }
        (self.repaint)();
    }

    /// Changes the shared state without a repaint; the window repaints itself while it shows a take.
    fn update_quietly(&self, change: impl FnOnce(&mut ControllerState)) {
        change(&mut lock(&self.shared));
    }

    /// Acts on one command.
    fn handle(&mut self, command: Command) {
        log::info!("command: {command:?}");
        // A script position sent before this command sits in its own queue; it must count first.
        self.drain_script();
        let press = command.press_qpc;
        // Only these commands can carry an API intent; the others have nothing to look up.
        let intent = match command.kind {
            CommandKind::StartStop | CommandKind::LabeledMark { .. } | CommandKind::Mute { .. } => self.tags.remove(press),
            _ => None,
        };
        match countdown_verdict(self.countdown.is_some(), &command.kind, intent.as_ref()) {
            Verdict::Cancel => self.cancel_countdown(),
            Verdict::Ignore => log::info!("command ignored: {:?} does nothing now", command.kind),
            Verdict::Pass => self.run_command(&command.kind, intent, press),
        }
        self.update(|state| {
            if state.recent.len() == RECENT_LIMIT {
                state.recent.pop_front();
            }
            state.recent.push_back(command);
        });
    }

    /// Carries out a command that the countdown let through.
    fn run_command(&mut self, kind: &CommandKind, intent: Option<ApiIntent>, press: i64) {
        // A pre-flight only listens to start/stop, which cancels it, and to the commands that change nothing in it.
        if self.is_preflight() && !matches!(kind, CommandKind::StartStop | CommandKind::Mute { .. } | CommandKind::GetStatus) {
            log::info!("command ignored: {kind:?} does nothing during a pre-flight");
            return;
        }
        // A sync test listens to the same few commands as a pre-flight.
        if self.is_sync() && !matches!(kind, CommandKind::StartStop | CommandKind::Mute { .. } | CommandKind::GetStatus) {
            log::info!("command ignored: {kind:?} does nothing during a sync test");
            return;
        }
        match kind {
            CommandKind::StartStop if self.is_preflight() => self.cancel_preflight(),
            CommandKind::StartStop if self.is_sync() => self.cancel_sync(),
            CommandKind::StartStop => match (self.active.is_some(), &intent) {
                (true, Some(ApiIntent::Start)) => log::info!("start ignored: a take is already running"),
                (false, Some(ApiIntent::Stop)) => log::info!("stop ignored: no take is running"),
                (true, _) => self.stop_take(),
                (false, _) => self.begin_take(),
            },
            CommandKind::Pause => self.pause(press),
            CommandKind::Resume => self.resume(press),
            CommandKind::PauseResume => {
                if self.active.as_ref().is_some_and(|active| active.paused) {
                    self.resume(press);
                } else {
                    self.pause(press);
                }
            }
            CommandKind::Mute { .. } => match intent {
                Some(ApiIntent::Mute(Some(name))) => self.toggle_mute(Some(name.as_str())),
                _ => self.toggle_mute(None),
            },
            CommandKind::SetProfile(name) => self.set_profile(name),
            CommandKind::Preflight { keep_files } => self.start_preflight(*keep_files),
            CommandKind::SyncTest => self.start_sync(),
            CommandKind::Mark(kind) => self.add_mark(*kind, press),
            CommandKind::LabeledMark { category, label } => {
                let source = if intent == Some(ApiIntent::Mark) { ::marks::Source::Api } else { ::marks::Source::Hotkey };
                self.add_labeled(*category, label.clone(), press, source);
            }
            CommandKind::GetStatus => log::info!("status requested (the control API reads it from the shared state)"),
        }
    }

    /// Starts the countdown when the settings ask for one, or else the take at once.
    fn begin_take(&mut self) {
        match countdown_length(self.settings.countdown_secs) {
            Some(secs) => self.start_countdown(secs),
            None => self.start_take(),
        }
    }

    /// Enters the countdown: arms Esc, shows the overlay unless a linked Prompter shows its own, and publishes the phase.
    fn start_countdown(&mut self, secs: u8) {
        let linked = lock(&self.shared).prompters > 0;
        let visible = !(self.settings.hide_countdown_when_linked && linked);
        self.countdown = Some(CountdownRun::new(secs, Instant::now(), visible));
        self.overlay.arm();
        if visible {
            self.overlay.show(secs);
        }
        self.update(|state| state.phase = Phase::Countdown(secs));
    }

    /// Moves the countdown on by a second when one is due, and starts the take when it reaches zero.
    fn poll_countdown(&mut self) {
        let Some(run) = &mut self.countdown else { return };
        let tick = run.tick(Instant::now());
        let visible = run.visible;
        match tick {
            None => {}
            Some(Tick::Running(left)) => {
                if visible {
                    self.overlay.show(left);
                }
                self.update(|state| state.phase = Phase::Countdown(left));
            }
            Some(Tick::Done) => {
                self.countdown = None;
                self.overlay.hide();
                self.start_take();
            }
        }
    }

    /// Ends a running countdown without a take: hides the overlay, releases Esc and announces the cancel.
    fn cancel_countdown(&mut self) {
        if self.countdown.take().is_none() {
            return;
        }
        self.overlay.hide();
        self.update(|state| {
            state.phase = Phase::Idle;
            state.countdown_cancels += 1;
        });
    }

    /// Starts a new take; on failure shows a red banner and stays idle.
    fn start_take(&mut self) {
        self.update(|state| {
            state.error = None;
            state.audio_notice = None;
            state.summary = None;
            state.cam_summary = None;
            state.marks_count = 0;
            state.recent_marks.clear();
            state.marks_warning = None;
            state.last_take = None;
            state.take_stats = None;
            state.banners.clear();
        });
        if let Err(err) = self.start_take_inner() {
            log::error!("could not start the recording: {err}");
            self.update(|state| {
                state.error = Some(err.to_string());
                // A countdown that just ran out leaves its last number behind otherwise.
                state.phase = Phase::Idle;
            });
        }
    }

    /// Picks the take folder, builds the session config from the active profile and starts the session.
    fn start_take_inner(&mut self) -> Result<(), StartError> {
        let profile = profiles::find(&self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        let take = takes::next_take_for(&self.settings, chrono::Local::now(), profile.container).map_err(|err| StartError::Take(err.to_string()))?;
        let fps = profile.fps;
        let composite_mode = profile.effective_composite_mode();
        let (tracks, missing) = track_specs(&self.settings);
        if !missing.is_empty() {
            log::warn!("recording without these audio devices: {}", missing.join(", "));
            let notice = format!("Audio device not found for {}. Recording anyway; those tracks stay silent until the device returns.", missing.join(", "));
            self.update(|state| state.audio_notice = Some(notice));
        }
        let take_folder = take.video_file.parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let base_name = take.video_file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let script = TakeScript::new(&take_folder, &base_name, chrono::Local::now());
        let started = Instant::now();
        let tracker = StatsTracker::new(take_folder.clone(), started);
        let cfg = self.session_config(profile, take.video_file.clone(), take_folder, base_name, tracks);
        let session =
            RecordingSession::start(cfg).map_err(|err| StartError::Session { file: take.video_file.display().to_string(), reason: err.to_string() })?;
        let marks_warning = session.marks_warning();
        let cam_notice = session.cam_notice().map(str::to_owned);
        self.active = Some(ActiveTake {
            session,
            marks_warning,
            script,
            paused: false,
            last_poll: None,
            last_mbps: 0.0,
            started,
            fps,
            tracker,
            monitor: Monitor::default(),
            preflight: None,
            sync: None,
        });
        // A camera file the profile asked for but the take could not have is a banner; the recording carries on.
        if let Some(notice) = cam_notice {
            self.update(|state| state.banners.push(notice));
        }
        self.update(|state| {
            state.phase = Phase::Recording;
            state.take = Some(take);
            state.fps = fps;
            state.composite_mode = composite_mode;
            state.stats = None;
            state.mbps = 0.0;
        });
        // A sidecar that could not be started leaves its reason for the banner.
        self.with_marks(|_| {});
        Ok(())
    }

    /// Builds the session config for a take or a pre-flight from the settings and the profile.
    fn session_config(&self, profile: Profile, output_file: PathBuf, take_folder: PathBuf, base_name: String, tracks: Vec<TrackSpec>) -> SessionConfig {
        let (mixdown, track_streams) = (profile.mixdown, profile.aac_tracks);
        SessionConfig {
            adapter: 0,
            monitor: 0,
            cursor: true,
            profile,
            output_file,
            tracks,
            audio_codec: audio_codec(self.settings.audio.codec),
            mixdown,
            track_streams,
            wav_sidecars: self.settings.audio.wav_sidecars,
            take_folder,
            base_name,
        }
    }

    /// Whether the running take is a pre-flight test.
    fn is_preflight(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.preflight.is_some())
    }

    /// Starts a pre-flight, only while idle; on failure shows a red banner and stays idle.
    fn start_preflight(&mut self, keep_files: bool) {
        if self.active.is_some() {
            log::info!("pre-flight ignored: a take is running");
            return;
        }
        self.update(|state| {
            state.error = None;
            state.preflight_result = None;
        });
        if let Err(err) = self.start_preflight_inner(keep_files) {
            log::error!("could not start the pre-flight: {err}");
            self.update(|state| state.error = Some(format!("The pre-flight could not start: {err}")));
        }
    }

    /// Records into a fresh folder under the temp folder, outside the takes root, so it takes no take number and never shows in recovery.
    ///
    /// There is no countdown, and no take, marks or script state of the window changes.
    fn start_preflight_inner(&mut self, keep_files: bool) -> Result<(), StartError> {
        let profile = profiles::find(&self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        let now = chrono::Local::now();
        let folder = std::env::temp_dir().join("Cutmark").join("preflight").join(now.format("%Y-%m-%d_%H-%M-%S").to_string());
        std::fs::create_dir_all(&folder).map_err(|err| StartError::Take(format!("could not create {}: {err}", folder.display())))?;
        let base_name = "preflight".to_owned();
        let extension = format!("{:?}", profile.container).to_lowercase();
        let output_file = folder.join(format!("{base_name}.{extension}"));
        let fps = profile.fps;
        let (tracks, missing) = track_specs(&self.settings);
        if !missing.is_empty() {
            log::warn!("pre-flight without these audio devices: {}", missing.join(", "));
        }
        let script = TakeScript::new(&folder, &base_name, now);
        let started = Instant::now();
        let tracker = StatsTracker::new(folder.clone(), started);
        let cfg = self.session_config(profile, output_file.clone(), folder.clone(), base_name, tracks);
        let session = match RecordingSession::start(cfg) {
            Ok(session) => session,
            Err(err) => {
                remove_folder(&folder);
                return Err(StartError::Session { file: output_file.display().to_string(), reason: err.to_string() });
            }
        };
        let marks_warning = session.marks_warning();
        self.active = Some(ActiveTake {
            session,
            marks_warning,
            script,
            paused: false,
            last_poll: None,
            last_mbps: 0.0,
            started,
            fps,
            tracker,
            monitor: Monitor::default(),
            preflight: Some(PreflightRun { folder, keep_files, began: Instant::now() }),
            sync: None,
        });
        self.update(|state| state.preflight = Some(PreflightProgress { status: preflight::RUNNING, progress: 0.0 }));
        Ok(())
    }

    /// Publishes the pre-flight's progress, and finishes it after 5 seconds of media time.
    fn poll_preflight(&mut self) {
        let Some(active) = &self.active else { return };
        let Some(run) = &active.preflight else { return };
        let media_ms = active.session.stats().frames.saturating_mul(1000) / u64::from(active.fps.max(1));
        if media_ms >= PREFLIGHT_MEDIA_MS || run.began.elapsed() >= PREFLIGHT_TIMEOUT {
            self.finish_preflight();
            return;
        }
        let progress = media_ms as f32 / PREFLIGHT_MEDIA_MS as f32;
        self.update_quietly(|state| state.preflight = Some(PreflightProgress { status: preflight::RUNNING, progress }));
    }

    /// Stops the pre-flight's session, builds the report, and deletes the temp folder unless it is kept.
    fn finish_preflight(&mut self) {
        if !self.is_preflight() {
            return;
        }
        let Some(active) = self.active.take() else { return };
        let Some(run) = active.preflight else { return };
        let fps = active.fps;
        // The report is built from the summary before the folder goes.
        let (stopped, cam) = active.session.stop_with_cam();
        let report = stopped.map(|summary| preflight::judge_with_cam(&summary, cam.as_ref(), fps));
        let keep = run.keep_files && report.is_ok();
        if !keep {
            remove_folder(&run.folder);
        }
        // The free space is read again right away, so the idle status bar shows what the test left.
        self.last_idle_check = None;
        match report {
            Ok(report) => {
                log::info!("pre-flight finished: {}", report.overall.label());
                let kept_folder = keep.then_some(run.folder);
                self.update(|state| {
                    state.preflight = None;
                    state.preflight_result = Some(PreflightResult { report, kept_folder });
                });
            }
            Err(err) => {
                log::error!("the pre-flight failed on stop: {err}");
                self.update(|state| {
                    state.preflight = None;
                    state.error = Some(format!("The pre-flight failed: {err}"));
                });
            }
        }
    }

    /// Stops the pre-flight's session and deletes its temp folder; there is no report.
    fn cancel_preflight(&mut self) {
        if !self.is_preflight() {
            return;
        }
        let Some(active) = self.active.take() else { return };
        let Some(run) = active.preflight else { return };
        if let Err(err) = active.session.stop() {
            log::warn!("the cancelled pre-flight did not stop cleanly: {err}");
        }
        remove_folder(&run.folder);
        self.last_idle_check = None;
        log::info!("pre-flight cancelled");
        self.update(|state| state.preflight = None);
    }

    /// Whether the running take is a camera sync test.
    fn is_sync(&self) -> bool {
        self.active.as_ref().is_some_and(|active| active.sync.is_some())
    }

    /// Starts a sync test, only while idle; on failure shows a red banner and stays idle.
    fn start_sync(&mut self) {
        if self.active.is_some() || self.countdown.is_some() || self.measuring.is_some() {
            log::info!("sync test ignored: a take, a countdown or a measurement is running");
            return;
        }
        self.update(|state| {
            state.error = None;
            state.sync_result = None;
        });
        if let Err(err) = self.start_sync_inner() {
            log::error!("could not start the sync test: {err}");
            self.update(|state| state.error = Some(format!("The sync test could not start: {err}")));
        }
    }

    /// Records into a fresh folder under the temp folder with the camera file on and its offset at zero.
    ///
    /// Like a pre-flight it has no countdown, take number, marks or script; the flashes come from [`Worker::poll_sync`].
    fn start_sync_inner(&mut self) -> Result<(), StartError> {
        let mut profile = profiles::find(&self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        match profile.webcam.as_mut() {
            // The test measures the camera as it is, so any offset already saved must not move the files.
            Some(webcam) if webcam.iso => webcam.offset_ms = 0,
            _ => return Err(StartError::Take("turn on the webcam and its camera file in the profile first".to_owned())),
        }
        let now = chrono::Local::now();
        let folder = std::env::temp_dir().join("Cutmark").join("synctest").join(now.format("%Y-%m-%d_%H-%M-%S").to_string());
        std::fs::create_dir_all(&folder).map_err(|err| StartError::Take(format!("could not create {}: {err}", folder.display())))?;
        let base_name = "synctest".to_owned();
        let extension = format!("{:?}", profile.container).to_lowercase();
        let output_file = folder.join(format!("{base_name}.{extension}"));
        let fps = profile.fps;
        let (tracks, missing) = track_specs(&self.settings);
        if !missing.is_empty() {
            log::warn!("sync test without these audio devices: {}", missing.join(", "));
        }
        let script = TakeScript::new(&folder, &base_name, now);
        let started = Instant::now();
        let tracker = StatsTracker::new(folder.clone(), started);
        let cfg = self.session_config(profile, output_file.clone(), folder.clone(), base_name, tracks);
        let session = match RecordingSession::start(cfg) {
            Ok(session) => session,
            Err(err) => {
                remove_folder(&folder);
                return Err(StartError::Session { file: output_file.display().to_string(), reason: err.to_string() });
            }
        };
        if let Some(notice) = session.cam_notice().map(str::to_owned) {
            if let Err(err) = session.stop() {
                log::warn!("the sync test did not stop cleanly: {err}");
            }
            remove_folder(&folder);
            return Err(StartError::Take(notice));
        }
        let marks_warning = session.marks_warning();
        self.active = Some(ActiveTake {
            session,
            marks_warning,
            script,
            paused: false,
            last_poll: None,
            last_mbps: 0.0,
            started,
            fps,
            tracker,
            monitor: Monitor::default(),
            preflight: None,
            sync: Some(SyncRun { folder, main_file: output_file, began: Instant::now(), flashed: 0 }),
        });
        self.update(|state| state.sync_test = Some(SyncProgress { status: SYNC_RECORDING, progress: 0.0 }));
        Ok(())
    }

    /// Fires the flashes when they are due, publishes the progress, and finishes the test after 12 seconds of media time.
    fn poll_sync(&mut self) {
        let Some(active) = &mut self.active else { return };
        let Some(run) = &mut active.sync else { return };
        let elapsed_ms = u64::try_from(run.began.elapsed().as_millis()).unwrap_or(u64::MAX);
        if SYNC_FLASH_TIMES_MS.get(run.flashed).is_some_and(|due| elapsed_ms >= *due) {
            run.flashed += 1;
            self.flash.flash(SYNC_FLASH_MS);
        }
        let media_ms = active.session.stats().frames.saturating_mul(1000) / u64::from(active.fps.max(1));
        if media_ms >= SYNC_MEDIA_MS || run.began.elapsed() >= SYNC_TIMEOUT {
            self.finish_sync();
            return;
        }
        let progress = media_ms as f32 / SYNC_MEDIA_MS as f32;
        self.update_quietly(|state| state.sync_test = Some(SyncProgress { status: SYNC_RECORDING, progress }));
    }

    /// Stops the sync test's session and measures its two files on a worker thread, which deletes the folder when done.
    fn finish_sync(&mut self) {
        if !self.is_sync() {
            return;
        }
        let Some(active) = self.active.take() else { return };
        let Some(run) = active.sync else { return };
        let stopped = active.session.stop();
        self.last_idle_check = None;
        let failure = match (stopped, find_cam_file(&run.folder, &run.main_file)) {
            (Err(err), _) => Some(format!("The sync test failed on stop: {err}")),
            (Ok(_), None) => Some("The sync test did not write a camera file.".to_owned()),
            (Ok(_), Some(cam)) => {
                let (tx, rx) = mpsc::channel();
                let (folder, main) = (run.folder.clone(), run.main_file.clone());
                let spawned = std::thread::Builder::new().name("sync-measure".to_owned()).spawn(move || {
                    let result = media_io::measure_offset(&main, &cam).map_err(|err| err.to_string());
                    remove_folder(&folder);
                    // The controller may have ended while the files were measured, and then nobody wants the result.
                    let _ = tx.send(result);
                });
                match spawned {
                    Ok(_) => {
                        self.measuring = Some(rx);
                        self.update(|state| state.sync_test = Some(SyncProgress { status: SYNC_MEASURING, progress: 1.0 }));
                        return;
                    }
                    Err(err) => Some(format!("The sync test could not start its measurement: {err}")),
                }
            }
        };
        remove_folder(&run.folder);
        self.update(|state| {
            state.sync_test = None;
            state.error = failure;
        });
    }

    /// Publishes the report once the measuring thread has one.
    fn poll_measuring(&mut self) {
        let Some(receiver) = &self.measuring else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the measurement ended without a result".to_owned()),
        };
        self.measuring = None;
        self.update(|state| {
            state.sync_test = None;
            state.sync_result = Some(result);
        });
    }

    /// Stops the sync test's session and deletes its temp folder; there is no report.
    fn cancel_sync(&mut self) {
        if !self.is_sync() {
            return;
        }
        let Some(active) = self.active.take() else { return };
        let Some(run) = active.sync else { return };
        if let Err(err) = active.session.stop() {
            log::warn!("the cancelled sync test did not stop cleanly: {err}");
        }
        remove_folder(&run.folder);
        self.last_idle_check = None;
        log::info!("sync test cancelled");
        self.update(|state| state.sync_test = None);
    }

    /// Stops the running take, keeps its summary for the window and returns to idle.
    fn stop_take(&mut self) {
        if self.is_preflight() {
            self.cancel_preflight();
            return;
        }
        if self.is_sync() {
            self.cancel_sync();
            return;
        }
        if self.active.is_none() {
            return;
        }
        self.commit_popup();
        let Some(active) = self.active.take() else { return };
        let marks_warning = active.marks_warning.clone();
        let video_file = lock(&self.shared).take.as_ref().map(|take| take.video_file.clone());
        // The counters as the take ends, for the recordingStopped event; the session's own summary comes later.
        let last = active.session.stats();
        self.update(|state| {
            let fps = u64::from(state.fps.max(1));
            state.last_take = state.take.as_ref().map(|take| LastTake {
                take: take.video_file.display().to_string(),
                elapsed_ms: last.frames.saturating_mul(1000) / fps,
                duplicated: last.duplicated,
                dropped: last.dropped,
            });
            state.phase = Phase::Idle;
            state.take = None;
            state.stats = None;
            state.take_stats = None;
            state.mbps = 0.0;
        });
        // The free space is read again right away, so the idle status bar shows what the take left.
        self.last_idle_check = None;
        let (stopped, cam) = active.session.stop_with_cam();
        let cam_file = cam.as_ref().map(|cam| cam.file.clone());
        // Finishing the marks happens inside stop and may leave a warning.
        if let Some(text) = marks_warning.take() {
            self.update(|state| state.marks_warning = Some(text));
        }
        match stopped {
            Ok(summary) => {
                log::info!("take finished: {} frames, {} bytes", summary.frames, summary.bytes_written);
                self.update(|state| {
                    state.summary = Some(summary);
                    state.cam_summary = cam;
                });
                // MP4 and MOV takes were written fragmented; a clean stop rewrites them as standard files.
                self.finalize_files(video_file.into_iter().chain(cam_file).collect());
            }
            Err(err) => {
                log::error!("the recording failed on stop: {err}");
                self.update(|state| state.error = Some(format!("the recording failed on stop: {err}")));
            }
        }
    }

    /// Rewrites each MP4 or MOV file of a cleanly stopped take as a standard file with the moov atom first,
    /// showing "Finalizing" with the progress. A file that cannot be rewritten stays fragmented and playable,
    /// and a warning banner names it. MKV files are final as written and are skipped.
    fn finalize_files(&mut self, files: Vec<PathBuf>) {
        let files: Vec<(PathBuf, media_io::Container)> = files
            .into_iter()
            .filter_map(|file| {
                let container = file.extension().and_then(|extension| extension.to_str()).and_then(media_io::Container::from_extension)?;
                container.needs_finalize().then_some((file, container))
            })
            .collect();
        if files.is_empty() {
            return;
        }
        let count = files.len() as f32;
        for (index, (file, container)) in files.iter().enumerate() {
            let before = index as f32;
            self.update(|state| state.finalizing = Some(FinalizeProgress { status: FINALIZING, progress: before / count }));
            let mut shown = -1i32;
            let result = media_io::mux::finalize(file, *container, &mut |done| {
                let progress = (before + done) / count;
                // The window is asked to repaint only when the shown percentage changes.
                let percent = (progress * 100.0) as i32;
                if percent != shown {
                    shown = percent;
                    self.update(|state| state.finalizing = Some(FinalizeProgress { status: FINALIZING, progress }));
                }
            });
            match result {
                Ok(()) => log::info!("finalized {}", file.display()),
                Err(err) => {
                    log::warn!("could not finalize {}: {err}", file.display());
                    let name = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    let text = format!("{name} could not be finished and stays in its fragmented form, which still plays: {err}");
                    self.update(|state| {
                        if state.banners.len() >= BANNER_LIMIT {
                            state.banners.remove(0);
                        }
                        state.banners.push(text);
                    });
                }
            }
        }
        self.update(|state| state.finalizing = None);
    }

    /// Pauses the running take at the moment of the press.
    fn pause(&mut self, press_qpc: i64) {
        let Some(active) = &mut self.active else { return };
        if active.paused {
            return;
        }
        active.session.pause();
        active.paused = true;
        self.with_marks(|marks| marks.pause(press_qpc));
        self.update(|state| state.phase = Phase::Paused);
    }

    /// Resumes a paused take at the moment of the press.
    fn resume(&mut self, press_qpc: i64) {
        let Some(active) = &mut self.active else { return };
        if !active.paused {
            return;
        }
        active.session.resume();
        active.paused = false;
        self.with_marks(|marks| marks.resume(press_qpc));
        self.update(|state| state.phase = Phase::Recording);
    }

    /// Adds a mark of the given kind to the running take; with no take it is ignored.
    fn add_mark(&mut self, kind: MarkKind, press_qpc: i64) {
        if self.active.is_none() {
            log::info!("mark {kind:?} ignored: no take is running");
            return;
        }
        match kind {
            MarkKind::Silent => self.with_marks(|marks| marks.note(press_qpc)),
            MarkKind::WithLabel => self.popup.open(press_qpc),
            MarkKind::HighlightLast => self.with_marks(|marks| marks.highlight_last(press_qpc)),
        }
    }

    /// Adds the popup's labeled point mark to the running take; with no take it is ignored.
    fn add_labeled(&mut self, category: ::marks::MarkType, label: Option<String>, press_qpc: i64, source: ::marks::Source) {
        if self.active.is_none() {
            log::info!("labeled mark ignored: no take is running");
            return;
        }
        self.with_marks(|marks| marks.labeled(press_qpc, category, label, source));
    }

    /// Saves an open popup so its mark lands in the take that is stopping.
    ///
    /// The popup's command is on the bus by the time `commit_now` returns. It is handled here, and any other
    /// command taken off the bus on the way waits in `deferred` for the main loop.
    fn commit_popup(&mut self) {
        self.popup.commit_now();
        while let Some(command) = self.bus.recv_timeout(Duration::ZERO) {
            if matches!(command.kind, CommandKind::LabeledMark { .. }) {
                self.handle(command);
            } else {
                self.deferred.push_back(command);
            }
        }
    }

    /// Changes the running take's marks, then publishes their count, list and any warning.
    fn with_marks(&mut self, change: impl FnOnce(&mut TakeMarks)) {
        let Some(active) = &mut self.active else { return };
        if let Some(marks) = active.session.marks_mut() {
            change(marks);
        }
        let warning = active.marks_warning.take();
        let (count, recent) = match active.session.marks_mut() {
            Some(marks) => (marks.count(), marks.recent().clone()),
            None => (0, VecDeque::new()),
        };
        self.update(|state| {
            state.marks_count = count;
            state.recent_marks = recent;
            if warning.is_some() {
                state.marks_warning = warning;
            }
        });
    }

    /// Toggles mute on the track with this name, or with none on the track named Mic, or else on the first Input track,
    /// in the settings and in the running take.
    fn toggle_mute(&mut self, name: Option<&str>) {
        let tracks = &mut self.settings.audio.tracks;
        let position = match name {
            Some(name) => tracks.iter().position(|track| track.name.eq_ignore_ascii_case(name)),
            None => tracks
                .iter()
                .position(|track| track.name.eq_ignore_ascii_case(MIC_TRACK_NAME))
                .or_else(|| tracks.iter().position(|track| track.kind == TrackKind::Input)),
        };
        let Some(track) = position.and_then(|index| tracks.get_mut(index)) else {
            log::warn!("mute: there is no matching track to mute");
            return;
        };
        track.muted = !track.muted;
        log::info!("track {} is now {}", track.name, if track.muted { "muted" } else { "unmuted" });
        self.forward_live_controls();
        self.publish_settings();
    }

    /// Switches the active profile, only while idle and only to a profile that exists.
    fn set_profile(&mut self, name: &str) {
        if self.active.is_some() {
            log::warn!("the profile cannot change while a take is running");
            return;
        }
        if profiles::find(name).is_none() {
            log::warn!("there is no profile named {name}");
            return;
        }
        self.settings.active_profile = name.to_owned();
        self.last_idle_check = None;
        self.publish_settings();
    }

    /// Shows the window the changes the controller made to the settings.
    fn publish_settings(&self) {
        let profile = self.settings.active_profile.clone();
        let muted: Vec<bool> = self.settings.audio.tracks.iter().map(|track| track.muted).collect();
        let names: Vec<String> = self.settings.audio.tracks.iter().map(|track| track.name.clone()).collect();
        self.update(|state| {
            state.active_profile = profile;
            state.track_muted = muted;
            state.track_names = names;
            state.settings_rev += 1;
        });
    }

    /// Sends every track's gain and mute from the settings to the running take.
    fn forward_live_controls(&self) {
        let Some(active) = &self.active else { return };
        // A take only has the enabled tracks, in settings order.
        for (index, track) in self.settings.audio.tracks.iter().filter(|track| track.enabled).enumerate() {
            active.session.set_track_gain_db(index, track.gain_db);
            active.session.set_track_muted(index, track.muted);
        }
    }

    /// Samples the running take's counters, works out the bitrate, and publishes both.
    fn poll_stats(&mut self) {
        let Some(active) = &mut self.active else { return };
        let mut stats = active.session.stats();
        let now = Instant::now();
        match active.last_poll {
            Some((then, bytes_then)) => {
                let seconds = now.duration_since(then).as_secs_f64();
                // Sampled faster than the bitrate should be, so the last value stays in between.
                if seconds >= BITRATE_WINDOW.as_secs_f64() {
                    active.last_mbps = stats.bytes_written.saturating_sub(bytes_then) as f64 * 8.0 / 1_000_000.0 / seconds;
                    active.last_poll = Some((now, stats.bytes_written));
                }
            }
            None => {
                active.last_mbps = 0.0;
                active.last_poll = Some((now, stats.bytes_written));
            }
        }
        let mbps = active.last_mbps;
        let counters = active.session.counters();
        let sample = active.tracker.due(now).then(|| active.tracker.sample(now, &stats, counters, active.fps));
        let mut alerts = match &sample {
            Some(sample) => {
                let tracks: Vec<(&str, bool)> = stats.tracks.iter().map(|track| (track.name.as_str(), track.device_lost)).collect();
                active.monitor.check(now.saturating_duration_since(active.started), sample, &tracks, active.paused)
            }
            None => Vec::new(),
        };
        // A lost camera is watched on every poll, also while paused, so its mark lands as near the loss as possible.
        alerts.extend(active.monitor.check_cam(stats.cam.is_some_and(|cam| cam.lost)));
        self.update_quietly(|state| {
            // Reading the counters resets the level peaks, so keep the ones the window has not seen yet.
            if state.stats_seq != state.ui_seen_seq
                && let Some(unseen) = &state.stats
            {
                for (new, old) in stats.tracks.iter_mut().zip(&unseen.tracks) {
                    new.peak_dbfs = new.peak_dbfs.max(old.peak_dbfs);
                    new.clipped |= old.clipped;
                }
            }
            state.stats = Some(stats);
            state.stats_seq += 1;
            state.mbps = mbps;
            if sample.is_some() {
                state.take_stats = sample;
            }
        });
        self.raise(alerts);
    }

    /// Acts on the warnings the monitor found: the System mark, the sound, a clean stop, the banner and the API event.
    fn raise(&mut self, alerts: Vec<Alert>) {
        for alert in alerts {
            log::warn!("warning {}: {}", alert.code, alert.message);
            if let Some((kind, label)) = &alert.mark {
                let qpc = capture::qpc_now();
                self.with_marks(|marks| marks.system(qpc, kind, label));
            }
            if alert.sound {
                stats::play_warning_sound();
            }
            if alert.stop {
                self.stop_take();
            }
            self.update(|state| {
                if alert.banner {
                    if state.banners.len() >= BANNER_LIMIT {
                        state.banners.remove(0);
                    }
                    state.banners.push(alert.message.clone());
                }
                state.warning_seq += 1;
                if state.warnings.len() >= WARNING_LOG_LIMIT {
                    state.warnings.pop_front();
                }
                let seq = state.warning_seq;
                state.warnings.push_back(WarningNote { seq, code: alert.code.to_owned(), message: alert.message.clone() });
            });
        }
    }

    /// While no take runs, reads the free space on the volume the next take goes to every couple of seconds,
    /// and the active profile's composite mode, since profile edits reach its file without a message.
    fn poll_idle_disk(&mut self) {
        if self.active.is_some() {
            return;
        }
        let now = Instant::now();
        if self.last_idle_check.is_some_and(|last| now.saturating_duration_since(last) < IDLE_DISK_INTERVAL) {
            return;
        }
        self.last_idle_check = Some(now);
        let free = stats::free_space(&crate::config::takes_root(&self.settings), 0);
        if lock(&self.shared).idle_free_bytes != free {
            self.update(|state| state.idle_free_bytes = free);
        }
        let profile = profiles::find(&self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        let composite_mode = profile.effective_composite_mode();
        if lock(&self.shared).composite_mode != composite_mode {
            self.update(|state| state.composite_mode = composite_mode);
        }
    }
}

/// A warning the monitor found, and what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Alert {
    /// The short machine-readable code the API event carries.
    code: &'static str,
    /// The text for the banner and the API event.
    message: String,
    /// Whether the message is also shown as a banner.
    banner: bool,
    /// Whether the warning sound plays.
    sound: bool,
    /// A System mark to add, as its `auto.kind` and its label.
    mark: Option<(&'static str, String)>,
    /// Whether the take is stopped cleanly.
    stop: bool,
}

/// Fires once when a condition starts and fires again only after the condition has cleared.
#[derive(Debug, Default)]
struct Rearm {
    fired: bool,
}

impl Rearm {
    /// Whether this is the first sample of a condition that is `active`; a clear sample rearms it.
    fn crossing(&mut self, active: bool) -> bool {
        if !active {
            self.fired = false;
            return false;
        }
        !std::mem::replace(&mut self.fired, true)
    }
}

/// The share of frames dropped across a rolling window.
#[derive(Debug)]
struct DroppedWindow {
    /// How far back the share is measured.
    window: Duration,
    /// The share above which the window counts as overloaded.
    ratio: f64,
    /// Time since the take started, frames written and frames dropped, oldest first.
    samples: VecDeque<(Duration, u64, u64)>,
}

impl DroppedWindow {
    /// A window of `window` that is over its limit above `ratio` of the frames.
    fn new(window: Duration, ratio: f64) -> Self {
        Self { window, ratio, samples: VecDeque::new() }
    }

    /// Adds the running totals seen `at` after the start and forgets samples older than the window.
    fn push(&mut self, at: Duration, frames: u64, dropped: u64) {
        self.samples.push_back((at, frames, dropped));
        // The oldest sample stays until the next one is old enough to take its place, so the span covers the whole window.
        while self.samples.get(1).is_some_and(|(next, _, _)| *next + self.window <= at) {
            self.samples.pop_front();
        }
    }

    /// Whether more than `ratio` of the frames in the last full window were dropped; false until a full window has passed.
    fn exceeded(&self) -> bool {
        let (Some(first), Some(last)) = (self.samples.front(), self.samples.back()) else {
            return false;
        };
        if last.0.saturating_sub(first.0) < self.window {
            return false;
        }
        let dropped = last.2.saturating_sub(first.2);
        let total = last.1.saturating_sub(first.1).saturating_add(dropped);
        total > 0 && dropped as f64 / total as f64 > self.ratio
    }
}

/// Watches a take's live numbers and says which warnings to raise, each once per crossing.
struct Monitor {
    /// Rearms when the disk is no longer low.
    low_disk: Rearm,
    /// Rearms when the dropped-frame share is back under the limit.
    overload: Rearm,
    /// The dropped-frame share across the last 10 seconds.
    dropped: DroppedWindow,
    /// Whether the disk-full stop was already asked for.
    stopped: bool,
    /// Whether each audio track's device was lost at the last check; empty until the first check.
    lost: Vec<bool>,
    /// Fires once when the webcam is lost.
    cam_lost: Rearm,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            low_disk: Rearm::default(),
            overload: Rearm::default(),
            dropped: DroppedWindow::new(OVERLOAD_WINDOW, OVERLOAD_RATIO),
            stopped: false,
            lost: Vec::new(),
            cam_lost: Rearm::default(),
        }
    }
}

impl Monitor {
    /// The warning to raise once when the webcam is lost: a System mark, a banner and the API event; the recording carries on.
    fn check_cam(&mut self, lost: bool) -> Vec<Alert> {
        if !self.cam_lost.crossing(lost) {
            return Vec::new();
        }
        vec![Alert {
            code: "device_lost",
            message: "The webcam was lost. Its camera file is finished, and the recording carries on without it.".to_owned(),
            banner: true,
            sound: false,
            mark: Some(("recorder.device_lost", "Webcam".to_owned())),
            stop: false,
        }]
    }

    /// The warnings to raise for the numbers `stats` taken `at` after the start; `tracks` holds each track's name and whether its device is lost.
    ///
    /// While `paused` only device loss is watched: nothing is written, so the disk and the encoder stay as they were.
    fn check(&mut self, at: Duration, stats: &TakeStats, tracks: &[(&str, bool)], paused: bool) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if self.lost.len() == tracks.len() {
            for ((name, lost), was_lost) in tracks.iter().zip(&mut self.lost) {
                if *lost && !*was_lost {
                    alerts.push(Alert {
                        code: "device_lost",
                        message: format!("The audio device of track {name} was lost. It records silence until the device returns."),
                        banner: true,
                        sound: false,
                        mark: Some(("recorder.device_lost", (*name).to_owned())),
                        stop: false,
                    });
                }
                *was_lost = *lost;
            }
        } else {
            // The first check only learns which devices are already lost.
            self.lost = tracks.iter().map(|(_, lost)| *lost).collect();
        }
        if paused {
            return alerts;
        }
        if stats.disk_free_bytes.is_some_and(|free| free < STOP_FREE_BYTES) {
            if !self.stopped {
                self.stopped = true;
                alerts.push(Alert { code: "disk_full", message: "Stopped: disk almost full".to_owned(), banner: true, sound: false, mark: None, stop: true });
            }
            return alerts;
        }
        let low = stats.disk_free_bytes.is_some_and(|free| free < LOW_FREE_BYTES) || stats.remaining_secs.is_some_and(|secs| secs < LOW_REMAINING_SECS);
        if self.low_disk.crossing(low) {
            let mut message = String::from("Low disk space");
            if let Some(free) = stats.disk_free_bytes {
                message.push_str(&format!(": {:.1} GB free", free as f64 / GIB as f64));
            }
            if let Some(secs) = stats.remaining_secs {
                message.push_str(&format!(", about {} min left at the current rate", secs.div_ceil(60)));
            }
            alerts.push(Alert { code: "disk_low", message, banner: true, sound: true, mark: None, stop: false });
        }
        self.dropped.push(at, stats.frames, stats.dropped);
        if self.overload.crossing(self.dropped.exceeded()) {
            alerts.push(Alert {
                code: "encoder_overload",
                message: "The encoder is overloaded: more than 0.5% of the frames were dropped in the last 10 seconds.".to_owned(),
                banner: true,
                sound: false,
                mark: Some(("recorder.encoder_overload", "Encoder overload".to_owned())),
                stop: false,
            });
        }
        alerts
    }
}

/// Takes every queued control message: the last settings replace `settings`, and a stop request or a dropped sender ends the run.
///
/// Returns whether the thread keeps running, and whether `settings` changed.
fn apply_control(control: &Receiver<Control>, settings: &mut Settings) -> (bool, bool) {
    let mut changed = false;
    loop {
        match control.try_recv() {
            Ok(Control::Settings(new)) => {
                *settings = *new;
                changed = true;
            }
            Ok(Control::Stop) | Err(TryRecvError::Disconnected) => return (false, changed),
            Err(TryRecvError::Empty) => return (true, changed),
        }
    }
}

/// Deletes a pre-flight's temp folder; one that stays behind is logged, never fatal.
fn remove_folder(folder: &Path) {
    if let Err(err) = std::fs::remove_dir_all(folder) {
        log::warn!("could not delete {}: {err}", folder.display());
    }
}

/// The camera file a sync test wrote: the one video file in its folder that is not the main file.
fn find_cam_file(folder: &Path, main_file: &Path) -> Option<PathBuf> {
    const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "mkv", "mov", "webm"];
    std::fs::read_dir(folder).ok()?.filter_map(Result::ok).map(|entry| entry.path()).find(|path| {
        path != main_file
            && path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
    })
}

/// Builds a spec for every enabled track in the settings, resolving each device, and names the tracks whose device is missing.
fn track_specs(settings: &Settings) -> (Vec<TrackSpec>, Vec<String>) {
    let mut specs = Vec::new();
    let mut missing = Vec::new();
    for track in settings.audio.tracks.iter().filter(|track| track.enabled) {
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

/// The audio codec the settings ask for, as the encoder takes it.
fn audio_codec(choice: AudioCodecChoice) -> media_io::AudioCodec {
    match choice {
        AudioCodecChoice::Aac320 => media_io::AudioCodec::Aac { kbps: 320 },
        AudioCodecChoice::Pcm24 => media_io::AudioCodec::Pcm24,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use chrono::{Local, TimeZone};

    use super::*;
    use crate::config::Container;

    fn secs(seconds: f64) -> Duration {
        Duration::from_secs_f64(seconds)
    }

    #[test]
    fn a_warning_fires_once_per_crossing_and_rearms_when_the_condition_clears() {
        let mut rearm = Rearm::default();
        assert!(!rearm.crossing(false));
        assert!(rearm.crossing(true));
        assert!(!rearm.crossing(true));
        assert!(!rearm.crossing(true));
        assert!(!rearm.crossing(false));
        assert!(rearm.crossing(true));
    }

    /// Fills a window with 60 fps samples every half second up to `until` seconds, and `dropped` frames from the sample at `drop_at`.
    fn feed(window: &mut DroppedWindow, until: u32, drop_at: u32, dropped: u64) {
        for step in 0..=until * 2 {
            let lost = if step >= drop_at * 2 { dropped } else { 0 };
            window.push(secs(f64::from(step) * 0.5), u64::from(step) * 30, lost);
        }
    }

    #[test]
    fn the_dropped_share_needs_a_full_window_and_more_than_half_a_percent() {
        // 3 of 603 frames is under 0.5%; 4 of 604 is over.
        let mut few = DroppedWindow::new(OVERLOAD_WINDOW, OVERLOAD_RATIO);
        feed(&mut few, 15, 10, 3);
        assert!(!few.exceeded());
        let mut many = DroppedWindow::new(OVERLOAD_WINDOW, OVERLOAD_RATIO);
        feed(&mut many, 15, 10, 4);
        assert!(many.exceeded());
        // Before 10 seconds of history there is no verdict, however many frames were dropped.
        let mut early = DroppedWindow::new(OVERLOAD_WINDOW, OVERLOAD_RATIO);
        feed(&mut early, 5, 1, 50);
        assert!(!early.exceeded());
    }

    #[test]
    fn the_dropped_share_forgets_drops_older_than_the_window() {
        let mut window = DroppedWindow::new(OVERLOAD_WINDOW, OVERLOAD_RATIO);
        feed(&mut window, 15, 10, 4);
        assert!(window.exceeded());
        // No further drops: 10 seconds on, the old ones are out of the window.
        for step in 31u32..=44 {
            window.push(secs(f64::from(step) * 0.5), u64::from(step) * 30, 4);
        }
        assert!(!window.exceeded());
    }

    #[test]
    fn the_monitor_warns_once_for_a_low_disk_and_again_after_it_recovers() {
        let mut monitor = Monitor::default();
        let low = TakeStats { disk_free_bytes: Some(8 * GIB), remaining_secs: Some(3600), ..TakeStats::default() };
        let fine = TakeStats { disk_free_bytes: Some(50 * GIB), remaining_secs: Some(36_000), ..TakeStats::default() };
        let codes = |alerts: Vec<Alert>| alerts.iter().map(|alert| alert.code).collect::<Vec<_>>();
        assert_eq!(codes(monitor.check(secs(0.5), &low, &[], false)), ["disk_low"]);
        assert!(monitor.check(secs(1.0), &low, &[], false).is_empty());
        assert!(monitor.check(secs(1.5), &fine, &[], false).is_empty());
        assert_eq!(codes(monitor.check(secs(2.0), &low, &[], false)), ["disk_low"]);
    }

    #[test]
    fn the_monitor_stops_the_take_once_under_2_gb_and_marks_a_lost_device_once() {
        let mut monitor = Monitor::default();
        let full = TakeStats { disk_free_bytes: Some(GIB), ..TakeStats::default() };
        let stops = monitor.check(secs(0.5), &full, &[], false);
        assert_eq!(stops.len(), 1);
        assert!(stops[0].stop && stops[0].banner && stops[0].message == "Stopped: disk almost full");
        assert!(monitor.check(secs(1.0), &full, &[], false).is_empty());

        let mut monitor = Monitor::default();
        let fine = TakeStats::default();
        assert!(monitor.check(secs(0.5), &fine, &[("Mic", false)], false).is_empty());
        let lost = monitor.check(secs(1.0), &fine, &[("Mic", true)], false);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].mark, Some(("recorder.device_lost", "Mic".to_owned())));
        assert!(lost[0].banner && lost[0].message.contains("track Mic"));
        assert!(monitor.check(secs(1.5), &fine, &[("Mic", true)], true).is_empty());
    }

    #[test]
    fn a_lost_webcam_is_marked_and_bannered_once_and_the_take_carries_on() {
        let mut monitor = Monitor::default();
        assert!(monitor.check_cam(false).is_empty());
        let lost = monitor.check_cam(true);
        assert_eq!(lost.len(), 1);
        assert_eq!(lost[0].code, "device_lost");
        assert_eq!(lost[0].mark, Some(("recorder.device_lost", "Webcam".to_owned())));
        assert!(lost[0].banner && !lost[0].stop);
        assert!(monitor.check_cam(true).is_empty());
    }

    #[test]
    fn a_countdown_is_only_for_a_length_above_zero() {
        assert_eq!(countdown_length(0), None);
        assert_eq!(countdown_length(3), Some(3));
        assert_eq!(countdown_length(5), Some(5));
    }

    #[test]
    fn a_countdown_ticks_once_a_second_and_ends_at_zero() {
        let start = Instant::now();
        let mut run = CountdownRun::new(3, start, true);
        assert_eq!(run.remaining, 3);
        assert_eq!(run.tick(start), None);
        assert_eq!(run.tick(start + Duration::from_millis(999)), None);
        assert_eq!(run.tick(start + Duration::from_secs(1)), Some(Tick::Running(2)));
        // The same second is not counted twice.
        assert_eq!(run.tick(start + Duration::from_millis(1500)), None);
        assert_eq!(run.tick(start + Duration::from_secs(2)), Some(Tick::Running(1)));
        assert_eq!(run.tick(start + Duration::from_secs(3)), Some(Tick::Done));
    }

    #[test]
    fn esc_cancels_a_countdown_and_does_nothing_without_one() {
        let escape = Some(ApiIntent::Escape);
        assert_eq!(countdown_verdict(true, &CommandKind::StartStop, escape.as_ref()), Verdict::Cancel);
        // Esc pressed just as the take began must not stop that take.
        assert_eq!(countdown_verdict(false, &CommandKind::StartStop, escape.as_ref()), Verdict::Ignore);
    }

    #[test]
    fn a_second_start_or_stop_cancels_a_countdown() {
        assert_eq!(countdown_verdict(true, &CommandKind::StartStop, None), Verdict::Cancel);
        assert_eq!(countdown_verdict(true, &CommandKind::StartStop, Some(&ApiIntent::Stop)), Verdict::Cancel);
        assert_eq!(countdown_verdict(true, &CommandKind::StartStop, Some(&ApiIntent::Start)), Verdict::Ignore);
        // With no countdown a start/stop is handled as usual.
        assert_eq!(countdown_verdict(false, &CommandKind::StartStop, None), Verdict::Pass);
    }

    #[test]
    fn pause_marks_and_profile_changes_are_ignored_during_a_countdown() {
        let ignored = [
            CommandKind::Pause,
            CommandKind::Resume,
            CommandKind::PauseResume,
            CommandKind::Mark(MarkKind::Silent),
            CommandKind::LabeledMark { category: ::marks::MarkType::Note, label: None },
            CommandKind::SetProfile("Any".to_owned()),
            CommandKind::Preflight { keep_files: false },
            CommandKind::SyncTest,
        ];
        for kind in &ignored {
            assert_eq!(countdown_verdict(true, kind, None), Verdict::Ignore, "{kind:?}");
            assert_eq!(countdown_verdict(false, kind, None), Verdict::Pass, "{kind:?}");
        }
        assert_eq!(countdown_verdict(true, &CommandKind::GetStatus, None), Verdict::Pass);
        assert_eq!(countdown_verdict(true, &CommandKind::Mute { track: 0 }, None), Verdict::Pass);
    }

    #[test]
    fn a_settings_change_before_start_changes_the_take_folder() -> Result<(), Box<dyn Error>> {
        let root = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-controller-settings", std::process::id()));
        let now = Local.with_ymd_and_hms(2026, 11, 19, 10, 0, 0).single().ok_or("the test date does not exist")?;
        let mut settings = Settings { root_folder: Some(root.clone()), ..Settings::default() };
        let (tx, rx) = mpsc::channel();
        let sender = ControlSender { tx };

        let before = takes::plan_take_for(&settings, now, Container::Mkv)?;
        assert_eq!(before.folder, root.join("Cutmark").join("2026-11-19_10-00-00_take01"));

        let mut changed = settings.clone();
        changed.project = "Test4a".to_owned();
        changed.naming_template = "{project}_{take}".to_owned();
        sender.set_settings(changed);
        let (keep_running, was_changed) = apply_control(&rx, &mut settings);
        assert!(keep_running && was_changed);

        let after = takes::plan_take_for(&settings, now, Container::Mkv)?;
        assert_eq!(after.folder, root.join("Test4a").join("Test4a_01"));
        assert_ne!(before.folder, after.folder);

        sender.stop();
        assert_eq!(apply_control(&rx, &mut settings), (false, false));
        Ok(())
    }
}
