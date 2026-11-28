//! The passes between zoom and frame: the cursor, the click effects and the spotlight. They are
//! drawn on the zoomed picture, so they grow with the zoom and the frame's corners cut them like
//! the rest of the picture.

use std::collections::BTreeMap;

use events::CursorShape;

use crate::RenderError;
use crate::atlas::{Atlas, Slot};
use crate::input::{ClickDraw, ClickEffect, CursorDraw, CursorLook, Highlight, StudioFrameInput};
use crate::layout::ViewRect;
use crate::sprites::{Sprite, premultiplied};

/// How long a click effect shows, from the moment the button goes down.
pub const CLICK_EFFECT_US: i64 = 400_000;

/// How long an idle cursor takes to fade out.
pub const IDLE_FADE_US: i64 = 300_000;

/// The atlas's side: room for 60 cursor frames of the largest size, or about a thousand of the
/// usual 64 pixels.
const CURSOR_ATLAS_SIDE: u32 = 2048;

/// The largest side of a cursor shape, as in the events file.
const MAX_SHAPE_SIDE: u32 = 256;

// Lengths of the click effects and the spotlight are in pixels of a 1080p source.
/// The ring's radius when it appears and when it goes.
const RING_RADIUS: (f32, f32) = (12.0, 32.0);
const RING_LINE: f32 = 3.5;
const RING_OPACITY: f32 = 0.9;
/// The ripple's radius when it appears and when it goes. It slows down as it spreads.
const RIPPLE_RADIUS: (f32, f32) = (6.0, 44.0);
const RIPPLE_LINE: f32 = 2.0;
const RIPPLE_FILL_OPACITY: f32 = 0.35;
const RIPPLE_LINE_OPACITY: f32 = 0.8;
/// The thinnest line in picture pixels, so that a small preview still shows it.
const MIN_LINE: f32 = 1.0;
/// How dark the picture gets outside the spotlight.
const SPOTLIGHT_DARKNESS: f32 = 0.55;
/// The width of the spotlight's soft edge, as a share of its radius.
const SPOTLIGHT_SOFTNESS: f32 = 0.4;

/// The cursor shapes of a take and the atlas their frames are drawn from.
pub(crate) struct Cursors {
    shapes: BTreeMap<u32, CursorShape>,
    pub(crate) atlas: Atlas<(u32, usize), ()>,
}

impl Cursors {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        // The shapes are BGRA, which this format takes as it is.
        let atlas = Atlas::new(device, "studio cursor atlas", CURSOR_ATLAS_SIDE, wgpu::TextureFormat::Bgra8Unorm);
        Self { shapes: BTreeMap::new(), atlas }
    }

    pub(crate) fn set_shapes<'a>(&mut self, shapes: impl IntoIterator<Item = &'a CursorShape>) -> Result<(), RenderError> {
        let mut taken = BTreeMap::new();
        for shape in shapes {
            let sides_fit = [shape.width, shape.height].iter().all(|side| (1..=MAX_SHAPE_SIDE).contains(side));
            let bytes = shape.width as usize * shape.height as usize * 4;
            if !sides_fit || shape.frames.iter().any(|frame| frame.bgra.len() != bytes) {
                return Err(RenderError::Shape { id: shape.id });
            }
            taken.insert(shape.id, shape.clone());
        }
        self.shapes = taken;
        self.atlas.clear();
        Ok(())
    }

    /// The shape's frame at a time, put into the atlas if it is not there yet.
    fn frame(&mut self, queue: &wgpu::Queue, id: u32, time_us: i64) -> Option<(Slot, &CursorShape)> {
        let shape = self.shapes.get(&id)?;
        let index = cursor_render::animation_frame(shape, time_us)?;
        let key = (id, index);
        let slot = match self.atlas.get(&key) {
            Some((slot, ())) => slot,
            None => {
                let pixels = &shape.frames.get(index)?.bgra;
                let size = (shape.width, shape.height);
                match self.atlas.insert(queue, key, size, pixels, ()) {
                    Some(slot) => slot,
                    // A frame draws one cursor, so nothing drawn now depends on the old entries.
                    None => {
                        self.atlas.clear();
                        self.atlas.insert(queue, key, size, pixels, ())?
                    }
                }
            }
        };
        Some((slot, shape))
    }
}

/// From source pixels to pixels of the zoomed picture.
#[derive(Clone, Copy)]
struct Mapping {
    view: ViewRect,
    /// Picture pixels per source pixel. This includes the zoom.
    scale: (f32, f32),
    /// Picture pixels per pixel of a 1080p source.
    unit: f32,
}

impl Mapping {
    fn new(view: ViewRect, source: (u32, u32), picture: (u32, u32)) -> Self {
        let scale = (picture.0 as f32 / view.width, picture.1 as f32 / view.height);
        let unit = source.0.min(source.1) as f32 / 1080.0 * (scale.0 + scale.1) / 2.0;
        Self { view, scale, unit }
    }

    /// None for a position that is not a number.
    fn point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        (x.is_finite() && y.is_finite()).then_some(((x - self.view.x) * self.scale.0, (y - self.view.y) * self.scale.1))
    }
}

/// What goes on the zoomed picture, in drawing order: the cursor, then the click effects, then the
/// spotlight.
pub(crate) fn sprites(cursors: &mut Cursors, queue: &wgpu::Queue, input: &StudioFrameInput<'_>, source: (u32, u32), picture: (u32, u32)) -> Vec<Sprite> {
    let look = &input.settings.cursor;
    let map = Mapping::new(input.view_rect, source, picture);
    let mut out = Vec::new();
    if let Some(cursor) = &input.cursor {
        out.extend(cursor_sprite(cursors, queue, cursor, input.t_us, look, &map));
    }
    for click in input.clicks {
        click_sprites(&mut out, click, input.t_us, look, &map);
    }
    if let (Highlight::Spotlight { radius }, Some(cursor)) = (look.highlight, &input.cursor)
        && let Some(center) = map.point(cursor.x, cursor.y)
        && radius.is_finite()
        && radius > 0.0
    {
        let radius = radius * map.unit;
        let size = (picture.0 as f32, picture.1 as f32);
        out.push(Sprite::spotlight(size, center, radius, radius * SPOTLIGHT_SOFTNESS, [0.0, 0.0, 0.0, SPOTLIGHT_DARKNESS]));
    }
    out
}

/// How solid a cursor that has been still for `idle_us` is: 1 until the time to hide, then down to
/// 0 over [`IDLE_FADE_US`].
pub(crate) fn idle_opacity(idle_us: i64, hide_after_s: Option<f32>) -> f32 {
    let Some(seconds) = hide_after_s.filter(|seconds| !seconds.is_nan()) else { return 1.0 };
    let hide_at = (f64::from(seconds.max(0.0)) * 1e6) as i64;
    let past = idle_us.saturating_sub(hide_at).clamp(0, IDLE_FADE_US);
    1.0 - past as f32 / IDLE_FADE_US as f32
}

fn cursor_sprite(cursors: &mut Cursors, queue: &wgpu::Queue, cursor: &CursorDraw, time_us: i64, look: &CursorLook, map: &Mapping) -> Option<Sprite> {
    let opacity = idle_opacity(cursor.idle_us, look.hide_idle_s);
    if !look.visible || opacity <= 0.0 || !(cursor.scale.is_finite() && cursor.scale > 0.0) {
        return None;
    }
    let (x, y) = map.point(cursor.x, cursor.y)?;
    let size = cursor.scale * in_range(look.size, cursor_render::MIN_SCALE, cursor_render::MAX_SCALE);
    // Picture pixels per pixel of the shape.
    let texel = (size * map.scale.0, size * map.scale.1);
    let (slot, shape) = cursors.frame(queue, cursor.shape, time_us)?;
    // The hotspot stays on the position at every size and zoom.
    let dest = [x - shape.hotspot_x as f32 * texel.0, y - shape.hotspot_y as f32 * texel.1, shape.width as f32 * texel.0, shape.height as f32 * texel.1];
    Some(Sprite::picture(slot, dest, texel, [opacity; 4]))
}

fn click_sprites(out: &mut Vec<Sprite>, click: &ClickDraw, time_us: i64, look: &CursorLook, map: &Mapping) {
    let age = time_us.saturating_sub(click.time_us);
    if !(0..CLICK_EFFECT_US).contains(&age) {
        return;
    }
    let Some(center) = map.point(click.x, click.y) else { return };
    let age = age as f32 / CLICK_EFFECT_US as f32;
    let fade = 1.0 - age;
    let unit = map.unit * in_range(look.click_size, 0.5, 3.0);
    let between = |(from, to): (f32, f32), part: f32| (from + (to - from) * part) * unit;
    let color = |opacity: f32| premultiplied(look.click_color, opacity);
    match look.click_effect {
        ClickEffect::None => {}
        ClickEffect::Ring => {
            let line = (RING_LINE * unit).max(MIN_LINE);
            out.push(Sprite::ring(center, between(RING_RADIUS, age), line, color(RING_OPACITY * fade)));
        }
        ClickEffect::Ripple => {
            let radius = between(RIPPLE_RADIUS, 1.0 - fade * fade);
            let line = (RIPPLE_LINE * unit).max(MIN_LINE);
            out.push(Sprite::disc(center, radius, color(RIPPLE_FILL_OPACITY * fade)));
            out.push(Sprite::ring(center, radius, line, color(RIPPLE_LINE_OPACITY * fade)));
        }
    }
}

/// `value` in `min..=max`, and `min` for a value that is not a number.
fn in_range(value: f32, min: f32, max: f32) -> f32 {
    if value.is_nan() { min } else { value.clamp(min, max) }
}
