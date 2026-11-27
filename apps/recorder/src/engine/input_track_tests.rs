use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use events::{
    AppSwitch, CaptureSource, CursorSample, CursorShape, EventsTrack, GeometryChange, GeometryEvent, InputEvent, MouseAction, MouseButton, MouseEvent,
    ScreenRect, ShapeFrame, Streams,
};

use super::*;

/// A logger the test drives: what it sends next, and whether it still runs.
#[derive(Clone, Default)]
struct Fake(Arc<Mutex<(VecDeque<InputEvent>, bool)>>);

impl Fake {
    fn send(&self, events: impl IntoIterator<Item = InputEvent>) {
        if let Ok(mut state) = self.0.lock() {
            state.0.extend(events);
        }
    }

    fn die(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.1 = true;
        }
    }
}

impl InputSource for Fake {
    fn try_next(&mut self) -> Option<InputEvent> {
        self.0.lock().ok()?.0.pop_front()
    }

    fn is_running(&self) -> bool {
        self.0.lock().is_ok_and(|state| !state.1)
    }
}

/// One tick is a millisecond.
fn core(fake: &Fake) -> Core {
    core_with_app(fake, None)
}

/// A core whose logger started with `exe` in front.
fn core_with_app(fake: &Fake, exe: Option<&str>) -> Core {
    let app = exe.map(|exe| AppSwitch { qpc: 0, exe: exe.to_owned(), display_name: exe.to_owned() });
    let info = SourceInfo { qpc_frequency: 1000, cursor_size: 32, displays: Vec::new(), capture: CaptureSource::Monitor(ScreenRect::default()), app };
    Core::new(Box::new(fake.clone()), info, Streams { cursor: true, clicks: true, keys: false, apps: true })
}

fn shape(id: u32) -> InputEvent {
    InputEvent::Shape(CursorShape { id, width: 1, height: 1, hotspot_x: 0, hotspot_y: 0, frames: vec![ShapeFrame { duration_us: 0, bgra: vec![0; 4] }] })
}

fn sample(qpc: i64, x: i32) -> InputEvent {
    InputEvent::Sample(CursorSample { qpc, x, y: 0, shape: Some(1), visible: true })
}

fn click(qpc: i64) -> InputEvent {
    InputEvent::Mouse(MouseEvent { qpc, x: 0, y: 0, action: MouseAction::Down(MouseButton::Left) })
}

fn temp_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("cutmark-input-track-{}-{name}.events", std::process::id()));
    let _ = std::fs::remove_file(&path);
    path
}

fn app(qpc: i64, exe: &str) -> InputEvent {
    InputEvent::App(AppSwitch { qpc, exe: exe.to_owned(), display_name: exe.to_owned() })
}

/// The app switches of a file as (microseconds, exe).
fn apps(track: &EventsTrack) -> Vec<(i64, &str)> {
    track.apps().iter().map(|timed| (timed.time_us, timed.event.exe.as_str())).collect()
}

/// The samples of a file as (microseconds, x).
fn samples(track: &EventsTrack) -> Vec<(i64, i32)> {
    track.samples().iter().map(|timed| (timed.time_us, timed.event.x)).collect()
}

fn read(path: &Path) -> Result<EventsTrack, EventsError> {
    let track = EventsTrack::open(path);
    let _ = std::fs::remove_file(path);
    track
}

#[test]
fn track_starts_the_capture_with_the_cursor_off() -> std::io::Result<()> {
    let track = InputTrack::start(core(&Fake::default()))?;
    let plan = plan_cursor(CursorMode::Track, move || Ok(track));
    assert!(!plan.burn_in && plan.track.is_some() && plan.failure.is_none());

    let hide = plan_cursor(CursorMode::Hide, || Err("Hide starts no logger".to_owned()));
    assert!(!hide.burn_in && hide.track.is_none() && hide.failure.is_none());
    let show = plan_cursor(CursorMode::Show, || Err("Show starts no logger".to_owned()));
    assert!(show.burn_in && show.track.is_none() && show.failure.is_none());
    Ok(())
}

#[test]
fn a_logger_that_cannot_start_leaves_the_cursor_in_the_video() {
    let plan = plan_cursor(CursorMode::Track, || Err("no raw input".to_owned()));
    assert!(plan.burn_in && plan.track.is_none());
    assert_eq!(plan.failure.as_deref(), Some(START_FAILED));
}

#[test]
fn a_take_file_starts_at_the_first_frame_and_leaves_out_the_pause() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core(&fake);
    fake.send([shape(1), sample(900, 1), click(950)]);
    let path = temp_file("pause");
    core.begin_take(&path, 1000);
    core.first_frame(1000);
    fake.send([sample(1100, 2)]);
    core.pump();
    core.pause(1200);
    fake.send([sample(1300, 3), click(1400)]);
    core.pump();
    core.resume(2200);
    fake.send([sample(2300, 4)]);
    core.finish_take(2500);

    let track = read(&path)?;
    assert!(track.is_complete());
    // The state at the start, then at the resume, 1000 ms of pause taken out; the click in the pause is gone.
    assert_eq!(samples(&track), [(0, 1), (100_000, 2), (200_000, 3), (300_000, 4)]);
    assert!(track.mouse().is_empty());
    assert!(core.take_notices().is_empty());
    Ok(())
}

#[test]
fn a_replay_file_starts_with_its_shapes_and_the_state_at_its_start() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core_with_app(&fake, Some("code.exe"));
    core.set_replay(Some(Duration::from_secs(60)));
    let rect = ScreenRect { x: 10, y: 20, width: 300, height: 200 };
    let window = InputEvent::Geometry(GeometryEvent { qpc: 900, change: GeometryChange::WindowRect(rect) });
    let later = InputEvent::Sample(CursorSample { qpc: 20_000, x: 2, y: 0, shape: Some(2), visible: true });
    fake.send([shape(1), shape(2), shape(3), window, sample(1_000, 1), click(1_500), later, click(20_500), sample(30_000, 3)]);
    let path = temp_file("replay");
    core.clip(10_000, 25_000).write(&path)?;

    let track = read(&path)?;
    assert!(track.is_complete());
    // The cursor from before the clip is stated at its start, with the shape the logger sent long before.
    assert_eq!(samples(&track), [(0, 1), (10_000_000, 2)]);
    assert_eq!(track.shapes().map(|shape| shape.id).collect::<Vec<_>>(), [1, 2]);
    let geometry: Vec<_> = track.geometry().iter().map(|timed| (timed.time_us, timed.event.change.clone())).collect();
    assert_eq!(geometry, [(0, GeometryChange::WindowRect(rect))]);
    assert_eq!(apps(&track), [(0, "code.exe")]);
    assert_eq!(track.mouse().len(), 1);
    Ok(())
}

#[test]
fn the_app_in_front_is_the_first_app_event_at_time_zero() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core_with_app(&fake, Some("code.exe"));
    let path = temp_file("front-app");
    core.begin_take(&path, 1000);
    core.first_frame(1000);
    // The logger's first report may name the app that was already in front.
    fake.send([app(1100, "code.exe"), app(1300, "chrome.exe")]);
    core.finish_take(2000);

    let track = read(&path)?;
    assert_eq!(apps(&track), [(0, "code.exe"), (300_000, "chrome.exe")]);
    Ok(())
}

#[test]
fn events_are_held_until_the_first_frame_and_timed_from_it() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core(&fake);
    fake.send([shape(1), sample(900, 1)]);
    let path = temp_file("held");
    core.begin_take(&path, 1000);
    // Over more than the history's margin, with a pause, all before the first frame is known.
    fake.send([sample(1010, 2), click(1020), sample(1100, 3), click(1200), sample(9000, 4)]);
    core.pump();
    core.pause(9100);
    fake.send([sample(9200, 5), click(9250)]);
    core.pump();
    core.resume(9300);
    fake.send([sample(9400, 6)]);
    core.pump();
    assert!(!path.exists());
    core.first_frame(1050);
    core.finish_take(9500);

    let track = read(&path)?;
    assert!(track.is_complete());
    assert_eq!(track.header().first_frame_qpc, 1050);
    assert_eq!(samples(&track), [(0, 2), (50_000, 3), (7_950_000, 4), (8_050_000, 5), (8_150_000, 6)]);
    assert_eq!(track.mouse().iter().map(|timed| timed.time_us).collect::<Vec<_>>(), [150_000]);
    assert!(core.take_notices().is_empty());
    Ok(())
}

#[test]
fn a_logger_that_dies_leaves_the_take_incomplete() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core(&fake);
    let path = temp_file("died");
    core.begin_take(&path, 1000);
    core.first_frame(1000);
    fake.send([shape(1), sample(1100, 1)]);
    core.pump();
    fake.die();
    core.pump();
    core.finish_take(2000);

    let track = read(&path)?;
    assert!(!track.is_complete());
    assert_eq!(samples(&track), [(100_000, 1)]);
    assert_eq!(core.take_notices().len(), 1);
    Ok(())
}
