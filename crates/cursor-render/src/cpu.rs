//! The CPU path: draws a plan onto an RGBA frame in system memory.
//!
//! The cursor is resampled by area: each frame pixel takes the average of the part of the picture it
//! covers. That copies the picture exactly at real size on whole pixels, keeps edges clean when it
//! is enlarged and does not alias when a playback frame is scaled down.

use events::EventsReader;
use media_io::player::video::RgbaFrame;

use crate::CursorStyle;
use crate::map::{CaptureMap, Rect};
use crate::plan::{CursorLayer, Plan, RingLayer, plan};

/// The click ring's colour: red, green and blue.
const RING_COLOR: [f64; 3] = [255.0, 196.0, 0.0];

/// Draws the cursor and its click rings onto a frame, for the time `t_us` in microseconds from the
/// first video frame.
pub fn render(frame: &mut RgbaFrame, events: &EventsReader, t_us: i64, map: &CaptureMap, style: &CursorStyle) {
    let plan = plan(events, t_us, map, style, frame.width, frame.height);
    draw(frame, &plan);
}

/// Draws a plan made for a frame of this size. A frame with too few bytes for its size is drawn as
/// far as its whole rows go.
pub fn draw(frame: &mut RgbaFrame, plan: &Plan<'_>) {
    for ring in &plan.rings {
        draw_ring(frame, ring);
    }
    if let Some(cursor) = &plan.cursor {
        draw_cursor(frame, cursor);
    }
}

/// Calls `paint` with the centre and the bytes of every frame pixel whose centre lies in both
/// rectangles.
fn for_pixels(frame: &mut RgbaFrame, bounds: Rect, clip: Rect, mut paint: impl FnMut(f64, f64, &mut [u8; 4])) {
    // The first pixel whose centre is at or past an edge.
    let first = |edge: f64, size: u32| (edge - 0.5).ceil().clamp(0.0, f64::from(size)) as usize;
    let (left, right) = (first(bounds.left.max(clip.left), frame.width), first(bounds.right.min(clip.right), frame.width));
    let (top, bottom) = (first(bounds.top.max(clip.top), frame.height), first(bounds.bottom.min(clip.bottom), frame.height));
    let row_len = frame.width as usize * 4;
    if row_len == 0 || left >= right {
        return;
    }
    for (y, row) in frame.data.chunks_exact_mut(row_len).enumerate().take(bottom).skip(top) {
        let (pixels, _) = row.as_chunks_mut::<4>();
        for (x, pixel) in pixels.iter_mut().enumerate().take(right).skip(left) {
            paint(x as f64 + 0.5, y as f64 + 0.5, pixel);
        }
    }
}

/// Blends a premultiplied red, green, blue and alpha colour, each 0 to 255, over a pixel.
fn blend(pixel: &mut [u8; 4], color: [f64; 4]) {
    let keep = 1.0 - color[3] / 255.0;
    for (channel, add) in pixel.iter_mut().zip(color) {
        *channel = (add + f64::from(*channel) * keep).round().clamp(0.0, 255.0) as u8;
    }
}

fn draw_ring(frame: &mut RgbaFrame, ring: &RingLayer) {
    let reach = ring.radius + ring.width / 2.0 + 1.0;
    let bounds = Rect { left: ring.x - reach, top: ring.y - reach, right: ring.x + reach, bottom: ring.y + reach };
    for_pixels(frame, bounds, ring.clip, |x, y, pixel| {
        let from_line = ((x - ring.x).hypot(y - ring.y) - ring.radius).abs();
        // Full inside the line, fading over the one pixel at its edge.
        let alpha = (ring.width / 2.0 + 0.5 - from_line).clamp(0.0, 1.0) * ring.opacity;
        if alpha > 0.0 {
            blend(pixel, [RING_COLOR[0] * alpha, RING_COLOR[1] * alpha, RING_COLOR[2] * alpha, 255.0 * alpha]);
        }
    });
}

/// The picture pixels one frame pixel covers along an axis, each with how much of it is covered.
/// `from` and `to` are in picture pixels and can reach outside the picture.
fn covered(from: f64, to: f64, size: u32) -> impl Iterator<Item = (usize, f64)> + Clone {
    let first = from.floor().clamp(0.0, f64::from(size)) as usize;
    let end = to.ceil().clamp(0.0, f64::from(size)) as usize;
    (first..end).map(move |index| (index, (to.min(index as f64 + 1.0) - from.max(index as f64)).max(0.0)))
}

fn draw_cursor(frame: &mut RgbaFrame, cursor: &CursorLayer<'_>) {
    let (width, height) = (cursor.shape.width, cursor.shape.height);
    let dest = cursor.dest;
    // Picture pixels per frame pixel.
    let step = (f64::from(width) / (dest.right - dest.left), f64::from(height) / (dest.bottom - dest.top));
    if !(step.0.is_finite() && step.1.is_finite() && step.0 > 0.0 && step.1 > 0.0) {
        return;
    }
    // A frame pixel that only partly overlaps the picture still takes part of it.
    let bounds = Rect { left: dest.left - 0.5, top: dest.top - 0.5, right: dest.right + 0.5, bottom: dest.bottom + 0.5 };
    let bgra = &cursor.picture.bgra;
    for_pixels(frame, bounds, cursor.clip, |x, y, pixel| {
        let (from_x, from_y) = ((x - 0.5 - dest.left) * step.0, (y - 0.5 - dest.top) * step.1);
        let across = covered(from_x, from_x + step.0, width);
        let mut sum = [0.0f64; 4];
        for (row, tall) in covered(from_y, from_y + step.1, height) {
            for (column, wide) in across.clone() {
                let at = (row * width as usize + column) * 4;
                let Some(source) = bgra.get(at..at + 4) else { continue };
                for (total, &value) in sum.iter_mut().zip(source) {
                    *total += f64::from(value) * wide * tall;
                }
            }
        }
        // Parts outside the picture count as clear.
        let [blue, green, red, alpha] = sum.map(|total| total / (step.0 * step.1));
        if alpha > 0.0 {
            blend(pixel, [red, green, blue, alpha]);
        }
    });
}
