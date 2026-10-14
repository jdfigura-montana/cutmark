//! util::fade: the linear fade rule shared by the mixer, waveform and peak measuring.

/// Linear fade factor (0.0..=1.0) at `local` samples into a clip of `len` samples.
///
/// Inside the fade-in: `local / fade_in`. Inside the fade-out:
/// `(len - local) / fade_out`. Inside both: the smaller. Otherwise 1.0.
/// Clip gain is not included. No allocation, no panics.
pub fn fade_gain(local: u64, len: u64, fade_in: u64, fade_out: u64) -> f32 {
    let mut f = 1.0f32;
    if fade_in > 0 && local < fade_in {
        f = f.min(local as f32 / fade_in as f32);
    }
    if fade_out > 0 {
        let from_end = len.saturating_sub(local);
        if from_end < fade_out {
            f = f.min(from_end as f32 / fade_out as f32);
        }
    }
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_without_fades() {
        assert_eq!(fade_gain(0, 1_000, 0, 0), 1.0);
        assert_eq!(fade_gain(999, 1_000, 0, 0), 1.0);
    }

    #[test]
    fn fade_in_halfway() {
        assert!((fade_gain(50, 1_000, 100, 0) - 0.5).abs() < 1e-6);
        assert_eq!(fade_gain(0, 1_000, 100, 0), 0.0);
        assert_eq!(fade_gain(100, 1_000, 100, 0), 1.0);
    }

    #[test]
    fn fade_out_halfway() {
        assert!((fade_gain(950, 1_000, 0, 100) - 0.5).abs() < 1e-6);
        assert_eq!(fade_gain(900, 1_000, 0, 100), 1.0);
    }

    #[test]
    fn overlapping_fades_take_the_smaller() {
        // len 100, fade_in 60, fade_out 80 overlap on 20..60.
        // local 40: in = 40/60 = 0.667, out = 60/80 = 0.75 -> in wins.
        assert!((fade_gain(40, 100, 60, 80) - 40.0 / 60.0).abs() < 1e-6);
        // local 55: in = 55/60 = 0.917, out = 45/80 = 0.5625 -> out wins.
        assert!((fade_gain(55, 100, 60, 80) - 0.5625).abs() < 1e-6);
    }

    #[test]
    fn past_the_end_does_not_panic() {
        assert_eq!(fade_gain(2_000, 1_000, 0, 100), 0.0);
    }
}
