//! remote: the app side of the Stream Deck remote control (spec:
//! docs/audio-editor/1.3-StreamDeck.md, "App side"). Owns the cutmark-remote
//! server and its command queue: every shortcut-table id runs through the same
//! dispatch as the keyboard (`App::run_shortcut_action`, handed in by app.rs so
//! this module stays free of egui), the remote-only ids run here. app.rs only
//! calls `start` and `poll`.

use std::time::{Duration, Instant};

use cutmark_remote::{CommandInfo, RemoteCommand, RemoteConfig, RemoteServer};
use serde_json::Value;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::editor::EditorError;
use crate::editor::message;
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::menu::{ProjectMenu, ProjectMenuAction};

/// The app's short name in the protocol and the discovery file's name.
const APP: &str = "audio";
/// The remote-only commands (spec table) with their plugin labels.
const REMOTE_ONLY: &[(&str, &str)] = &[
    ("track.mute", "Track Mute"),
    ("track.solo", "Track Solo"),
    ("track.arm", "Track Arm"),
    ("track.volume.step", "Track Volume"),
    ("zoom.step", "Zoom"),
    ("zoom.fit", "Zoom to Fit"),
    ("jog.step", "Jog"),
    ("marker.step", "Markers"),
];
/// A track-volume turn is one undo step: the gesture ends 500 ms after the last tick.
const GESTURE_END: Duration = Duration::from_millis(500);
/// Decibels per track-volume tick (spec: 0.5 dB per tick).
const DB_PER_TICK: f32 = 0.5;
/// The track volume control's range, the header slider's range.
const VOLUME_RANGE: std::ops::RangeInclusive<f32> = -60.0..=12.0;
/// The remote has no pointer over the timeline: zoom uses a nominal lane
/// width, so the step and the clamping match + and - while the anchor sits at
/// the view center.
const NOMINAL_LANE_W: f32 = 1000.0;
/// The most ticks one command applies. The plugin coalesces rotation ticks;
/// the cap bounds the loop a bad client could ask for.
const MAX_TICKS: i64 = 100;

/// The app's remote control server plus its gesture state. `App` holds an
/// `Option<Remote>`: `None` runs the app without the remote.
pub struct Remote {
    /// The server: socket, discovery files and the command queue.
    server: RemoteServer,
    /// The running track-volume turn, if any.
    volume: Option<VolumeGesture>,
}

/// One track-volume turn: the track it changes and when the last tick came.
struct VolumeGesture {
    /// The track the turn changes.
    track: Uuid,
    /// When the last tick of the turn arrived.
    last: Instant,
}

/// Starts the remote server: app "audio", this crate's version, this exe, and
/// as the command list every shortcut-table id plus the remote-only ones.
/// `wake` runs whenever a command arrives (the app passes `request_repaint`).
/// When another instance of the app already runs the remote, or the server
/// cannot start, logs why and returns `None`: the app runs without the remote.
pub fn start(wake: Box<dyn Fn() + Send>) -> Option<Remote> {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            log::warn!("remote control not started: the exe path could not be read ({error})");
            return None;
        }
    };
    let mut commands: Vec<CommandInfo> =
        crate::shortcuts::table().entries().iter().map(|entry| CommandInfo { id: entry.id.to_string(), label: entry.label.to_string() }).collect();
    commands.extend(REMOTE_ONLY.iter().map(|(id, label)| CommandInfo { id: (*id).to_string(), label: (*label).to_string() }));
    let config = RemoteConfig { app: APP, version: env!("CARGO_PKG_VERSION").to_string(), exe, commands, dir: None };
    match RemoteServer::start(config) {
        Ok(server) => {
            server.set_wake(wake);
            Some(Remote { server, volume: None })
        }
        Err(error) => {
            log::info!("remote control not started: {error}");
            None
        }
    }
}

/// Drops queued commands without running them: on the start screen nothing
/// dispatches, and stale commands must not fire when the next project opens.
pub fn clear(remote: &mut Remote) {
    while remote.server.try_recv().is_some() {}
}

/// What one command did. `poll` turns the error forms into replies; the tests
/// read them directly.
enum Handled {
    /// The command ran and left a Project menu action to apply.
    Project(ProjectMenuAction),
    /// The command ran; nothing to apply.
    Ran,
    /// A guard or bad args stopped the command: the error code and message
    /// for the client.
    Error(String, String),
    /// The id has no action behind it.
    Unknown,
}

/// The dispatch of one shortcut id, as app.rs runs it for the keyboard.
type Dispatch<'a> = dyn FnMut(&mut Editor, &mut ProjectMenu, &str) -> crate::app::ShortcutResult + 'a;

/// Drains the command queue and runs every command: a shortcut id through the
/// keyboard's dispatch (`dispatch`, built by app.rs over its egui context), a
/// remote-only id here. Errors go to the client as `blocked`, `bad_args` or
/// `unknown_command` replies. Returns the Project menu action a command left
/// behind (New or Open), else `None`. Also ends the track-volume gesture 500
/// ms after its last tick, making the turn one undo step. Called once per
/// frame while a project is open; `messages` takes what the keyboard path
/// would show in the messages dialog.
pub fn poll(
    remote: &mut Remote,
    editor: &mut Editor,
    menu: &mut ProjectMenu,
    messages: &mut Option<ConfirmDialog>,
    dispatch: &mut Dispatch,
) -> ProjectMenuAction {
    let mut action = ProjectMenuAction::None;
    while let Some(command) = remote.server.try_recv() {
        let id = command.id.clone();
        match handle(&mut remote.volume, command, editor, menu, messages, dispatch) {
            Handled::Project(left) => action = left,
            Handled::Ran => {}
            Handled::Error(code, why) => remote.server.reply_error(&id, &code, &why),
            Handled::Unknown => remote.server.reply_error(&id, "unknown_command", &format!("the app does not accept the command {id}")),
        }
    }
    end_volume_gesture(&mut remote.volume, editor);
    action
}

/// Runs one command.
fn handle(
    volume: &mut Option<VolumeGesture>,
    command: RemoteCommand,
    editor: &mut Editor,
    menu: &mut ProjectMenu,
    messages: &mut Option<ConfirmDialog>,
    dispatch: &mut Dispatch,
) -> Handled {
    let id = command.id.as_str();
    match id {
        "track.mute" | "track.solo" | "track.arm" => {
            let track = match track_id(editor, &command.args) {
                Ok(track) => track,
                Err(why) => return Handled::Error("bad_args".to_string(), why),
            };
            if editor.is_locked() {
                return blocked("an export is running");
            }
            if editor.is_recording() {
                return blocked("a take is being recorded");
            }
            if id == "track.mute" {
                op(editor.apply(Action::SetTrackMute { id: track, mute: !is_muted(editor, track) }))
            } else if id == "track.solo" {
                op(editor.apply(Action::SetTrackSolo { id: track, solo: !is_soloed(editor, track) }))
            } else {
                editor.toggle_arm(track);
                Handled::Ran
            }
        }
        "track.volume.step" => volume_step(volume, editor, &command.args),
        "zoom.step" => zoom_step(editor, &command.args),
        "zoom.fit" => {
            // The same fit the View menu and the backslash key use: the whole project.
            let (spp, scroll_x) = crate::ui::timeline::zoom_to_fit(editor.project_length(), editor.project.sample_rate, NOMINAL_LANE_W);
            editor.set_view_x(spp, scroll_x);
            Handled::Ran
        }
        "jog.step" => jog_step(editor, &command.args),
        "marker.step" => {
            let ticks = match ticks(&command.args) {
                Ok(ticks) => ticks,
                Err(why) => return Handled::Error("bad_args".to_string(), why),
            };
            for _ in 0..ticks.unsigned_abs() {
                if ticks > 0 {
                    editor.jump_next_marker();
                } else {
                    editor.jump_prev_marker();
                }
            }
            Handled::Ran
        }
        _ => match dispatch(editor, menu, id) {
            crate::app::ShortcutResult::Ran(outcome) => {
                if let Some(dialog) = outcome.dialog {
                    *messages = Some(dialog);
                } else if !outcome.messages.is_empty() {
                    let title = message::dialog_title(&outcome.messages);
                    let texts = outcome.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n");
                    *messages = Some(ConfirmDialog::new(title, texts));
                }
                if outcome.project == ProjectMenuAction::None { Handled::Ran } else { Handled::Project(outcome.project) }
            }
            crate::app::ShortcutResult::Blocked(reason) => blocked(crate::app::App::blocked_reason(&reason)),
            crate::app::ShortcutResult::Unhandled => Handled::Unknown,
        },
    }
}

/// One track-volume tick: the same undo behavior as dragging the track's
/// volume control, one undo step per gesture, the gesture ending 500 ms after
/// the last tick. Clamped to the track control's range.
fn volume_step(volume: &mut Option<VolumeGesture>, editor: &mut Editor, args: &Value) -> Handled {
    let track = match track_id(editor, args) {
        Ok(track) => track,
        Err(why) => return Handled::Error("bad_args".to_string(), why),
    };
    let ticks = match ticks(args) {
        Ok(ticks) => ticks,
        Err(why) => return Handled::Error("bad_args".to_string(), why),
    };
    if editor.is_locked() {
        return blocked("an export is running");
    }
    if editor.is_recording() {
        return blocked("a take is being recorded");
    }
    // A drag on a slider or a clip owns the gesture: the remote must not fold
    // its own turn into it.
    if editor.gesture_base.is_some() && !volume.as_ref().is_some_and(|turn| turn.track == track) {
        return blocked("a drag gesture is in progress");
    }
    if !volume.as_ref().is_some_and(|turn| turn.track == track) {
        editor.begin_gesture();
        *volume = Some(VolumeGesture { track, last: Instant::now() });
    }
    if let Some(turn) = volume {
        turn.last = Instant::now();
    }
    let current = editor.project.tracks.iter().find(|t| t.id == track).map(|t| t.volume_db).unwrap_or(0.0);
    let target = (current + DB_PER_TICK * ticks as f32).clamp(*VOLUME_RANGE.start(), *VOLUME_RANGE.end());
    op(editor.preview(Action::SetTrackVolume { id: track, db: target }))
}

/// Zoom ticks: the same 1.5 step as the + and - keys, applied per tick.
fn zoom_step(editor: &mut Editor, args: &Value) -> Handled {
    let ticks = match ticks(args) {
        Ok(ticks) => ticks,
        Err(why) => return Handled::Error("bad_args".to_string(), why),
    };
    let mut spp = editor.view.samples_per_pixel;
    let mut scroll_x = editor.view.scroll_x;
    for _ in 0..ticks.unsigned_abs() {
        let factor = if ticks > 0 { 1.5 } else { 1.0 / 1.5 };
        let (next_spp, next_scroll) = crate::ui::timeline::zoom_at(spp, scroll_x, factor, NOMINAL_LANE_W / 2.0);
        spp = next_spp;
        scroll_x = crate::ui::timeline::clamp_scroll_x(editor, spp, NOMINAL_LANE_W, next_scroll);
    }
    editor.set_view_x(spp, scroll_x);
    Handled::Ran
}

/// One jog step: 10 ms (fine) or 1 s (coarse) per tick, snapped when Snap is
/// on. Blocked while a take is recorded, like every pointer edit on the
/// timeline.
fn jog_step(editor: &mut Editor, args: &Value) -> Handled {
    let ticks = match ticks(args) {
        Ok(ticks) => ticks,
        Err(why) => return Handled::Error("bad_args".to_string(), why),
    };
    let size = args.get("size").and_then(Value::as_str).unwrap_or("fine");
    let step_ms: u64 = match size {
        "fine" => 10,
        "coarse" => 1000,
        _ => return Handled::Error("bad_args".to_string(), format!("size must be \"fine\" or \"coarse\", not {size}")),
    };
    if editor.is_recording() {
        return blocked("a take is being recorded");
    }
    let step = step_ms.saturating_mul(u64::from(editor.project.sample_rate) / 1000);
    let amount = step.saturating_mul(ticks.unsigned_abs());
    let target = if ticks >= 0 { editor.view.playhead.saturating_add(amount) } else { editor.view.playhead.saturating_sub(amount) };
    let (snapped, _) = editor.snap_pos(target, &[], None, !editor.view.snap, crate::ui::timeline::metrics::SNAP_PX);
    editor.seek(snapped);
    Handled::Ran
}

/// Ends the track-volume gesture 500 ms after its last tick: the whole turn
/// becomes one undo entry from the gesture base.
fn end_volume_gesture(volume: &mut Option<VolumeGesture>, editor: &mut Editor) {
    if volume.as_ref().is_some_and(|turn| turn.last.elapsed() >= GESTURE_END) {
        *volume = None;
        editor.end_gesture();
    }
}

/// The id of the track a 1-based `track` argument names, or why the argument
/// is bad (a `bad_args` reply).
fn track_id(editor: &Editor, args: &Value) -> Result<Uuid, String> {
    let number = args.get("track").and_then(Value::as_u64).ok_or_else(|| "the track argument is missing or not a number".to_string())?;
    let index =
        usize::try_from(number).map_err(|_| format!("there is no track {number}"))?.checked_sub(1).ok_or_else(|| "the track number starts at 1".to_string())?;
    editor.project.tracks.get(index).map(|track| track.id).ok_or_else(|| format!("there is no track {number}"))
}

/// The `ticks` argument, or why it is missing (a `bad_args` reply).
fn ticks(args: &Value) -> Result<i64, String> {
    let ticks = args.get("ticks").and_then(Value::as_i64).ok_or_else(|| "the ticks argument is missing or not a whole number".to_string())?;
    Ok(ticks.clamp(-MAX_TICKS, MAX_TICKS))
}

/// Whether a track is muted, for the toggle.
fn is_muted(editor: &Editor, track: Uuid) -> bool {
    editor.project.tracks.iter().find(|t| t.id == track).is_some_and(|t| t.mute)
}

/// Whether a track is soloed, for the toggle.
fn is_soloed(editor: &Editor, track: Uuid) -> bool {
    editor.project.tracks.iter().find(|t| t.id == track).is_some_and(|t| t.solo)
}

/// A guard stopped the command: `blocked`, with the reason for the client.
fn blocked(why: &str) -> Handled {
    Handled::Error("blocked".to_string(), why.to_string())
}

/// An ops result: an error the command's guard let through (an empty
/// selection, say) is still a `blocked` reply.
fn op(result: Result<(), EditorError>) -> Handled {
    match result {
        Ok(()) => Handled::Ran,
        Err(error) => Handled::Error("blocked".to_string(), error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::record::FakeInput;
    use crate::editor::selection::Selection;
    use crate::model::project::ViewState;
    use serde_json::json;

    /// An editor over a project with the given tracks (clips as `(start, len)`
    /// pairs) in a folder the caller keeps alive for as long as it is used.
    fn test_editor(tracks: &[&[(u64, u64)]]) -> (Editor, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = crate::io::paths::create_layout(dir.path(), "Remote").expect("layout");
        let editor = Editor::with_project(crate::model::fixtures::project_with_clips(tracks), ViewState::default(), paths);
        (editor, dir)
    }

    /// One remote command with the given JSON args.
    fn command(id: &str, args: Value) -> RemoteCommand {
        RemoteCommand { id: id.to_string(), args }
    }

    /// The real keyboard dispatch against a fresh egui context.
    fn dispatch() -> impl FnMut(&mut Editor, &mut ProjectMenu, &str) -> crate::app::ShortcutResult {
        let ctx = eframe::egui::Context::default();
        move |editor, menu, id| crate::app::App::run_shortcut_action(&ctx, editor, menu, id)
    }

    /// Runs one command through `handle` with an empty menu and no messages.
    fn run(volume: &mut Option<VolumeGesture>, command: RemoteCommand, editor: &mut Editor) -> Handled {
        let mut menu = ProjectMenu::default();
        let mut messages = None;
        let mut dispatch = dispatch();
        handle(volume, command, editor, &mut menu, &mut messages, &mut dispatch)
    }

    /// track.mute, track.solo and track.arm toggle their track's state.
    #[test]
    fn track_commands_toggle_the_track() {
        let (mut editor, _dir) = test_editor(&[&[], &[]]);
        let mut volume = None;
        assert!(matches!(run(&mut volume, command("track.mute", json!({ "track": 1 })), &mut editor), Handled::Ran));
        assert!(editor.project.tracks[0].mute);
        assert!(matches!(run(&mut volume, command("track.mute", json!({ "track": 1 })), &mut editor), Handled::Ran));
        assert!(!editor.project.tracks[0].mute);

        assert!(matches!(run(&mut volume, command("track.solo", json!({ "track": 2 })), &mut editor), Handled::Ran));
        assert!(editor.project.tracks[1].solo);
        assert!(!editor.project.tracks[0].solo, "a solo toggle does not touch the other track");

        assert!(matches!(run(&mut volume, command("track.arm", json!({ "track": 1 })), &mut editor), Handled::Ran));
        assert_eq!(editor.armed_track(), Some(editor.project.tracks[0].id));
    }

    /// track.volume.step changes the volume by 0.5 dB per tick, clamped to
    /// the track control's range.
    #[test]
    fn volume_step_changes_and_clamps() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        let mut volume = None;
        assert!(matches!(run(&mut volume, command("track.volume.step", json!({ "track": 1, "ticks": -6 })), &mut editor), Handled::Ran));
        assert_eq!(editor.project.tracks[0].volume_db, -3.0);
        // Far past the range: capped at MAX_TICKS per command, clamped to the
        // slider's floor.
        assert!(matches!(run(&mut volume, command("track.volume.step", json!({ "track": 1, "ticks": -500 })), &mut editor), Handled::Ran));
        assert_eq!(editor.project.tracks[0].volume_db, -53.0, "MAX_TICKS ticks of 0.5 dB");
        assert!(matches!(run(&mut volume, command("track.volume.step", json!({ "track": 1, "ticks": -100 })), &mut editor), Handled::Ran));
        assert_eq!(editor.project.tracks[0].volume_db, -60.0, "clamped to the control's floor");
    }

    /// jog.step moves the playhead 10 ms (fine) or 1 s (coarse) per tick.
    #[test]
    fn jog_step_moves_the_playhead() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        editor.view.snap = false;
        let rate = u64::from(editor.project.sample_rate);
        let mut volume = None;
        assert!(matches!(run(&mut volume, command("jog.step", json!({ "ticks": 10, "size": "fine" })), &mut editor), Handled::Ran));
        assert_eq!(editor.view.playhead, 100 * rate / 1000, "10 ticks x 10 ms");
        assert!(matches!(run(&mut volume, command("jog.step", json!({ "ticks": -2, "size": "coarse" })), &mut editor), Handled::Ran));
        assert_eq!(editor.view.playhead, 0, "two coarse seconds back, clamped at the start");
    }

    /// zoom.fit fits the project, zoom.step applies the same step as the + and
    /// - keys (a positive tick multiplies the samples per pixel, like +).
    #[test]
    fn zoom_commands_change_the_view() {
        let (mut editor, _dir) = test_editor(&[&[(0, 480_000)]]);
        let mut volume = None;
        let before = editor.view.samples_per_pixel;
        assert!(matches!(run(&mut volume, command("zoom.step", json!({ "ticks": 1 })), &mut editor), Handled::Ran));
        assert_eq!(editor.view.samples_per_pixel, before * 1.5);

        assert!(matches!(run(&mut volume, command("zoom.fit", json!({})), &mut editor), Handled::Ran));
        let (spp, scroll_x) = crate::ui::timeline::zoom_to_fit(editor.project_length(), editor.project.sample_rate, NOMINAL_LANE_W);
        assert_eq!(editor.view.samples_per_pixel, spp);
        assert_eq!(editor.view.scroll_x, scroll_x);
    }

    /// marker.step jumps to the next or previous marker by its sign.
    #[test]
    fn marker_step_jumps_by_sign() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        let mut volume = None;
        let _ = editor.add_marker_at_playhead();
        editor.seek(u64::from(editor.project.sample_rate));
        let _ = editor.add_marker_at_playhead();
        editor.jump_start();
        assert!(matches!(run(&mut volume, command("marker.step", json!({ "ticks": 1 })), &mut editor), Handled::Ran));
        assert_eq!(editor.view.playhead, u64::from(editor.project.sample_rate));
        assert!(matches!(run(&mut volume, command("marker.step", json!({ "ticks": -1 })), &mut editor), Handled::Ran));
        assert_eq!(editor.view.playhead, 0);
    }

    /// A shortcut id runs the same action as the key: select_all selects the
    /// whole project.
    #[test]
    fn a_shortcut_id_dispatches_its_action() {
        let (mut editor, _dir) = test_editor(&[&[(0, 1000)]]);
        let mut volume = None;
        assert!(matches!(run(&mut volume, command("select_all", json!({})), &mut editor), Handled::Ran));
        assert!(matches!(editor.selection(), Selection::Range { start: 0, end: 1000, .. }), "select_all ran and covered the clip");
        // An id with no action behind it asks for unknown_command.
        assert!(matches!(run(&mut volume, command("help_topics", json!({})), &mut editor), Handled::Unknown));
    }

    /// While a take is recorded the blocked commands answer blocked.
    #[test]
    fn record_blocked_commands_answer_blocked_while_recording() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        editor.set_fake_input(FakeInput::default());
        editor.toggle_arm(editor.project.tracks[0].id);
        editor.start_recording().expect("the take starts on the fake input");
        let mut volume = None;
        // A shortcut id outside the four recording keys.
        assert!(
            matches!(
                run(&mut volume, command("toggle_snap", json!({})), &mut editor),
                Handled::Error(code, _) if code == "blocked"
            ),
            "toggle_snap is blocked while recording"
        );
        // jog.step is ignored (a blocked reply) while recording, per the spec.
        assert!(matches!(
            run(&mut volume, command("jog.step", json!({ "ticks": 1, "size": "fine" })), &mut editor),
            Handled::Error(code, _) if code == "blocked"
        ));
        // The recording keys still run.
        assert!(matches!(run(&mut volume, command("add_marker", json!({})), &mut editor), Handled::Ran));
    }

    /// A missing track number, an out-of-range one and bad args answer
    /// bad_args.
    #[test]
    fn bad_track_and_bad_args_answer_bad_args() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        let mut volume = None;
        for args in [json!({ "track": 0 }), json!({ "track": 2 }), json!({ "track": "1" }), json!({})] {
            assert!(
                matches!(
                    run(&mut volume, command("track.mute", args), &mut editor),
                    Handled::Error(code, _) if code == "bad_args"
                ),
                "a bad track answers bad_args"
            );
        }
        assert!(
            matches!(
                run(&mut volume, command("track.volume.step", json!({ "track": 1 })), &mut editor),
                Handled::Error(code, _) if code == "bad_args"
            ),
            "missing ticks answer bad_args"
        );
        assert!(
            matches!(
                run(&mut volume, command("jog.step", json!({ "ticks": 1, "size": "huge" })), &mut editor),
                Handled::Error(code, _) if code == "bad_args"
            ),
            "a bad size answers bad_args"
        );
    }

    /// Several volume ticks in a row are one undo step, closed 500 ms after
    /// the last tick.
    #[test]
    fn a_volume_gesture_of_several_ticks_is_one_undo_step() {
        let (mut editor, _dir) = test_editor(&[&[]]);
        let mut volume = None;
        for _ in 0..3 {
            assert!(matches!(run(&mut volume, command("track.volume.step", json!({ "track": 1, "ticks": 1 })), &mut editor), Handled::Ran));
            // Nothing lands in the history while the turn runs.
            assert!(editor.undo_label().is_none(), "a running gesture records nothing");
        }
        assert_eq!(editor.project.tracks[0].volume_db, 1.5);
        // The gesture ends 500 ms after the last tick: one undo step.
        if let Some(turn) = volume.as_mut() {
            turn.last = Instant::now() - GESTURE_END;
        }
        end_volume_gesture(&mut volume, &mut editor);
        assert_eq!(editor.undo_label(), Some("Track Volume"), "the whole turn is one undo step");
        let _ = editor.undo();
        assert_eq!(editor.project.tracks[0].volume_db, 0.0, "undo reverts the whole turn");
    }
}
