//! From desktop pixels to frame pixels.
//!
//! This follows how the Recorder makes the video. Windows Graphics Capture fills a buffer that keeps
//! the size the source had when the capture started, because the capture never recreates its frame
//! pool. A source that changes size later therefore sits unscaled at the buffer's top left corner:
//! a larger one is cut off, and a smaller one leaves the rest of the buffer without a picture. The
//! compositor then fits the whole buffer into the video, centred, as `capture::fit_rect` does.

use events::{CaptureSource, DisplayInfo, EventsReader, GeometryChange, ScreenRect};

/// A rectangle in frame pixels. The right and bottom edges are outside it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub left: f64,
    /// Top edge.
    pub top: f64,
    /// Right edge.
    pub right: f64,
    /// Bottom edge.
    pub bottom: f64,
}

/// What the events file cannot say about a take: the size of its video.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureMap {
    video_width: u32,
    video_height: u32,
}

/// The mapping at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    /// The desktop position of the capture buffer's top left pixel.
    origin: (i32, i32),
    /// The part of the buffer that shows the source, from its top left corner, in desktop pixels.
    visible: (i32, i32),
    /// The frame position of the buffer's top left corner.
    offset: (f64, f64),
    /// Frame pixels per desktop pixel.
    scale: (f64, f64),
}

impl Projection {
    /// Whether a desktop position is in the captured picture.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        let (dx, dy) = (x - f64::from(self.origin.0), y - f64::from(self.origin.1));
        dx >= 0.0 && dy >= 0.0 && dx < f64::from(self.visible.0) && dy < f64::from(self.visible.1)
    }

    /// The frame position of a desktop position, which may be outside the captured picture.
    pub fn to_frame(&self, x: f64, y: f64) -> (f64, f64) {
        (self.offset.0 + (x - f64::from(self.origin.0)) * self.scale.0, self.offset.1 + (y - f64::from(self.origin.1)) * self.scale.1)
    }

    /// Frame pixels per desktop pixel, horizontal and vertical.
    pub fn scale(&self) -> (f64, f64) {
        self.scale
    }

    /// The captured picture's place in the frame. Nothing is drawn outside it.
    pub fn area(&self) -> Rect {
        let (right, bottom) = (self.offset.0 + f64::from(self.visible.0) * self.scale.0, self.offset.1 + f64::from(self.visible.1) * self.scale.1);
        Rect { left: self.offset.0, top: self.offset.1, right, bottom }
    }
}

impl CaptureMap {
    /// A map for a take whose video has this size in pixels. A frame scaled down for playback is
    /// handled when it is projected, so this is the size of the file's picture.
    pub fn new(video_width: u32, video_height: u32) -> Self {
        Self { video_width, video_height }
    }

    /// The mapping at a time, for a frame of the given size. None when nothing can be placed: a size
    /// is zero, the captured monitor is gone, or a window capture has no window rectangle.
    pub fn project(&self, events: &EventsReader, time_us: i64, frame_width: u32, frame_height: u32) -> Option<Projection> {
        let (buffer, current) = source_at(events, time_us)?;
        if buffer.width <= 0 || buffer.height <= 0 || self.video_width == 0 || self.video_height == 0 || frame_width == 0 || frame_height == 0 {
            return None;
        }
        let visible = (current.width.clamp(0, buffer.width), current.height.clamp(0, buffer.height));
        let (left, top, width, height) = fit(buffer.width as u32, buffer.height as u32, self.video_width, self.video_height);
        let to_frame = (f64::from(frame_width) / f64::from(self.video_width), f64::from(frame_height) / f64::from(self.video_height));
        Some(Projection {
            origin: (current.x, current.y),
            visible,
            offset: (f64::from(left) * to_frame.0, f64::from(top) * to_frame.1),
            scale: (f64::from(width) / f64::from(buffer.width) * to_frame.0, f64::from(height) / f64::from(buffer.height) * to_frame.1),
        })
    }
}

/// The largest centred rectangle with the source's aspect ratio that fits the video, as left, top,
/// width and height. The rounding is the same as in `capture::fit_rect`. No size may be zero.
fn fit(source_width: u32, source_height: u32, video_width: u32, video_height: u32) -> (u32, u32, u32, u32) {
    let [sw, sh, vw, vh] = [source_width, source_height, video_width, video_height].map(u64::from);
    let (width, height) = if sw * vh > vw * sh { (vw, ((vw * sh + sw / 2) / sw).clamp(1, vh)) } else { (((vh * sw + sh / 2) / sh).clamp(1, vw), vh) };
    (((vw - width) / 2) as u32, ((vh - height) / 2) as u32, width as u32, height as u32)
}

/// The capture buffer's rectangle as it was when the capture started (only its size is used), and
/// the source's rectangle at a time.
fn source_at(events: &EventsReader, time_us: i64) -> Option<(ScreenRect, ScreenRect)> {
    match events.header().source {
        CaptureSource::Region(region) => Some((region, region)),
        CaptureSource::Monitor(monitor) => monitor_at(events, monitor, time_us).map(|current| (monitor, current)),
        CaptureSource::Window { .. } => {
            let rects = || {
                events.geometry().iter().filter_map(|geometry| match geometry.event.change {
                    GeometryChange::WindowRect(rect) => Some((geometry.time_us, rect)),
                    GeometryChange::Displays(_) => None,
                })
            };
            // The window at the first video frame sized the buffer. A track that only learns of the
            // window later uses the first rectangle it has.
            let first = rects().take_while(|(at, _)| *at <= 0).last().or_else(|| rects().next())?.1;
            let current = rects().take_while(|(at, _)| *at <= time_us).last().map_or(first, |(_, rect)| rect);
            Some((first, current))
        }
    }
}

/// Where the captured monitor is at a time. Monitors are followed by name, because a layout change
/// can move one. None once it is no longer in the layout.
fn monitor_at(events: &EventsReader, captured: ScreenRect, time_us: i64) -> Option<ScreenRect> {
    let at_start = &events.header().displays;
    let same_place = |display: &&DisplayInfo| (display.rect.x, display.rect.y) == (captured.x, captured.y);
    // The size can differ when the resolution changed between the capture's start and the take's.
    let Some(name) = at_start.iter().find(|display| display.rect == captured).or_else(|| at_start.iter().find(same_place)).map(|display| &display.name) else {
        return Some(captured);
    };
    let changed = events.geometry().iter().take_while(|geometry| geometry.time_us <= time_us).filter_map(|geometry| match &geometry.event.change {
        GeometryChange::Displays(displays) => Some(displays),
        GeometryChange::WindowRect(_) => None,
    });
    changed.last().unwrap_or(at_start).iter().find(|display| &display.name == name).map(|display| display.rect)
}
