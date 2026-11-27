//! Cursor pictures: recovering alpha from two renders, making monochrome and inverting cursors
//! visible on any background, and deduplicating shapes.
//!
//! A cursor is drawn once on black and once on white. A pixel that covers the background looks the
//! same on both, a transparent one follows the background, and an inverting one is darker on white
//! than on black. This one rule covers alpha, masked, monochrome and inverting cursors.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use super::{CursorShape, ShapeFrame};

/// The largest shape width and height, in pixels.
pub(crate) const MAX_SIDE: u32 = 256;

/// Rounding slack before a pixel counts as inverting.
const INVERT_SLACK: i32 = 8;
/// Blue, green and red of the dark body of a converted cursor.
const DARK: u8 = 26;
/// Blue, green and red of the outline of a converted cursor.
const LIGHT: u8 = 255;
/// Handles remembered before the handle cache starts over; some apps make a new cursor each frame.
const MAX_HANDLES: usize = 4096;

/// One cursor frame with its alpha recovered.
pub(crate) struct Matte {
    /// Premultiplied BGRA; inverting pixels are left clear here and flagged below.
    pub(crate) bgra: Vec<u8>,
    /// One flag per pixel: the cursor inverts what is behind it.
    pub(crate) inverting: Vec<bool>,
}

impl Matte {
    pub(crate) fn has_inverting(&self) -> bool {
        self.inverting.contains(&true)
    }
}

/// Recovers premultiplied BGRA from a render on black and a render on white. Both are 32-bit
/// buffers of the same size; their fourth bytes are ignored.
pub(crate) fn recover_alpha(on_black: &[u8], on_white: &[u8]) -> Matte {
    let pixels = on_black.len().min(on_white.len()) / 4;
    let mut bgra = Vec::with_capacity(pixels * 4);
    let mut inverting = Vec::with_capacity(pixels);
    for (black, white) in on_black.as_chunks::<4>().0.iter().zip(on_white.as_chunks::<4>().0) {
        let difference = black.iter().zip(white).take(3).map(|(&b, &w)| i32::from(w) - i32::from(b)).sum::<i32>() / 3;
        if difference < -INVERT_SLACK {
            bgra.extend_from_slice(&[0; 4]);
            inverting.push(true);
            continue;
        }
        let alpha = (255 - difference.clamp(0, 255)) as u8;
        bgra.extend(black.iter().take(3).map(|&channel| channel.min(alpha)));
        bgra.push(alpha);
        inverting.push(false);
    }
    Matte { bgra, inverting }
}

/// Turns a monochrome or inverting cursor into a dark body with a light outline. The result is one
/// pixel larger on every side, so the outline never clips; the hotspot moves by (1, 1).
pub(crate) fn dark_with_outline(matte: &Matte, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let (width, height) = (width as usize, height as usize);
    let (out_width, out_height) = (width + 2, height + 2);
    // The body is every pixel the cursor shows: inverting, or at least half covered.
    let body = |x: usize, y: usize| -> bool {
        if x >= width || y >= height {
            return false;
        }
        let at = y * width + x;
        matte.inverting.get(at).copied().unwrap_or(false) || matte.bgra.get(at * 4 + 3).is_some_and(|&alpha| alpha >= 128)
    };
    // Output pixel (x, y) is source pixel (x - 1, y - 1); wrapping sends -1 out of range.
    let source = |x: usize| x.wrapping_sub(1);
    let mut out = Vec::with_capacity(out_width * out_height * 4);
    for y in 0..out_height {
        for x in 0..out_width {
            let pixel = if body(source(x), source(y)) {
                [DARK, DARK, DARK, 255]
            } else if (y.saturating_sub(1)..=y + 1).any(|ny| (x.saturating_sub(1)..=x + 1).any(|nx| body(source(nx), source(ny)))) {
                [LIGHT, LIGHT, LIGHT, 255]
            } else {
                [0; 4]
            };
            out.extend_from_slice(&pixel);
        }
    }
    (out, out_width as u32, out_height as u32)
}

/// Cuts a top-down 32-bit picture down to the given size, keeping the top left corner.
fn crop(bgra: &[u8], width: u32, new_width: u32, new_height: u32) -> Vec<u8> {
    bgra.chunks_exact(width as usize * 4).take(new_height as usize).flat_map(|row| row.iter().take(new_width as usize * 4)).copied().collect()
}

/// A cursor picture before it has an id.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ShapeImage {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) hotspot_x: u32,
    pub(crate) hotspot_y: u32,
    /// Duration in microseconds and premultiplied BGRA for each frame.
    pub(crate) frames: Vec<(u32, Vec<u8>)>,
}

impl ShapeImage {
    /// Builds the final picture from the recovered frames. A monochrome cursor, or one with any
    /// inverting pixel in any frame, is converted in every frame so the animation stays steady.
    pub(crate) fn from_mattes(width: u32, height: u32, hotspot: (u32, u32), monochrome: bool, mattes: Vec<(u32, Matte)>) -> Self {
        let convert = monochrome || mattes.iter().any(|(_, matte)| matte.has_inverting());
        let (mut out_width, mut out_height) = (width, height);
        let (mut hotspot_x, mut hotspot_y) = hotspot;
        if convert {
            (out_width, out_height) = (width + 2, height + 2);
            (hotspot_x, hotspot_y) = (hotspot_x + 1, hotspot_y + 1);
        }
        let (capped_width, capped_height) = (out_width.min(MAX_SIDE), out_height.min(MAX_SIDE));
        let frames = mattes
            .into_iter()
            .map(|(duration_us, matte)| {
                if !convert {
                    return (duration_us, matte.bgra);
                }
                let (bgra, padded_width, _) = dark_with_outline(&matte, width, height);
                (duration_us, crop(&bgra, padded_width, capped_width, capped_height))
            })
            .collect();
        Self {
            width: capped_width,
            height: capped_height,
            hotspot_x: hotspot_x.min(capped_width.saturating_sub(1)),
            hotspot_y: hotspot_y.min(capped_height.saturating_sub(1)),
            frames,
        }
    }

    /// A hash of the size, hotspot, timing and pixels.
    fn content_hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }

    fn into_shape(self, id: u32) -> CursorShape {
        CursorShape {
            id,
            width: self.width,
            height: self.height,
            hotspot_x: self.hotspot_x,
            hotspot_y: self.hotspot_y,
            frames: self.frames.into_iter().map(|(duration_us, bgra)| ShapeFrame { duration_us, bgra }).collect(),
        }
    }
}

/// What a freshly read picture turned out to be.
pub(crate) enum Resolved {
    /// The same pixels were sent before under this id.
    Known(u32),
    /// A new shape; send it, then call [`ShapeCache::commit`] with the hash.
    New { shape: CursorShape, hash: u64 },
}

/// Shape ids by cursor handle, so a cursor is read once, and by pixel hash, so equal pictures share
/// an id.
#[derive(Default)]
pub(crate) struct ShapeCache {
    /// None for a handle that could not be read, so it is not tried again every tick.
    by_handle: HashMap<isize, Option<u32>>,
    by_hash: HashMap<u64, u32>,
    last_id: u32,
}

impl ShapeCache {
    /// What is known about a handle: nothing (None), or its shape id if it could be read.
    pub(crate) fn lookup(&self, handle: isize) -> Option<Option<u32>> {
        self.by_handle.get(&handle).copied()
    }

    /// Remembers that a handle could not be read.
    pub(crate) fn failed(&mut self, handle: isize) {
        self.remember(handle, None);
    }

    /// Matches a picture read from `handle` against the shapes already sent.
    pub(crate) fn resolve(&mut self, handle: isize, image: ShapeImage) -> Resolved {
        let hash = image.content_hash();
        if let Some(&id) = self.by_hash.get(&hash) {
            self.remember(handle, Some(id));
            return Resolved::Known(id);
        }
        Resolved::New { shape: image.into_shape(self.last_id + 1), hash }
    }

    /// Records a new shape once it was sent. Without this call the same picture is offered again.
    pub(crate) fn commit(&mut self, handle: isize, hash: u64, id: u32) {
        self.last_id = id;
        self.by_hash.insert(hash, id);
        self.remember(handle, Some(id));
    }

    fn remember(&mut self, handle: isize, id: Option<u32>) {
        if self.by_handle.len() >= MAX_HANDLES {
            self.by_handle.clear();
        }
        self.by_handle.insert(handle, id);
    }
}

#[cfg(test)]
mod tests {
    use super::{DARK, LIGHT, MAX_SIDE, Matte, Resolved, ShapeCache, ShapeImage, dark_with_outline, recover_alpha};

    const CLEAR_PIXEL: [u8; 4] = [0; 4];
    const DARK_PIXEL: [u8; 4] = [DARK, DARK, DARK, 255];
    const LIGHT_PIXEL: [u8; 4] = [LIGHT, LIGHT, LIGHT, 255];

    /// Renders a fake monochrome cursor from its AND and XOR bits, the way Windows applies them:
    /// background AND and_mask, then XOR xor_mask.
    fn render(bits: &[(bool, bool)], background: u8) -> Vec<u8> {
        bits.iter()
            .flat_map(|&(and, xor)| {
                let value = if and { background } else { 0 } ^ if xor { 255 } else { 0 };
                [value, value, value, 0]
            })
            .collect()
    }

    fn matte_of(bits: &[(bool, bool)]) -> Matte {
        recover_alpha(&render(bits, 0), &render(bits, 255))
    }

    fn pixel(bgra: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * width + x) * 4) as usize;
        [bgra[at], bgra[at + 1], bgra[at + 2], bgra[at + 3]]
    }

    const BLACK: (bool, bool) = (false, false);
    const WHITE: (bool, bool) = (false, true);
    const CLEAR: (bool, bool) = (true, false);
    const INVERT: (bool, bool) = (true, true);

    #[test]
    fn the_four_monochrome_pixel_kinds_are_told_apart() {
        let matte = matte_of(&[BLACK, WHITE, CLEAR, INVERT]);
        assert_eq!(matte.bgra, [[0, 0, 0, 255], [255, 255, 255, 255], CLEAR_PIXEL, CLEAR_PIXEL].concat());
        assert_eq!(matte.inverting, [false, false, false, true]);
    }

    #[test]
    fn half_alpha_comes_back_premultiplied() {
        // 50% red: premultiplied (0, 0, 128) over black, plus 127 of white over white.
        let matte = recover_alpha(&[0, 0, 128, 0], &[127, 127, 255, 0]);
        assert_eq!(matte.bgra, [0, 0, 128, 128]);
        assert!(!matte.has_inverting());
    }

    #[test]
    fn an_inverting_bar_turns_dark_with_a_light_outline() {
        // A one pixel wide, two pixel tall I-beam stem in a 1x2 bitmap.
        let (out, width, height) = dark_with_outline(&matte_of(&[INVERT, INVERT]), 1, 2);
        assert_eq!((width, height), (3, 4));
        for y in 0..height {
            for x in 0..width {
                let expected = if x == 1 && (y == 1 || y == 2) { DARK_PIXEL } else { LIGHT_PIXEL };
                assert_eq!(pixel(&out, width, x, y), expected, "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn the_outline_stops_one_pixel_from_the_body() {
        let (out, width, _) = dark_with_outline(&matte_of(&[BLACK, CLEAR, CLEAR, CLEAR]), 4, 1);
        let row: Vec<[u8; 4]> = (0..width).map(|x| pixel(&out, width, x, 1)).collect();
        assert_eq!(row, [LIGHT_PIXEL, DARK_PIXEL, LIGHT_PIXEL, CLEAR_PIXEL, CLEAR_PIXEL, CLEAR_PIXEL]);
    }

    #[test]
    fn white_monochrome_pixels_turn_dark_too() {
        let (out, width, _) = dark_with_outline(&matte_of(&[WHITE]), 1, 1);
        assert_eq!(pixel(&out, width, 1, 1), DARK_PIXEL);
    }

    #[test]
    fn conversion_pads_the_picture_and_moves_the_hotspot() {
        let image = ShapeImage::from_mattes(2, 1, (1, 0), true, vec![(0, matte_of(&[BLACK, CLEAR]))]);
        assert_eq!((image.width, image.height, image.hotspot_x, image.hotspot_y), (4, 3, 2, 1));
        assert_eq!(image.frames[0].1.len(), 4 * 3 * 4);
    }

    #[test]
    fn a_color_cursor_is_left_alone() {
        let matte = recover_alpha(&[10, 20, 30, 0], &[10, 20, 30, 0]);
        let image = ShapeImage::from_mattes(1, 1, (0, 0), false, vec![(0, matte)]);
        assert_eq!((image.width, image.height, image.hotspot_x, image.hotspot_y), (1, 1, 0, 0));
        assert_eq!(image.frames[0].1, [10, 20, 30, 255]);
    }

    #[test]
    fn one_inverting_frame_converts_every_frame() {
        let image = ShapeImage::from_mattes(1, 1, (0, 0), false, vec![(1, matte_of(&[WHITE])), (2, matte_of(&[INVERT]))]);
        assert_eq!((image.width, image.height), (3, 3));
        assert_eq!(image.frames[0].1, image.frames[1].1);
    }

    #[test]
    fn conversion_stays_within_the_size_cap() {
        let bits = vec![BLACK; (MAX_SIDE * MAX_SIDE) as usize];
        let image = ShapeImage::from_mattes(MAX_SIDE, MAX_SIDE, (MAX_SIDE - 1, 0), true, vec![(0, matte_of(&bits))]);
        assert_eq!((image.width, image.height), (MAX_SIDE, MAX_SIDE));
        assert_eq!(image.hotspot_x, MAX_SIDE - 1);
        assert_eq!(image.frames[0].1.len(), (MAX_SIDE * MAX_SIDE * 4) as usize);
    }

    fn image(value: u8, hotspot_x: u32) -> ShapeImage {
        ShapeImage { width: 1, height: 1, hotspot_x, hotspot_y: 0, frames: vec![(0, vec![value; 4])] }
    }

    /// Resolves and commits, returning the id and whether the shape was new.
    fn add(cache: &mut ShapeCache, handle: isize, picture: ShapeImage) -> (u32, bool) {
        match cache.resolve(handle, picture) {
            Resolved::Known(id) => (id, false),
            Resolved::New { shape, hash } => {
                cache.commit(handle, hash, shape.id);
                (shape.id, true)
            }
        }
    }

    #[test]
    fn equal_pixels_share_one_id() {
        let mut cache = ShapeCache::default();
        assert_eq!(add(&mut cache, 100, image(7, 0)), (1, true));
        assert_eq!(add(&mut cache, 200, image(7, 0)), (1, false));
        assert_eq!(cache.lookup(100), Some(Some(1)));
        assert_eq!(cache.lookup(200), Some(Some(1)));
    }

    #[test]
    fn different_pixels_or_hotspot_get_new_ids() {
        let mut cache = ShapeCache::default();
        assert_eq!(add(&mut cache, 100, image(7, 0)), (1, true));
        assert_eq!(add(&mut cache, 200, image(8, 0)), (2, true));
        assert_eq!(add(&mut cache, 300, image(7, 1)), (3, true));
    }

    #[test]
    fn a_shape_that_was_not_sent_is_offered_again_with_the_same_id() {
        let mut cache = ShapeCache::default();
        assert!(matches!(cache.resolve(100, image(7, 0)), Resolved::New { .. }));
        assert_eq!(cache.lookup(100), None);
        assert_eq!(add(&mut cache, 100, image(7, 0)), (1, true));
    }

    #[test]
    fn a_failed_handle_is_remembered() {
        let mut cache = ShapeCache::default();
        assert_eq!(cache.lookup(100), None);
        cache.failed(100);
        assert_eq!(cache.lookup(100), Some(None));
    }
}
