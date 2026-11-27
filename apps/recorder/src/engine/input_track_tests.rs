use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use events::{CaptureSource, CursorSample, CursorShape, EventsTrack, InputEvent, MouseAction, MouseButton, MouseEvent, ScreenRect, ShapeFrame, Streams};

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
    let info = SourceInfo { qpc_frequency: 1000, cursor_size: 32, displays: Vec::new(), capture: CaptureSource::Monitor(ScreenRect::default()) };
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
fn a_replay_snapshot_includes_the_events_and_what_they_rely_on() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core(&fake);
    core.set_replay(Some(Duration::from_secs(60)));
    fake.send([shape(1), sample(1_000, 1), click(1_500), sample(20_000, 2), click(20_500), sample(30_000, 3)]);
    let path = temp_file("replay");
    core.clip(10_000, 25_000).write(&path)?;

    let track = read(&path)?;
    assert!(track.is_complete());
    // The cursor from before the clip is stated at its start, with the shape the logger sent long before.
    assert_eq!(samples(&track), [(0, 1), (10_000_000, 2)]);
    assert_eq!(track.mouse().len(), 1);
    assert!(track.cursor_at(0).is_some_and(|state| state.shape.is_some()));
    Ok(())
}

#[test]
fn a_logger_that_dies_leaves_the_take_incomplete() -> Result<(), EventsError> {
    let fake = Fake::default();
    let mut core = core(&fake);
    let path = temp_file("died");
    core.begin_take(&path, 1000);
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
