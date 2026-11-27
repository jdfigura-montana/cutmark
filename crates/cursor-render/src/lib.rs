//! Draws the cursor of a take back onto its video frames, from the take's input track and a style.
//!
//! See the input track spec, "Cursor renderer". The work is in two steps, so that a GPU path can be
//! added beside the CPU one:
//!
//! 1. [`plan`] decides what one frame gets: the click rings and the cursor picture, with their
//!    places in frame pixels. It touches no pixels.
//! 2. [`render`] draws that plan onto an RGBA frame on the CPU.

mod cpu;
mod map;
mod plan;

#[cfg(test)]
mod tests;

pub use cpu::{draw, render};
pub use map::{CaptureMap, Projection, Rect};
pub use plan::{CLICK_RING_US, CursorLayer, Plan, RingLayer, SMOOTHING_WINDOW_US, plan};

/// The smallest cursor scale: real size.
pub const MIN_SCALE: f32 = 1.0;

/// The largest cursor scale.
pub const MAX_SCALE: f32 = 3.0;

/// How the cursor is drawn. The default is the cursor exactly as recorded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorStyle {
    /// Size relative to the recorded cursor, [`MIN_SCALE`] to [`MAX_SCALE`]. A value outside the
    /// range is drawn at the nearest end.
    pub scale: f32,
    /// Evens out the cursor's path with a short moving average.
    pub smoothing: bool,
    /// Draws a ring where a mouse button went down.
    pub click_ring: bool,
}

impl Default for CursorStyle {
    fn default() -> Self {
        Self { scale: MIN_SCALE, smoothing: false, click_ring: false }
    }
}

impl CursorStyle {
    /// The scale that is drawn: in range, and real size for a value that is not a number.
    pub fn effective_scale(&self) -> f32 {
        if self.scale.is_nan() { MIN_SCALE } else { self.scale.clamp(MIN_SCALE, MAX_SCALE) }
    }
}
