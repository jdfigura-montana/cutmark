use std::fs;
use std::path::PathBuf;

use events::{
    CaptureSource, Chord, CursorSample, DisplayInfo, EventsWriter, GeometryChange, GeometryEvent, Header, InputEvent, Modifiers, MouseAction, MouseButton,
    MouseEvent, ScreenRect, Streams,
};

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const S: i64 = 1_000_000;

/// A second monitor to the right of the first, recorded into a video of half its size.
const MONITOR: ScreenRect = ScreenRect { x: 1920, y: 0, width: 1920, height: 1080 };

fn video() -> VideoFacts {
    VideoFacts { size: (960, 540), start_us: 20_000, duration_us: 20 * S, screen: true }
}

/// An events file whose clock counts microseconds from the first video frame.
fn track_bytes(source: CaptureSource, events: impl IntoIterator<Item = InputEvent>) -> Result<Vec<u8>, events::EventsError> {
    let header = Header {
        qpc_frequency: S,
        first_frame_qpc: 0,
        source,
        displays: vec![DisplayInfo { name: "two".to_owned(), rect: MONITOR, dpi: 96, primary: false }],
        cursor_size: 32,
        streams: Streams { cursor: true, clicks: true, keys: true, apps: false },
    };
    let mut writer = EventsWriter::new(Vec::new(), &header)?;
    for event in events {
        writer.push(event)?;
    }
    writer.finish(20 * S)
}

fn sample(qpc: i64, x: i32, y: i32) -> InputEvent {
    InputEvent::Sample(CursorSample { qpc, x, y, shape: None, visible: true })
}

fn click(qpc: i64, x: i32, y: i32) -> InputEvent {
    InputEvent::Mouse(MouseEvent { qpc, x, y, action: MouseAction::Down(MouseButton::Left) })
}

fn chord(qpc: i64, key: &str) -> InputEvent {
    InputEvent::Chord(Chord { qpc, modifiers: Modifiers { ctrl: true, shift: true, ..Modifiers::default() }, key_name: key.to_owned() })
}

#[test]
fn to_source_maps_a_monitor_into_the_video() -> TestResult {
    let track = EventsReader::from_bytes(&track_bytes(CaptureSource::Monitor(MONITOR), [])?)?;
    let map = to_source(&track, video());
    assert_eq!(map(0, 1920, 0), Some((0.0, 0.0)));
    assert_eq!(map(0, 2880, 540), Some((480.0, 270.0)));
    // The first monitor, and the far edge of the second, are outside the captured frame.
    assert_eq!((map(0, 100, 100), map(0, 3840, 540)), (None, None));
    // A video that does not show the screen holds no desktop position.
    assert_eq!(to_source(&track, VideoFacts { screen: false, ..video() })(0, 2880, 540), None);
    Ok(())
}

#[test]
fn to_source_follows_a_captured_window_as_it_moves() -> TestResult {
    let rect = |qpc, x| InputEvent::Geometry(GeometryEvent { qpc, change: GeometryChange::WindowRect(ScreenRect { x, y: 100, width: 960, height: 540 }) });
    let track = EventsReader::from_bytes(&track_bytes(CaptureSource::Window { handle: 1 }, [rect(0, 100), rect(5 * S, 400)])?)?;
    let map = to_source(&track, video());
    assert_eq!(map(S, 580, 370), Some((480.0, 270.0)));
    assert_eq!(map(6 * S, 880, 370), Some((480.0, 270.0)));
    assert_eq!(map(6 * S, 300, 370), None);
    Ok(())
}

#[test]
fn a_shortcut_shows_for_the_duration_and_a_newer_one_replaces_it() {
    let chords = [(S, "Ctrl+S".to_owned()), (2 * S, "F5".to_owned())];
    let at = |time_us| chord_at(&chords, time_us, 1.5);
    assert_eq!((at(S - 1), at(S), at(2 * S - 1)), (None, Some("Ctrl+S"), Some("Ctrl+S")));
    assert_eq!((at(2 * S), at(3 * S + 499_999), at(3 * S + 500_000)), (Some("F5"), Some("F5"), None));
}

#[test]
fn the_cursor_is_idle_from_its_last_move() {
    let point = |time_us, x: f32| CursorPoint { time_us, position: Some((x, 0.0)) };
    let moves = move_times(&[point(S, 1.0), point(2 * S, 2.0), point(3 * S, 2.0)]);
    assert_eq!(moves, [S, 2 * S]);
    assert_eq!((idle_us(&moves, 2 * S), idle_us(&moves, 5 * S), idle_us(&moves, S / 2)), (0, 3 * S, S / 2));
}

#[test]
fn the_settings_become_the_renderers() {
    let defaults = render_settings(&StudioSettings::default(), None);
    assert_eq!(defaults.cursor, render::CursorLook::default());
    assert_eq!(defaults.keys, render::KeysLook::default());
    assert!(!defaults.frame.enabled && defaults.frame.shadow == render::Shadow::Soft);
    assert_eq!((defaults.frame.padding, defaults.frame.corner_radius), (0.06, 12.0));
    let mut settings = StudioSettings::default();
    settings.cursor.highlight = Highlight::Spotlight;
    settings.cursor.size_percent = 250;
    settings.frame.background = BackgroundKind::Image;
    settings.frame.gradient_preset = 3;
    let made = render_settings(&settings, None);
    assert_eq!((made.cursor.highlight, made.cursor.size), (render::Highlight::Spotlight { radius: 80.0 }, 2.5));
    // No picture has been loaded, so the gradient stands in.
    assert!(matches!(made.frame.background, render::Background::Gradient(3)));
}

fn fresh_dir(name: &str) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("cutmark-studio-take-{name}-{}", std::process::id()));
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn the_first_open_creates_the_studio_file_with_segments() -> TestResult {
    let dir = fresh_dir("first-open")?;
    let (take, events, studio) = (dir.join("take01.mkv"), dir.join("take01.events"), dir.join("take01.studio.json"));
    let input = [sample(0, 2880, 540), click(5 * S, 2880, 540), chord(5 * S + 500_000, "S"), click(15 * S, 100, 100)];
    fs::write(&events, track_bytes(CaptureSource::Monitor(MONITOR), input)?)?;

    let opened = StudioTake::open(&take, &events, video())?;
    let saved = settings::load(&studio)?;
    assert!(!saved.first_open);
    assert_eq!(saved.settings, opened.settings);
    // One segment: the click and the shortcut. The click on the other monitor makes none.
    let lane: Vec<(i64, i64)> = saved.settings.zoom.segments.iter().map(|segment| (segment.start_us, segment.end_us)).collect();
    assert_eq!(lane, [(4_500_000, 7 * S)]);
    // Positions in the file are 20 ms after the track's.
    assert_eq!(opened.zoom_starts_ms(), [4_520]);
    assert_eq!(opened.track_time_us(5_020), 5 * S);

    // A preview picture half the video's size gets everything at half the position.
    let values = opened.frame_values(5 * S + 100_000, (480, 270));
    assert!(values.view.width < 480.0 && values.view.x > 0.0, "{:?}", values.view);
    assert_eq!(values.clicks, [render::ClickDraw { x: 240.0, y: 135.0, time_us: 5 * S }]);
    let cursor = values.cursor.ok_or("no cursor")?;
    assert_eq!((cursor.x, cursor.y, cursor.scale, cursor.idle_us), (240.0, 135.0, 0.25, 5 * S + 100_000));
    assert_eq!(values.chord, None);
    assert_eq!(opened.frame_values(6 * S, (480, 270)).chord, Some("Ctrl+Shift+S"));

    // A lane the user emptied stays empty.
    let mut emptied = saved.settings;
    emptied.zoom.segments.clear();
    settings::save(&studio, &emptied)?;
    assert!(StudioTake::open(&take, &events, video())?.settings.zoom.segments.is_empty());
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn a_damaged_studio_file_is_left_as_it_is() -> TestResult {
    let dir = fresh_dir("damaged")?;
    let (take, events, studio) = (dir.join("take01.mkv"), dir.join("take01.events"), dir.join("take01.studio.json"));
    fs::write(&events, track_bytes(CaptureSource::Monitor(MONITOR), [])?)?;
    fs::write(&studio, b"{")?;
    assert!(matches!(StudioTake::open(&take, &events, video()), Err(StudioError::Settings(_))));
    assert_eq!(fs::read(&studio)?, b"{");
    fs::remove_dir_all(&dir)?;
    Ok(())
}
