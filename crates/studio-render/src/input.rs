//! What one frame is drawn from. The caller computes every value that depends on the take over
//! time (the view, the smoothed cursor, how long it has been idle, the current shortcut), so that
//! the renderer keeps no history and any frame can be drawn on its own.

use crate::layout::ViewRect;
use crate::style::FrameStyle;

/// Everything one frame needs besides the source picture.
#[derive(Clone, Copy, Debug)]
pub struct StudioFrameInput<'a> {
    /// The frame's time in microseconds from the first video frame. It drives the click effects
    /// and animated cursors.
    pub t_us: i64,
    /// The part of the source that is shown, in source pixels. Where it reaches past the source,
    /// the edge pixels repeat. It is stretched over the picture's place, so with the frame off it
    /// should have the output's shape.
    pub view_rect: ViewRect,
    /// The cursor; None while there is none to place. The spotlight goes with it.
    pub cursor: Option<CursorDraw>,
    /// The clicks to show. Those that have not happened yet or are over are left out, so a list
    /// longer than needed does no harm.
    pub clicks: &'a [ClickDraw],
    /// The shortcut on screen now, as its keys joined by `+`, for example `Ctrl+Shift+S`. The
    /// caller applies the Keys panel's Duration by choosing when this is Some.
    pub chord: Option<&'a str>,
    /// How things look.
    pub settings: &'a RenderSettings,
}

/// The cursor at the frame's time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorDraw {
    /// The hotspot, already smoothed, in source pixels, horizontal.
    pub x: f32,
    /// The hotspot, vertical.
    pub y: f32,
    /// The id of the shape, from [`crate::StudioRenderer::set_cursor_shapes`]. An unknown id draws
    /// no cursor picture.
    pub shape: u32,
    /// Source pixels per pixel of the shape's picture: 1 when the video shows the desktop at real
    /// size.
    pub scale: f32,
    /// How long the cursor has been still, in microseconds; 0 while it moves.
    pub idle_us: i64,
}

/// A mouse button going down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickDraw {
    /// Where, in source pixels, horizontal.
    pub x: f32,
    /// Where, vertical.
    pub y: f32,
    /// When, in microseconds from the first video frame.
    pub time_us: i64,
}

/// What a click looks like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClickEffect {
    /// Nothing.
    None,
    /// A ring that grows and fades.
    #[default]
    Ring,
    /// A soft disc with a thin edge that spreads out and fades.
    Ripple,
}

/// What marks the cursor's place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Highlight {
    /// Nothing.
    #[default]
    Off,
    /// Everything outside a soft circle around the cursor is darkened.
    Spotlight {
        /// The circle's radius in pixels of a 1080p source, before zoom.
        radius: f32,
    },
}

/// The Cursor panel, as the renderer takes it. Smoothing is not here: [`CursorDraw`] arrives
/// smoothed. The default is the spec's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorLook {
    /// Whether the cursor picture is drawn. Click effects and the spotlight do not depend on it.
    pub visible: bool,
    /// The size relative to the recorded cursor, 1 to 3.
    pub size: f32,
    /// The cursor fades out once it has been still this many seconds; None never hides it.
    pub hide_idle_s: Option<f32>,
    /// What a click looks like.
    pub click_effect: ClickEffect,
    /// The click effect's sRGB colour.
    pub click_color: [u8; 3],
    /// The click effect's size relative to its normal one, 0.5 to 3.
    pub click_size: f32,
    /// What marks the cursor's place.
    pub highlight: Highlight,
}

impl Default for CursorLook {
    fn default() -> Self {
        Self {
            visible: true,
            size: 1.5,
            hide_idle_s: Some(3.0),
            click_effect: ClickEffect::Ring,
            click_color: [255; 3],
            click_size: 1.0,
            highlight: Highlight::Off,
        }
    }
}

/// Where the shortcut overlay sits in the output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeysPosition {
    /// Centred along the bottom edge.
    #[default]
    BottomCenter,
    /// In the bottom left corner.
    BottomLeft,
    /// In the bottom right corner.
    BottomRight,
    /// Centred along the top edge.
    TopCenter,
}

/// How large the shortcut overlay is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeysSize {
    /// Small key caps.
    Small,
    /// The usual size.
    #[default]
    Medium,
    /// Large key caps.
    Large,
}

/// The Keys panel, as the renderer takes it. The default is the spec's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeysLook {
    /// Whether shortcuts are drawn.
    pub show: bool,
    /// Where they sit.
    pub position: KeysPosition,
    /// How large they are.
    pub size: KeysSize,
}

impl Default for KeysLook {
    fn default() -> Self {
        Self { show: true, position: KeysPosition::BottomCenter, size: KeysSize::Medium }
    }
}

/// How a Studio frame looks. Numbers outside their ranges count as the nearest end.
#[derive(Clone, Debug, Default)]
pub struct RenderSettings {
    /// The cursor, its click effects and the highlight.
    pub cursor: CursorLook,
    /// The shortcut overlay.
    pub keys: KeysLook,
    /// The frame around the picture.
    pub frame: FrameStyle,
}
