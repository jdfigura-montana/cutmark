//! The Recorder's side of the Stream Deck remote control (spec:
//! docs/suite/recorder-stream-deck.md). Owns the cutmark-remote server, turns each command id
//! into the bus command the matching hotkey or button sends, guards it the way that button
//! would, and builds the published state. It is free of egui: app.rs polls it in `logic`,
//! which also runs while the window is hidden in the tray.

use std::path::Path;

use cutmark_remote::{CommandInfo, RemoteCommand, RemoteConfig, RemoteServer};
use serde_json::{Value, json};

use crate::config::{self, Settings};
use crate::engine::bus::{BusSender, CommandKind, MarkKind};
use crate::engine::controller::{ControllerState, Phase};
use crate::tracks_ui::{GAIN_MAX_DB, GAIN_MIN_DB};

/// The app's short name in the protocol and the discovery file's name.
const APP: &str = "recorder";

/// The commands the Recorder accepts (spec "Commands") with their plugin labels.
const COMMANDS: &[(&str, &str)] = &[
    ("start_stop", "Start or Stop Recording"),
    ("pause_resume", "Pause or Resume"),
    ("mark", "Mark"),
    ("mark_label", "Mark with Label"),
    ("highlight_last", "Highlight Last 30 Seconds"),
    ("mute_mic", "Mute Mic"),
    ("replay_toggle", "Replay Buffer"),
    ("replay_save", "Save Replay"),
    ("track_mute", "Track Mute"),
    ("track_gain", "Track Gain"),
];

/// The peak a track publishes while it has no reading, and the lowest it publishes.
const PEAK_FLOOR_DBFS: f32 = -60.0;

/// The running remote control server. The app holds an `Option<Remote>`: `None` runs it without.
pub struct Remote {
    /// The server: socket, discovery files and the command queue.
    server: RemoteServer,
}

impl Remote {
    /// Starts the server with this exe and the command list. `wake` runs whenever a command
    /// arrives. When another Recorder already serves the remote, or the server cannot start,
    /// logs why and returns `None`.
    pub fn start(wake: Box<dyn Fn() + Send>) -> Option<Self> {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => {
                log::warn!("remote control not started: the exe path could not be read ({error})");
                return None;
            }
        };
        let commands = COMMANDS.iter().map(|(id, label)| CommandInfo { id: (*id).to_owned(), label: (*label).to_owned() }).collect();
        let config = RemoteConfig { app: APP, version: env!("CARGO_PKG_VERSION").to_owned(), exe, commands, dir: None };
        match RemoteServer::start(config) {
            Ok(server) => {
                server.set_wake(wake);
                Some(Self { server })
            }
            Err(error) => {
                log::info!("remote control not started: {error}");
                None
            }
        }
    }

    /// The next command that arrived, or `None` when the queue is empty.
    pub fn try_recv(&self) -> Option<RemoteCommand> {
        self.server.try_recv()
    }

    /// Tells the client why the command `id` did not run.
    pub fn reply(&self, id: &str, refusal: &Refusal) {
        self.server.reply_error(id, refusal.code, &refusal.message);
    }

    /// Replaces the published state.
    pub fn publish(&self, state: Value) {
        self.server.publish(state);
    }
}

/// The Settings status line for the switch and the server (spec "Recorder app" 1).
pub fn status_text(remote: Option<&Remote>) -> &'static str {
    match remote {
        Some(remote) if remote.server.is_connected() => "Stream Deck connected",
        Some(_) => "Waiting for Stream Deck",
        None => "Off",
    }
}

/// What the guards look at: the same things the Recorder's buttons go by.
pub struct Guards<'a> {
    /// The controller's phase.
    pub phase: Phase,
    /// Whether the replay buffer is on.
    pub replay_on: bool,
    /// Whether the profile's errors block a start.
    pub profile_blocked: bool,
    /// The settings on screen: the folders and the tracks.
    pub settings: &'a Settings,
}

/// Why a command did not run: the protocol's error code and the reason for the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// `blocked`, `bad_args` or `unknown_command`.
    pub code: &'static str,
    /// The reason, for the client.
    pub message: String,
}

/// A track gain the app must apply the way the track's slider does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GainChange {
    /// Index of the track in the settings.
    pub track: usize,
    /// The new gain, already inside the slider's range.
    pub gain_db: f32,
}

/// What an accepted command asks for.
#[derive(Debug, Clone, PartialEq)]
enum Action {
    /// A bus command, the same one the hotkey or button sends.
    Send(CommandKind),
    /// A gain change; the bus has no command for it, the slider edits the settings.
    Gain(GainChange),
}

/// Runs one command: sends its bus command through `bus`, or returns the gain change for the
/// app to apply.
pub fn run(command: &RemoteCommand, guards: &Guards<'_>, bus: &BusSender) -> Result<Option<GainChange>, Refusal> {
    match plan(command, guards)? {
        Action::Send(kind) => {
            crate::app::send(bus, kind);
            Ok(None)
        }
        Action::Gain(change) => Ok(Some(change)),
    }
}

/// Maps a command id and its args to what it asks for, or why it is refused.
fn plan(command: &RemoteCommand, guards: &Guards<'_>) -> Result<Action, Refusal> {
    let kind = match command.id.as_str() {
        "start_stop" => {
            if guards.phase == Phase::Idle {
                if guards.profile_blocked {
                    return Err(blocked("Starting is blocked until the profile errors are fixed"));
                }
                check_folder(&config::takes_root(guards.settings))?;
            }
            CommandKind::StartStop
        }
        "pause_resume" => {
            if guards.phase == Phase::Idle {
                return Err(blocked("No take is running"));
            }
            CommandKind::PauseResume
        }
        "mark" => mark_kind(&command.args)?,
        "mark_label" => CommandKind::Mark(MarkKind::WithLabel),
        "highlight_last" => CommandKind::Mark(MarkKind::HighlightLast),
        "mute_mic" => CommandKind::Mute { track: 0 },
        "replay_toggle" => {
            if !guards.replay_on {
                check_folder(&crate::takes::replays_folder(guards.settings))?;
            }
            CommandKind::ReplayToggle
        }
        "replay_save" => {
            if !guards.replay_on {
                return Err(blocked("The replay buffer is off"));
            }
            CommandKind::ReplaySave
        }
        "track_mute" => CommandKind::Mute { track: track_index(guards.settings, &command.args)? },
        "track_gain" => {
            let track = track_index(guards.settings, &command.args)?;
            let delta = command.args.get("delta_db").and_then(Value::as_f64).ok_or_else(|| bad_args("The delta_db argument is missing or not a number"))?;
            let current = guards.settings.audio.tracks.get(track).map_or(0.0, |settings| settings.gain_db);
            let gain_db = (current + delta as f32).clamp(GAIN_MIN_DB, GAIN_MAX_DB);
            return Ok(Action::Gain(GainChange { track, gain_db }));
        }
        other => return Err(Refusal { code: "unknown_command", message: format!("The Recorder does not accept the command {other}") }),
    };
    Ok(Action::Send(kind))
}

/// The bus command of a silent mark in the `category` argument, note when there is none. A note
/// is the mark hotkey's own command, so it also adds the Review mark the hotkey adds.
fn mark_kind(args: &Value) -> Result<CommandKind, Refusal> {
    let category = match args.get("category") {
        None | Some(Value::Null) => "note",
        Some(value) => value.as_str().ok_or_else(|| bad_args("The category argument is not a text"))?,
    };
    let category = match category {
        "note" => return Ok(CommandKind::Mark(MarkKind::Silent)),
        "retake" => ::marks::MarkType::Retake,
        "cut" => ::marks::MarkType::Cut,
        "broll" => ::marks::MarkType::BRoll,
        "chapter" => ::marks::MarkType::Chapter,
        other => return Err(bad_args(&format!("There is no mark category {other}"))),
    };
    Ok(CommandKind::LabeledMark { category, label: None })
}

/// The settings index of the track a 1-based `track` argument names.
fn track_index(settings: &Settings, args: &Value) -> Result<usize, Refusal> {
    let number = args.get("track").and_then(Value::as_u64).ok_or_else(|| bad_args("The track argument is missing or not a whole number"))?;
    usize::try_from(number)
        .ok()
        .and_then(|number| number.checked_sub(1))
        .filter(|index| *index < settings.audio.tracks.len())
        .ok_or_else(|| bad_args(&format!("There is no track {number}")))
}

/// Refuses with the controller's own text when `folder` cannot be recorded into.
fn check_folder(folder: &Path) -> Result<(), Refusal> {
    crate::takes::check_folder(folder).map_err(|error| blocked(&format!("Can't save to {}: {error}", folder.display())))
}

fn blocked(why: &str) -> Refusal {
    Refusal { code: "blocked", message: why.to_owned() }
}

fn bad_args(why: &str) -> Refusal {
    Refusal { code: "bad_args", message: why.to_owned() }
}

/// The state object the Recorder publishes (spec "State"). Tracks are every settings track, so
/// their numbers are the ones `track_mute` and `track_gain` take.
pub fn state_value(state: &ControllerState, settings: &Settings) -> Value {
    let has_take = state.phase.has_take();
    let phase = match state.phase {
        Phase::Idle => "idle",
        Phase::Countdown(_) => "countdown",
        Phase::Recording => "recording",
        Phase::Paused => "paused",
    };
    // The deck's timecode: the take's frames, so paused time does not count.
    let elapsed_ms = match &state.stats {
        Some(stats) if has_take && state.fps > 0 => stats.frames.saturating_mul(1000) / u64::from(state.fps),
        _ => 0,
    };
    let tracks: Vec<Value> = settings
        .audio
        .tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            json!({
                "index": index + 1,
                "name": track.name,
                "muted": track.muted,
                "gain_db": track.gain_db,
                "peak_dbfs": track_peak(state, settings, index).max(PEAK_FLOOR_DBFS),
            })
        })
        .collect();
    // The seconds left, only while the countdown runs.
    let countdown_s = match state.phase {
        Phase::Countdown(seconds) => Some(seconds),
        _ => None,
    };
    json!({
        "phase": phase,
        "countdown_s": countdown_s,
        "elapsed_ms": elapsed_ms,
        "replay_on": state.replay_on,
        "mic_muted": settings.audio.tracks.first().is_some_and(|track| track.muted),
        "marks": if has_take { state.marks_count } else { 0 },
        "tracks": tracks,
    })
}

/// The newest peak of the settings track at `index`: the take's reading while a take runs, else
/// the idle watch's (app and desktop-except tracks only), else the floor.
fn track_peak(state: &ControllerState, settings: &Settings, index: usize) -> f32 {
    let tracks = &settings.audio.tracks;
    let Some(track) = tracks.get(index).filter(|track| track.enabled) else { return PEAK_FLOOR_DBFS };
    if state.phase.has_take() {
        // A take lists the enabled tracks only; tracks that share a name keep their order.
        let earlier = tracks.iter().take(index).filter(|other| other.enabled && other.name == track.name).count();
        let reading = state.stats.as_ref().and_then(|stats| stats.tracks.iter().filter(|snapshot| snapshot.name == track.name).nth(earlier));
        reading.map_or(PEAK_FLOOR_DBFS, |snapshot| snapshot.peak_dbfs)
    } else {
        state.idle_apps.iter().find(|idle| idle.index == index).map_or(PEAK_FLOOR_DBFS, |idle| idle.peak_dbfs)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::TrackSettings;
    use crate::engine::bus::Bus;

    /// Settings with two tracks whose folders can be written.
    fn settings() -> Settings {
        let mut settings =
            Settings { root_folder: Some(std::env::temp_dir().join(format!("cutmark-remote-test-{}", std::process::id()))), ..Settings::default() };
        settings.audio.tracks = vec![
            TrackSettings { name: "Mic".to_owned(), ..TrackSettings::default() },
            TrackSettings { name: "Desktop".to_owned(), muted: true, gain_db: -6.0, ..TrackSettings::default() },
        ];
        settings
    }

    fn guards(settings: &Settings, phase: Phase, replay_on: bool) -> Guards<'_> {
        Guards { phase, replay_on, profile_blocked: false, settings }
    }

    fn command(id: &str, args: Value) -> RemoteCommand {
        RemoteCommand { id: id.to_owned(), args }
    }

    /// Every bus command id reaches its command on a real bus.
    #[test]
    fn each_command_id_reaches_its_bus_command() {
        let settings = settings();
        let guards = guards(&settings, Phase::Recording, true);
        let labeled = |category| CommandKind::LabeledMark { category, label: None };
        let cases = [
            ("start_stop", json!({}), CommandKind::StartStop),
            ("pause_resume", json!({}), CommandKind::PauseResume),
            ("mark", json!({}), CommandKind::Mark(MarkKind::Silent)),
            ("mark", json!({ "category": "note" }), CommandKind::Mark(MarkKind::Silent)),
            ("mark", json!({ "category": "retake" }), labeled(::marks::MarkType::Retake)),
            ("mark", json!({ "category": "cut" }), labeled(::marks::MarkType::Cut)),
            ("mark", json!({ "category": "broll" }), labeled(::marks::MarkType::BRoll)),
            ("mark", json!({ "category": "chapter" }), labeled(::marks::MarkType::Chapter)),
            ("mark_label", json!({}), CommandKind::Mark(MarkKind::WithLabel)),
            ("highlight_last", json!({}), CommandKind::Mark(MarkKind::HighlightLast)),
            ("mute_mic", json!({}), CommandKind::Mute { track: 0 }),
            ("replay_toggle", json!({}), CommandKind::ReplayToggle),
            ("replay_save", json!({}), CommandKind::ReplaySave),
            ("track_mute", json!({ "track": 2 }), CommandKind::Mute { track: 1 }),
        ];
        let bus = Bus::default();
        for (id, args, expected) in cases {
            assert_eq!(run(&command(id, args), &guards, &bus.sender()), Ok(None), "{id} is accepted");
            let sent = bus.recv_timeout(Duration::from_millis(5)).map(|sent| sent.kind);
            assert_eq!(sent, Some(expected), "{id} sends its command");
        }
        // Every listed id has an action behind it.
        for (id, _) in COMMANDS {
            let planned = plan(&command(id, json!({ "track": 1, "delta_db": 1.0 })), &guards);
            assert!(planned.is_ok(), "{id} is mapped");
        }
    }

    #[test]
    fn track_gain_adds_the_delta_and_clamps_to_the_slider_range() {
        let settings = settings();
        let guards = guards(&settings, Phase::Idle, false);
        for (delta, expected) in [(1.5, -4.5), (100.0, GAIN_MAX_DB), (-100.0, GAIN_MIN_DB)] {
            let planned = plan(&command("track_gain", json!({ "track": 2, "delta_db": delta })), &guards);
            assert_eq!(planned, Ok(Action::Gain(GainChange { track: 1, gain_db: expected })));
        }
    }

    #[test]
    fn refused_commands_send_nothing() {
        let settings = settings();
        let idle = guards(&settings, Phase::Idle, false);
        let bus = Bus::default();
        let code = |command: RemoteCommand, guards: &Guards<'_>| run(&command, guards, &bus.sender()).err().map(|refusal| refusal.code);
        assert_eq!(code(command("replay_save", json!({})), &idle), Some("blocked"), "the buffer is off");
        assert_eq!(code(command("pause_resume", json!({})), &idle), Some("blocked"), "no take runs");
        let profile_errors = Guards { profile_blocked: true, ..guards(&settings, Phase::Idle, false) };
        assert_eq!(code(command("start_stop", json!({})), &profile_errors), Some("blocked"));
        for args in [json!({}), json!({ "track": 0 }), json!({ "track": 3 }), json!({ "track": "1" }), json!({ "track": 1.5 })] {
            assert_eq!(code(command("track_mute", args), &idle), Some("bad_args"));
        }
        assert_eq!(code(command("track_gain", json!({ "track": 3, "delta_db": 1.0 })), &idle), Some("bad_args"));
        assert_eq!(code(command("track_gain", json!({ "track": 1 })), &idle), Some("bad_args"));
        assert_eq!(code(command("mark", json!({ "category": "system" })), &idle), Some("bad_args"));
        assert_eq!(code(command("mark", json!({ "category": 3 })), &idle), Some("bad_args"));
        assert_eq!(code(command("record_harder", json!({})), &idle), Some("unknown_command"));
        assert_eq!(bus.recv_timeout(Duration::from_millis(5)), None);
    }

    /// A folder that cannot be created refuses a start and a replay turn-on, never a stop.
    #[test]
    fn a_bad_folder_blocks_turning_on_only() -> Result<(), std::io::Error> {
        let file = std::env::temp_dir().join(format!("cutmark-remote-test-file-{}", std::process::id()));
        std::fs::write(&file, b"")?;
        let settings = Settings { root_folder: Some(file.join("inside-a-file")), ..settings() };
        for (id, phase, replay_on) in [("start_stop", Phase::Idle, false), ("replay_toggle", Phase::Idle, false)] {
            let refusal = plan(&command(id, json!({})), &guards(&settings, phase, replay_on)).err();
            assert!(refusal.is_some_and(|refusal| refusal.code == "blocked" && refusal.message.starts_with("Can't save to ")), "{id}");
        }
        assert_eq!(plan(&command("start_stop", json!({})), &guards(&settings, Phase::Recording, false)), Ok(Action::Send(CommandKind::StartStop)));
        assert_eq!(plan(&command("replay_toggle", json!({})), &guards(&settings, Phase::Idle, true)), Ok(Action::Send(CommandKind::ReplayToggle)));
        std::fs::remove_file(&file)
    }

    #[test]
    fn the_state_object_has_the_spec_shape() {
        let settings = settings();
        let state = ControllerState { replay_on: true, marks_count: 4, ..ControllerState::default() };
        assert_eq!(
            state_value(&state, &settings),
            json!({
                "phase": "idle",
                "countdown_s": null,
                "elapsed_ms": 0,
                "replay_on": true,
                "mic_muted": false,
                "marks": 0,
                "tracks": [
                    { "index": 1, "name": "Mic", "muted": false, "gain_db": 0.0, "peak_dbfs": -60.0 },
                    { "index": 2, "name": "Desktop", "muted": true, "gain_db": -6.0, "peak_dbfs": -60.0 },
                ],
            })
        );
        let paused = ControllerState { phase: Phase::Paused, marks_count: 4, ..ControllerState::default() };
        let value = state_value(&paused, &settings);
        assert_eq!((&value["phase"], &value["marks"]), (&json!("paused"), &json!(4)));
        let countdown = ControllerState { phase: Phase::Countdown(3), ..ControllerState::default() };
        let value = state_value(&countdown, &settings);
        assert_eq!((&value["phase"], &value["countdown_s"]), (&json!("countdown"), &json!(3)));
    }
}
