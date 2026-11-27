#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;
use crate::format::{Input, next_chunk, put_chunk};

const FREQUENCY: i64 = 10_000_000;
const FIRST_FRAME: i64 = 5_000_000_000;
/// A chunk's type, length and CRC.
const CHUNK_HEADER_LEN: usize = 12;

/// The QPC stamp `ms` milliseconds after the first frame.
fn at(ms: i64) -> i64 {
    FIRST_FRAME + ms * (FREQUENCY / 1000)
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> ScreenRect {
    ScreenRect { x, y, width, height }
}

fn display(name: &str, x: i32, dpi: u32) -> DisplayInfo {
    DisplayInfo { name: name.to_owned(), rect: rect(x, 0, 2560, 1440), dpi, primary: x == 0 }
}

fn header() -> Header {
    Header {
        qpc_frequency: FREQUENCY,
        first_frame_qpc: FIRST_FRAME,
        source: CaptureSource::Monitor(rect(0, 0, 2560, 1440)),
        displays: vec![display(r"\\.\DISPLAY1", 0, 144), display(r"\\.\DISPLAY2", -2560, 96)],
        cursor_size: 32,
        streams: Streams { cursor: true, clicks: true, keys: true, apps: true },
    }
}

fn shape(id: u32, frames: u32) -> CursorShape {
    let frames = (0..frames).map(|n| ShapeFrame { duration_us: n * 1000, bgra: (0..32 * 48 * 4).map(|i| (i as u32 + n + id) as u8).collect() });
    CursorShape { id, width: 32, height: 48, hotspot_x: 3, hotspot_y: 4, frames: frames.collect() }
}

fn sample(ms: i64, x: i32, y: i32, shape: Option<u32>) -> CursorSample {
    CursorSample { qpc: at(ms), x, y, shape, visible: shape.is_some() }
}

fn samples() -> Vec<CursorSample> {
    vec![
        sample(-8, 100, 200, Some(1)),
        sample(0, -2560, -40, Some(1)),
        sample(4, i32::MAX, i32::MIN, Some(2)),
        sample(9, 0, 0, None),
        // Shown again with the shape used before it was hidden.
        sample(13, 5, 6, Some(2)),
        CursorSample { qpc: at(263), x: 5, y: 6, shape: Some(1), visible: false },
    ]
}

fn mouse() -> Vec<MouseEvent> {
    use MouseButton::{Left, Middle, Right, X1, X2};
    let mut actions: Vec<MouseAction> = [Left, Right, Middle, X1, X2].into_iter().flat_map(|b| [MouseAction::Down(b), MouseAction::Up(b)]).collect();
    actions.extend([MouseAction::Wheel(-120), MouseAction::HorizontalWheel(i16::MAX)]);
    actions.into_iter().enumerate().map(|(n, action)| MouseEvent { qpc: at(n as i64), x: -(n as i32), y: n as i32, action }).collect()
}

fn chords() -> Vec<Chord> {
    vec![
        Chord { qpc: at(20), modifiers: Modifiers { ctrl: true, ..Modifiers::default() }, key_name: "S".to_owned() },
        Chord { qpc: at(30), modifiers: Modifiers { ctrl: true, alt: true, shift: true, win: true }, key_name: "Tab".to_owned() },
        Chord { qpc: at(40), modifiers: Modifiers::default(), key_name: "F24".to_owned() },
    ]
}

fn apps() -> Vec<AppSwitch> {
    vec![AppSwitch { qpc: at(50), exe: "notepad.exe".to_owned(), display_name: "Bloc-notes \u{e9}".to_owned() }]
}

fn geometry() -> Vec<GeometryEvent> {
    vec![
        GeometryEvent { qpc: at(1), change: GeometryChange::WindowRect(rect(-10, 20, 800, 600)) },
        GeometryEvent { qpc: at(60), change: GeometryChange::Displays(vec![display(r"\\.\DISPLAY1", 0, 120)]) },
    ]
}

fn everything() -> Vec<InputEvent> {
    let mut events = vec![InputEvent::Shape(shape(1, 1)), InputEvent::Shape(shape(2, 3))];
    events.extend(samples().into_iter().map(InputEvent::Sample));
    events.extend(mouse().into_iter().map(InputEvent::Mouse));
    events.extend(chords().into_iter().map(InputEvent::Chord));
    events.extend(apps().into_iter().map(InputEvent::App));
    events.extend(geometry().into_iter().map(InputEvent::Geometry));
    events
}

fn write(header: &Header, events: Vec<InputEvent>) -> Vec<u8> {
    let mut writer = EventsWriter::new(Vec::new(), header).unwrap();
    for event in events {
        writer.push(event).unwrap();
    }
    writer.finish(at(1000)).unwrap()
}

/// The type of each chunk and the offset just past it.
fn chunks(bytes: &[u8]) -> Vec<([u8; 4], usize)> {
    let mut input = Input(bytes);
    std::iter::from_fn(|| next_chunk(&mut input).map(|(kind, _)| (kind, bytes.len() - input.0.len()))).collect()
}

fn kinds(bytes: &[u8]) -> Vec<String> {
    chunks(bytes).iter().map(|(kind, _)| String::from_utf8_lossy(kind).into_owned()).collect()
}

fn events_of<T: Clone>(list: &[Timed<T>]) -> Vec<T> {
    list.iter().map(|timed| timed.event.clone()).collect()
}

#[test]
fn every_chunk_type_round_trips() {
    let bytes = write(&header(), everything());
    assert_eq!(kinds(&bytes), ["HEAD", "SHAP", "SHAP", "SAMP", "MOUS", "KEYS", "APPS", "GEOM", "TAIL"]);
    assert_eq!(&bytes[CHUNK_HEADER_LEN..CHUNK_HEADER_LEN + 5], b"CMEVT");

    let track = EventsTrack::from_bytes(&bytes).unwrap();
    assert_eq!(track.header(), &header());
    assert_eq!(track.shapes().cloned().collect::<Vec<_>>(), [shape(1, 1), shape(2, 3)]);
    assert_eq!(events_of(track.samples()), samples());
    assert_eq!(events_of(track.mouse()), mouse());
    assert_eq!(events_of(track.chords()), chords());
    assert_eq!(events_of(track.apps()), apps());
    assert_eq!(events_of(track.geometry()), geometry());
    assert!(track.is_complete());
    let counts = Counts { shapes: 2, samples: 6, mouse: 12, chords: 3, apps: 1, geometry: 2 };
    assert_eq!(track.tail(), Some(&Tail { end_qpc: at(1000), counts }));
}

#[test]
fn every_capture_source_round_trips() {
    for source in [CaptureSource::Region(rect(-5, 6, 7, 8)), CaptureSource::Window { handle: -42 }] {
        let header = Header { source, streams: Streams { cursor: true, ..Streams::default() }, ..header() };
        let track = EventsTrack::from_bytes(&write(&header, Vec::new())).unwrap();
        assert_eq!(track.header(), &header);
    }
}

#[test]
fn times_are_microseconds_from_the_first_frame() {
    let track = EventsTrack::from_bytes(&write(&header(), everything())).unwrap();
    let times: Vec<i64> = track.samples().iter().map(|s| s.time_us).collect();
    assert_eq!(times, [-8000, 0, 4000, 9000, 13000, 263_000]);
    assert_eq!(track.chords()[0].time_us, 20_000);
    // Rounded down on both sides of zero, and no overflow after days of uptime.
    let header = Header { qpc_frequency: 3_000_000, first_frame_qpc: i64::MAX / 2, ..header() };
    assert_eq!(header.micros(i64::MAX / 2 + 4), 1);
    assert_eq!(header.micros(i64::MAX / 2 - 4), -2);
    assert_eq!(header.micros(i64::MAX / 2 + 3_000_000 * 86_400), 86_400_000_000);
}

#[test]
fn cursor_and_clicks_are_found_by_time() {
    let track = EventsTrack::from_bytes(&write(&header(), everything())).unwrap();
    assert_eq!(track.cursor_at(-8001), None);
    let before = track.cursor_at(-1).unwrap();
    assert_eq!((before.x, before.y, before.shape.map(|s| s.id)), (100, 200, Some(1)));
    assert_eq!(track.cursor_at(0).unwrap().x, -2560);
    assert_eq!(track.cursor_at(4000).unwrap().shape, Some(&shape(2, 3)));
    let hidden = track.cursor_at(12_999).unwrap();
    assert_eq!((hidden.visible, hidden.shape), (false, None));
    assert_eq!(track.cursor_at(i64::MAX).unwrap().shape, track.shape(1));

    let clicks: Vec<MouseAction> = track.clicks().map(|c| c.event.action).collect();
    assert_eq!(clicks.len(), 5);
    assert!(clicks.iter().all(|action| matches!(action, MouseAction::Down(_))));
}

/// Two flushes of cursor motion: samples 0 to 9, then 10 to 19, with no `TAIL`.
fn two_seconds() -> Vec<u8> {
    let mut writer = EventsWriter::new(Vec::new(), &header()).unwrap();
    for second in 0..2 {
        for n in second * 10..second * 10 + 10 {
            writer.push(InputEvent::Sample(sample(n, n as i32, 0, Some(1)))).unwrap();
        }
        writer.flush_second().unwrap();
    }
    writer.get_ref().clone()
}

#[test]
fn flushed_data_reads_without_a_tail() {
    let mut writer = EventsWriter::new(Vec::new(), &header()).unwrap();
    writer.push(InputEvent::Sample(sample(0, 1, 2, None))).unwrap();
    assert_eq!(kinds(writer.get_ref()), ["HEAD"]);
    writer.flush_second().unwrap();
    writer.flush_second().unwrap();
    assert_eq!(kinds(writer.get_ref()), ["HEAD", "SAMP"], "an empty flush writes nothing");
    let track = EventsTrack::from_bytes(writer.get_ref()).unwrap();
    assert_eq!(track.samples().len(), 1);
    assert!(!track.is_complete());
}

#[test]
fn a_file_cut_mid_chunk_reads_up_to_the_last_whole_chunk() {
    let bytes = two_seconds();
    let ends: Vec<usize> = chunks(&bytes).iter().map(|(_, end)| *end).collect();
    assert_eq!(ends.len(), 3);
    for length in 0..=bytes.len() {
        let read = EventsTrack::from_bytes(&bytes[..length]);
        if length < ends[0] {
            assert!(matches!(read, Err(EventsError::BadHeader)), "at {length}");
            continue;
        }
        let track = read.unwrap();
        let whole_seconds = ends.iter().filter(|end| **end <= length).count() - 1;
        assert_eq!(track.samples().len(), whole_seconds * 10, "at {length}");
        assert!(!track.is_complete());
    }
}

#[test]
fn a_cut_tail_reads_as_incomplete() {
    let bytes = write(&header(), everything());
    let track = EventsTrack::from_bytes(&bytes[..bytes.len() - 1]).unwrap();
    assert!(!track.is_complete());
    assert_eq!(track.geometry().len(), 2);
}

#[test]
fn a_corrupted_chunk_stops_the_reader_there() {
    let bytes = two_seconds();
    let first_end = chunks(&bytes)[1].1;
    let second_start = chunks(&bytes)[0].1;
    // A flip anywhere in the second SAMP chunk: type, length, CRC or payload.
    for offset in first_end..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[offset] ^= 0x40;
        let track = EventsTrack::from_bytes(&damaged).unwrap();
        assert_eq!(track.samples().len(), 10, "at {offset}");
    }
    // Damage in the first one also hides the good chunk after it.
    let mut damaged = bytes.clone();
    damaged[second_start + CHUNK_HEADER_LEN] ^= 1;
    assert!(EventsTrack::from_bytes(&damaged).unwrap().samples().is_empty());
}

#[test]
fn a_malformed_payload_with_a_good_crc_stops_the_reader() {
    for kind in [*b"HEAD", *b"SHAP", *b"SAMP", *b"MOUS", *b"KEYS", *b"APPS", *b"GEOM", *b"TAIL"] {
        let mut bytes = two_seconds();
        // As a count, these bytes claim about four billion records.
        put_chunk(&mut bytes, kind, &[0xff; 40][..39]).unwrap();
        bytes.extend_from_slice(&two_seconds()[chunks(&two_seconds())[0].1..]);
        let track = EventsTrack::from_bytes(&bytes).unwrap();
        assert_eq!(track.samples().len(), 20, "{kind:?}");
        assert!(!track.is_complete());
    }
}

#[test]
fn unknown_chunks_are_skipped() {
    let whole = write(&header(), everything());
    let head_end = chunks(&whole)[0].1;
    let mut bytes = whole[..head_end].to_vec();
    put_chunk(&mut bytes, *b"ZZZZ", b"from a later version").unwrap();
    bytes.extend_from_slice(&whole[head_end..]);
    assert_eq!(kinds(&bytes)[1], "ZZZZ");
    assert_eq!(EventsTrack::from_bytes(&bytes).unwrap(), EventsTrack::from_bytes(&whole).unwrap());
}

#[test]
fn no_keys_chunk_without_chords() {
    let events: Vec<InputEvent> = everything().into_iter().filter(|e| !matches!(e, InputEvent::Chord(_))).collect();
    let bytes = write(&header(), events);
    assert!(!kinds(&bytes).contains(&"KEYS".to_owned()));
    assert!(EventsTrack::from_bytes(&bytes).unwrap().chords().is_empty());
}

#[test]
fn a_stream_that_is_off_is_never_written() {
    let off = Header { streams: Streams::default(), ..header() };
    let bytes = write(&off, everything());
    assert_eq!(kinds(&bytes), ["HEAD", "GEOM", "TAIL"]);

    let keys_off = Header { streams: Streams { keys: false, ..header().streams }, ..header() };
    let bytes = write(&keys_off, everything());
    assert!(!kinds(&bytes).contains(&"KEYS".to_owned()));
    assert_eq!(EventsTrack::from_bytes(&bytes).unwrap().tail().unwrap().counts.chords, 0);
}

#[test]
fn records_that_do_not_fit_are_refused() {
    let mut writer = EventsWriter::new(Vec::new(), &header()).unwrap();
    let wide = CursorShape { width: 257, ..shape(1, 1) };
    let empty = CursorShape { frames: Vec::new(), ..shape(1, 1) };
    let short = CursorShape { height: 49, ..shape(1, 1) };
    for bad in [wide, empty, short] {
        assert!(matches!(writer.push(InputEvent::Shape(bad)), Err(EventsError::InvalidRecord(_))));
    }
    let long = AppSwitch { qpc: 0, exe: "a".repeat(70_000), display_name: String::new() };
    assert!(matches!(writer.push(InputEvent::App(long)), Err(EventsError::InvalidRecord(_))));
    assert_eq!(kinds(&writer.finish(at(0)).unwrap()), ["HEAD", "TAIL"]);

    let stopped = Header { qpc_frequency: 0, ..header() };
    assert!(matches!(EventsWriter::new(Vec::new(), &stopped), Err(EventsError::InvalidRecord(_))));
}

#[test]
fn a_failed_write_ends_the_writer() {
    /// Accepts the header, then fails like a full disk.
    struct Full(usize);
    impl std::io::Write for Full {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.checked_sub(1).ok_or(std::io::ErrorKind::StorageFull)?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = EventsWriter::new(Full(1), &header()).unwrap();
    writer.push(InputEvent::Sample(sample(0, 0, 0, None))).unwrap();
    assert!(matches!(writer.flush_second(), Err(EventsError::Io(_))));
    assert_eq!(writer.counts(), Counts::default());
    assert!(matches!(writer.push(InputEvent::Sample(sample(1, 0, 0, None))), Err(EventsError::Broken)));
    assert!(matches!(writer.finish(at(1)), Err(EventsError::Broken)));
}

#[test]
fn other_files_and_versions_are_errors() {
    assert!(matches!(EventsTrack::from_bytes(b"not an events file at all"), Err(EventsError::BadHeader)));
    let whole = write(&header(), Vec::new());
    let mut payload = whole[CHUNK_HEADER_LEN..chunks(&whole)[0].1].to_vec();
    payload[5] = 2;
    let mut later = Vec::new();
    put_chunk(&mut later, *b"HEAD", &payload).unwrap();
    assert!(matches!(EventsTrack::from_bytes(&later), Err(EventsError::UnsupportedVersion(2))));
}

#[test]
fn a_file_on_disk_reads_back_and_is_never_overwritten() {
    let path = std::env::temp_dir().join(format!("cutmark-events-test-{}.events", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut writer = EventsWriter::create(&path, &header()).unwrap();
    writer.push(InputEvent::Sample(sample(0, 1, 2, None))).unwrap();
    writer.flush_second().unwrap();
    // Readable while the writer still holds it open, as after a crash.
    let during = EventsTrack::open(&path);
    drop(writer.finish(at(5)).unwrap());
    let after = EventsTrack::open(&path);
    let again = EventsWriter::create(&path, &header()).map(|_| ());
    std::fs::remove_file(&path).unwrap();

    let during = during.unwrap();
    assert_eq!((during.samples().len(), during.is_complete()), (1, false));
    assert!(after.unwrap().is_complete());
    assert!(matches!(again, Err(EventsError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists));
}

#[test]
fn an_hour_of_motion_stays_under_5_mb() {
    // The worst case for size: the cursor moves at every 4 ms sample for the whole hour, so all
    // 900,000 are written, with up to 50 us of timer jitter and uneven steps.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut random = move |range: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state % range) as i64
    };
    let mut writer = EventsWriter::new(Vec::new(), &header()).unwrap();
    writer.push(InputEvent::Shape(shape(1, 1))).unwrap();
    writer.push(InputEvent::Shape(shape(2, 1))).unwrap();
    let (mut x, mut y) = (1280i32, 720i32);
    let mut last = None;
    for n in 0..900_000i64 {
        let qpc = FIRST_FRAME + n * 40_000 + random(1001) - 500;
        // Drift one way for two seconds, then the other.
        let drift = if (n / 500) % 2 == 0 { 3 } else { -3 };
        x += drift + random(5) as i32 - 2;
        y += random(7) as i32 - 3;
        let sample = CursorSample { qpc, x, y, shape: Some(1 + ((n / 2000) % 2) as u32), visible: true };
        writer.push(InputEvent::Sample(sample)).unwrap();
        last = Some(sample);
        if n % 250 == 249 {
            writer.flush_second().unwrap();
        }
    }
    let bytes = writer.finish(at(3_600_000)).unwrap();
    println!("one hour of motion: {} bytes", bytes.len());
    assert!(bytes.len() < 5_000_000, "{} bytes", bytes.len());
    let track = EventsTrack::from_bytes(&bytes).unwrap();
    assert_eq!(track.samples().len(), 900_000);
    assert_eq!(track.samples().last().map(|s| s.event), last);
    assert!(track.is_complete());
}
