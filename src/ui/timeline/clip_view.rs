//! ui::timeline::clip_view: drawing and hit-testing a single clip.

use eframe::egui;

use crate::io::import::Loaded;
use crate::model::clip::Clip;

use crate::ui::theme::{THEME, Theme};

use super::Geom;

/// Lowest clip gain in dB.
pub const GAIN_MIN_DB: f32 = -60.0;

/// Highest clip gain in dB.
pub const GAIN_MAX_DB: f32 = 24.0;

/// Full dB span of the gain line, from the body bottom to the body top.
pub const GAIN_SPAN_DB: f32 = GAIN_MAX_DB - GAIN_MIN_DB;

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
    /// The fade-in handle square at the top-left corner of the body.
    FadeInHandle,
    /// The fade-out handle square at the top-right corner of the body.
    FadeOutHandle,
    /// The horizontal gain line across the body.
    GainLine,
}

/// The clip body: everything below the header strip.
pub fn body_rect(clip_rect: egui::Rect) -> egui::Rect {
    let header_h = THEME.header_strip_h.min(clip_rect.height());
    egui::Rect::from_min_max(
        egui::pos2(clip_rect.left(), clip_rect.top() + header_h),
        egui::pos2(clip_rect.right(), clip_rect.bottom()),
    )
}

/// Window y of the gain line across the clip body.
///
/// +24 dB sits at the body top, -60 dB at the body bottom, 0 dB at 24/84 of
/// the height; the value is clamped to GAIN_MIN_DB..GAIN_MAX_DB.
pub fn gain_line_y(clip: &Clip, clip_rect: egui::Rect) -> f32 {
    let body = body_rect(clip_rect);
    let gain = clip.gain_db.clamp(GAIN_MIN_DB, GAIN_MAX_DB);
    body.top() + (GAIN_MAX_DB - gain) / GAIN_SPAN_DB * body.height()
}

/// Clip gain in dB for a window y in the body (the inverse of gain_line_y).
#[cfg(test)]
pub fn gain_db_at_y(y: f32, clip_rect: egui::Rect) -> f32 {
    let body = body_rect(clip_rect);
    if body.height() <= 0.0 {
        return GAIN_MIN_DB;
    }
    let t = ((y - body.top()) / body.height()).clamp(0.0, 1.0);
    GAIN_MAX_DB - t * GAIN_SPAN_DB
}

/// Grab rectangle of a fade handle square; `fade_out` picks the right one.
///
/// The square is centered on the top edge of the body, inset from the clip
/// edge by the fade length (a fade of 0 puts it at the edge itself).
pub fn fade_handle_rect(
    clip: &Clip,
    clip_rect: egui::Rect,
    spp: f64,
    fade_out: bool,
) -> egui::Rect {
    let body = body_rect(clip_rect);
    let fade = if fade_out {
        clip.fade_out
    } else {
        clip.fade_in
    };
    let fade_px = (fade as f64 / spp.max(f64::EPSILON)) as f32;
    let center_x = if fade_out {
        (clip_rect.right() - fade_px).max(clip_rect.left())
    } else {
        (clip_rect.left() + fade_px).min(clip_rect.right())
    };
    let half = THEME.fade_handle_sz * 0.5 + THEME.fade_handle_grab;
    egui::Rect::from_center_size(
        egui::pos2(center_x, body.top()),
        egui::vec2(half * 2.0, half * 2.0),
    )
}

/// Screen rect of the gain value text area at the right of the header strip
/// (the double-click target for typing a gain value).
pub fn gain_text_rect(clip_rect: egui::Rect) -> egui::Rect {
    let header_h = THEME.header_strip_h.min(clip_rect.height());
    let width = THEME.gain_text_w.min(clip_rect.width());
    egui::Rect::from_min_max(
        egui::pos2(clip_rect.right() - width, clip_rect.top()),
        egui::pos2(clip_rect.right(), clip_rect.top() + header_h),
    )
}

/// On-screen rectangle of a clip over its track lane (the lane minus 2 px margins).
pub fn clip_rect(clip: &Clip, g: &Geom, track_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            g.sample_to_x(clip.start),
            track_rect.top() + THEME.clip_margin,
        ),
        egui::pos2(
            g.sample_to_x(clip.end()),
            track_rect.bottom() - THEME.clip_margin,
        ),
    )
}

/// Which zone of the clip the point is in, or `None` when outside.
///
/// Hit priority: the fade handle squares first, then the outer 6 px edges,
/// then the gain line, then the 18 px header, then the body. Clips narrower
/// than 18 px have no edge zones.
pub fn hit_test(clip: &Clip, clip_rect: egui::Rect, spp: f64, pos: egui::Pos2) -> Option<Zone> {
    if fade_handle_rect(clip, clip_rect, spp, false).contains(pos) {
        return Some(Zone::FadeInHandle);
    }
    if fade_handle_rect(clip, clip_rect, spp, true).contains(pos) {
        return Some(Zone::FadeOutHandle);
    }
    if !clip_rect.contains(pos) {
        return None;
    }
    if clip_rect.width() >= THEME.header_strip_h {
        if pos.x <= clip_rect.left() + THEME.trim_edge_w {
            return Some(Zone::LeftEdge);
        }
        if pos.x >= clip_rect.right() - THEME.trim_edge_w {
            return Some(Zone::RightEdge);
        }
    }
    let body = body_rect(clip_rect);
    let gy = gain_line_y(clip, clip_rect);
    if pos.x >= body.left()
        && pos.x <= body.right()
        && pos.y >= body.top() - THEME.gain_hit_px
        && pos.y <= body.bottom() + THEME.gain_hit_px
        && (pos.y - gy).abs() <= THEME.gain_hit_px
    {
        return Some(Zone::GainLine);
    }
    if pos.y <= clip_rect.top() + THEME.header_strip_h.min(clip_rect.height()) {
        Some(Zone::Header)
    } else {
        Some(Zone::Body)
    }
}

/// Draws one clip over its track lane.
///
/// `track_rect` is the clip's track lane in window points; the clip body fills
/// the lane minus 2 px at the top and bottom. Clips outside the visible sample
/// range are skipped; clips narrower than 3 px draw as a single 1 px line.
/// The clip takes its body, header and waveform color from the track color of
/// `track_index`. Offline media gets a red fill and a "Media offline" label.
/// `media` is the loaded audio for the clip's bin entry, `None` when offline
/// or not loaded; its waveform fills the body below the header strip. Selected
/// clips draw with a bright border. The clip gain line and the fade curves
/// draw over the waveform; `hover` highlights the hovered handle.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    clip: &Clip,
    media_name: &str,
    offline: bool,
    media: Option<&Loaded>,
    g: &Geom,
    track_rect: egui::Rect,
    track_index: usize,
    wave_scale: f32,
    selected: bool,
    hover: Option<Zone>,
    muted: bool,
) {
    let (vis_start, vis_end) = g.visible_range();
    if clip.end() <= vis_start || clip.start >= vis_end {
        return;
    }
    let rect = clip_rect(clip, g, track_rect);

    // Clips on muted tracks draw at 45% opacity: every color below is
    // alpha-scaled through this closure.
    let scale = if muted { 0.45 } else { 1.0 };
    let dim = |color: egui::Color32| color.gamma_multiply(scale);

    // Track color of this clip: translucent body, stronger header strip and a
    // solid waveform stroke.
    let tc = dim(THEME.track_color(track_index));
    let body_fill =
        egui::Color32::from_rgba_unmultiplied(tc.r(), tc.g(), tc.b(), (0x22 as f32 * scale) as u8);
    let header_fill =
        egui::Color32::from_rgba_unmultiplied(tc.r(), tc.g(), tc.b(), (0x66 as f32 * scale) as u8);

    // Extremely narrow clip: a single 1 px line, no text.
    if rect.width() < THEME.thin_clip_w {
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.left(), rect.bottom()),
            ],
            egui::Stroke::new(THEME.hairline, body_fill),
        );
        return;
    }

    let header_h = THEME.header_strip_h.min(rect.height());
    let header_rect =
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.min.y + header_h));
    let body = egui::Rect::from_min_max(egui::pos2(rect.min.x, header_rect.max.y), rect.max);

    painter.rect_filled(
        body,
        THEME.clip_radius,
        if offline {
            dim(THEME.offline_body)
        } else {
            body_fill
        },
    );
    painter.rect_filled(
        header_rect,
        THEME.clip_radius,
        if offline {
            dim(THEME.offline_header)
        } else {
            header_fill
        },
    );

    // Waveform in the body (below the header), for online, loaded media.
    if !offline && let Some(loaded) = media {
        super::waveform::draw(painter, clip, loaded, g, body, wave_scale, tc, muted);
    }

    // Clip gain line and fade curves, over the waveform.
    if !offline {
        draw_gain_and_fades(painter, clip, g, rect, body, hover, muted);
    }

    // Unselected: 1 px inner border in the track color; selected: 2 px accent.
    let border = if selected {
        egui::Stroke::new(THEME.selection_border_w, dim(THEME.selection_border))
    } else {
        egui::Stroke::new(
            THEME.hairline,
            egui::Color32::from_rgba_unmultiplied(
                tc.r(),
                tc.g(),
                tc.b(),
                (0x55 as f32 * scale) as u8,
            ),
        )
    };
    painter.rect_stroke(rect, THEME.clip_radius, border, egui::StrokeKind::Inside);

    // Header text: the name pinned to the visible left edge and the gain
    // pinned to the visible right edge when the clip runs off the lanes, so
    // both stay readable; the name truncates with an ellipsis before the gain.
    let text_color = dim(if offline {
        THEME.offline_text
    } else {
        THEME.clip_text_color
    });
    let clipped = painter.with_clip_rect(rect);
    let gain_text = format!("{:+.1} dB", clip.gain_db);
    let gain_font = Theme::mono(THEME.clip_gain_size);
    let gain_galley = painter.layout_no_wrap(gain_text, gain_font, text_color);
    // Visible right edge of the lanes: the gain pins here when the clip's
    // right edge is scrolled out of view.
    let lane_right = g.lane_left + g.lane_width;
    let gain_right = if rect.right() > lane_right {
        lane_right - THEME.clip_text_pad
    } else {
        rect.right() - THEME.clip_text_pad
    };
    let gain_left = gain_right - gain_galley.size().x;
    // Visible left edge: the name pins here (16 px in) when the clip's left
    // edge is scrolled out of view.
    let name_x = if rect.left() < g.lane_left {
        g.lane_left + 16.0
    } else {
        rect.left() + THEME.clip_text_pad
    };
    let name_max_w = gain_left - name_x - 6.0;
    if name_max_w > 8.0 {
        let name_galley = fit_galley(
            painter,
            media_name,
            clip_name_font(),
            text_color,
            name_max_w,
        );
        clipped.galley(
            egui::pos2(name_x, header_rect.center().y - name_galley.size().y * 0.5),
            name_galley,
            text_color,
        );
    }
    // The gain value at the right of the header is the double-click target.
    clipped.galley(
        egui::pos2(
            gain_left,
            header_rect.center().y - gain_galley.size().y * 0.5,
        ),
        gain_galley,
        text_color,
    );
    if offline {
        clipped.text(
            body.center(),
            egui::Align2::CENTER_CENTER,
            "Media offline",
            egui::FontId::proportional(THEME.clip_name_size),
            dim(THEME.offline_text),
        );
    }
}

/// Font of the clip name in the header strip.
fn clip_name_font() -> egui::FontId {
    egui::FontId::proportional(THEME.clip_name_size)
}

/// Lays out `text` in `font`, truncating with an ellipsis until it fits
/// `max_w` (or a single character remains).
fn fit_galley(
    painter: &egui::Painter,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let full = painter.layout_no_wrap(text.to_owned(), font.clone(), color);
    if full.size().x <= max_w {
        return full;
    }
    let mut candidate: String = text.to_owned();
    loop {
        let char_count = candidate.chars().count();
        if char_count <= 1 {
            return full;
        }
        candidate = candidate.chars().take(char_count - 1).collect();
        candidate.push('\u{2026}');
        let galley = painter.layout_no_wrap(candidate.clone(), font.clone(), color);
        if galley.size().x <= max_w {
            return galley;
        }
        // Keep trimming; the trailing ellipsis is re-added on the next pass.
        if candidate.chars().count() <= 2 {
            return galley;
        }
    }
}

/// Draws the clip gain line, the fade curves and the fade handle squares.
///
/// Each fade curve is a line from the bottom of the body at the clip edge up
/// to the top of the body at the end of the fade; the area outside the curve
/// (between the curve and the clip edge above it) is darkened. The handles
/// draw as 8x8 squares at the clip's real top corners, brighter while
/// hovered. On a muted track everything draws at 45% opacity.
fn draw_gain_and_fades(
    painter: &egui::Painter,
    clip: &Clip,
    g: &Geom,
    rect: egui::Rect,
    body: egui::Rect,
    hover: Option<Zone>,
    muted: bool,
) {
    let scale = if muted { 0.45 } else { 1.0 };
    let dim = |color: egui::Color32| color.gamma_multiply(scale);
    let clipped = painter.with_clip_rect(body);
    // Gain line: a dark under-shadow plus a bright line, visible on light and
    // dark waveforms alike.
    let gy = gain_line_y(clip, rect);
    if gy >= body.top() - 0.5 && gy <= body.bottom() + 0.5 {
        clipped.line_segment(
            [egui::pos2(body.left(), gy), egui::pos2(body.right(), gy)],
            egui::Stroke::new(THEME.gain_shadow_w, dim(THEME.gain_shadow)),
        );
        clipped.line_segment(
            [egui::pos2(body.left(), gy), egui::pos2(body.right(), gy)],
            egui::Stroke::new(THEME.hairline, dim(THEME.gain_line)),
        );
    }

    let shade = egui::Stroke::NONE;
    let shade_fill = dim(THEME.fade_shade);
    let curve = egui::Stroke::new(1.2, dim(THEME.gain_curve));
    if clip.fade_in > 0 {
        let fx = (rect.left() + (clip.fade_in as f64 / g.spp.max(f64::EPSILON)) as f32)
            .min(rect.right());
        // Outside the curve: the triangle above the fade-in line.
        clipped.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(rect.left(), body.top()),
                egui::pos2(fx, body.top()),
                egui::pos2(rect.left(), body.bottom()),
            ],
            shade_fill,
            shade,
        ));
        clipped.line_segment(
            [
                egui::pos2(rect.left(), body.bottom()),
                egui::pos2(fx, body.top()),
            ],
            curve,
        );
    }
    if clip.fade_out > 0 {
        let fx = (rect.right() - (clip.fade_out as f64 / g.spp.max(f64::EPSILON)) as f32)
            .max(rect.left());
        clipped.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(rect.right(), body.top()),
                egui::pos2(fx, body.top()),
                egui::pos2(rect.right(), body.bottom()),
            ],
            shade_fill,
            shade,
        ));
        clipped.line_segment(
            [
                egui::pos2(rect.right(), body.bottom()),
                egui::pos2(fx, body.top()),
            ],
            curve,
        );
    }

    for (zone, fade_out) in [(Zone::FadeInHandle, false), (Zone::FadeOutHandle, true)] {
        // The square always sits on the clip's real top corner.
        let corner_x = if fade_out { rect.right() } else { rect.left() };
        let square = egui::Rect::from_center_size(
            egui::pos2(corner_x, body.top()),
            egui::vec2(THEME.fade_handle_sz, THEME.fade_handle_sz),
        );
        let fill = if hover == Some(zone) {
            dim(THEME.fade_handle_hover)
        } else {
            dim(THEME.fade_handle_fill)
        };
        clipped.rect_filled(square, THEME.corner_r, fill);
        clipped.rect_stroke(
            square,
            THEME.corner_r,
            egui::Stroke::new(THEME.hairline, dim(THEME.fade_handle_stroke)),
            egui::StrokeKind::Inside,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_clip(gain_db: f32, fade_in: u64, fade_out: u64) -> Clip {
        Clip {
            id: uuid::Uuid::nil(),
            media_id: uuid::Uuid::nil(),
            start: 0,
            src_in: 0,
            src_out: 1_000,
            gain_db,
            fade_in,
            fade_out,
        }
    }

    #[test]
    fn hit_test_zones_on_a_200x80_clip() {
        // 0 dB gain line sits about 40 px down; points are away from it.
        let clip = test_clip(0.0, 0, 0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 80.0));
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(2.0, 65.0)),
            Some(Zone::LeftEdge)
        );
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(197.0, 65.0)),
            Some(Zone::RightEdge)
        );
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(100.0, 10.0)),
            Some(Zone::Header)
        );
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(100.0, 55.0)),
            Some(Zone::Body)
        );
        assert_eq!(hit_test(&clip, rect, 100.0, egui::pos2(250.0, 50.0)), None);
    }

    #[test]
    fn a_12px_wide_clip_has_no_edge_zones() {
        let clip = test_clip(0.0, 0, 0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(12.0, 80.0));
        // The gain line sits at about y = 40 and the fade handle squares
        // around the body corners at y = 18; the points avoid all of them.
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(2.0, 60.0)),
            Some(Zone::Body)
        );
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(10.0, 60.0)),
            Some(Zone::Body)
        );
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(6.0, 10.0)),
            Some(Zone::Header)
        );
    }

    #[test]
    fn fade_in_handle_wins_over_edge_and_header() {
        // No fade: the handle square is centered on the body's top-left corner,
        // half inside the header and the left edge zone - and still wins.
        let clip = test_clip(0.0, 0, 0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 80.0));
        let grab = fade_handle_rect(&clip, rect, 100.0, false);
        let p = grab.center();
        assert!(
            p.x <= rect.left() + THEME.trim_edge_w,
            "handle must be at the left edge"
        );
        assert!(
            p.y <= rect.top() + THEME.header_strip_h,
            "handle must be in the header row"
        );
        assert_eq!(hit_test(&clip, rect, 100.0, p), Some(Zone::FadeInHandle));
    }

    #[test]
    fn fade_out_handle_hits_at_the_right_body_corner() {
        let clip = test_clip(0.0, 0, 300);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 80.0));
        let grab = fade_handle_rect(&clip, rect, 100.0, true);
        assert_eq!(grab.center().x, rect.right() - 3.0);
        assert_eq!(
            hit_test(&clip, rect, 100.0, grab.center()),
            Some(Zone::FadeOutHandle)
        );
    }

    #[test]
    fn gain_line_y_maps_db_across_the_body() {
        // Body height 84 px: +24 dB at the body top, -60 dB at the bottom and
        // 0 dB at 24/84 of the height.
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(200.0, 84.0 + THEME.header_strip_h),
        );
        let body = body_rect(rect);
        assert!((body.height() - 84.0).abs() < 1e-4);
        let top = gain_line_y(&test_clip(24.0, 0, 0), rect);
        let zero = gain_line_y(&test_clip(0.0, 0, 0), rect);
        let bottom = gain_line_y(&test_clip(-60.0, 0, 0), rect);
        assert!((top - body.top()).abs() < 1e-4);
        assert!((bottom - body.bottom()).abs() < 1e-4);
        assert!((zero - (body.top() + 24.0 / 84.0 * body.height())).abs() < 1e-4);
        assert!((gain_db_at_y(body.top(), rect) - 24.0).abs() < 1e-4);
        assert!((gain_db_at_y(body.bottom(), rect) + 60.0).abs() < 1e-4);
        assert!(gain_db_at_y(body.top() + 24.0, rect).abs() < 1e-4);
    }

    #[test]
    fn gain_line_hit_zone_around_the_line() {
        let clip = test_clip(-36.0, 0, 0);
        let rect = egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(200.0, 84.0 + THEME.header_strip_h),
        );
        let gy = gain_line_y(&clip, rect);
        assert_eq!(
            hit_test(&clip, rect, 100.0, egui::pos2(100.0, gy)),
            Some(Zone::GainLine)
        );
        assert_eq!(
            hit_test(
                &clip,
                rect,
                100.0,
                egui::pos2(100.0, gy + THEME.gain_hit_px + 0.5)
            ),
            Some(Zone::Body)
        );
    }
}
