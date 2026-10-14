//! ui::widgets::meter: peak/RMS meter widget with clip light.

use std::collections::VecDeque;

use eframe::egui::Color32;

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
            Color32::from_rgb(220, 60, 60)
        } else if db > -18.0 {
            Color32::from_rgb(220, 200, 60)
        } else {
            Color32::from_rgb(60, 200, 90)
        }
    }
}

impl Default for ChannelBallistics {
    fn default() -> ChannelBallistics {
        ChannelBallistics::new()
    }
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
        assert_eq!(
            ChannelBallistics::color_for(-30.0),
            Color32::from_rgb(60, 200, 90)
        );
        assert_eq!(
            ChannelBallistics::color_for(-12.0),
            Color32::from_rgb(220, 200, 60)
        );
        assert_eq!(
            ChannelBallistics::color_for(-1.0),
            Color32::from_rgb(220, 60, 60)
        );
    }
}
