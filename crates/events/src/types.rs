//! Record types. Positions and rectangles are physical virtual desktop pixels, and every `qpc`
//! field is a tick count at [`Header::qpc_frequency`].

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
    /// One window; its rectangle arrives as [`GeometryChange::WindowRect`].
    Window {
        /// The HWND as a number. It means nothing after the recording session.
        handle: i64,
    },
}

/// One monitor in the display layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Display device name.
    pub name: String,
    /// The monitor's rectangle.
    pub rect: ScreenRect,
    /// Effective DPI; 96 is 100% scaling.
    pub dpi: u32,
    /// True for the primary monitor.
    pub primary: bool,
}

/// Which streams were on for the take. The writer drops events of a stream that is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Streams {
    /// Cursor samples and shapes.
    pub cursor: bool,
    /// Mouse buttons and the wheel.
    pub clicks: bool,
    /// Keyboard shortcuts.
    pub keys: bool,
    /// Foreground app changes.
    pub apps: bool,
}

/// The contents of the `HEAD` chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// QPC ticks per second; must be above zero.
    pub qpc_frequency: i64,
    /// QPC of the take's first video frame, which is time zero for readers.
    pub first_frame_qpc: i64,
    /// What the take captures.
    pub source: CaptureSource,
    /// The display layout at take start, primary monitor first, each with its DPI.
    pub displays: Vec<DisplayInfo>,
    /// The Windows cursor size setting at take start, as the base cursor size in pixels.
    pub cursor_size: u32,
    /// Which streams are on.
    pub streams: Streams,
}

impl Header {
    /// Converts a QPC stamp to microseconds from the first video frame, rounded down. Stamps from
    /// before that frame are negative.
    pub fn micros(&self, qpc: i64) -> i64 {
        let ticks = i128::from(qpc) - i128::from(self.first_frame_qpc);
        let micros = (ticks * 1_000_000).div_euclid(i128::from(self.qpc_frequency.max(1)));
        micros.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }
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
    /// Id of the shape; None while hidden or when the shape could not be read.
    pub shape: Option<u32>,
    /// Whether Windows was showing the cursor.
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

/// One distinct cursor picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CursorShape {
    /// Id that samples refer to.
    pub id: u32,
    /// Width in pixels, 1 to 256.
    pub width: u32,
    /// Height in pixels, 1 to 256.
    pub height: u32,
    /// Hotspot, from the left edge.
    pub hotspot_x: u32,
    /// Hotspot, from the top edge.
    pub hotspot_y: u32,
    /// One frame, or several for an animated cursor; at most 1024.
    pub frames: Vec<ShapeFrame>,
}

/// A physical mouse button.
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
    /// QPC stamp of the event.
    pub qpc: i64,
    /// Cursor position, horizontal.
    pub x: i32,
    /// Cursor position, vertical.
    pub y: i32,
    /// What happened.
    pub action: MouseAction,
}

/// The modifier keys held during a shortcut.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// A Ctrl key.
    pub ctrl: bool,
    /// An Alt key.
    pub alt: bool,
    /// A Shift key.
    pub shift: bool,
    /// A Windows key.
    pub win: bool,
}

/// A keyboard shortcut that went down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    /// QPC stamp of the key.
    pub qpc: i64,
    /// The modifiers held with the key.
    pub modifiers: Modifiers,
    /// The virtual key's name, for example S, F5 or Tab.
    pub key_name: String,
}

/// An app coming to the foreground.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppSwitch {
    /// QPC stamp of the change.
    pub qpc: i64,
    /// The executable's file name.
    pub exe: String,
    /// The app's display name.
    pub display_name: String,
}

/// What changed on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryChange {
    /// The captured window's visible frame.
    WindowRect(ScreenRect),
    /// A new display layout.
    Displays(Vec<DisplayInfo>),
}

/// A geometry change and when it was seen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeometryEvent {
    /// QPC stamp of the change.
    pub qpc: i64,
    /// What changed.
    pub change: GeometryChange,
}

/// One input event to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// A cursor sample.
    Sample(CursorSample),
    /// A cursor picture; push it before the first sample that uses it.
    Shape(CursorShape),
    /// A button or wheel event.
    Mouse(MouseEvent),
    /// A window rectangle or display layout change.
    Geometry(GeometryEvent),
    /// A keyboard shortcut.
    Chord(Chord),
    /// A foreground app change.
    App(AppSwitch),
}

/// How many records of each kind the file holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Cursor shapes.
    pub shapes: u64,
    /// Cursor samples.
    pub samples: u64,
    /// Button and wheel events.
    pub mouse: u64,
    /// Keyboard shortcuts.
    pub chords: u64,
    /// App switches.
    pub apps: u64,
    /// Geometry changes.
    pub geometry: u64,
}

/// The contents of the `TAIL` chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tail {
    /// QPC when the take ended.
    pub end_qpc: i64,
    /// What the writer wrote.
    pub counts: Counts,
}
