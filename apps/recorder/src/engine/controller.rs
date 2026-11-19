//! The engine controller: one thread that handles every command and owns the recording session.
//!
//! The window only draws [`ControllerState`] and sends commands, so hotkeys and tray clicks work
//! while the window is hidden and no frames run.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use thiserror::Error;

use super::bus::{Bus, BusSender, Command, CommandKind, MarkKind};
use super::marks::{MarksWarning, RecentMark, TakeMarks};
use super::session::{RecordingSession, SessionConfig, StatsSnapshot, Summary, TrackSpec};
use crate::config::{AudioCodecChoice, Profile, Settings, TrackKind};
use crate::label_popup::PopupHandle;
use crate::profiles;
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

/// What the Recorder is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// No take is running.
    #[default]
    Idle,
    /// A take is running.
    Recording,
    /// A take is running but paused.
    Paused,
}

/// Everything the window draws, published by the controller thread.
#[derive(Default)]
pub struct ControllerState {
    /// Idle, recording or paused.
    pub phase: Phase,
    /// The paths of the running take.
    pub take: Option<TakePaths>,
    /// Frames per second of the running take, for the elapsed clock.
    pub fps: u32,
    /// The summary of the last finished take, kept until the next one starts.
    pub summary: Option<Summary>,
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
    /// Whether each track in the settings is muted, in settings order.
    pub track_muted: Vec<bool>,
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
    handle: Option<JoinHandle<()>>,
}

impl Controller {
    /// Starts the "controller" thread with a settings snapshot, a callback that asks the window to repaint and a
    /// listener that is called with the state after every change, so the tray can follow while no frames run.
    /// The label popup sends its saved marks to this controller's bus.
    pub fn start(settings: Settings, repaint: Box<dyn Fn() + Send>, listener: Box<dyn Fn(&ControllerState) + Send>, popup: PopupHandle) -> Self {
        let bus = Bus::default();
        let bus_sender = bus.sender();
        popup.connect(bus.sender());
        let (control_tx, control_rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(ControllerState {
            fps: 60,
            active_profile: settings.active_profile.clone(),
            track_muted: settings.audio.tracks.iter().map(|track| track.muted).collect(),
            ..ControllerState::default()
        }));
        let worker =
            Worker { bus, control: control_rx, shared: Arc::clone(&shared), repaint, listener, settings, active: None, popup, deferred: VecDeque::new() };
        let handle = match std::thread::Builder::new().name("controller".to_owned()).spawn(move || worker.run()) {
            Ok(handle) => Some(handle),
            Err(err) => {
                log::error!("the controller thread could not be started: {err}");
                lock(&shared).error = Some(format!("the recording engine could not start: {err}"));
                None
            }
        };
        Self { bus: bus_sender, control: ControlSender { tx: control_tx }, shared, handle }
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
    paused: bool,
    /// The time and byte count of the last bitrate sample.
    last_poll: Option<(Instant, u64)>,
    /// The bitrate of the last sample, in megabits per second.
    last_mbps: f64,
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
    shared: Arc<Mutex<ControllerState>>,
    repaint: Box<dyn Fn() + Send>,
    listener: Box<dyn Fn(&ControllerState) + Send>,
    settings: Settings,
    active: Option<ActiveTake>,
    popup: PopupHandle,
    /// Commands that were taken off the bus while a stop collected the popup's mark; handled before new ones.
    deferred: VecDeque<Command>,
}

impl Worker {
    /// Handles settings, commands and stats samples until told to stop, then finishes any running take.
    fn run(mut self) {
        while self.drain_control() {
            let wait = if self.active.is_some() { ACTIVE_POLL } else { IDLE_POLL };
            if let Some(command) = self.deferred.pop_front().or_else(|| self.bus.recv_timeout(wait)) {
                self.handle(command);
            }
            self.poll_stats();
        }
        self.stop_take();
    }

    /// Applies the queued settings; false once the stop request came or the app dropped its handles.
    fn drain_control(&mut self) -> bool {
        let (keep_running, changed) = apply_control(&self.control, &mut self.settings);
        if changed {
            self.forward_live_controls();
        }
        keep_running
    }

    /// Changes the shared state, tells the listener about it and asks the window to repaint.
    fn update(&self, change: impl FnOnce(&mut ControllerState)) {
        {
            let mut state = lock(&self.shared);
            change(&mut state);
            (self.listener)(&state);
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
        let press = command.press_qpc;
        match &command.kind {
            CommandKind::StartStop => {
                if self.active.is_some() {
                    self.stop_take();
                } else {
                    self.start_take();
                }
            }
            CommandKind::Pause => self.pause(press),
            CommandKind::Resume => self.resume(press),
            CommandKind::PauseResume => {
                if self.active.as_ref().is_some_and(|active| active.paused) {
                    self.resume(press);
                } else {
                    self.pause(press);
                }
            }
            CommandKind::Mute { .. } => self.toggle_mic_mute(),
            CommandKind::SetProfile(name) => self.set_profile(name),
            CommandKind::Mark(kind) => self.add_mark(*kind, press),
            CommandKind::LabeledMark { category, label } => self.add_labeled(*category, label.clone(), press),
            CommandKind::GetStatus => log::info!("status requested (the control API answers it in a later phase)"),
        }
        self.update(|state| {
            if state.recent.len() == RECENT_LIMIT {
                state.recent.pop_front();
            }
            state.recent.push_back(command);
        });
    }

    /// Starts a new take; on failure shows a red banner and stays idle.
    fn start_take(&mut self) {
        self.update(|state| {
            state.error = None;
            state.audio_notice = None;
            state.summary = None;
            state.marks_count = 0;
            state.recent_marks.clear();
            state.marks_warning = None;
        });
        if let Err(err) = self.start_take_inner() {
            log::error!("could not start the recording: {err}");
            self.update(|state| state.error = Some(err.to_string()));
        }
    }

    /// Picks the take folder, builds the session config from the active profile and starts the session.
    fn start_take_inner(&mut self) -> Result<(), StartError> {
        let profile = profiles::find(&self.settings.active_profile).unwrap_or_else(Profile::tutorial_av1_1440p60);
        let take = takes::next_take_for(&self.settings, chrono::Local::now(), profile.container).map_err(|err| StartError::Take(err.to_string()))?;
        let fps = profile.fps;
        let (tracks, missing) = track_specs(&self.settings);
        if !missing.is_empty() {
            log::warn!("recording without these audio devices: {}", missing.join(", "));
            let notice = format!("Audio device not found for {}. Recording anyway; those tracks stay silent until the device returns.", missing.join(", "));
            self.update(|state| state.audio_notice = Some(notice));
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
        let session =
            RecordingSession::start(cfg).map_err(|err| StartError::Session { file: take.video_file.display().to_string(), reason: err.to_string() })?;
        let marks_warning = session.marks_warning();
        self.active = Some(ActiveTake { session, marks_warning, paused: false, last_poll: None, last_mbps: 0.0 });
        self.update(|state| {
            state.phase = Phase::Recording;
            state.take = Some(take);
            state.fps = fps;
            state.stats = None;
            state.mbps = 0.0;
        });
        // A sidecar that could not be started leaves its reason for the banner.
        self.with_marks(|_| {});
        Ok(())
    }

    /// Stops the running take, keeps its summary for the window and returns to idle.
    fn stop_take(&mut self) {
        if self.active.is_none() {
            return;
        }
        self.commit_popup();
        let Some(active) = self.active.take() else { return };
        let marks_warning = active.marks_warning.clone();
        self.update(|state| {
            state.phase = Phase::Idle;
            state.take = None;
            state.stats = None;
            state.mbps = 0.0;
        });
        let stopped = active.session.stop();
        // Finishing the marks happens inside stop and may leave a warning.
        if let Some(text) = marks_warning.take() {
            self.update(|state| state.marks_warning = Some(text));
        }
        match stopped {
            Ok(summary) => {
                log::info!("take finished: {} frames, {} bytes", summary.frames, summary.bytes_written);
                self.update(|state| state.summary = Some(summary));
            }
            Err(err) => {
                log::error!("the recording failed on stop: {err}");
                self.update(|state| state.error = Some(format!("the recording failed on stop: {err}")));
            }
        }
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
    fn add_labeled(&mut self, category: ::marks::MarkType, label: Option<String>, press_qpc: i64) {
        if self.active.is_none() {
            log::info!("labeled mark ignored: no take is running");
            return;
        }
        self.with_marks(|marks| marks.labeled(press_qpc, category, label));
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

    /// Toggles mute on the track named Mic, or else on the first Input track, in the settings and in the running take.
    fn toggle_mic_mute(&mut self) {
        let tracks = &mut self.settings.audio.tracks;
        let position = tracks
            .iter()
            .position(|track| track.name.eq_ignore_ascii_case(MIC_TRACK_NAME))
            .or_else(|| tracks.iter().position(|track| track.kind == TrackKind::Input));
        let Some(track) = position.and_then(|index| tracks.get_mut(index)) else {
            log::warn!("mute hotkey: there is no Mic or Input track to mute");
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
        self.publish_settings();
    }

    /// Shows the window the changes the controller made to the settings.
    fn publish_settings(&self) {
        let profile = self.settings.active_profile.clone();
        let muted: Vec<bool> = self.settings.audio.tracks.iter().map(|track| track.muted).collect();
        self.update(|state| {
            state.active_profile = profile;
            state.track_muted = muted;
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
        });
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
