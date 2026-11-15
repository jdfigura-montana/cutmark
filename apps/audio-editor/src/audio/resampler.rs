//! audio::resampler: project-rate to device-rate conversion for the output stream.

/// Scratch capacity in project-rate frames.
const CAP_FRAMES: usize = 16_384 + 8;

/// Converts project-rate stereo frames to device-rate output with cubic Hermite
/// interpolation. All buffers are preallocated; `fill` never allocates.
pub struct DeviceResampler {
    /// project_rate / device_rate.
    ratio: f64,
    /// Fractional read position in project-rate frames.
    pos: f64,
    /// Global frame index of `buf[0]`.
    base: i64,
    /// Frames currently held in `buf`.
    len: usize,
    /// Interleaved stereo scratch holding frames `base..base+len`, preallocated once.
    buf: Vec<f32>,
}

impl DeviceResampler {
    /// Creates a resampler for `project_rate` material played on a `device_rate` output.
    pub fn new(project_rate: u32, device_rate: u32) -> DeviceResampler {
        let ratio = project_rate as f64 / device_rate.max(1) as f64;
        let buf = vec![0.0f32; CAP_FRAMES * 2];
        // Three leading zero frames so the first stencil never indexes before the buffer.
        DeviceResampler {
            ratio,
            pos: 0.0,
            base: -3,
            len: 3,
            buf,
        }
    }

    /// Renders `out.len() / 2` device-rate stereo frames, pulling project-rate
    /// stereo frames from `pull` as needed. Frames are kept between calls so
    /// there are no seams at block boundaries.
    pub fn fill(&mut self, out: &mut [f32], pull: &mut dyn FnMut(&mut [f32])) {
        let out_frames = out.len() / 2;
        if out_frames == 0 {
            return;
        }
        // Drop consumed frames no longer needed by the interpolation stencil.
        let i0 = self.pos.floor() as i64;
        let low = i0 - 1;
        let drop = (low - self.base).max(0) as usize;
        if drop > 0 {
            self.buf.copy_within(drop * 2.., 0);
            self.base += drop as i64;
            self.len -= drop;
        }
        let have_end = self.base + self.len as i64 - 1;
        // Fit the request into the preallocated scratch; halve until it fits.
        let mut eff = out_frames;
        let mut need;
        loop {
            let high = (self.pos + (eff as f64 - 1.0) * self.ratio).floor() as i64 + 2;
            need = (high - have_end).max(0) as usize;
            if need <= CAP_FRAMES - self.len {
                break;
            }
            if eff == 0 {
                for s in out.iter_mut() {
                    *s = 0.0;
                }
                return;
            }
            eff /= 2;
        }
        if need > 0 {
            pull(&mut self.buf[self.len * 2..(self.len + need) * 2]);
            self.len += need;
        }
        for k in 0..eff {
            let t = self.pos + k as f64 * self.ratio;
            let i = t.floor();
            let frac = (t - i) as f32;
            let idx = (i - self.base as f64) as usize;
            for ch in 0..2 {
                let p0 = self.buf[(idx - 1) * 2 + ch];
                let p1 = self.buf[idx * 2 + ch];
                let p2 = self.buf[(idx + 1) * 2 + ch];
                let p3 = self.buf[(idx + 2) * 2 + ch];
                out[k * 2 + ch] = hermite(p0, p1, p2, p3, frac);
            }
        }
        for s in out[eff * 2..].iter_mut() {
            *s = 0.0;
        }
        self.pos += eff as f64 * self.ratio;
    }
}

/// Catmull-Rom style cubic Hermite interpolation between p1 and p2 at `u` in [0, 1).
#[inline]
fn hermite(p0: f32, p1: f32, p2: f32, p3: f32, u: f32) -> f32 {
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 1.5 * (p1 - p2) + 0.5 * (p3 - p0);
    ((c3 * u + c2) * u + c1) * u + p1
}

#[cfg(test)]
mod tests {
    use super::{DeviceResampler, hermite};

    const PROJECT_RATE: f64 = 44_100.0;
    const DEVICE_RATE: f64 = 48_000.0;

    /// Runs `calls` fills of `size` frames; returns (all output, total frames pulled).
    fn run(
        res: &mut DeviceResampler,
        calls: usize,
        size: usize,
        source: &mut dyn FnMut(&mut [f32]),
    ) -> (Vec<f32>, usize) {
        let mut out_all = Vec::new();
        let mut pulled = 0usize;
        for _ in 0..calls {
            let start = out_all.len();
            out_all.resize(start + size * 2, f32::NAN);
            let mut count = 0usize;
            res.fill(&mut out_all[start..], &mut |buf: &mut [f32]| {
                count += buf.len() / 2;
                source(buf);
            });
            pulled += count;
        }
        (out_all, pulled)
    }

    #[test]
    fn constant_input_stays_constant() {
        let mut res = DeviceResampler::new(44_100, 48_000);
        let mut source = |buf: &mut [f32]| buf.fill(0.5);
        let (out, _) = run(&mut res, 10, 480, &mut source);
        // The pre-roll zero frames affect only the first two frames; after that
        // the constant must be exact.
        for (i, v) in out.iter().enumerate().skip(4) {
            assert!((v - 0.5).abs() < 1e-4, "sample {i} was {v}");
        }
    }

    #[test]
    fn pulled_frames_track_the_ratio() {
        let mut res = DeviceResampler::new(44_100, 48_000);
        let mut source = |buf: &mut [f32]| buf.fill(0.5);
        // Odd block size keeps the interpolation stencil off exact boundaries.
        let (out, pulled) = run(&mut res, 100, 479, &mut source);
        let out_frames = out.len() as f64 / 2.0;
        let expected = out_frames * (PROJECT_RATE / DEVICE_RATE);
        let diff = pulled as f64 - expected;
        assert!(
            diff.abs() <= 2.0,
            "pulled {pulled} vs expected {expected} (diff {diff})"
        );
    }

    #[test]
    fn sine_has_no_jumps_at_call_boundaries() {
        let mut res = DeviceResampler::new(44_100, 48_000);
        let mut n = 0u64;
        let mut source = |buf: &mut [f32]| {
            for frame in buf.as_chunks_mut::<2>().0 {
                let t = 2.0 * std::f64::consts::PI * 1_000.0 * n as f64 / PROJECT_RATE;
                let v = 0.9 * t.sin() as f32;
                frame[0] = v;
                frame[1] = v;
                n += 1;
            }
        };
        let mut out = Vec::new();
        // Varied block sizes put call boundaries at arbitrary phases.
        for size in [513usize, 256, 333, 129, 700, 64] {
            let (chunk, _) = run(&mut res, 8, size, &mut source);
            out.extend_from_slice(&chunk);
        }
        let max_step = 0.9 * 2.0 * std::f64::consts::PI * 1_000.0 / DEVICE_RATE;
        for w in out.windows(2) {
            let d = (w[1] - w[0]).abs() as f64;
            assert!(d <= max_step + 1e-3, "jump of {d} between adjacent samples");
        }
    }

    #[test]
    fn hermite_reproduces_endpoints_and_midpoint() {
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 1.0) - 2.0).abs() < 1e-6);
        assert!((hermite(0.0, 0.0, 1.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
        assert!((hermite(0.5, 0.5, 0.5, 0.5, 0.3) - 0.5).abs() < 1e-6);
    }
}
