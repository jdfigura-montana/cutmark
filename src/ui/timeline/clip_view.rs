//! ui::timeline::clip_view: drawing and hit-testing a single clip.

use eframe::egui;

use crate::io::import::Loaded;
use crate::model::clip::Clip;

use super::Geom;

/// Height of the clip header strip in points.
pub const HEADER_H: f32 = 18.0;

/// Fill of an offline clip's body.
fn offline_body() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(170, 45, 45, 220)
}
/// Fill of an offline clip's header strip.
fn offline_header() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(120, 25, 25, 230)
}
/// Horizontal padding of the header text.
const TEXT_PAD: f32 = 4.0;

/// Draws one clip over its track lane.
///
/// `track_rect` is the clip's track lane in window points; the clip body fills
/// the lane minus 2 px at the top and bottom. Clips outside the visible sample
/// range are skipped; clips narrower than 3 px draw as a single 1 px line.
/// Offline media gets a red fill and a "Media offline" label. `media` is the
/// loaded audio for the clip's bin entry, `None` when offline or not loaded;
/// its waveform fills the body below the header strip.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    clip: &Clip,
    media_name: &str,
    offline: bool,
    media: Option<&Loaded>,
    g: &Geom,
    track_rect: egui::Rect,
    visuals: &egui::Visuals,
    wave_scale: f32,
) {
    let (vis_start, vis_end) = g.visible_range();
    if clip.end() <= vis_start || clip.start >= vis_end {
        return;
    }
    let rect = egui::Rect::from_min_max(
        egui::pos2(g.sample_to_x(clip.start), track_rect.top() + 2.0),
        egui::pos2(g.sample_to_x(clip.end()), track_rect.bottom() - 2.0),
    );

    // Extremely narrow clip: a single 1 px line, no text.
    if rect.width() < 3.0 {
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.left(), rect.bottom()),
            ],
            egui::Stroke::new(1.0, body_fill(visuals)),
        );
        return;
    }

    let header_h = HEADER_H.min(rect.height());
    let header_rect =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + header_h));
    let body_rect = egui::Rect::from_min_max(egui::pos2(rect.min.x, header_rect.max.y), rect.max);

    painter.rect_filled(
        body_rect,
        2.0,
        if offline {
            offline_body()
        } else {
            body_fill(visuals)
        },
    );
    painter.rect_filled(
        header_rect,
        2.0,
        if offline {
            offline_header()
        } else {
            darken(body_fill(visuals), 0.7)
        },
    );

    // Waveform in the body (below the header), for online, loaded media.
    if !offline && let Some(loaded) = media {
        super::waveform::draw(painter, clip, loaded, g, body_rect, wave_scale);
    }
    painter.rect_stroke(
        rect,
        2.0,
        visuals.widgets.noninteractive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let text_color = if offline {
        egui::Color32::WHITE
    } else {
        visuals.strong_text_color()
    };
    let clipped = painter.with_clip_rect(rect);
    clipped.text(
        egui::pos2(rect.left() + TEXT_PAD, header_rect.center().y),
        egui::Align2::LEFT_CENTER,
        media_name,
        egui::FontId::proportional(12.0),
        text_color,
    );
    if clip.gain_db != 0.0 {
        clipped.text(
            egui::pos2(rect.right() - TEXT_PAD, header_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{:+.1} dB", clip.gain_db),
            egui::FontId::proportional(11.0),
            text_color,
        );
    }
    if offline {
        clipped.text(
            body_rect.center(),
            egui::Align2::CENTER_CENTER,
            "Media offline",
            egui::FontId::proportional(12.0),
            egui::Color32::WHITE,
        );
    }
}

/// The clip body fill: a lighter shade than the track lane.
fn body_fill(visuals: &egui::Visuals) -> egui::Color32 {
    visuals.widgets.hovered.bg_fill
}

/// Darkens a color by the factor `f` (0.0..1.0) per channel.
fn darken(color: egui::Color32, f: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (color.r() as f32 * f) as u8,
        (color.g() as f32 * f) as u8,
        (color.b() as f32 * f) as u8,
    )
}
