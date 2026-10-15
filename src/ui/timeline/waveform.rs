//! ui::timeline::waveform: waveform painting from cached peaks.

use eframe::egui;

use crate::io::import::Loaded;
use crate::model::clip::Clip;
use crate::model::media::{PeakLevel, Peaks};
use crate::util::db::db_to_linear;
use crate::util::fade::fade_gain;

use crate::ui::theme::THEME;

use super::Geom;

/// Samples per pixel below which the conformed samples are read directly.
const DIRECT_READ_SPP: f64 = 256.0;

/// Stroke of the waveform lines, in the clip's track color.
fn wave_stroke(color: egui::Color32) -> egui::Stroke {
    egui::Stroke::new(THEME.wave_width, color)
}

/// Index of the coarsest peak level whose block size fits within `spp`.
///
/// Returns `None` when `spp < 256`: then the conformed samples are read
/// directly instead of using the peaks.
pub fn choose_level(peaks: &Peaks, spp: f64) -> Option<usize> {
    if spp < DIRECT_READ_SPP {
        return None;
    }
    let mut best = None;
    for (i, level) in peaks.levels.iter().enumerate() {
        if f64::from(level.block_size) <= spp {
            best = Some(i);
        }
    }
    best
}

/// Min and max over the blocks covering source frames `from..to` of one channel.
///
/// The range is in frames. Returns `(0.0, 0.0)` for an empty range, a range at
/// or past the end of the audio, or a channel beyond `channels`.
pub fn level_minmax(level: &PeakLevel, channels: u16, ch: u16, from: u64, to: u64) -> (f32, f32) {
    if from >= to || ch >= channels || channels == 0 {
        return (0.0, 0.0);
    }
    let bs = u64::from(level.block_size);
    let block_count = level.data.len() / usize::from(channels);
    let first = (from / bs) as usize;
    let last = (((to - 1) / bs) as usize).min(block_count.saturating_sub(1));
    if first >= block_count {
        return (0.0, 0.0);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for b in first..=last {
        let e = level.data[b * usize::from(channels) + usize::from(ch)];
        min = min.min(e[0]);
        max = max.max(e[1]);
    }
    if min.is_finite() && max.is_finite() {
        (min, max)
    } else {
        (0.0, 0.0)
    }
}

/// Min and max over raw interleaved samples of one channel across frames
/// `from..to`. Returns `(0.0, 0.0)` for an empty or past-the-end range.
pub fn samples_minmax(samples: &[f32], channels: u16, ch: u16, from: u64, to: u64) -> (f32, f32) {
    if from >= to || ch >= channels || channels == 0 {
        return (0.0, 0.0);
    }
    let channels = usize::from(channels);
    let frame_count = samples.len() / channels;
    let start = (from as usize).min(frame_count);
    let end = (to as usize).min(frame_count);
    if start >= end {
        return (0.0, 0.0);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for frame in start..end {
        let s = samples[frame * channels + usize::from(ch)];
        min = min.min(s);
        max = max.max(s);
    }
    (min, max)
}

/// Linear amplitude of a clip at `local` samples into the clip: clip gain
/// times the linear fade-in and fade-out (the smaller when both apply).
pub fn envelope(clip: &Clip, local: u64) -> f32 {
    db_to_linear(clip.gain_db) * fade_gain(local, clip.len(), clip.fade_in, clip.fade_out)
}

/// Draws the waveform of one clip inside its body rect.
///
/// Stereo media draws two lanes (L above R), mono one lane, each with a faint
/// center line. The whole clip is added as a single shape. Offline or missing
/// media is never passed here; track volume does not affect the drawing.
pub fn draw(
    painter: &egui::Painter,
    clip: &Clip,
    loaded: &Loaded,
    g: &Geom,
    body: egui::Rect,
    wave_scale: f32,
    color: egui::Color32,
) {
    if body.height() < 1.0 {
        return;
    }
    let peaks = &loaded.peaks;
    let channels = usize::from(peaks.channels).clamp(1, 2);
    let lane_h = body.height() / channels as f32;
    let level = choose_level(peaks, g.spp);

    // Visible portion of the clip in pixels.
    let x_left = g.sample_to_x(clip.start).max(body.left());
    let x_right = g.sample_to_x(clip.end()).min(body.right());
    let mut shapes: Vec<egui::Shape> = Vec::new();

    // Faint center line per lane.
    for lane in 0..channels {
        let cy = body.top() + lane_h * (lane as f32 + 0.5);
        shapes.push(egui::Shape::line_segment(
            [egui::pos2(x_left, cy), egui::pos2(x_right, cy)],
            egui::Stroke::new(THEME.hairline, THEME.wave_center),
        ));
    }

    if g.spp < 1.0 {
        draw_samples(
            clip,
            loaded,
            g,
            body,
            channels,
            lane_h,
            wave_scale,
            color,
            &mut shapes,
        );
    } else {
        draw_columns(
            clip,
            loaded,
            level,
            g,
            x_left,
            x_right,
            body.top(),
            channels,
            lane_h,
            wave_scale,
            color,
            &mut shapes,
        );
    }

    painter.add(egui::Shape::Vec(shapes));
}

/// Waveform at spp >= 1: one vertical min/max line per pixel column.
#[allow(clippy::too_many_arguments)]
fn draw_columns(
    clip: &Clip,
    loaded: &Loaded,
    level: Option<usize>,
    g: &Geom,
    x_left: f32,
    x_right: f32,
    body_top: f32,
    channels: usize,
    lane_h: f32,
    wave_scale: f32,
    color: egui::Color32,
    shapes: &mut Vec<egui::Shape>,
) {
    let stroke = wave_stroke(color);
    let peak_channels = loaded.peaks.channels;
    let first_col = (x_left - g.lane_left) as i64;
    let last_col = (x_right - g.lane_left).ceil() as i64;
    for col in first_col..=last_col {
        let x = g.lane_left + col as f32;
        let tl_from = g.x_to_sample(x);
        let tl_to = g.x_to_sample(x + 1.0);
        let t0 = tl_from.max(clip.start);
        let t1 = tl_to.min(clip.end());
        if t0 >= t1 {
            continue;
        }
        let src_from = clip.src_in + (t0 - clip.start);
        let src_to = clip.src_in + (t1 - clip.start);
        let local = ((tl_from + tl_to) / 2).saturating_sub(clip.start);
        let env = envelope(clip, local) * wave_scale;
        for lane in 0..channels {
            let (min, max) = match level {
                Some(i) => level_minmax(
                    &loaded.peaks.levels[i],
                    peak_channels,
                    lane as u16,
                    src_from,
                    src_to,
                ),
                None => samples_minmax(
                    &loaded.samples,
                    peak_channels,
                    lane as u16,
                    src_from,
                    src_to,
                ),
            };
            let top = body_top + lane_h * lane as f32;
            let cy = top + lane_h * 0.5;
            let half = (lane_h * 0.5 - 1.0).max(0.0);
            let y_min = (cy - max * env * half).clamp(top, top + lane_h);
            let y_max = (cy - min * env * half).clamp(top, top + lane_h);
            if y_min == y_max {
                continue;
            }
            shapes.push(egui::Shape::line_segment(
                [egui::pos2(x + 0.5, y_min), egui::pos2(x + 0.5, y_max)],
                stroke,
            ));
        }
    }
}

/// Waveform at spp < 1: a polyline through the individual samples, with a dot
/// per sample when spp < 0.125.
#[allow(clippy::too_many_arguments)]
fn draw_samples(
    clip: &Clip,
    loaded: &Loaded,
    g: &Geom,
    body: egui::Rect,
    channels: usize,
    lane_h: f32,
    wave_scale: f32,
    color: egui::Color32,
    shapes: &mut Vec<egui::Shape>,
) {
    let peak_channels = usize::from(loaded.peaks.channels).max(1);
    let frame_count = loaded.samples.len() / peak_channels;
    let stroke = wave_stroke(color);
    for lane in 0..channels {
        let top = lane_h * lane as f32;
        let cy = top + lane_h * 0.5;
        let half = (lane_h * 0.5 - 1.0).max(0.0);
        let ch = lane.min(peak_channels - 1);
        let mut points: Vec<egui::Pos2> = Vec::new();
        for frame in clip.src_in..clip.src_out.min(frame_count as u64) {
            let sample = loaded.samples[frame as usize * peak_channels + ch];
            let tl = clip.start + (frame - clip.src_in);
            let x = g.sample_to_x(tl);
            if x < body.left() - 1.0 || x > body.right() + 1.0 {
                continue;
            }
            let y = (cy - sample * envelope(clip, frame - clip.src_in) * wave_scale * half)
                .clamp(top, top + lane_h);
            points.push(egui::pos2(x, y));
        }
        if points.len() > 1 {
            if g.spp < 0.125 {
                for &p in &points {
                    shapes.push(egui::Shape::circle_filled(
                        p,
                        THEME.wave_dot_r,
                        stroke.color,
                    ));
                }
            }
            shapes.push(egui::Shape::line(points, stroke));
        } else if g.spp < 0.125 {
            for &p in &points {
                shapes.push(egui::Shape::circle_filled(
                    p,
                    THEME.wave_dot_r,
                    stroke.color,
                ));
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::peaks;

    fn ramp(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| -1.0 + 2.0 * i as f32 / (len - 1) as f32)
            .collect()
    }

    fn sample_peaks() -> Peaks {
        peaks::build(&ramp(4096), 1)
    }

    #[test]
    fn choose_level_picks_coarsest_fitting_level() {
        let peaks = sample_peaks();
        assert_eq!(choose_level(&peaks, 100.0), None);
        assert_eq!(choose_level(&peaks, 256.0), Some(0));
        assert_eq!(choose_level(&peaks, 1000.0), Some(0));
        assert_eq!(choose_level(&peaks, 1024.0), Some(1));
        assert_eq!(choose_level(&peaks, 20_000.0), Some(3));
    }

    #[test]
    fn level_minmax_over_ramp_middle_range() {
        let level = &sample_peaks().levels[0];
        // Monotonic ramp: min is the first frame, max the last frame of the range.
        let expect = |f: u64| -1.0 + 2.0 * f as f32 / 4095.0;
        let (min, max) = level_minmax(level, 1, 0, 512, 1024);
        assert_eq!(min, expect(512));
        assert_eq!(max, expect(1023));
    }

    #[test]
    fn level_minmax_empty_or_past_end_is_zero() {
        let level = &sample_peaks().levels[0];
        assert_eq!(level_minmax(level, 1, 0, 512, 512), (0.0, 0.0));
        assert_eq!(level_minmax(level, 1, 0, 5000, 6000), (0.0, 0.0));
        assert_eq!(level_minmax(level, 1, 1, 0, 256), (0.0, 0.0));
    }

    #[test]
    fn samples_minmax_picks_requested_channel() {
        let samples: Vec<f32> = (0..512).flat_map(|_| [0.5f32, -0.5]).collect();
        assert_eq!(samples_minmax(&samples, 2, 0, 0, 512), (0.5, 0.5));
        assert_eq!(samples_minmax(&samples, 2, 1, 0, 512), (-0.5, -0.5));
        assert_eq!(samples_minmax(&samples, 2, 1, 512, 600), (0.0, 0.0));
    }

    #[test]
    fn envelope_unity_without_gain_or_fades() {
        let clip = test_clip(0.0, 0, 0);
        assert_eq!(envelope(&clip, 0), 1.0);
        assert_eq!(envelope(&clip, 999), 1.0);
    }

    #[test]
    fn envelope_plus_six_db_doubles() {
        let clip = test_clip(6.0206, 0, 0);
        assert!((envelope(&clip, 500) - 2.0).abs() < 1e-3);
    }

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
}
