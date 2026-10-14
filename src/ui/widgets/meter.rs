//! ui::widgets::meter: peak/RMS meter widget with clip light, plus the UI-side
//! meter ballistics state fed from `Editor::read_meters` every frame.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};
use uuid::Uuid;

use crate::ui::theme::THEME;
use crate::util::db::linear_to_db;

/// Duration of the RMS averaging window, in seconds.
const RMS_WINDOW: f64 = 0.3;
/// Peak falloff, in dB per second.
const PEAK_FALL: f32 = 20.0;
/// Floor for peak and RMS readings, in dB.
const FLOOR: f32 = -60.0;
/// How long the peak-hold marker stays, in seconds.
const HOLD_TIME: f32 = 2.0;

/// Pure meter ballistics: RMS window, peak falloff, hold and clip latch.
/// No drawing; the widget layer renders the state.
pub struct ChannelBallistics {
    /// RMS window entries: (time, sum of squares, frames).
    pub rms: VecDeque<(f64, f32, u32)>,
    /// Current displayed peak in dBFS, falling at 20 dB/s.
    pub peak_db: f32,
    /// Peak-hold value in dBFS.
    pub hold_db: f32,
    /// Seconds since the hold value was set.
    pub hold_age: f32,
    /// Latched clip flag; stays until `clear_clip`.
    pub clip: bool,
}

impl ChannelBallistics {
    /// Fresh ballistics resting at the floor.
    pub fn new() -> ChannelBallistics {
        ChannelBallistics {
            rms: VecDeque::new(),
            peak_db: FLOOR,
            hold_db: FLOOR,
            hold_age: 0.0,
            clip: false,
        }
    }

    /// Feeds one batch of meter values and advances the ballistics by `dt`.
    pub fn update(
        &mut self,
        now: f64,
        dt: f32,
        peak: f32,
        sum_sq: f32,
        frames: u32,
        clipped: bool,
    ) {
        // Peak: rises instantly, falls 20 dB/s, floored at -60.
        let inst_db = linear_to_db(peak);
        let fallen = self.peak_db - PEAK_FALL * dt;
        self.peak_db = inst_db.max(fallen).max(FLOOR);
        // Hold: latched for HOLD_TIME, then follows the falling peak.
        if inst_db >= self.hold_db {
            self.hold_db = inst_db;
            self.hold_age = 0.0;
        } else {
            self.hold_age += dt;
            if self.hold_age >= HOLD_TIME {
                self.hold_db = self.peak_db;
            }
        }
        // RMS: 300 ms window of sum of squares over frames.
        self.rms.push_back((now, sum_sq, frames));
        let cutoff = now - RMS_WINDOW;
        while let Some(&(t, _, _)) = self.rms.front() {
            if t < cutoff {
                self.rms.pop_front();
            } else {
                break;
            }
        }
        self.clip = self.clip || clipped;
    }

    /// Current RMS in dBFS, -60 when the window is empty.
    pub fn rms_db(&self) -> f32 {
        let sum_sq: f32 = self.rms.iter().map(|(_, s, _)| s).sum();
        let frames: u32 = self.rms.iter().map(|(_, _, f)| f).sum();
        if frames == 0 {
            return FLOOR;
        }
        (10.0 * (sum_sq / frames as f32).log10()).max(FLOOR)
    }

    /// Clears the latched clip flag.
    pub fn clear_clip(&mut self) {
        self.clip = false;
    }

    /// Bar color for a level in dBFS.
    pub fn color_for(db: f32) -> Color32 {
        if db > -6.0 {
            THEME.meter_red
        } else if db > -18.0 {
            THEME.meter_yellow
        } else {
            THEME.meter_green
        }
    }
}

impl Default for ChannelBallistics {
    fn default() -> ChannelBallistics {
        ChannelBallistics::new()
    }
}

/// Scale tick values printed on the meter, in dBFS.
const TICKS: [f32; 8] = [0.0, -6.0, -12.0, -18.0, -24.0, -36.0, -48.0, -60.0];

/// One channel's raw meter reading from the audio thread: peak, sum of
/// squares, frames and clip flag (the same tuple `Editor::read_meters` yields).
pub type ChannelReading = (f32, f32, u32, bool);

/// UI-side meter state: ballistics for every track and the master, the master
/// peak-hold readout and the keep-repainting tail after playback stops.
#[derive(Default)]
pub struct MeterState {
    /// Ballistics per track id (L and R).
    pub tracks: HashMap<Uuid, [ChannelBallistics; 2]>,
    /// Master ballistics (L and R).
    pub master: [ChannelBallistics; 2],
    /// Highest master peak since the last reset, in dBFS; None before any.
    pub master_peak_db: Option<f32>,
    /// Repaint the meters for one more second after playback stopped.
    hold_until: Option<Instant>,
}

impl MeterState {
    /// Marks the meters active (playing or previewing): repaint for 1 s more.
    pub fn mark_active(&mut self) {
        self.hold_until = Some(Instant::now() + Duration::from_secs(1));
    }

    /// True while the meters should keep repainting after playback stopped.
    pub fn meter_tail(&self) -> bool {
        self.hold_until.is_some_and(|t| Instant::now() < t)
    }

    /// Feeds one frame of readings from `Editor::read_meters` into the
    /// ballistics, advancing them by `dt` seconds at egui time `now`.
    pub fn update(
        &mut self,
        track_readings: &[(Uuid, [ChannelReading; 2])],
        master: &[ChannelReading; 2],
        now: f64,
        dt: f32,
    ) {
        for (id, [l, r]) in track_readings {
            let entry = self.tracks.entry(*id).or_default();
            entry[0].update(now, dt, l.0, l.1, l.2, l.3);
            entry[1].update(now, dt, r.0, r.1, r.2, r.3);
        }
        self.tracks
            .retain(|id, _| track_readings.iter().any(|(tid, _)| tid == id));
        self.master[0].update(now, dt, master[0].0, master[0].1, master[0].2, master[0].3);
        self.master[1].update(now, dt, master[1].0, master[1].1, master[1].2, master[1].3);
        // Master peak readout: the highest finite peak since the last reset.
        for (peak, _, _, _) in master {
            let db = linear_to_db(*peak);
            if db.is_finite() && self.master_peak_db.is_none_or(|p| db > p) {
                self.master_peak_db = Some(db);
            }
        }
    }
}

/// Maps a dBFS value to a y position inside a bar area (bottom = -60 dB).
fn db_y(area: Rect, db: f32) -> f32 {
    let t = ((db - FLOOR) / -FLOOR).clamp(0.0, 1.0);
    area.bottom() - t * area.height()
}

/// Draws one channel's bar: solid RMS, thin peak line and hold marker.
fn draw_bar(painter: &egui::Painter, area: Rect, ch: &ChannelBallistics) {
    painter.rect_filled(area, 0.0, THEME.meter_bar_bg);
    let rms = ch.rms_db();
    if rms > FLOOR {
        let bar = Rect::from_min_max(
            Pos2::new(area.left(), db_y(area, rms)),
            Pos2::new(area.right(), area.bottom()),
        );
        painter.rect_filled(bar, 0.0, ChannelBallistics::color_for(rms));
    }
    if ch.peak_db > FLOOR {
        let y = db_y(area, ch.peak_db);
        painter.line_segment(
            [Pos2::new(area.left(), y), Pos2::new(area.right(), y)],
            Stroke::new(THEME.meter_peak_w, ChannelBallistics::color_for(ch.peak_db)),
        );
    }
    if ch.hold_db > FLOOR {
        let y = db_y(area, ch.hold_db);
        painter.line_segment(
            [Pos2::new(area.left(), y), Pos2::new(area.right(), y)],
            Stroke::new(THEME.meter_hold_w, THEME.meter_hold),
        );
    }
}

/// Draws a stereo (L and R side by side) meter with ballistics into `rect`.
///
/// Ticks always draw; tick labels draw only when `labels` is set and the meter
/// is tall enough (used by the master strip). A clip light sits on top of each
/// bar; clicking the meter clears both clip lights. Returns true when clicked.
pub fn stereo_meter(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: Rect,
    channels: &mut [ChannelBallistics; 2],
    labels: bool,
) -> bool {
    let painter = ui.painter_at(rect);
    let resp = ui.interact(rect, id, egui::Sense::click());
    let clicked = resp.clicked();
    if clicked {
        for ch in channels.iter_mut() {
            ch.clear_clip();
        }
    }

    // Clip lights on top, one per channel.
    let light_rect =
        Rect::from_min_size(rect.left_top(), Vec2::new(rect.width(), THEME.clip_light_h));
    let bars_top = rect.top() + THEME.clip_light_h + THEME.space_xs;
    let bar_area = Rect::from_min_max(
        Pos2::new(rect.left(), bars_top),
        Pos2::new(rect.right(), rect.bottom()),
    );
    let label_gutter = if labels { THEME.meter_label_w } else { 0.0 };
    let bars_w = (bar_area.width() - label_gutter).max(THEME.space_xs);
    let bar_w = ((bars_w - THEME.meter_bar_gap) * 0.5).max(THEME.hairline);
    let l_rect = Rect::from_min_max(
        bar_area.left_top(),
        Pos2::new(bar_area.left() + bar_w, bar_area.bottom()),
    );
    let r_rect = Rect::from_min_max(
        Pos2::new(bar_area.left() + bars_w - bar_w, bar_area.top()),
        Pos2::new(bar_area.left() + bars_w, bar_area.bottom()),
    );
    for (bar, ch) in [(l_rect, &channels[0]), (r_rect, &channels[1])] {
        let light = Rect::from_min_max(
            Pos2::new(bar.left(), light_rect.top()),
            Pos2::new(bar.right(), light_rect.bottom()),
        );
        let color = if ch.clip {
            THEME.clip_light_on
        } else {
            THEME.clip_light_off
        };
        painter.rect_filled(light, THEME.corner_r, color);
    }
    draw_bar(&painter, l_rect, &channels[0]);
    draw_bar(&painter, r_rect, &channels[1]);

    // Ticks across both bars; labels in the gutter when there is room.
    let show_labels = labels && bar_area.height() >= THEME.meter_label_room;
    for &t in &TICKS {
        let y = db_y(bar_area, t);
        painter.line_segment(
            [
                Pos2::new(bar_area.left(), y),
                Pos2::new(bar_area.left() + bars_w, y),
            ],
            Stroke::new(THEME.hairline, THEME.meter_tick),
        );
        if show_labels {
            painter.text(
                Pos2::new(bar_area.left() + bars_w + THEME.meter_label_pad_x, y),
                egui::Align2::LEFT_CENTER,
                format!("{}", t as i32),
                egui::FontId::proportional(THEME.meter_label_size),
                THEME.meter_label,
            );
        }
    }
    // Hover highlight for this clickable control.
    if resp.hovered() && !clicked {
        painter.rect_filled(rect, THEME.corner_r, THEME.hover_overlay);
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps `ballistics` forward in 10 ms slices, advancing the clock.
    fn step(
        b: &mut ChannelBallistics,
        now: &mut f64,
        seconds: f32,
        peak: f32,
        sum_sq: f32,
        frames: u32,
    ) {
        for _ in 0..(seconds / 0.01) as usize {
            b.update(*now, 0.01, peak, sum_sq, frames, false);
            *now += 0.01;
        }
    }

    #[test]
    fn peak_falls_twenty_db_per_second_to_the_floor() {
        let mut b = ChannelBallistics::new();
        let mut now = 0.0f64;
        b.update(0.0, 0.01, 1.0, 0.0, 0, false);
        assert!((b.peak_db - 0.0).abs() < 1e-4);
        step(&mut b, &mut now, 1.0, 0.0, 0.0, 0);
        assert!(
            (b.peak_db - (-20.0)).abs() < 0.5,
            "peak_db was {}",
            b.peak_db
        );
        step(&mut b, &mut now, 3.0, 0.0, 0.0, 0);
        assert!((b.peak_db - FLOOR).abs() < 1e-4, "must floor at -60");
    }

    #[test]
    fn hold_stays_two_seconds_then_drops() {
        let mut b = ChannelBallistics::new();
        let mut now = 0.0f64;
        b.update(0.0, 0.01, 1.0, 0.0, 0, false);
        step(&mut b, &mut now, 1.9, 0.0, 0.0, 0);
        assert!((b.hold_db - 0.0).abs() < 1e-3, "hold must stay at 0 dB");
        step(&mut b, &mut now, 0.5, 0.0, 0.0, 0);
        assert!(
            b.hold_db < -19.0,
            "after 2 s the hold follows the falling peak: {}",
            b.hold_db
        );
    }

    #[test]
    fn rms_of_constant_half_settles_at_minus_six_and_falls_to_floor() {
        let mut b = ChannelBallistics::new();
        let mut now = 0.0f64;
        // 480 frames of a constant 0.5 signal: sum_sq = 480 * 0.25 per batch.
        step(&mut b, &mut now, 1.0, 0.5, 480.0 * 0.25, 480);
        assert!(
            (b.rms_db() - (-6.0206)).abs() < 0.05,
            "rms_db was {}",
            b.rms_db()
        );
        step(&mut b, &mut now, 0.4, 0.0, 0.0, 480);
        assert!(
            (b.rms_db() - FLOOR).abs() < 1e-4,
            "rms_db was {}",
            b.rms_db()
        );
    }

    #[test]
    fn clip_latches_until_cleared() {
        let mut b = ChannelBallistics::new();
        let mut now = 0.0f64;
        b.update(0.0, 0.01, 1.0, 0.0, 0, true);
        assert!(b.clip);
        step(&mut b, &mut now, 0.1, 0.0, 0.0, 0);
        assert!(b.clip, "clip must stay set");
        b.clear_clip();
        assert!(!b.clip);
    }

    #[test]
    fn colors_split_at_minus_eighteen_and_minus_six() {
        assert_eq!(ChannelBallistics::color_for(-30.0), THEME.meter_green);
        assert_eq!(ChannelBallistics::color_for(-12.0), THEME.meter_yellow);
        assert_eq!(ChannelBallistics::color_for(-1.0), THEME.meter_red);
    }
}
