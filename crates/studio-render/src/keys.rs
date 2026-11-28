//! The shortcut overlay: a chord as a row of key caps in screen space, so it does not zoom. The
//! caps are drawn into a picture of their own, which the frame pass lays over everything else. That
//! way they are mixed as sRGB values whatever the target's format is.

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};

use crate::RenderError;
use crate::atlas::{Atlas, Slot};
use crate::input::{KeysPosition, KeysSize};
use crate::layout::{ContentRect, scale_from_1080p};
use crate::sprites::{Sprite, premultiplied};

/// Instrument Sans (SIL Open Font License), from the ui-kit's font files.
const FONT: &[u8] = include_bytes!("../../ui-kit/assets/fonts/InstrumentSans-Medium.ttf");

const GLYPH_ATLAS_SIDE: u32 = 1024;

/// A cap's height in pixels at 1080p.
const SMALL_CAP: f32 = 40.0;
const MEDIUM_CAP: f32 = 52.0;
const LARGE_CAP: f32 = 68.0;
/// The distance from the output's edges, in pixels at 1080p.
const MARGIN: f32 = 56.0;
// The rest are shares of a cap's height.
/// The font's size, from its lowest descender to its highest ascender.
const TEXT: f32 = 0.52;
/// The room beside a key's name. A cap is never narrower than it is tall.
const SIDE_ROOM: f32 = 0.34;
const GAP: f32 = 0.16;
const RADIUS: f32 = 0.22;
const BORDER: f32 = 0.03;

/// Dark glass: the fill, the light edge and the text, each an sRGB colour with its opacity.
const FILL: ([u8; 3], f32) = ([18, 20, 26], 0.78);
const EDGE: ([u8; 3], f32) = ([255, 255, 255], 0.22);
const INK: ([u8; 3], f32) = ([244, 246, 250], 1.0);

/// The keys of a chord such as `Ctrl+Shift+S`. A `+` where a key's name belongs is that key.
pub(crate) fn split_chord(chord: &str) -> Vec<&str> {
    let mut keys = Vec::new();
    let mut rest = chord;
    while let Some(first) = rest.chars().next() {
        // The search starts after the first character, so a name can be or start with a plus.
        let end = rest[first.len_utf8()..].find('+').map_or(rest.len(), |at| at + first.len_utf8());
        keys.push(&rest[..end]);
        rest = rest.get(end + 1..).unwrap_or("");
    }
    keys
}

/// Where an overlay of `size` goes in an output.
pub(crate) fn place(output: (u32, u32), size: (u32, u32), position: KeysPosition) -> ContentRect {
    let margin = (MARGIN * scale_from_1080p(output)).round() as u32;
    let (width, height) = (size.0.min(output.0), size.1.min(output.1));
    let (spare_x, spare_y) = (output.0 - width, output.1 - height);
    let x = match position {
        KeysPosition::BottomCenter | KeysPosition::TopCenter => spare_x / 2,
        KeysPosition::BottomLeft => margin.min(spare_x),
        KeysPosition::BottomRight => spare_x.saturating_sub(margin),
    };
    let y = match position {
        KeysPosition::TopCenter => margin.min(spare_y),
        _ => spare_y.saturating_sub(margin),
    };
    ContentRect { x, y, width, height }
}

/// A glyph's pixels are placed from the pen: left of and above their top left corner, with the pen
/// on the baseline.
#[derive(Clone, Copy)]
struct GlyphPlace {
    left: f32,
    top: f32,
    width: u32,
    height: u32,
}

/// One cap of a laid out chord.
struct Cap {
    x: u32,
    width: u32,
    glyphs: Vec<(GlyphId, f32)>,
    text_width: f32,
}

/// A chord's caps in a row, in pixels of the output.
struct Row {
    width: u32,
    height: u32,
    text: f32,
    caps: Vec<Cap>,
}

/// The picture of a chord, kept while the chord and its size stay the same.
struct Drawn {
    chord: String,
    size: KeysSize,
    picture: (u32, u32),
    view: wgpu::TextureView,
}

pub(crate) struct Keys {
    font: FontRef<'static>,
    atlas: Atlas<(GlyphId, u32), GlyphPlace>,
    drawn: Option<Drawn>,
}

/// What the frame pass needs, and the sprites that still have to be drawn into the picture.
pub(crate) struct Overlay {
    pub size: (u32, u32),
    pub view: wgpu::TextureView,
    /// None when the picture from an earlier frame still holds.
    pub draw: Option<Vec<Sprite>>,
}

impl Keys {
    pub(crate) fn new(device: &wgpu::Device) -> Result<Self, RenderError> {
        let font = FontRef::try_from_slice(FONT).map_err(|error| RenderError::Font(error.to_string()))?;
        let atlas = Atlas::new(device, "studio glyph atlas", GLYPH_ATLAS_SIDE, crate::PICTURE_FORMAT);
        Ok(Self { font, atlas, drawn: None })
    }

    pub(crate) fn atlas(&self) -> (&wgpu::TextureView, u32) {
        (&self.atlas.view, self.atlas.side)
    }

    /// The overlay for a chord in an output of this size; None for a chord without a key.
    pub(crate) fn overlay(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, output: (u32, u32), chord: &str, size: KeysSize) -> Option<Overlay> {
        if let Some(drawn) = &self.drawn
            && drawn.chord == chord
            && drawn.size == size
        {
            return Some(Overlay { size: drawn.picture, view: drawn.view.clone(), draw: None });
        }
        let row = self.row(output, &split_chord(chord), size)?;
        let sprites = match self.sprites(queue, &row) {
            Some(sprites) => sprites,
            // The atlas ran full part of the way. Nothing was drawn yet, so all of it starts over.
            None => {
                self.atlas.clear();
                self.sprites(queue, &row)?
            }
        };
        let picture = (row.width, row.height);
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let view = crate::texture(device, "studio keys", picture, crate::PICTURE_FORMAT, usage).create_view(&wgpu::TextureViewDescriptor::default());
        self.drawn = Some(Drawn { chord: chord.to_owned(), size, picture, view: view.clone() });
        Some(Overlay { size: picture, view, draw: Some(sprites) })
    }

    /// Lays a chord out. A row too wide for the output is made smaller as a whole.
    fn row(&self, output: (u32, u32), keys: &[&str], size: KeysSize) -> Option<Row> {
        if keys.is_empty() {
            return None;
        }
        let scale = scale_from_1080p(output);
        let cap = match size {
            KeysSize::Small => SMALL_CAP,
            KeysSize::Medium => MEDIUM_CAP,
            KeysSize::Large => LARGE_CAP,
        };
        let height = (cap * scale).min(output.1 as f32).round().max(1.0);
        let row = self.row_of_height(keys, height);
        let room = (output.0 as f32 - 2.0 * MARGIN * scale).max(1.0);
        let row = if row.width as f32 > room { self.row_of_height(keys, (height * room / row.width as f32).floor().max(1.0)) } else { row };
        // Rounding or a tiny output can still leave it too wide; the end is then cut off.
        Some(Row { width: row.width.min(output.0), ..row })
    }

    fn row_of_height(&self, keys: &[&str], height: f32) -> Row {
        let text = (height * TEXT).max(1.0);
        let font = self.font.as_scaled(PxScale::from(text));
        let gap = (height * GAP).round() as u32;
        let mut x = 0;
        let caps = keys
            .iter()
            .map(|key| {
                let mut pen = 0.0;
                let mut last = None;
                let glyphs = key
                    .chars()
                    .map(|character| {
                        let id = font.glyph_id(character);
                        pen += last.map_or(0.0, |last| font.kern(last, id));
                        let at = (id, pen);
                        pen += font.h_advance(id);
                        last = Some(id);
                        at
                    })
                    .collect();
                let width = (pen + 2.0 * SIDE_ROOM * height).max(height).round() as u32;
                let cap = Cap { x, width, glyphs, text_width: pen };
                x += width + gap;
                cap
            })
            .collect();
        Row { width: x.saturating_sub(gap).max(1), height: height as u32, text, caps }
    }

    /// The sprites of a row; None when the glyph atlas has no room for one of its glyphs.
    fn sprites(&mut self, queue: &wgpu::Queue, row: &Row) -> Option<Vec<Sprite>> {
        let height = row.height as f32;
        let capital =
            self.font.outline_glyph(self.font.glyph_id('H').with_scale(PxScale::from(row.text))).map_or(row.text * 0.6, |outline| -outline.px_bounds().min.y);
        // Capitals sit in the middle of the cap.
        let baseline = ((height + capital) / 2.0).round();
        let radius = height * RADIUS;
        let border = (height * BORDER).max(1.0);
        let mut out = Vec::new();
        for cap in &row.caps {
            let rect = [cap.x as f32, 0.0, cap.width as f32, height];
            out.push(Sprite::rounded(rect, radius, 0.0, premultiplied(FILL.0, FILL.1)));
            out.push(Sprite::rounded(rect, radius, border, premultiplied(EDGE.0, EDGE.1)));
            let start = cap.x as f32 + (cap.width as f32 - cap.text_width) / 2.0;
            for &(id, pen) in &cap.glyphs {
                let Some((slot, glyph)) = self.glyph(queue, id, row.text)? else { continue };
                let dest = [(start + pen).round() + glyph.left, baseline + glyph.top, glyph.width as f32, glyph.height as f32];
                out.push(Sprite::picture(slot, dest, (1.0, 1.0), premultiplied(INK.0, INK.1)));
            }
        }
        Some(out)
    }

    /// A glyph in the atlas, as white with its coverage as alpha. The outer None means the atlas
    /// is full, the inner one a glyph with nothing to draw, such as a space.
    fn glyph(&mut self, queue: &wgpu::Queue, id: GlyphId, text: f32) -> Option<Option<(Slot, GlyphPlace)>> {
        let key = (id, text.to_bits());
        if let Some(entry) = self.atlas.get(&key) {
            return Some(Some(entry));
        }
        let Some(outline) = self.font.outline_glyph(id.with_scale_and_position(PxScale::from(text), point(0.0, 0.0))) else {
            return Some(None);
        };
        let bounds = outline.px_bounds();
        let (width, height) = (bounds.width().ceil() as u32, bounds.height().ceil() as u32);
        // A glyph as large as the atlas only happens at output sizes no GPU takes.
        if width == 0 || height == 0 || width + 2 > self.atlas.side || height + 2 > self.atlas.side {
            return Some(None);
        }
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        outline.draw(|x, y, coverage| {
            if x < width && y < height {
                let at = (y as usize * width as usize + x as usize) * 4;
                pixels[at..at + 4].fill((coverage.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        });
        let place = GlyphPlace { left: bounds.min.x, top: bounds.min.y, width, height };
        let slot = self.atlas.insert(queue, key, (width, height), &pixels, place)?;
        Some(Some((slot, place)))
    }
}
