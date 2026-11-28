//! Where things go in the output. No GPU here.

use crate::style::{FrameStyle, MAX_CORNER_RADIUS, MAX_PADDING, MIN_PADDING};

/// The part of the source that is shown, in source pixels from its top left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewRect {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width, more than zero.
    pub width: f32,
    /// The height, more than zero.
    pub height: f32,
}

impl ViewRect {
    /// The whole of a source of this size: no zoom.
    pub fn full(width: u32, height: u32) -> Self {
        Self { x: 0.0, y: 0.0, width: width as f32, height: height as f32 }
    }

    pub(crate) fn is_usable(self) -> bool {
        [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite()) && self.width > 0.0 && self.height > 0.0
    }
}

/// The place of the picture in the output, in whole output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentRect {
    /// The left edge.
    pub x: u32,
    /// The top edge.
    pub y: u32,
    /// The width, at least 1.
    pub width: u32,
    /// The height, at least 1.
    pub height: u32,
}

/// Where the picture goes. With the frame off it fills the output. With the frame on it is the
/// largest rectangle of the view's shape that fits inside the padding, centred.
pub fn content_rect(output: (u32, u32), view: ViewRect, style: &FrameStyle) -> ContentRect {
    let (out_w, out_h) = output;
    if !style.enabled {
        return ContentRect { x: 0, y: 0, width: out_w, height: out_h };
    }
    let pad = clamp_or_zero(style.padding, MIN_PADDING, MAX_PADDING) * out_w.min(out_h) as f32;
    let room_w = (out_w as f32 - 2.0 * pad).max(1.0);
    let room_h = (out_h as f32 - 2.0 * pad).max(1.0);
    let scale = (room_w / view.width).min(room_h / view.height);
    let width = ((view.width * scale).round() as u32).clamp(1, out_w);
    let height = ((view.height * scale).round() as u32).clamp(1, out_h);
    ContentRect { x: (out_w - width) / 2, y: (out_h - height) / 2, width, height }
}

/// The factor from a length in pixels at 1080p to a length in this output.
pub(crate) fn scale_from_1080p(output: (u32, u32)) -> f32 {
    output.0.min(output.1) as f32 / 1080.0
}

/// The corner radius in output pixels: scaled, and never more than half the picture's shorter side.
pub(crate) fn corner_radius_px(output: (u32, u32), content: ContentRect, style: &FrameStyle) -> f32 {
    if !style.enabled {
        return 0.0;
    }
    let radius = clamp_or_zero(style.corner_radius, 0.0, MAX_CORNER_RADIUS) * scale_from_1080p(output);
    radius.min(content.width.min(content.height) as f32 / 2.0)
}

/// The part of a picture that covers the output without distortion, as texture coordinates: the
/// offset, then the size.
pub(crate) fn cover_uv(image: (u32, u32), output: (u32, u32)) -> [f32; 4] {
    let image_aspect = image.0 as f32 / image.1 as f32;
    let output_aspect = output.0 as f32 / output.1 as f32;
    let (w, h) = if image_aspect > output_aspect { (output_aspect / image_aspect, 1.0) } else { (1.0, image_aspect / output_aspect) };
    [(1.0 - w) / 2.0, (1.0 - h) / 2.0, w, h]
}

/// `value` in `min..=max`, and `min` for a value that is not a number.
fn clamp_or_zero(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() { min } else { value.clamp(min, max) }
}
