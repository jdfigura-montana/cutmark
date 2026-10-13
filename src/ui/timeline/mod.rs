//! ui::timeline: the central timeline view (ruler, lanes, playhead, zoom, scroll).

use eframe::egui;

use crate::editor::Editor;
use crate::util::time::seconds_to_samples;

pub mod clip_view;
pub mod interaction;
pub mod lane;
pub mod ruler;
pub mod track_header;
pub mod waveform;

/// Height of the ruler strip in points.
pub const RULER_H: f32 = 28.0;
/// Width of the track header column in points.
pub const HEADER_W: f32 = 200.0;
/// Height of the horizontal scrollbar strip in points.
pub const SCROLLBAR_H: f32 = 12.0;
/// Smallest zoom: one thirty-second of a sample per pixel.
pub const MIN_SPP: f64 = 1.0 / 32.0;
/// Largest zoom: 4 194 304 samples per pixel.
pub const MAX_SPP: f64 = 4_194_304.0;

/// Ruler tick step, in seconds, from the coarsest-fitting list.
const TICK_STEPS: [f64; 21] = [
    0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0,
    120.0, 300.0, 600.0, 1800.0, 3600.0,
];

/// Geometry of the timeline lane area for one frame.
pub struct Geom {
    /// Left edge of the lane area in window points.
    pub lane_left: f32,
    /// Width of the lane area in points.
    pub lane_width: f32,
    /// Horizontal zoom in samples per pixel.
    pub spp: f64,
    /// First visible sample.
    pub scroll_x: u64,
}

impl Geom {
    /// Window x position of a timeline sample.
    pub fn sample_to_x(&self, sample: u64) -> f32 {
        self.lane_left + ((sample as f64 - self.scroll_x as f64) / self.spp) as f32
    }

    /// Timeline sample at a window x position, clamped at 0.
    pub fn x_to_sample(&self, x: f32) -> u64 {
        let dx = (x - self.lane_left) as f64;
        ((dx * self.spp + self.scroll_x as f64).max(0.0)) as u64
    }

    /// First and one-past-last visible sample.
    pub fn visible_range(&self) -> (u64, u64) {
        (
            self.scroll_x,
            self.scroll_x + (self.lane_width as f64 * self.spp).ceil() as u64,
        )
    }
}

/// Zooms horizontally by `factor`, keeping the sample under the anchor pixel fixed.
///
/// `anchor_offset_px` is measured from the left edge of the lane area. The new
/// samples-per-pixel is clamped to MIN_SPP..=MAX_SPP and the new scroll
/// position never goes below zero.
pub fn zoom_at(spp: f64, scroll_x: u64, factor: f64, anchor_offset_px: f32) -> (f64, u64) {
    let new_spp = (spp * factor).clamp(MIN_SPP, MAX_SPP);
    let anchor = scroll_x as f64 + f64::from(anchor_offset_px) * spp;
    let new_scroll = anchor - f64::from(anchor_offset_px) * new_spp;
    (new_spp, new_scroll.round().max(0.0) as u64)
}

/// Smallest ruler tick step (in samples) that is at least `min_px` wide at
/// the given zoom and sample rate. Uses the largest step when none fit.
pub fn tick_step(spp: f64, rate: u32, min_px: f32) -> u64 {
    let min_px = f64::from(min_px);
    for step in TICK_STEPS {
        let samples = seconds_to_samples(step, rate);
        if samples as f64 / spp >= min_px {
            return samples;
        }
    }
    seconds_to_samples(TICK_STEPS[TICK_STEPS.len() - 1], rate)
}

/// Total scrollable span in samples: the larger of project length and visible
/// width, plus 10 percent extra.
fn scroll_extent(project_length: u64, visible: u64) -> u64 {
    (project_length.max(visible) as f64 * 1.1).ceil() as u64
}

/// Draws the whole timeline into the remaining center space and handles its
/// input (zoom, scroll, playhead clicks). Not a ScrollArea: everything is
/// painted by hand over one allocated rect.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    // The timeline owns horizontal zoom; egui's own keyboard and Ctrl+wheel
    // UI zoom must not fire here.
    ui.ctx().options_mut(|o| o.zoom_with_keyboard = false);

    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click());

    let lane_left = avail.left() + HEADER_W;
    let lane_right = avail.right();
    let lane_width = lane_right - lane_left;
    let ruler_rect = egui::Rect::from_min_max(
        egui::pos2(lane_left, avail.top()),
        egui::pos2(lane_right, avail.top() + RULER_H),
    );
    let header_rect = egui::Rect::from_min_max(
        egui::pos2(avail.left(), ruler_rect.bottom()),
        egui::pos2(lane_left, avail.bottom()),
    );
    let lane_rect = egui::Rect::from_min_max(
        egui::pos2(lane_left, ruler_rect.bottom()),
        egui::pos2(lane_right, avail.bottom() - SCROLLBAR_H),
    );
    let scroll_rect = egui::Rect::from_min_max(
        egui::pos2(lane_left, lane_rect.bottom()),
        egui::pos2(lane_right, avail.bottom()),
    );

    let content_h: f32 = editor.project.tracks.iter().map(|t| t.height).sum();
    let max_scroll_y = (content_h - lane_rect.height()).max(0.0);

    // Input first: it may change spp, scroll_x, scroll_y and the playhead.
    handle_input(
        ui,
        editor,
        lane_left,
        lane_width,
        ruler_rect,
        lane_rect,
        max_scroll_y,
        &resp,
    );

    let view = editor.view.clone();
    let g = Geom {
        lane_left,
        lane_width,
        spp: view.samples_per_pixel,
        scroll_x: view.scroll_x,
    };
    let visible = g.visible_range().1 - g.visible_range().0;
    let length = editor.project_length();
    let max_scroll_x = scroll_extent(length, visible).saturating_sub(visible);

    draw(
        ui,
        editor,
        &g,
        ruler_rect,
        header_rect,
        lane_rect,
        scroll_rect,
        visible,
        max_scroll_x,
    );
}

/// Wheel, keyboard and click input over the timeline.
#[allow(clippy::too_many_arguments)]
fn handle_input(
    ui: &egui::Ui,
    editor: &mut Editor,
    lane_left: f32,
    lane_width: f32,
    ruler_rect: egui::Rect,
    lane_rect: egui::Rect,
    max_scroll_y: f32,
    resp: &egui::Response,
) {
    if !resp.hovered() {
        return;
    }

    // Clicking the ruler or empty lane space sets the playhead.
    if resp.clicked()
        && !ui.ctx().egui_wants_keyboard_input()
        && let Some(p) = resp.interact_pointer_pos()
        && (ruler_rect.contains(p) || lane_rect.contains(p))
        && p.y < lane_rect.bottom()
    {
        let g = Geom {
            lane_left,
            lane_width,
            spp: editor.view.samples_per_pixel,
            scroll_x: editor.view.scroll_x,
        };
        editor.set_playhead(g.x_to_sample(p.x));
    }

    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }

    ui.input(|i| {
        let zoom = i.zoom_delta();
        if zoom != 1.0 {
            // Ctrl + wheel: zoom anchored at the mouse, 1.25 per notch.
            // egui keeps the delta out of smooth_scroll_delta when the zoom
            // modifier is down, so this does exactly one thing.
            if let Some(p) = i.pointer.latest_pos() {
                let offset = (p.x - lane_left).clamp(0.0, lane_width);
                let factor = if zoom > 1.0 { 1.25 } else { 1.0 / 1.25 };
                let (spp, scroll_x) = zoom_at(
                    editor.view.samples_per_pixel,
                    editor.view.scroll_x,
                    factor,
                    offset,
                );
                editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
            }
        } else if i.modifiers.shift {
            // Shift + wheel: egui already folds the wheel delta into the
            // horizontal component; scroll left/right by ~100 px per notch.
            let d = i.smooth_scroll_delta().x;
            if d != 0.0 {
                let spp = editor.view.samples_per_pixel;
                let new_x = (editor.view.scroll_x as f64 - d as f64)
                    .round()
                    .clamp(0.0, f64::from(u32::MAX)) as u64;
                editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, new_x));
            }
        } else {
            // Plain wheel: scroll tracks vertically.
            let d = i.smooth_scroll_delta().y;
            if d != 0.0 {
                let y = (editor.view.scroll_y - d).clamp(0.0, max_scroll_y);
                editor.set_scroll_y(y);
            }
        }

        // + / - (and = and the numpad keys) zoom by 1.5, anchored on the
        // playhead if visible, else the view center.
        let factor = if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
            Some(1.5)
        } else if i.key_pressed(egui::Key::Minus) {
            Some(1.0 / 1.5)
        } else {
            None
        };
        if let Some(factor) = factor {
            let g = Geom {
                lane_left,
                lane_width,
                spp: editor.view.samples_per_pixel,
                scroll_x: editor.view.scroll_x,
            };
            let playhead_x = g.sample_to_x(editor.view.playhead);
            let offset = if playhead_x >= lane_left && playhead_x <= lane_left + lane_width {
                playhead_x - lane_left
            } else {
                lane_width / 2.0
            };
            let (spp, scroll_x) = zoom_at(
                editor.view.samples_per_pixel,
                editor.view.scroll_x,
                factor,
                offset,
            );
            editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
        }
    });
}

/// Largest legal scroll_x for the given zoom, so the view cannot scroll past
/// the (10 percent padded) end of the project.
fn clamp_scroll_x(editor: &Editor, spp: f64, lane_width: f32, scroll_x: u64) -> u64 {
    let visible = (lane_width as f64 * spp).ceil() as u64;
    let length = editor.project_length();
    scroll_x.min(scroll_extent(length, visible).saturating_sub(visible))
}

/// Draws the header column, lanes, ruler, markers, playhead and scrollbar.
#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    g: &Geom,
    ruler_rect: egui::Rect,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_rect: egui::Rect,
    visible: u64,
    max_scroll_x: u64,
) {
    let all = ruler_rect
        .union(header_rect)
        .union(lane_rect)
        .union(scroll_rect);
    let painter = ui.painter_at(all);
    let visuals = ui.visuals().clone();
    let view = editor.view.clone();

    // Track lanes: alternating shades and separators (ui::timeline::lane).
    let lane_painter = ui.painter_at(lane_rect);
    let mut y = lane_rect.top() - view.scroll_y;
    for (i, track) in editor.project.tracks.iter().enumerate() {
        let bottom = y + track.height;
        if bottom >= lane_rect.top() && y <= lane_rect.bottom() {
            lane::draw(
                &lane_painter,
                egui::Rect::from_min_max(
                    egui::pos2(g.lane_left, y),
                    egui::pos2(lane_rect.right(), bottom),
                ),
                i % 2 == 0,
                visuals.extreme_bg_color,
                visuals.faint_bg_color,
                visuals.widgets.noninteractive.bg_stroke,
            );
        }
        y = bottom;
    }

    // Ruler: ticks, labels, markers and their faint lines.
    ruler::draw(
        &painter,
        ruler_rect,
        lane_rect,
        g,
        editor.project.sample_rate,
        &editor.project.markers,
        visuals.text_color(),
        visuals.widgets.noninteractive.bg_stroke,
    );

    // Playhead: a line through ruler and lanes with a handle in the ruler.
    let playhead_color = egui::Color32::from_rgb(255, 140, 60);
    let ph_x = g.sample_to_x(view.playhead);
    if ph_x >= g.lane_left - 1.0 && ph_x <= lane_rect.right() + 1.0 {
        painter.line_segment(
            [
                egui::pos2(ph_x, ruler_rect.top()),
                egui::pos2(ph_x, lane_rect.bottom()),
            ],
            egui::Stroke::new(1.5, playhead_color),
        );
        let tri = vec![
            egui::pos2(ph_x - 5.0, ruler_rect.top() + 1.0),
            egui::pos2(ph_x + 5.0, ruler_rect.top() + 1.0),
            egui::pos2(ph_x, ruler_rect.top() + 10.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            tri,
            playhead_color,
            egui::Stroke::NONE,
        ));
    }

    // Horizontal scrollbar: trough plus draggable thumb.
    painter.rect_filled(scroll_rect, 2.0, visuals.faint_bg_color);
    let frac = (visible as f32 / scroll_extent(editor.project_length(), visible) as f32).min(1.0);
    let thumb_w = (scroll_rect.width() * frac).max(20.0);
    let scroll_max = max_scroll_x as f32;
    let thumb_x = g.lane_left
        + if scroll_max > 0.0 {
            (view.scroll_x as f32 / scroll_max) * (scroll_rect.width() - thumb_w)
        } else {
            0.0
        };
    let thumb_rect = egui::Rect::from_min_max(
        egui::pos2(thumb_x, scroll_rect.top() + 1.0),
        egui::pos2(thumb_x + thumb_w, scroll_rect.bottom() - 1.0),
    );
    let thumb_color = if thumb_held(ui, thumb_rect) {
        visuals.widgets.active.bg_fill
    } else if thumb_hover(ui, thumb_rect) {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    painter.rect_filled(thumb_rect, 3.0, thumb_color);
    let thumb = ui.interact(
        thumb_rect,
        egui::Id::new("timeline_hscroll_thumb"),
        egui::Sense::click_and_drag(),
    );
    if thumb.dragged() {
        let d = thumb.drag_delta().x;
        if d != 0.0 {
            let new_x = (view.scroll_x as f64 - d as f64 * g.spp)
                .round()
                .clamp(0.0, max_scroll_x as f64) as u64;
            editor.set_view_x(view.samples_per_pixel, new_x);
        }
    }

    // Track headers: grip, rename, menu, resize, reorder (with lanes scrolled).
    track_header::show(ui, editor, header_rect, lane_rect, view.scroll_y);
}

/// True while the pointer hovers the scrollbar thumb.
fn thumb_hover(ui: &egui::Ui, thumb_rect: egui::Rect) -> bool {
    ui.input(|i| {
        i.pointer
            .latest_pos()
            .is_some_and(|p| thumb_rect.contains(p))
    })
}

/// True while the primary button is down over the scrollbar thumb.
fn thumb_held(ui: &egui::Ui, thumb_rect: egui::Rect) -> bool {
    ui.input(|i| {
        i.pointer.primary_down()
            && i.pointer
                .latest_pos()
                .is_some_and(|p| thumb_rect.contains(p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(spp: f64, scroll_x: u64) -> Geom {
        Geom {
            lane_left: 10.0,
            lane_width: 1000.0,
            spp,
            scroll_x,
        }
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_1() {
        let g = geom(1.0, 0);
        assert_eq!(g.sample_to_x(500), 510.0);
        assert_eq!(g.x_to_sample(510.0), 500);
        assert_eq!(g.x_to_sample(g.sample_to_x(1_000)), 1_000);
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_100() {
        let g = geom(100.0, 2_000);
        assert_eq!(g.sample_to_x(5_000), 40.0);
        assert_eq!(g.x_to_sample(40.0), 5_000);
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_quarter() {
        let g = geom(0.25, 0);
        assert_eq!(g.sample_to_x(7), 38.0);
        assert_eq!(g.x_to_sample(38.0), 7);
    }

    #[test]
    fn x_to_sample_clamps_at_zero() {
        let g = geom(1.0, 0);
        assert_eq!(g.x_to_sample(0.0), 0);
        assert_eq!(g.x_to_sample(-50.0), 0);
    }

    #[test]
    fn x_left_of_the_view_maps_inside_the_timeline() {
        // Only x below lane_left - scroll_x/spp clamps; otherwise it is a
        // legitimate sample before the view start.
        let g = geom(1.0, 1_000);
        assert_eq!(g.x_to_sample(0.0), 990);
    }

    #[test]
    fn visible_range_follows_scroll_and_zoom() {
        assert_eq!(geom(1.0, 0).visible_range(), (0, 1000));
        assert_eq!(geom(10.0, 2_500).visible_range(), (2_500, 12_500));
    }

    #[test]
    fn zoom_at_keeps_the_anchor_sample_at_factors_half_and_two() {
        let spp = 100.0;
        let scroll_x = 100_000;
        let offset = 50.0;
        let anchor = scroll_x as f64 + offset as f64 * spp;

        let (spp_in, scroll_in) = zoom_at(spp, scroll_x, 2.0, offset);
        let kept = scroll_in as f64 + offset as f64 * spp_in;
        assert!((kept - anchor).abs() <= 1.0);

        let (spp_out, scroll_out) = zoom_at(spp, scroll_x, 0.5, offset);
        let kept = scroll_out as f64 + offset as f64 * spp_out;
        assert!((kept - anchor).abs() <= 1.0);
    }

    #[test]
    fn zoom_at_clamps_at_min_and_max_spp() {
        let (spp, _) = zoom_at(MAX_SPP, 1_000, 2.0, 100.0);
        assert_eq!(spp, MAX_SPP);
        let (spp, _) = zoom_at(MIN_SPP, 1_000, 0.5, 100.0);
        assert_eq!(spp, MIN_SPP);
    }

    #[test]
    fn zoom_at_never_makes_scroll_x_negative() {
        let (_, scroll_x) = zoom_at(1_000_000.0, 1_000, 2.0, 10.0);
        assert_eq!(scroll_x, 0);
        let (_, scroll_x) = zoom_at(1_000_000.0, 0, 2.0, 0.0);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn tick_step_48k_spp_10_min_90_px() {
        assert_eq!(tick_step(10.0, 48_000, 90.0), 960);
    }

    #[test]
    fn tick_step_48k_spp_48000_min_90_px() {
        assert_eq!(tick_step(48_000.0, 48_000, 90.0), 5_760_000);
    }

    #[test]
    fn tick_step_falls_back_to_largest() {
        // Extremely zoomed in: even 1 ms is far more than 90 px.
        assert_eq!(tick_step(0.1, 48_000, 90.0), 48);
        // Extremely zoomed out: 3600 s is the last step.
        assert_eq!(tick_step(4_194_304.0, 48_000, 90.0), 172_800_000);
    }
}
