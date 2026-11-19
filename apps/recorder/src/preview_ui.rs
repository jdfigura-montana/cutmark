//! Draws the live monitor preview in the main area of the window.

use capture::{Preview, PreviewFrame};
use egui::{Align2, Color32, ColorImage, Context, FontId, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2};

/// The dark colour around and behind the picture.
const BACKDROP: Color32 = Color32::from_gray(14);

/// Height over width of the area before the first frame says what the monitor's shape is.
const AREA_ASPECT: f32 = 9.0 / 16.0;

/// The share of the free height the preview area may take, so the rest of the window stays reachable.
const MAX_HEIGHT_SHARE: f32 = 0.5;

/// The least height the preview area gets, in points.
const MIN_AREA_HEIGHT: f32 = 180.0;

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
}

impl PreviewView {
    /// Forgets what was sent to the preview and what it showed; call it when a new preview starts.
    pub fn reset(&mut self) {
        self.shown_seq = 0;
        self.sent_size = None;
    }

    /// Draws the newest preview frame, or "Starting preview..." until one arrives.
    pub fn show(&mut self, ui: &mut Ui, preview: &Preview) {
        let area = allocate_area(ui);
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
        ui.painter().image(texture.id(), Rect::from_min_size(snapped, size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }

    /// Draws a plain dark area with a message, for when the preview is off or could not start.
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
    }
}

/// Reserves the full width and paints the dark backdrop; returns the area.
fn allocate_area(ui: &mut Ui) -> Rect {
    let width = ui.available_width().max(1.0);
    let max_height = (ui.available_height() * MAX_HEIGHT_SHARE).max(MIN_AREA_HEIGHT);
    let height = (width * AREA_ASPECT).min(max_height);
    let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(area, 0.0, BACKDROP);
    area
}

/// Draws a message in the middle of the area.
fn draw_message(ui: &Ui, area: Rect, message: &str) {
    ui.painter().text(area.center(), Align2::CENTER_CENTER, message, FontId::proportional(16.0), Color32::GRAY);
}
