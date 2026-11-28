//! What one frame gets, decided without touching pixels. The CPU path draws a [`Plan`], and a GPU
//! path can draw the same one.

use events::{CursorShape, EventsReader, MouseAction, ShapeFrame};

use crate::CursorStyle;
use crate::map::{CaptureMap, Rect};

/// How long a click ring shows, from the moment the button goes down.
pub const CLICK_RING_US: i64 = 400_000;

/// The width of the smoothing average. It is centred on the frame's time, so the cursor does not
/// lag behind the video.
pub const SMOOTHING_WINDOW_US: i64 = 50_000;

/// A ring's radius when it appears and when it goes, in desktop pixels at scale 1.
const RING_RADIUS: (f64, f64) = (10.0, 24.0);
/// A ring's line width in desktop pixels at scale 1.
const RING_WIDTH: f64 = 3.0;
/// The thinnest ring line in frame pixels, so a scaled down frame still shows it.
const RING_MIN_WIDTH: f64 = 1.0;
/// A ring's opacity when it appears; it fades to nothing.
const RING_OPACITY: f64 = 0.9;

/// One click ring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingLayer {
    /// The centre in frame pixels, horizontal.
    pub x: f64,
    /// The centre, vertical.
    pub y: f64,
    /// The radius to the middle of the line, in frame pixels.
    pub radius: f64,
    /// The line width in frame pixels.
    pub width: f64,
    /// 0 is invisible and 1 is solid.
    pub opacity: f64,
    /// The captured picture's place in the frame.
    pub clip: Rect,
}

/// The cursor picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorLayer<'a> {
    /// The shape, which gives the picture's size in pixels.
    pub shape: &'a CursorShape,
    /// The animation frame to show: premultiplied BGRA.
    pub picture: &'a ShapeFrame,
    /// Where the whole picture goes in frame pixels. It can reach past the clip.
    pub dest: Rect,
    /// The captured picture's place in the frame.
    pub clip: Rect,
}

/// Everything drawn on one frame: the rings first, then the cursor over them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan<'a> {
    /// The click rings, oldest first.
    pub rings: Vec<RingLayer>,
    /// The cursor; None when it is hidden, outside the captured picture or has no shape.
    pub cursor: Option<CursorLayer<'a>>,
}

/// What a frame of the given size gets at `time_us`, in microseconds from the first video frame.
pub fn plan<'a>(events: &'a EventsReader, time_us: i64, map: &CaptureMap, style: &CursorStyle, frame_width: u32, frame_height: u32) -> Plan<'a> {
    let rings = if style.click_ring { rings(events, time_us, map, style, (frame_width, frame_height)) } else { Vec::new() };
    Plan { rings, cursor: cursor(events, time_us, map, style, (frame_width, frame_height)) }
}

fn cursor<'a>(events: &'a EventsReader, time_us: i64, map: &CaptureMap, style: &CursorStyle, frame: (u32, u32)) -> Option<CursorLayer<'a>> {
    let state = events.cursor_at(time_us)?;
    let shape = state.shape.filter(|_| state.visible)?;
    let (x, y) = match style.smoothing.then(|| smoothed(events, time_us)).flatten() {
        Some(position) => position,
        None => (f64::from(state.x), f64::from(state.y)),
    };
    let projection = map.project(events, time_us, frame.0, frame.1)?;
    if !projection.contains(x, y) {
        return None;
    }
    let (x, y) = projection.to_frame(x, y);
    let scale = f64::from(style.effective_scale());
    let (kx, ky) = (projection.scale().0 * scale, projection.scale().1 * scale);
    // The hotspot stays on the recorded position at every scale.
    let (left, top) = (x - f64::from(shape.hotspot_x) * kx, y - f64::from(shape.hotspot_y) * ky);
    let dest = Rect { left, top, right: left + f64::from(shape.width) * kx, bottom: top + f64::from(shape.height) * ky };
    let picture = shape.frames.get(animation_frame(shape, time_us)?)?;
    Some(CursorLayer { shape, picture, dest, clip: projection.area() })
}

/// The index of the frame an animated cursor shows; None for a shape without frames. The animation
/// runs on the video's clock, so the same time always gives the same frame. The Studio renderer
/// uses this too.
pub fn animation_frame(shape: &CursorShape, time_us: i64) -> Option<usize> {
    let total: i64 = shape.frames.iter().map(|frame| i64::from(frame.duration_us)).sum();
    if total == 0 {
        return (!shape.frames.is_empty()).then_some(0);
    }
    let mut left = time_us.rem_euclid(total);
    shape.frames.iter().position(|frame| {
        let shows = left < i64::from(frame.duration_us);
        left -= i64::from(frame.duration_us);
        shows
    })
}

/// The cursor's average position over the smoothing window. Each sample counts for as long as it
/// held, since samples are only written on change. The average stays within the unbroken visible
/// stretch around `time_us`, so a place where the cursor was hidden never pulls on it.
fn smoothed(events: &EventsReader, time_us: i64) -> Option<(f64, f64)> {
    let samples = events.samples();
    let current = samples.partition_point(|sample| sample.time_us <= time_us).checked_sub(1)?;
    let half = SMOOTHING_WINDOW_US / 2;
    let (from, to) = (time_us.saturating_sub(half), time_us.saturating_add(half));
    let (mut x, mut y, mut total) = (0.0, 0.0, 0.0);
    // Adds what one sample held inside the window; false once the stretch or the window ends.
    let mut add = |index: usize| -> bool {
        let Some(sample) = samples.get(index).filter(|sample| sample.event.visible) else { return false };
        let until = samples.get(index + 1).map_or(to, |next| next.time_us);
        let held = until.min(to).saturating_sub(sample.time_us.max(from));
        if held > 0 {
            x += f64::from(sample.event.x) * held as f64;
            y += f64::from(sample.event.y) * held as f64;
            total += held as f64;
        }
        sample.time_us < to && until > from
    };
    for index in (0..=current).rev() {
        if !add(index) {
            break;
        }
    }
    for index in current + 1..samples.len() {
        if !add(index) {
            break;
        }
    }
    (total > 0.0).then(|| (x / total, y / total))
}

/// The rings of the buttons that went down within the last [`CLICK_RING_US`]. Each stays where it
/// was clicked in the picture, even when a window moves afterwards.
fn rings(events: &EventsReader, time_us: i64, map: &CaptureMap, style: &CursorStyle, frame: (u32, u32)) -> Vec<RingLayer> {
    let mouse = events.mouse();
    let first = mouse.partition_point(|event| event.time_us <= time_us.saturating_sub(CLICK_RING_US));
    let recent = mouse.iter().skip(first).take_while(|event| event.time_us <= time_us);
    recent
        .filter(|event| matches!(event.event.action, MouseAction::Down(_)))
        .filter_map(|click| {
            let projection = map.project(events, click.time_us, frame.0, frame.1)?;
            let (x, y) = (f64::from(click.event.x), f64::from(click.event.y));
            if !projection.contains(x, y) {
                return None;
            }
            let (x, y) = projection.to_frame(x, y);
            let age = time_us.saturating_sub(click.time_us) as f64 / CLICK_RING_US as f64;
            let k = (projection.scale().0 + projection.scale().1) / 2.0 * f64::from(style.effective_scale());
            let radius = (RING_RADIUS.0 + (RING_RADIUS.1 - RING_RADIUS.0) * age) * k;
            Some(RingLayer { x, y, radius, width: (RING_WIDTH * k).max(RING_MIN_WIDTH), opacity: RING_OPACITY * (1.0 - age), clip: projection.area() })
        })
        .collect()
}
