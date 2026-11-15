//! ui::timeline::waveform: waveform painting from cached peaks.

use eframe::egui;

use crate::editor::record::RecordStatus;
use crate::io::import::Loaded;
use crate::io::record_writer::PEAK_FRAMES;
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
/// or past the end of the audio, a channel beyond `channels`, or a level whose
/// block size is 0 (a corrupt level draws nothing).
pub fn level_minmax(level: &PeakLevel, channels: u16, ch: u16, from: u64, to: u64) -> (f32, f32) {
    if from >= to || ch >= channels || channels == 0 || level.block_size == 0 {
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
    if min.is_finite() && max.is_finite() { (min, max) } else { (0.0, 0.0) }
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
/// Draws the waveform of one clip inside its body rect.
///
/// Stereo media draws two lanes (L above R), mono one lane, each with a faint
/// center line in the track color at alpha 0x33, a 1 px black 35% divider
/// between the lanes, and filled solid min/max columns in the track color.
/// Offline or missing media is never passed here; track volume does not
/// affect the drawing. On a muted track everything draws at 45% opacity.
#[allow(clippy::too_many_arguments)]
pub fn draw(painter: &egui::Painter, clip: &Clip, loaded: &Loaded, g: &Geom, body: egui::Rect, wave_scale: f32, color: egui::Color32, muted: bool) {
    if body.height() < 1.0 {
        return;
    }
    let scale = if muted { 0.45 } else { 1.0 };
    let dim = |c: egui::Color32| c.gamma_multiply(scale);
    let peaks = &loaded.peaks;
    let channels = usize::from(peaks.channels).clamp(1, 2);
    let lane_h = body.height() / channels as f32;
    let level = choose_level(peaks, g.spp);

    // Visible portion of the clip in pixels.
    let x_left = g.sample_to_x(clip.start).max(body.left());
    let x_right = g.sample_to_x(clip.end()).min(body.right());
    let mut shapes = guides(body, channels, x_left, x_right, color, scale);

    let wave_color = dim(color);
    if g.spp < 1.0 {
        draw_samples(clip, loaded, g, body, channels, lane_h, wave_scale, wave_color, &mut shapes);
    } else {
        draw_columns(clip, loaded, level, g, x_left, x_right, body.top(), channels, lane_h, wave_scale, wave_color, &mut shapes);
    }

    painter.add(egui::Shape::Vec(shapes));
}

/// The lane guides under a waveform: a faint center line per lane in the wave
/// color at alpha 0x33 and, for two lanes, a 1 px black 35% divider between L
/// and R. `scale` is the track's opacity factor (0.45 when muted).
fn guides(body: egui::Rect, channels: usize, x_left: f32, x_right: f32, color: egui::Color32, scale: f32) -> Vec<egui::Shape> {
    let lane_h = body.height() / channels as f32;
    let center = egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), (0x33 as f32 * scale) as u8);
    let divider = egui::Color32::from_rgba_unmultiplied(0, 0, 0, (89.0 * scale) as u8);
    let mut shapes: Vec<egui::Shape> = Vec::new();
    // Faint center line per lane.
    for lane in 0..channels {
        let cy = body.top() + lane_h * (lane as f32 + 0.5);
        shapes.push(egui::Shape::line_segment([egui::pos2(x_left, cy), egui::pos2(x_right, cy)], egui::Stroke::new(THEME.hairline, center)));
    }
    // Divider between the two lanes of stereo media.
    if channels == 2 {
        let dy = body.top() + lane_h;
        shapes.push(egui::Shape::line_segment([egui::pos2(x_left, dy), egui::pos2(x_right, dy)], egui::Stroke::new(THEME.hairline, divider)));
    }
    shapes
}

/// One pixel column of the live waveform: the `[minL, maxL, minR, maxR]` entry
/// for the take frames `rel_from..rel_to` (relative to the record start).
///
/// `peaks` holds one entry per `PEAK_FRAMES` frames. A column no wider than
/// one entry uses the entry under its center (the nearest one, so zoomed in
/// the same entry just repeats); a wider column combines every entry it
/// covers. `None` when the column starts past the last complete entry (the
/// writer has not reported it yet).
pub fn live_column(peaks: &[[f32; 4]], rel_from: u64, rel_to: u64) -> Option<[f32; 4]> {
    let group = PEAK_FRAMES as u64;
    let span = rel_to.saturating_sub(rel_from);
    let (first, last) = if span <= group {
        let middle = (rel_from + span / 2) / group;
        (middle, middle)
    } else {
        (rel_from / group, (rel_to - 1) / group)
    };
    let first = usize::try_from(first).ok()?;
    if first >= peaks.len() {
        return None;
    }
    let last = usize::try_from(last).unwrap_or(usize::MAX).min(peaks.len() - 1);
    let mut entry = peaks[first];
    for next in &peaks[first + 1..=last] {
        entry = [entry[0].min(next[0]), entry[1].max(next[1]), entry[2].min(next[2]), entry[3].max(next[3])];
    }
    Some(entry)
}

/// Draws the live waveform of the take being recorded inside its body rect,
/// from the writer's peak entries: filled min/max columns in `color`, one lane
/// for a mono take and two (L above R) for a stereo take, with the same lane
/// guides as a clip. `wave_scale` is the view's vertical waveform scale.
pub fn draw_live(painter: &egui::Painter, status: &RecordStatus<'_>, g: &Geom, body: egui::Rect, wave_scale: f32, color: egui::Color32) {
    if body.height() < 1.0 {
        return;
    }
    let channels = usize::from(status.channels).clamp(1, 2);
    let lane_h = body.height() / channels as f32;
    let end = status.start.saturating_add(status.frames);
    // Only the visible columns: the take may start far off screen.
    let x_left = g.sample_to_x(status.start).max(body.left()).max(g.lane_left);
    let x_right = g.sample_to_x(end).min(body.right()).min(g.lane_left + g.lane_width);
    let mut shapes = guides(body, channels, x_left, x_right, color, 1.0);
    let first_col = (x_left - g.lane_left) as i64;
    let last_col = (x_right - g.lane_left).ceil() as i64;
    for col in first_col..=last_col {
        let x = g.lane_left + col as f32;
        let from = g.x_to_sample(x).max(status.start);
        let to = g.x_to_sample(x + 1.0).min(end);
        if from >= to {
            continue;
        }
        let Some(entry) = live_column(status.peaks, from - status.start, to - status.start) else {
            continue;
        };
        for lane in 0..channels {
            let (min, max) = (entry[lane * 2], entry[lane * 2 + 1]);
            let top = body.top() + lane_h * lane as f32;
            let cy = top + lane_h * 0.5;
            let half = (lane_h * 0.5 - THEME.wave_lane_pad).max(0.0);
            let y_min = (cy - max * wave_scale * half).clamp(top, top + lane_h);
            let y_max = (cy - min * wave_scale * half).clamp(top, top + lane_h);
            if y_min == y_max {
                continue;
            }
            shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(x, y_min), egui::pos2(x + 1.0, y_max)), 0.0, color));
        }
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
                Some(i) => level_minmax(&loaded.peaks.levels[i], peak_channels, lane as u16, src_from, src_to),
                None => samples_minmax(&loaded.samples, peak_channels, lane as u16, src_from, src_to),
            };
            let top = body_top + lane_h * lane as f32;
            let cy = top + lane_h * 0.5;
            let half = (lane_h * 0.5 - THEME.wave_lane_pad).max(0.0);
            let y_min = (cy - max * env * half).clamp(top, top + lane_h);
            let y_max = (cy - min * env * half).clamp(top, top + lane_h);
            if y_min == y_max {
                continue;
            }
            // Filled solid column in the track color.
            shapes.push(egui::Shape::rect_filled(egui::Rect::from_min_max(egui::pos2(x, y_min), egui::pos2(x + 1.0, y_max)), 0.0, stroke.color));
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
        let half = (lane_h * 0.5 - THEME.wave_lane_pad).max(0.0);
        let ch = lane.min(peak_channels - 1);
        let mut points: Vec<egui::Pos2> = Vec::new();
        for frame in clip.src_in..clip.src_out.min(frame_count as u64) {
            let sample = loaded.samples[frame as usize * peak_channels + ch];
            let tl = clip.start + (frame - clip.src_in);
            let x = g.sample_to_x(tl);
            if x < body.left() - 1.0 || x > body.right() + 1.0 {
                continue;
            }
            let y = (cy - sample * envelope(clip, frame - clip.src_in) * wave_scale * half).clamp(top, top + lane_h);
            points.push(egui::pos2(x, y));
        }
        if points.len() > 1 {
            if g.spp < 0.125 {
                for &p in &points {
                    shapes.push(egui::Shape::circle_filled(p, THEME.wave_dot_r, stroke.color));
                }
            }
            shapes.push(egui::Shape::line(points, stroke));
        } else if g.spp < 0.125 {
            for &p in &points {
                shapes.push(egui::Shape::circle_filled(p, THEME.wave_dot_r, stroke.color));
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::peaks;

    fn ramp(len: usize) -> Vec<f32> {
        (0..len).map(|i| -1.0 + 2.0 * i as f32 / (len - 1) as f32).collect()
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
    fn level_minmax_with_block_size_zero_draws_nothing() {
        // A corrupt level used to divide by zero here.
        let level = PeakLevel { block_size: 0, data: vec![[-1.0, 1.0]; 8] };
        assert_eq!(level_minmax(&level, 1, 0, 0, 1), (0.0, 0.0));
        assert_eq!(level_minmax(&level, 2, 1, 0, 4_096), (0.0, 0.0));
        assert_eq!(level_minmax(&level, 1, 0, 1_000, u64::MAX), (0.0, 0.0));
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
        Clip { id: uuid::Uuid::nil(), media_id: uuid::Uuid::nil(), start: 0, src_in: 0, src_out: 1_000, gain_db, fade_in, fade_out }
    }

    // ---- The live waveform ----

    /// Three peak entries: L runs +-0.1, +-0.3, +-0.5 and R +-0.2, +-0.4, +-0.6.
    fn three_entries() -> [[f32; 4]; 3] {
        [[-0.1, 0.1, -0.2, 0.2], [-0.3, 0.3, -0.4, 0.4], [-0.5, 0.5, -0.6, 0.6]]
    }

    #[test]
    fn live_column_uses_the_nearest_entry_and_repeats_it_when_zoomed_in() {
        let peaks = three_entries();
        // Columns of one frame (zoomed in) all read the entry they sit in.
        assert_eq!(live_column(&peaks, 0, 1), Some(peaks[0]));
        assert_eq!(live_column(&peaks, 1_000, 1_001), Some(peaks[0]));
        assert_eq!(live_column(&peaks, 1_024, 1_025), Some(peaks[1]));
        assert_eq!(live_column(&peaks, 2_047, 2_048), Some(peaks[1]));
        // A column of one entry's width straddling a boundary uses the entry
        // under its center.
        assert_eq!(live_column(&peaks, 800, 1_312), Some(peaks[1]));
        assert_eq!(live_column(&peaks, 700, 1_212), Some(peaks[0]));
        assert_eq!(live_column(&peaks, 0, 1_024), Some(peaks[0]));
    }

    #[test]
    fn live_column_combines_the_entries_a_wide_column_covers() {
        let peaks = three_entries();
        // 3,072 frames cover all three entries.
        assert_eq!(live_column(&peaks, 0, 3_072), Some([-0.5, 0.5, -0.6, 0.6]), "min of the mins, max of the maxes, per side");
        assert_eq!(live_column(&peaks, 1_024, 3_072), Some([-0.5, 0.5, -0.6, 0.6]));
        assert_eq!(live_column(&peaks, 0, 2_048), Some([-0.3, 0.3, -0.4, 0.4]));
    }

    #[test]
    fn live_column_is_none_past_the_written_peaks() {
        let peaks = three_entries();
        assert_eq!(live_column(&peaks, 3_072, 3_073), None);
        assert_eq!(live_column(&peaks, 100_000, 100_001), None);
        assert_eq!(live_column(&[], 0, 10), None);
        // A wide column that runs past the end uses the entries that exist.
        assert_eq!(live_column(&peaks, 2_048, 100_000), Some(peaks[2]));
    }

    /// The filled record-red columns painted by `draw_live` for `channels` lanes.
    fn live_columns(channels: u16) -> Vec<egui::Rect> {
        // Eight entries (8,192 frames): L +-1.0, R +-0.5.
        let peaks = vec![[-1.0, 1.0, -0.5, 0.5]; 8];
        let status = RecordStatus { track_id: uuid::Uuid::nil(), start: 0, frames: 8_192, channels, peaks: &peaks };
        let g = Geom { lane_left: 0.0, lane_width: 400.0, spp: 32.0, scroll_x: 0 };
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(256.0, 40.0));
        let mut h = crate::ui::test_util::Headless::new(400.0, 200.0);
        let out = h.frame(Vec::new(), |ui| {
            draw_live(ui.painter(), &status, &g, body, 1.0, THEME.record_red);
        });
        out.rects().into_iter().filter(|r| r.fill == THEME.record_red).map(|r| r.rect).collect()
    }

    #[test]
    fn the_live_waveform_has_one_lane_for_mono_and_two_for_stereo() {
        // Mono: 256 columns (8,192 frames / 32 spp), one lane of 40 px; the
        // +-1.0 peaks fill it from top + 1 to bottom - 1.
        let mono = live_columns(1);
        assert_eq!(mono.len(), 256, "one column per pixel");
        assert!(mono.iter().all(|r| (r.height() - 38.0).abs() < 0.01));
        assert!(mono.iter().all(|r| r.top() >= 0.0 && r.bottom() <= 40.0));

        // Stereo: two 20 px lanes. L (+-1.0) fills the upper one, R (+-0.5)
        // half of the lower one; both are drawn for every column.
        let stereo = live_columns(2);
        assert_eq!(stereo.len(), 512, "two lanes per pixel");
        let upper: Vec<_> = stereo.iter().filter(|r| r.center().y < 20.0).collect();
        let lower: Vec<_> = stereo.iter().filter(|r| r.center().y >= 20.0).collect();
        assert_eq!((upper.len(), lower.len()), (256, 256));
        assert!(upper.iter().all(|r| (r.height() - 18.0).abs() < 0.01), "L");
        assert!(lower.iter().all(|r| (r.height() - 9.0).abs() < 0.01), "R");
        assert!(upper.iter().all(|r| r.bottom() <= 20.0));
        assert!(lower.iter().all(|r| r.top() >= 20.0));
    }

    #[test]
    fn the_live_waveform_only_covers_the_written_frames() {
        // 3 entries written, 5,000 frames recorded: columns past the third
        // entry (3,072 frames = 96 px) draw nothing yet.
        let peaks = three_entries();
        let status = RecordStatus { track_id: uuid::Uuid::nil(), start: 0, frames: 5_000, channels: 1, peaks: &peaks };
        let g = Geom { lane_left: 0.0, lane_width: 400.0, spp: 32.0, scroll_x: 0 };
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(160.0, 40.0));
        let mut h = crate::ui::test_util::Headless::new(400.0, 200.0);
        let out = h.frame(Vec::new(), |ui| {
            draw_live(ui.painter(), &status, &g, body, 1.0, THEME.record_red);
        });
        let columns: Vec<_> = out.rects().into_iter().filter(|r| r.fill == THEME.record_red).collect();
        assert_eq!(columns.len(), 96, "3,072 frames / 32 spp");
        assert!(columns.iter().all(|r| r.rect.right() <= 96.5));
    }
}
