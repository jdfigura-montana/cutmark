//! The input logger: cursor samples, cursor shapes, mouse buttons and wheel, and geometry changes,
//! all stamped on the same QPC clock as capture.
//!
//! See the input track spec: Recording, items 1 to 4 and 7. Positions and rectangles are physical
//! virtual desktop pixels (Per-Monitor DPI Aware v2).

// The logger that drives the filter and the shape cache exists only on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
mod filter;
#[cfg_attr(not(windows), allow(dead_code))]
mod shape;

#[cfg(windows)]
mod logger;
#[cfg(windows)]
mod render;
#[cfg(not(windows))]
mod stub;

#[cfg(windows)]
pub use logger::InputLogger;
#[cfg(not(windows))]
pub use stub::InputLogger;

/// A rectangle in physical virtual desktop pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
}

/// What the take captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    /// A whole monitor.
    Monitor(ScreenRect),
    /// A fixed region of the desktop.
    Region(ScreenRect),
    /// One window; the logger reports its rectangle whenever it moves or resizes.
    Window {
        /// The HWND as an isize.
        handle: isize,
    },
}

/// What to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputOptions {
    /// What the take captures.
    pub source: CaptureSource,
    /// Whether to log mouse buttons and the wheel.
    pub clicks: bool,
}

/// One monitor in the display layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Display device name, for example \\.\DISPLAY1.
    pub name: String,
    /// The monitor's rectangle.
    pub rect: ScreenRect,
    /// Effective DPI; 96 is 100% scaling.
    pub dpi: u32,
    /// True for the primary monitor.
    pub primary: bool,
}

/// What the logger read when it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputStartInfo {
    /// QPC ticks per second, the unit of every `qpc` field.
    pub qpc_frequency: i64,
    /// The Windows cursor size setting, as the base cursor size in pixels; 32 is the default.
    pub cursor_size: u32,
    /// The display layout, primary monitor first, each with its DPI.
    pub displays: Vec<DisplayInfo>,
}

/// The cursor at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorSample {
    /// QPC stamp of the reading.
    pub qpc: i64,
    /// Hotspot position, horizontal.
    pub x: i32,
    /// Hotspot position, vertical.
    pub y: i32,
    /// Id of a shape sent earlier; None while hidden or when the shape could not be read.
    pub shape: Option<u32>,
    /// Whether Windows is showing the cursor.
    pub visible: bool,
}

/// One frame of a cursor shape.
#[derive(Clone, PartialEq, Eq)]
pub struct ShapeFrame {
    /// How long the frame shows, in microseconds; 0 for a cursor that is not animated.
    pub duration_us: u32,
    /// Top-down 32-bit BGRA with premultiplied alpha, `width * height * 4` bytes.
    pub bgra: Vec<u8>,
}

impl std::fmt::Debug for ShapeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShapeFrame").field("duration_us", &self.duration_us).field("bytes", &self.bgra.len()).finish()
    }
}

/// One distinct cursor picture, sent once, before the first sample that uses it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorShape {
    /// Id that samples refer to, counted from 1 within one logger.
    pub id: u32,
    /// Width in pixels, at most 256.
    pub width: u32,
    /// Height in pixels, at most 256.
    pub height: u32,
    /// Hotspot, from the left edge.
    pub hotspot_x: u32,
    /// Hotspot, from the top edge.
    pub hotspot_y: u32,
    /// One frame, or several for an animated cursor.
    pub frames: Vec<ShapeFrame>,
}

/// A physical mouse button, as Raw Input reports it (before any left and right swap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
    /// First extra button.
    X1,
    /// Second extra button.
    X2,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// The vertical wheel turned; 120 is one notch, positive away from the user.
    Wheel(i16),
    /// The horizontal wheel turned; 120 is one notch, positive to the right.
    HorizontalWheel(i16),
}

/// A button or wheel event with the cursor position at that moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// QPC stamp taken when the event was read.
    pub qpc: i64,
    /// Latest cursor position, horizontal.
    pub x: i32,
    /// Latest cursor position, vertical.
    pub y: i32,
    /// What happened.
    pub action: MouseAction,
}

/// What changed on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryChange {
    /// The captured window's visible frame; sent once at start, then on every move or resize.
    WindowRect(ScreenRect),
    /// The new display layout after a resolution, DPI or monitor change.
    Displays(Vec<DisplayInfo>),
}

/// A geometry change and when the logger saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeometryEvent {
    /// QPC stamp of the check that found the change.
    pub qpc: i64,
    /// What changed.
    pub change: GeometryChange,
}

/// One entry of the input log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// The cursor moved, changed shape or visibility, or the keep-alive came due.
    Sample(CursorSample),
    /// A cursor picture not sent before.
    Shape(CursorShape),
    /// A button or wheel event.
    Mouse(MouseEvent),
    /// A window rectangle or display layout change.
    Geometry(GeometryEvent),
}

#[cfg(all(test, windows))]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};

    use super::{CaptureSource, InputEvent, InputLogger, InputOptions, MouseAction, ScreenRect};

    /// Hand test: move the mouse, click, scroll and hover over text and links while it runs.
    #[test]
    #[ignore = "reads real input for 5 seconds; run by hand"]
    fn input_hand_test() {
        // The recorder is Per-Monitor DPI aware; the test process must be too, for true DPI values.
        // SAFETY: a plain call made before any window or DPI dependent work in this process.
        #[allow(unsafe_code)]
        let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        let options = InputOptions { source: CaptureSource::Monitor(ScreenRect::default()), clicks: true };
        let mut logger = InputLogger::start(options).unwrap();
        println!("start: {:?}", logger.start_info());
        println!("logging for 5 s: move, click, scroll, and hover over text, links and window edges");
        let (mut samples, mut downs, mut ups, mut wheels, mut geometry) = (0u32, 0u32, 0u32, 0u32, 0u32);
        let mut shapes = Vec::new();
        let mut count = |event: InputEvent| match event {
            InputEvent::Sample(_) => samples += 1,
            InputEvent::Shape(shape) => shapes.push(shape),
            InputEvent::Mouse(mouse) => match mouse.action {
                MouseAction::Down(_) => downs += 1,
                MouseAction::Up(_) => ups += 1,
                MouseAction::Wheel(_) | MouseAction::HorizontalWheel(_) => wheels += 1,
            },
            InputEvent::Geometry(_) => geometry += 1,
        };
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Ok(event) = logger.receiver().recv_timeout(Duration::from_millis(50)) {
                count(event);
            }
        }
        let alive = logger.is_running();
        logger.stop();
        while let Ok(event) = logger.receiver().try_recv() {
            count(event);
        }
        println!("samples {samples}, shapes {}, button downs {downs}, button ups {ups}, wheel {wheels}, geometry {geometry}", shapes.len());
        for shape in &shapes {
            println!("shape {}: {}x{}, hotspot {},{}, {} frame(s)", shape.id, shape.width, shape.height, shape.hotspot_x, shape.hotspot_y, shape.frames.len());
        }
        println!("dropped events: {}", logger.dropped());
        assert!(alive, "the logger thread ended early");
        // The keep-alive alone gives about 4 samples a second.
        assert!(samples >= 15, "too few samples: {samples}");
        assert!(!shapes.is_empty(), "no cursor shape was read");
    }
}
