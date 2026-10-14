//! audio::scrub: the variable-speed scrubbing reader.

use crate::audio::mix_plan::MixPlan;
use crate::audio::mixer;

/// Preallocated stereo scratch window length in frames.
const WIN_FRAMES: usize = 8 * 8192 + 8;

/// Variable-speed scrub reader: chases the mouse position with a tape-style
/// pitch shift. All buffers are allocated in [`Scrub::new`]; [`Scrub::process`]
/// never allocates, locks or panics (audio-thread safe).
pub struct Scrub {
    /// Read position in project samples.
    pos: f64,
    /// Where the mouse is, in project samples.
    target: f64,
    /// Read speed in samples per output sample, clamped to -4.0..=4.0.
    speed: f64,
    /// Declick / silence-when-still gain.
    gain: f32,
    /// Preallocated stereo interleaved scratch for the mix window.
    window: Box<[f32]>,
}

impl Scrub {
    /// Allocates the scratch window and starts stopped at sample 0.
    pub fn new(_rate: u32) -> Scrub {
        Scrub {
            pos: 0.0,
            target: 0.0,
            speed: 0.0,
            gain: 0.0,
            window: vec![0.0; WIN_FRAMES * 2].into_boxed_slice(),
        }
    }

    /// Jumps the read head to `pos` and stops moving (silent until a target arrives).
    pub fn start(&mut self, pos: u64) {
        self.pos = pos as f64;
        self.target = pos as f64;
        self.speed = 0.0;
        self.gain = 0.0;
    }

    /// Sets the position the read head chases; the last call wins.
    pub fn set_target(&mut self, pos: u64) {
        self.target = pos as f64;
    }

    /// Current read position in project samples.
    pub fn position(&self) -> f64 {
        self.pos
    }

    /// Renders stereo interleaved output while scrubbing. No allocation.
    pub fn process(&mut self, plan: &MixPlan, out: &mut [f32]) {
        let rate = f64::from(plan.rate.max(1));
        // One-pole coefficient for a 10 ms time constant.
        let smooth = 1.0 - (-1.0 / (0.01 * rate)).exp();
        let smooth_g = smooth as f32;
        // The window must hold 8 * chunk + 4 frames, so chunk <= 8192.
        let max_chunk = (WIN_FRAMES - 4) / 8;
        let mut cursor = 0usize;
        while cursor < out.len() {
            let frames = ((out.len() - cursor) / 2).min(max_chunk);
            if frames == 0 {
                break;
            }
            // Render the mix window centred on the current read position.
            let w_start = (self.pos.floor() as i64 - 4 * frames as i64 - 2).max(0) as u64;
            let wf = 8 * frames + 4;
            mixer::render(plan, w_start, &mut self.window[..wf * 2], true);
            let slice = &mut out[cursor..cursor + frames * 2];
            for i in 0..frames {
                // The read head chases the mouse with a 50 ms time constant.
                let desired = ((self.target - self.pos) / (rate * 0.05)).clamp(-4.0, 4.0);
                self.speed += (desired - self.speed) * smooth;
                let g_target = if self.speed.abs() > 0.02 { 1.0 } else { 0.0 };
                self.gain += (g_target - self.gain) * smooth_g;
                self.pos = (self.pos + self.speed).max(0.0);
                let idx = self.pos - w_start as f64;
                let (l, r) = read_window(&self.window, wf, idx);
                let o = i * 2;
                slice[o] = l * self.gain;
                slice[o + 1] = r * self.gain;
            }
            cursor += frames * 2;
        }
    }
}

/// Linear interpolation of the stereo window at fractional frame `idx`.
/// Reads outside the rendered window give 0.0.
fn read_window(window: &[f32], wf: usize, idx: f64) -> (f32, f32) {
    if idx < 0.0 {
        return (0.0, 0.0);
    }
    let i = idx.floor();
    if i + 1.0 >= wf as f64 {
        return (0.0, 0.0);
    }
    let i = i as usize;
    let frac = (idx - idx.floor()) as f32;
    let l0 = window[i * 2];
    let r0 = window[i * 2 + 1];
    let l1 = window[i * 2 + 2];
    let r1 = window[i * 2 + 3];
    let l = l0 + (l1 - l0) * frac;
    let r = r0 + (r1 - r0) * frac;
    (l, r)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::Scrub;
    use crate::audio::meters::{MeterBank, StereoMeter};
    use crate::audio::mix_plan::{ClipPlan, MixPlan, TrackPlan};

    const RATE: u32 = 48_000;

    /// A one-track plan with one mono clip of ``samples``, panned centre (0.7071).
    fn plan(samples: Vec<f32>) -> MixPlan {
        let len = samples.len() as u64;
        MixPlan {
            rate: RATE,
            length: len,
            master_gain: 1.0,
            tracks: vec![TrackPlan {
                audible: true,
                gain_l: 0.707_106_77,
                gain_r: 0.707_106_77,
                clips: vec![ClipPlan {
                    start: 0,
                    len,
                    src_in: 0,
                    channels: 1,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                    samples: Some(Arc::from(samples.into_boxed_slice())),
                }],
            }],
            meters: Arc::new(MeterBank {
                track_ids: Vec::new(),
                tracks: Vec::new(),
            }),
            master: Arc::new(StereoMeter::new()),
        }
    }

    #[test]
    fn forward_scrub_moves_and_respects_speed_clamp() {
        let plan = plan(vec![0.5; 200_000]);
        let mut s = Scrub::new(RATE);
        s.start(1_000);
        s.set_target(49_000);
        let mut prev = s.position();
        for _ in 0..48 {
            let mut out = vec![0.0f32; 512 * 2];
            s.process(&plan, &mut out);
            let now = s.position();
            assert!(now > prev, "pos should move forward: {prev} -> {now}");
            assert!(
                now - prev <= 4.0 * 512.0 + 1e-6,
                "speed exceeded 4x: {}",
                now - prev
            );
            prev = now;
        }
        assert!(
            s.position() > 11_000.0,
            "pos {} should have moved well forward in 0.5 s",
            s.position()
        );
    }

    #[test]
    fn backward_target_moves_pos_backward() {
        let plan = plan(vec![0.5; 200_000]);
        let mut s = Scrub::new(RATE);
        s.start(20_000);
        s.set_target(2_000);
        let mut prev = s.position();
        for _ in 0..20 {
            let mut out = vec![0.0f32; 512 * 2];
            s.process(&plan, &mut out);
            assert!(s.position() < prev, "pos should move backward");
            prev = s.position();
        }
    }

    #[test]
    fn still_target_fades_to_silence() {
        let plan = plan(vec![0.5; 200_000]);
        let mut s = Scrub::new(RATE);
        s.start(5_000);
        s.set_target(5_000);
        let mut out = vec![0.0f32; 4_800 * 2];
        s.process(&plan, &mut out);
        for &v in &out[4_800 * 2 - 480 * 2..] {
            assert!(v.abs() < 1e-4, "tail should be silent, got {v}");
        }
    }

    #[test]
    fn steady_forward_scrub_holds_constant_level() {
        let plan = plan(vec![0.5; 200_000]);
        let mut s = Scrub::new(RATE);
        s.start(10_000);
        s.set_target(30_000);
        let mut all: Vec<f32> = Vec::new();
        for _ in 0..10 {
            let mut out = vec![0.0f32; 512 * 2];
            s.process(&plan, &mut out);
            all.extend_from_slice(&out);
        }
        let expected = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        for w in all.chunks(2).skip(3_072) {
            assert!(
                (w[0] - expected).abs() < 0.01,
                "value {} should be near {expected}",
                w[0]
            );
        }
        for i in 3_072..all.len() / 2 - 1 {
            let a = all[i * 2];
            let b = all[(i + 1) * 2];
            assert!(
                (a - b).abs() < 0.01,
                "jump {a} -> {b} at frame {i} exceeds 0.01"
            );
        }
    }

    #[test]
    fn backward_scrub_over_ramp_decreases() {
        let n = 200_000;
        let samples: Vec<f32> = (0..n).map(|i| i as f32 / n as f32).collect();
        let plan = plan(samples);
        let mut s = Scrub::new(RATE);
        s.start(20_000);
        s.set_target(2_000);
        let mut all: Vec<f32> = Vec::new();
        for _ in 0..10 {
            let mut out = vec![0.0f32; 512 * 2];
            s.process(&plan, &mut out);
            all.extend_from_slice(&out);
        }
        // Skip the 10 ms gain ramp-in, then values must fall over time.
        let start = 3_072;
        let mut prev = all[start * 2];
        for i in start + 1..all.len() / 2 {
            let v = all[i * 2];
            assert!(v <= prev + 1e-4, "value rose from {prev} to {v} at {i}");
            prev = v;
        }
    }
}
