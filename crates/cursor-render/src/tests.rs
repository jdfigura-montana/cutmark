use events::{
    CaptureSource, CursorSample, CursorShape, DisplayInfo, EventsReader, EventsWriter, GeometryChange, GeometryEvent, Header, InputEvent, MouseAction,
    MouseButton, MouseEvent, ScreenRect, ShapeFrame, Streams,
};
use media_io::player::video::RgbaFrame;

use crate::{CLICK_RING_US, CaptureMap, CursorStyle, plan, render};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The shape is 4 by 4 with its hotspot at (1, 2). The hotspot pixel is red, the rest blue.
const HOTSPOT: (u32, u32) = (1, 2);
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GRAY: [u8; 4] = [100, 100, 100, 255];

fn rect(x: i32, y: i32, width: i32, height: i32) -> ScreenRect {
    ScreenRect { x, y, width, height }
}

fn display(name: &str, rect: ScreenRect) -> DisplayInfo {
    DisplayInfo { name: name.to_owned(), rect, dpi: 96, primary: rect.x == 0 && rect.y == 0 }
}

fn shape(id: u32, frames: Vec<ShapeFrame>) -> InputEvent {
    InputEvent::Shape(CursorShape { id, width: 4, height: 4, hotspot_x: HOTSPOT.0, hotspot_y: HOTSPOT.1, frames })
}

/// The test shape's picture, as BGRA.
fn arrow() -> InputEvent {
    let bgra = (0..16).flat_map(|at| if (at % 4, at / 4) == HOTSPOT { [0, 0, 255, 255] } else { [255, 0, 0, 255] }).collect();
    shape(1, vec![ShapeFrame { duration_us: 0, bgra }])
}

// The test clock ticks once per microsecond and the first frame is at zero, so a QPC stamp is a time.
fn sample(time_us: i64, x: i32, y: i32) -> InputEvent {
    InputEvent::Sample(CursorSample { qpc: time_us, x, y, shape: Some(1), visible: true })
}

fn mouse(time_us: i64, x: i32, y: i32, action: MouseAction) -> InputEvent {
    InputEvent::Mouse(MouseEvent { qpc: time_us, x, y, action })
}

fn window(time_us: i64, rect: ScreenRect) -> InputEvent {
    InputEvent::Geometry(GeometryEvent { qpc: time_us, change: GeometryChange::WindowRect(rect) })
}

fn displays(time_us: i64, displays: Vec<DisplayInfo>) -> InputEvent {
    InputEvent::Geometry(GeometryEvent { qpc: time_us, change: GeometryChange::Displays(displays) })
}

fn track_on(source: CaptureSource, layout: Vec<DisplayInfo>, events: Vec<InputEvent>) -> Result<EventsReader, events::EventsError> {
    let header = Header {
        qpc_frequency: 1_000_000,
        first_frame_qpc: 0,
        source,
        displays: layout,
        cursor_size: 32,
        streams: Streams { cursor: true, clicks: true, keys: false, apps: false },
    };
    let mut writer = EventsWriter::new(Vec::new(), &header)?;
    for event in events {
        writer.push(event)?;
    }
    EventsReader::from_bytes(&writer.finish(10_000_000)?)
}

fn track(source: CaptureSource, events: Vec<InputEvent>) -> Result<EventsReader, events::EventsError> {
    track_on(source, Vec::new(), events)
}

/// A 100 by 100 monitor with the arrow and the events.
fn monitor_track(mut events: Vec<InputEvent>) -> Result<EventsReader, events::EventsError> {
    events.insert(0, arrow());
    track(CaptureSource::Monitor(rect(0, 0, 100, 100)), events)
}

fn gray(width: u32, height: u32) -> RgbaFrame {
    RgbaFrame { width, height, pts_ms: 0, data: GRAY.repeat((width * height) as usize) }
}

fn drawn(events: &EventsReader, time_us: i64, video: (u32, u32), frame: (u32, u32), style: CursorStyle) -> RgbaFrame {
    let mut picture = gray(frame.0, frame.1);
    render(&mut picture, events, time_us, &CaptureMap::new(video.0, video.1), &style);
    picture
}

fn pixel(frame: &RgbaFrame, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * frame.width + x) * 4) as usize;
    [frame.data[at], frame.data[at + 1], frame.data[at + 2], frame.data[at + 3]]
}

/// The pixels that are no longer gray, as left, top, right and bottom with the last two outside.
fn changed(frame: &RgbaFrame) -> Option<(u32, u32, u32, u32)> {
    let hits = (0..frame.height).flat_map(|y| (0..frame.width).map(move |x| (x, y))).filter(|&(x, y)| pixel(frame, x, y) != GRAY);
    hits.fold(None, |found, (x, y)| {
        let (left, top, right, bottom) = found.unwrap_or((x, y, x + 1, y + 1));
        Some((left.min(x), top.min(y), right.max(x + 1), bottom.max(y + 1)))
    })
}

/// Where the cursor's hotspot pixel landed.
fn red_at(frame: &RgbaFrame) -> Option<(u32, u32)> {
    (0..frame.height).flat_map(|y| (0..frame.width).map(move |x| (x, y))).find(|&(x, y)| pixel(frame, x, y) == RED)
}

fn plain() -> CursorStyle {
    CursorStyle::default()
}

fn ringed() -> CursorStyle {
    CursorStyle { click_ring: true, ..CursorStyle::default() }
}

#[test]
fn the_default_style_is_the_cursor_as_recorded() {
    assert_eq!(plain(), CursorStyle { scale: 1.0, smoothing: false, click_ring: false });
}

#[test]
fn the_hotspot_lands_on_the_recorded_position() -> TestResult {
    let events = monitor_track(vec![sample(0, 40, 50)])?;
    let frame = drawn(&events, 0, (100, 100), (100, 100), plain());
    assert_eq!(pixel(&frame, 40, 50), RED);
    assert_eq!(changed(&frame), Some((39, 48, 43, 52)));
    assert_eq!(pixel(&frame, 39, 48), BLUE);
    assert_eq!(pixel(&frame, 42, 51), BLUE);
    Ok(())
}

#[test]
fn premultiplied_alpha_is_blended_over_the_frame() -> TestResult {
    // Half covered pure green.
    let picture = ShapeFrame { duration_us: 0, bgra: [0, 128, 0, 128].repeat(16) };
    let events = track(CaptureSource::Monitor(rect(0, 0, 100, 100)), vec![shape(1, vec![picture]), sample(0, 40, 50)])?;
    let frame = drawn(&events, 0, (100, 100), (100, 100), plain());
    assert_eq!(pixel(&frame, 40, 50), [50, 178, 50, 255]);
    Ok(())
}

#[test]
fn nothing_is_drawn_before_the_first_sample_or_while_hidden_or_shapeless() -> TestResult {
    let hidden = InputEvent::Sample(CursorSample { qpc: 200, x: 40, y: 50, shape: Some(1), visible: false });
    let shapeless = InputEvent::Sample(CursorSample { qpc: 300, x: 40, y: 50, shape: None, visible: true });
    let events = monitor_track(vec![sample(100, 40, 50), hidden, shapeless])?;
    for time in [99, 200, 300] {
        assert_eq!(changed(&drawn(&events, time, (100, 100), (100, 100), plain())), None, "at {time}");
    }
    assert!(changed(&drawn(&events, 100, (100, 100), (100, 100), plain())).is_some());
    Ok(())
}

#[test]
fn a_region_is_mapped_from_its_own_corner() -> TestResult {
    let events = track(CaptureSource::Region(rect(-200, 300, 100, 80)), vec![arrow(), sample(0, -190, 320)])?;
    assert_eq!(red_at(&drawn(&events, 0, (100, 80), (100, 80), plain())), Some((10, 20)));
    Ok(())
}

#[test]
fn the_picture_is_fitted_into_the_video_and_follows_a_scaled_down_frame() -> TestResult {
    // A 200 by 100 region in a 200 by 200 video leaves bars of 50 above and below.
    let events = track(CaptureSource::Region(rect(1000, 500, 200, 100)), vec![arrow(), sample(0, 1080, 540)])?;
    let map = CaptureMap::new(200, 200);
    let full = map.project(&events, 0, 200, 200).ok_or("no projection")?;
    assert_eq!(full.to_frame(1080.0, 540.0), (80.0, 90.0));
    assert_eq!(full.scale(), (1.0, 1.0));
    let half = plan(&events, 0, &map, &plain(), 100, 100).cursor.ok_or("no cursor")?;
    // The hotspot is at (40, 45), and the cursor is half as large as in the full frame.
    assert_eq!((half.dest.left, half.dest.top, half.dest.right, half.dest.bottom), (39.5, 44.0, 41.5, 46.0));
    assert_eq!((half.clip.left, half.clip.top, half.clip.right, half.clip.bottom), (0.0, 25.0, 100.0, 75.0));
    Ok(())
}

#[test]
fn the_cursor_is_not_drawn_into_the_bars_beside_the_picture() -> TestResult {
    // At the top edge of the picture, the two rows above the hotspot would fall into the bar.
    let events = track(CaptureSource::Region(rect(0, 0, 100, 50)), vec![arrow(), sample(0, 40, 0)])?;
    let frame = drawn(&events, 0, (100, 100), (100, 100), plain());
    assert_eq!(changed(&frame), Some((39, 25, 43, 27)));
    Ok(())
}

#[test]
fn a_cursor_outside_the_capture_area_is_not_drawn() -> TestResult {
    // Each position is one pixel past an edge; the picture around it would still overlap the area.
    for (x, y) in [(99, 220), (200, 220), (150, 199), (150, 280)] {
        let events =
            track(CaptureSource::Region(rect(100, 200, 100, 80)), vec![arrow(), sample(0, x, y), mouse(0, x, y, MouseAction::Down(MouseButton::Left))])?;
        assert_eq!(changed(&drawn(&events, 0, (100, 80), (100, 80), ringed())), None, "at {x},{y}");
    }
    let corner = track(CaptureSource::Region(rect(100, 200, 100, 80)), vec![arrow(), sample(0, 199, 279)])?;
    assert_eq!(red_at(&drawn(&corner, 0, (100, 80), (100, 80), plain())), Some((99, 79)));
    Ok(())
}

#[test]
fn a_moving_window_is_followed() -> TestResult {
    let events = track(
        CaptureSource::Window { handle: 7 },
        vec![arrow(), window(0, rect(300, 200, 100, 80)), sample(0, 310, 220), window(1_000, rect(500, 100, 100, 80)), sample(1_000, 530, 140)],
    )?;
    assert_eq!(red_at(&drawn(&events, 999, (100, 80), (100, 80), plain())), Some((10, 20)));
    assert_eq!(red_at(&drawn(&events, 1_000, (100, 80), (100, 80), plain())), Some((30, 40)));
    Ok(())
}

#[test]
fn a_resized_window_stays_unscaled_at_the_top_left_of_its_first_size() -> TestResult {
    let events = track(
        CaptureSource::Window { handle: 7 },
        vec![
            arrow(),
            window(0, rect(0, 0, 100, 80)),
            sample(0, 60, 40),
            window(1_000, rect(0, 0, 50, 80)),
            window(2_000, rect(0, 0, 200, 160)),
            sample(3_000, 150, 40),
        ],
    )?;
    let red = |time| red_at(&drawn(&events, time, (100, 80), (100, 80), plain()));
    assert_eq!(red(0), Some((60, 40)));
    // Narrower than the cursor's position: that part of the buffer no longer shows the window.
    assert_eq!(red(1_000), None);
    assert_eq!(red(2_000), Some((60, 40)));
    // Past the buffer, which kept its first size.
    assert_eq!(red(3_000), None);
    Ok(())
}

#[test]
fn a_window_track_without_a_window_rectangle_draws_nothing() -> TestResult {
    let events = track(CaptureSource::Window { handle: 7 }, vec![arrow(), sample(0, 10, 10)])?;
    assert_eq!(changed(&drawn(&events, 0, (100, 80), (100, 80), plain())), None);
    Ok(())
}

#[test]
fn a_monitor_is_followed_by_name_through_display_changes() -> TestResult {
    let left = |rect| display("left", rect);
    let main = display("main", rect(0, 0, 100, 100));
    let events = track_on(
        CaptureSource::Monitor(rect(100, 0, 100, 100)),
        vec![main.clone(), left(rect(100, 0, 100, 100))],
        vec![
            arrow(),
            sample(0, 130, 40),
            // The captured monitor moves and drops to half its resolution.
            displays(1_000, vec![main.clone(), left(rect(-50, 0, 50, 50))]),
            sample(1_000, -20, 40),
            sample(1_500, -20, 60),
            // Then it is unplugged.
            displays(2_000, vec![main]),
            sample(2_000, 130, 40),
        ],
    )?;
    let red = |time| red_at(&drawn(&events, time, (100, 100), (100, 100), plain()));
    assert_eq!(red(0), Some((30, 40)));
    assert_eq!(red(1_000), Some((30, 40)));
    assert_eq!(red(1_500), None);
    assert_eq!(red(2_000), None);
    Ok(())
}

#[test]
fn a_click_ring_shows_for_the_frames_after_a_button_goes_down() -> TestResult {
    let down = 1_000_000;
    // The cursor is hidden, so every changed pixel is the ring's.
    let hidden = InputEvent::Sample(CursorSample { qpc: 0, x: 50, y: 50, shape: None, visible: false });
    let events = monitor_track(vec![
        hidden,
        mouse(down, 50, 50, MouseAction::Down(MouseButton::Right)),
        mouse(down + 50_000, 50, 50, MouseAction::Up(MouseButton::Right)),
    ])?;
    let ring = |time| changed(&drawn(&events, time, (100, 100), (100, 100), ringed()));
    assert_eq!(ring(down - 1), None);
    assert!(ring(down).is_some());
    assert!(ring(down + CLICK_RING_US - 10_000).is_some());
    assert_eq!(ring(down + CLICK_RING_US), None);
    // The ring grows around the click and leaves its middle alone.
    let (first, later) = (ring(down).ok_or("no ring")?, ring(down + CLICK_RING_US / 2).ok_or("no ring")?);
    assert!(later.0 < first.0 && later.2 > first.2, "{first:?} {later:?}");
    assert_eq!((first.0 + first.2, first.1 + first.3), (100, 100));
    assert_eq!(pixel(&drawn(&events, down, (100, 100), (100, 100), ringed()), 50, 50), GRAY);
    assert_eq!(changed(&drawn(&events, down, (100, 100), (100, 100), plain())), None);
    Ok(())
}

#[test]
fn only_a_button_going_down_makes_a_ring() -> TestResult {
    let events = monitor_track(vec![
        mouse(0, 50, 50, MouseAction::Up(MouseButton::Left)),
        mouse(0, 50, 50, MouseAction::Wheel(120)),
        mouse(0, 50, 50, MouseAction::HorizontalWheel(-120)),
    ])?;
    assert!(plan(&events, 0, &CaptureMap::new(100, 100), &ringed(), 100, 100).rings.is_empty());
    Ok(())
}

#[test]
fn a_ring_stays_where_it_was_clicked_when_the_window_moves() -> TestResult {
    let events = track(
        CaptureSource::Window { handle: 7 },
        vec![window(0, rect(0, 0, 100, 80)), mouse(0, 20, 30, MouseAction::Down(MouseButton::Left)), window(1_000, rect(40, 40, 100, 80))],
    )?;
    let plan = plan(&events, 100_000, &CaptureMap::new(100, 80), &ringed(), 100, 80);
    assert_eq!(plan.rings.iter().map(|ring| (ring.x, ring.y)).collect::<Vec<_>>(), [(20.0, 30.0)]);
    Ok(())
}

#[test]
fn scale_enlarges_the_cursor_around_its_hotspot() -> TestResult {
    let events = monitor_track(vec![sample(0, 40, 50)])?;
    let frame = drawn(&events, 0, (100, 100), (100, 100), CursorStyle { scale: 2.0, ..plain() });
    assert_eq!(changed(&frame), Some((38, 46, 46, 54)));
    // The red hotspot pixel covers two by two, from the recorded position.
    for (x, y) in [(40, 50), (41, 50), (40, 51), (41, 51)] {
        assert_eq!(pixel(&frame, x, y), RED, "{x},{y}");
    }
    assert_eq!(pixel(&frame, 39, 50), BLUE);
    assert_eq!(pixel(&frame, 42, 50), BLUE);
    Ok(())
}

#[test]
fn a_scale_out_of_range_is_held_to_it() -> TestResult {
    let events = monitor_track(vec![sample(0, 40, 50)])?;
    let width = |scale| changed(&drawn(&events, 0, (100, 100), (100, 100), CursorStyle { scale, ..plain() })).map(|(left, _, right, _)| right - left);
    assert_eq!(width(0.1), Some(4));
    assert_eq!(width(f32::NAN), Some(4));
    assert_eq!(width(3.0), Some(12));
    assert_eq!(width(9.0), Some(12));
    Ok(())
}

#[test]
fn smoothing_averages_the_path_around_the_frame_time() -> TestResult {
    // A steady move of 10 pixels every 10 ms, then a jump while hidden.
    let mut events: Vec<InputEvent> = (0..=6).map(|step| sample(i64::from(step) * 10_000, 10 + step * 10, 50)).collect();
    events.push(InputEvent::Sample(CursorSample { qpc: 70_000, x: 0, y: 0, shape: Some(1), visible: false }));
    events.push(sample(80_000, 90, 90));
    let events = monitor_track(events)?;
    let smooth = CursorStyle { smoothing: true, ..plain() };
    let at = |time, style| red_at(&drawn(&events, time, (100, 100), (100, 100), style));
    assert_eq!(at(30_000, plain()), Some((40, 50)));
    // From 5 ms to 55 ms: 5 ms at 10, 10 ms each at 20 to 50 and 5 ms at 60 average to 35.
    let cursor = plan(&events, 30_000, &CaptureMap::new(100, 100), &smooth, 100, 100).cursor.ok_or("no cursor")?;
    assert_eq!(cursor.dest.left + f64::from(HOTSPOT.0), 35.0);
    // Neither the hidden position nor the one after it pulls on the cursor on either side.
    assert_eq!(at(80_000, smooth), Some((90, 90)));
    let before_hidden = plan(&events, 69_000, &CaptureMap::new(100, 100), &smooth, 100, 100).cursor.ok_or("no cursor")?;
    assert_eq!(before_hidden.dest.top + f64::from(HOTSPOT.1), 50.0);
    Ok(())
}

#[test]
fn an_animated_cursor_shows_the_frame_of_the_time() -> TestResult {
    let solid = |value: u8| ShapeFrame { duration_us: 100, bgra: [value, value, value, 255].repeat(16) };
    let events = track(CaptureSource::Monitor(rect(0, 0, 100, 100)), vec![shape(1, vec![solid(10), solid(20)]), sample(0, 40, 50)])?;
    let shade = |time| pixel(&drawn(&events, time, (100, 100), (100, 100), plain()), 40, 50)[0];
    assert_eq!([shade(0), shade(99), shade(100), shade(199), shade(200)], [10, 10, 20, 20, 10]);
    Ok(())
}

#[test]
fn a_frame_with_too_few_bytes_is_left_without_a_crash() -> TestResult {
    let events = monitor_track(vec![sample(0, 40, 50)])?;
    let mut short = RgbaFrame { width: 100, height: 100, pts_ms: 0, data: vec![0; 10] };
    render(&mut short, &events, 0, &CaptureMap::new(100, 100), &ringed());
    assert_eq!(short.data, vec![0; 10]);
    Ok(())
}
