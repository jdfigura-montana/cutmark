//! ui::timeline::clip_view: drawing and hit-testing a single clip.

use eframe::egui;

use crate::io::import::Loaded;
use crate::model::clip::Clip;

use super::Geom;

/// Height of the clip header strip in points.
pub const HEADER_H: f32 = 18.0;

/// Width of the trimmable edge zones in points.
pub const EDGE_W: f32 = 6.0;

/// Which part of a clip a pointer is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The top header strip: selects and drags to move.
    Header,
    /// The body below the header: playhead clicks and range drags.
    Body,
    /// The left 6 px: trims the clip start.
    LeftEdge,
    /// The right 6 px: trims the clip end.
    RightEdge,
}

/// On-screen rectangle of a clip over its track lane (the lane minus 2 px margins).
pub fn clip_rect(clip: &Clip, g: &Geom, track_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(g.sample_to_x(clip.start), track_rect.top() + 2.0),
        egui::pos2(g.sample_to_x(clip.end()), track_rect.bottom() - 2.0),
    )
}

/// Which zone of `clip_rect` the point is in, or `None` when outside.
///
/// The outer 6 px on each side (full height) are the edges and win over the
/// header and body; the header is the top 18 px. Clips narrower than 18 px
/// have no edge zones.
pub fn hit_test(clip_rect: egui::Rect, pos: egui::Pos2) -> Option<Zone> {
    if !clip_rect.contains(pos) {
        return None;
    }
    if clip_rect.width() >= 18.0 {
        if pos.x <= clip_rect.left() + EDGE_W {
            return Some(Zone::LeftEdge);
        }
        if pos.x >= clip_rect.right() - EDGE_W {
            return Some(Zone::RightEdge);
        }
    }
    if pos.y <= clip_rect.top() + HEADER_H.min(clip_rect.height()) {
        Some(Zone::Header)
    } else {
        Some(Zone::Body)
    }
}

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
/// its waveform fills the body below the header strip. Selected clips draw
/// with a bright border and a lighter header strip.
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
    selected: bool,
) {
    let (vis_start, vis_end) = g.visible_range();
    if clip.end() <= vis_start || clip.start >= vis_end {
        return;
    }
    let rect = clip_rect(clip, g, track_rect);

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
        } else if selected {
            lighten(body_fill(visuals), 0.45)
        } else {
            darken(body_fill(visuals), 0.7)
        },
    );

    // Waveform in the body (below the header), for online, loaded media.
    if !offline && let Some(loaded) = media {
        super::waveform::draw(painter, clip, loaded, g, body_rect, wave_scale);
    }
    let border = if selected {
        egui::Stroke::new(2.0, selection_border())
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    painter.rect_stroke(rect, 2.0, border, egui::StrokeKind::Inside);

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

/// Bright border of a selected clip.
fn selection_border() -> egui::Color32 {
    egui::Color32::from_rgb(120, 170, 255)
}

/// Darkens a color by the factor `f` (0.0..1.0) per channel.
fn darken(color: egui::Color32, f: f32) -> egui::Color32 {
    egui::Color32::from_rgb(
        (color.r() as f32 * f) as u8,
        (color.g() as f32 * f) as u8,
        (color.b() as f32 * f) as u8,
    )
}

/// Lightens a color toward white by the fraction `f` (0.0..1.0) per channel.
fn lighten(color: egui::Color32, f: f32) -> egui::Color32 {
    let mix = |c: u8| (c as f32 + (255.0 - c as f32) * f) as u8;
    egui::Color32::from_rgb(mix(color.r()), mix(color.g()), mix(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_test_zones_on_a_200x80_clip() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 80.0));
        assert_eq!(hit_test(rect, egui::pos2(2.0, 40.0)), Some(Zone::LeftEdge));
        assert_eq!(
            hit_test(rect, egui::pos2(197.0, 40.0)),
            Some(Zone::RightEdge)
        );
        assert_eq!(hit_test(rect, egui::pos2(100.0, 10.0)), Some(Zone::Header));
        assert_eq!(hit_test(rect, egui::pos2(100.0, 50.0)), Some(Zone::Body));
        assert_eq!(hit_test(rect, egui::pos2(250.0, 50.0)), None);
    }

    #[test]
    fn a_12px_wide_clip_has_no_edge_zones() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(12.0, 80.0));
        assert_eq!(hit_test(rect, egui::pos2(2.0, 40.0)), Some(Zone::Body));
        assert_eq!(hit_test(rect, egui::pos2(10.0, 40.0)), Some(Zone::Body));
        assert_eq!(hit_test(rect, egui::pos2(6.0, 10.0)), Some(Zone::Header));
    }
}
