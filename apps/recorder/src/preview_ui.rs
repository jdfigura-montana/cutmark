//! Draws the live monitor preview in the main area of the window.

use capture::{Preview, PreviewFrame};
use egui::{Align2, Color32, ColorImage, Context, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};
use ui_kit::tokens::{color, space, typography};

/// The dark colour around and behind the picture (`bg.track.empty`).
const BACKDROP: Color32 = color::BG_TRACK_EMPTY;

/// Space left below the preview for the transport deck R7 adds (spec "Setup mode").
const DECK_SPACE: f32 = 88.0;

/// Width of the recording preview's rim (spec "Record mode": 1px).
const RIM: f32 = 1.0;

/// Alpha of the recording rim (spec "Record mode": 35%).
const RIM_ALPHA: f32 = 0.35;

/// Alpha of the REC pill's `sem.record` tint fill (spec "Record mode").
const PILL_TINT: f32 = 0.3;

/// Height of the REC pill.
const PILL_HEIGHT: f32 = 24.0;

/// Diameter of the REC pill's dot.
const PILL_DOT: f32 = 6.0;

/// The preview picture and what the window knows about it.
#[derive(Default)]
pub struct PreviewView {
    /// The one texture the frames are uploaded into.
    texture: Option<TextureHandle>,
    /// The sequence number of the frame in the texture.
    shown_seq: u64,
    /// The size of the frame in the texture, in pixels.
    shown_size: (u32, u32),
    /// The area size in pixels that was last sent to the preview.
    sent_size: Option<(u32, u32)>,
    /// The frame that is in the texture, kept so a click can read its pixels.
    frame: Option<PreviewFrame>,
    /// Where the picture was last drawn, in points.
    image_rect: Option<Rect>,
}

/// A spot on the drawn picture: the frame that was showing and the pixel under the pointer.
#[derive(Debug, Clone)]
pub struct Pick {
    /// The frame that was showing.
    pub frame: PreviewFrame,
    /// Pixel column in the frame.
    pub x: u32,
    /// Pixel row in the frame.
    pub y: u32,
}

impl PreviewView {
    /// Forgets what was sent to the preview and what it showed; call it when a new preview starts.
    pub fn reset(&mut self) {
        self.shown_seq = 0;
        self.sent_size = None;
        self.frame = None;
        self.image_rect = None;
    }

    /// The pixel of the picture under a point of the window, or None when the point is off the picture.
    pub fn pick(&self, pos: Pos2) -> Option<Pick> {
        let (frame, rect) = (self.frame.as_ref()?, self.image_rect?);
        let (x, y) = frame_pixel(rect, pos, (frame.width, frame.height))?;
        Some(Pick { frame: frame.clone(), x, y })
    }

    /// Draws the newest preview frame letterboxed in the centered 16:9 box, or "Starting preview..." until one arrives.
    pub fn show(&mut self, ui: &mut Ui, preview: &Preview) {
        let area = allocate_area(ui);
        self.draw_frame(ui, preview, area);
    }

    /// Draws the recording preview (spec "Record mode"): the 16:9 box grown to
    /// fill the space, the `sem.record` rim at 35% around it and the REC pill
    /// at its top left ("PAUSED" in `sem.meter.warn` while paused); returns
    /// the box rect.
    pub fn show_recording(&mut self, ui: &mut Ui, preview: Option<&Preview>, enabled: bool, paused: bool) -> Rect {
        let area = allocate_full(ui);
        let scale = (area.width() / 16.0).min(area.height() / 9.0);
        let box_rect = Rect::from_center_size(area.center(), Vec2::new(16.0 * scale, 9.0 * scale));
        match (preview, enabled) {
            (Some(preview), true) => self.draw_frame(ui, preview, box_rect),
            (_, true) => draw_message(ui, box_rect, "Starting preview..."),
            (_, false) => draw_message(ui, box_rect, "Preview off"),
        }
        let painter = ui.painter();
        painter.rect_stroke(box_rect, 0.0, Stroke::new(RIM, color::SEM_RECORD.gamma_multiply(RIM_ALPHA)), StrokeKind::Inside);
        rec_pill(ui, box_rect, paused);
        box_rect
    }

    /// Uploads the newest frame and letterboxes it into `area`, or draws the
    /// "Starting preview..." message until one arrives.
    fn draw_frame(&mut self, ui: &mut Ui, preview: &Preview, area: Rect) {
        let ctx = ui.ctx().clone();
        let pixels_per_point = ctx.pixels_per_point();
        let size_px = ((area.width() * pixels_per_point).round() as u32, (area.height() * pixels_per_point).round() as u32);
        if self.sent_size != Some(size_px) {
            preview.set_target_size(size_px.0, size_px.1);
            self.sent_size = Some(size_px);
        }
        if let Some(frame) = preview.latest()
            && (self.texture.is_none() || frame.seq != self.shown_seq)
        {
            self.upload(&ctx, &frame);
        }
        let Some(texture) = &self.texture else {
            draw_message(ui, area, "Starting preview...");
            return;
        };
        // The frame is sized in physical pixels, so one texel is one screen pixel when it fills the area.
        let image = Vec2::new(self.shown_size.0 as f32, self.shown_size.1 as f32) / pixels_per_point;
        let mut scale = (area.width() / image.x).min(area.height() / image.y);
        if (scale - 1.0).abs() < 0.02 {
            scale = 1.0;
        }
        let size = image * scale;
        let min = area.center() - size / 2.0;
        let snapped = pos2((min.x * pixels_per_point).round() / pixels_per_point, (min.y * pixels_per_point).round() / pixels_per_point);
        let drawn = Rect::from_min_size(snapped, size);
        ui.painter().image(texture.id(), drawn, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        self.image_rect = Some(drawn);
    }

    /// Draws the centered 16:9 box with a message, for when the preview is off or could not start.
    pub fn placeholder(ui: &mut Ui, message: &str) {
        let area = allocate_area(ui);
        draw_message(ui, area, message);
    }

    /// Copies a frame into the texture, creating the texture the first time.
    fn upload(&mut self, ctx: &Context, frame: &PreviewFrame) {
        let (width, height) = (frame.width as usize, frame.height as usize);
        if frame.rgba.len() != width * height * 4 {
            return;
        }
        let image = ColorImage::from_rgba_unmultiplied([width, height], &frame.rgba);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::LINEAR),
            None => self.texture = Some(ctx.load_texture("preview", image, TextureOptions::LINEAR)),
        }
        self.shown_seq = frame.seq;
        self.shown_size = (frame.width, frame.height);
        self.frame = Some(frame.clone());
    }
}

/// The frame pixel under a point of the drawn picture, or None when the point is outside it or the picture has no area.
fn frame_pixel(rect: Rect, pos: Pos2, size: (u32, u32)) -> Option<(u32, u32)> {
    if !rect.contains(pos) || rect.width() <= 0.0 || rect.height() <= 0.0 || size.0 == 0 || size.1 == 0 {
        return None;
    }
    let x = ((pos.x - rect.min.x) / rect.width() * size.0 as f32) as u32;
    let y = ((pos.y - rect.min.y) / rect.height() * size.1 as f32) as u32;
    Some((x.min(size.0 - 1), y.min(size.1 - 1)))
}

/// Reserves the space under the panel toolbar, leaving [`DECK_SPACE`] at the
/// bottom for the deck, and paints the dark 16:9 preview box centered in the
/// rest of that space, horizontally and vertically; returns the box.
fn allocate_area(ui: &mut Ui) -> Rect {
    let width = ui.available_width().max(1.0);
    let height = (ui.available_height() - DECK_SPACE).max(1.0);
    let (space_rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let scale = (space_rect.width() / 16.0).min(space_rect.height() / 9.0);
    let box_rect = Rect::from_center_size(space_rect.center(), Vec2::new(16.0 * scale, 9.0 * scale));
    ui.painter().rect_filled(box_rect, 0.0, BACKDROP);
    box_rect
}

/// Draws a message in the middle of the area.
fn draw_message(ui: &Ui, area: Rect, message: &str) {
    ui.painter().text(area.center(), Align2::CENTER_CENTER, message, ui_kit::fonts::medium(typography::HEADLINE_SIZE), color::TEXT_TERTIARY);
}

/// Reserves the whole remaining space for the recording preview (spec
/// "Record mode": the preview box grows to fill the center).
fn allocate_full(ui: &mut Ui) -> Rect {
    let size = Vec2::new(ui.available_width().max(1.0), ui.available_height().max(1.0));
    ui.allocate_exact_size(size, Sense::hover()).0
}

/// The REC pill at the preview box's top left: a `sem.record` tint capsule
/// with a dot and "REC", or "PAUSED" in `sem.meter.warn` while paused
/// (spec "Record mode").
fn rec_pill(ui: &Ui, box_rect: Rect, paused: bool) {
    let mark = if paused { color::SEM_METER_WARN } else { Color32::WHITE };
    let galley = ui.painter().layout_no_wrap(crate::record_ui::pill_label(paused).to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), mark);
    let width = space::SCALE_8 + PILL_DOT + space::SCALE_4 + galley.size().x + space::SCALE_8;
    let rect = Rect::from_min_size(pos2(box_rect.left() + space::SCALE_8, box_rect.top() + space::SCALE_8), vec2(width, PILL_HEIGHT));
    let painter = ui.painter();
    painter.rect_filled(rect, PILL_HEIGHT / 2.0, color::SEM_RECORD.gamma_multiply(PILL_TINT));
    painter.circle_filled(pos2(rect.left() + space::SCALE_8 + PILL_DOT / 2.0, rect.center().y), PILL_DOT / 2.0, mark);
    painter.galley(pos2(rect.left() + space::SCALE_8 + PILL_DOT + space::SCALE_4, rect.center().y - galley.size().y / 2.0), galley, mark);
}
