//! ui::timeline::ruler: the timecode ruler with markers.

use eframe::egui;

use crate::model::marker::Marker;
use crate::util::time::samples_to_timecode;

use crate::ui::theme::{THEME, Theme};

use super::{Geom, tick_step};

/// Draws the ruler strip, its ticks and labels, marker triangles with faint
/// lines down through the lane area.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    ruler_rect: egui::Rect,
    lane_rect: egui::Rect,
    g: &Geom,
    rate: u32,
    markers: &[Marker],
    text: egui::Color32,
    stroke: egui::Stroke,
) {
    // Major ticks with timecode labels, plus minor ticks between them.
    let step = tick_step(g.spp, rate, THEME.ruler_min_tick_px);
    let (start, end) = g.visible_range();
    let first = start / step * step;
    let minor = step / 5;
    let minor_px = minor as f64 / g.spp;
    let show_minor = minor_px >= 8.0;

    let mut m = first;
    while m <= end {
        let x = g.sample_to_x(m);
        if x >= ruler_rect.left() - 1.0 {
            if x > ruler_rect.right() + 1.0 {
                break;
            }
            painter.line_segment(
                [
                    egui::pos2(x, ruler_rect.bottom() - THEME.ruler_tick_h),
                    egui::pos2(x, ruler_rect.bottom()),
                ],
                stroke,
            );
            let tc = samples_to_timecode(m, rate);
            painter.text(
                egui::pos2(
                    x + THEME.ruler_label_pad_x,
                    ruler_rect.top() + THEME.ruler_label_pad_y,
                ),
                egui::Align2::LEFT_TOP,
                tc,
                Theme::mono(THEME.ruler_label_size),
                text,
            );
            if show_minor {
                for k in 1..5u64 {
                    let sub = m.checked_add(minor * k);
                    let Some(s) = sub else { break };
                    let mx = g.sample_to_x(s);
                    if mx > ruler_rect.right() {
                        break;
                    }
                    painter.line_segment(
                        [
                            egui::pos2(mx, ruler_rect.bottom() - THEME.ruler_minor_tick_h),
                            egui::pos2(mx, ruler_rect.bottom()),
                        ],
                        stroke,
                    );
                }
            }
        }
        m += step;
    }

    // Markers: a small downward triangle on the ruler and a faint vertical
    // line down through the lane area.
    let marker_color = THEME.marker_color;
    for marker in markers {
        let x = g.sample_to_x(marker.position);
        if x < g.lane_left - 1.0 || x > ruler_rect.right() + 1.0 {
            continue;
        }
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(
                    x - THEME.marker_tri_w,
                    ruler_rect.bottom() - THEME.marker_tri_h,
                ),
                egui::pos2(
                    x + THEME.marker_tri_w,
                    ruler_rect.bottom() - THEME.marker_tri_h,
                ),
                egui::pos2(x, ruler_rect.bottom()),
            ],
            marker_color,
            egui::Stroke::NONE,
        ));
        painter.line_segment(
            [
                egui::pos2(x, ruler_rect.bottom()),
                egui::pos2(x, lane_rect.bottom()),
            ],
            egui::Stroke::new(
                THEME.hairline,
                egui::Color32::from_rgba_premultiplied(
                    marker_color.r() / 4,
                    marker_color.g() / 4,
                    marker_color.b() / 4,
                    marker_color.a() / 4,
                ),
            ),
        );
    }
}
