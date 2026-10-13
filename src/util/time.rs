//! util::time: samples <-> timecode conversion.

/// Formats a sample position as `HH:MM:SS.mmm` at the given sample rate.
/// Milliseconds are rounded down.
pub fn samples_to_timecode(samples: u64, rate: u32) -> String {
    let total_ms = samples * 1000 / u64::from(rate);
    let hh = total_ms / 3_600_000;
    let mm = (total_ms / 60_000) % 60;
    let ss = (total_ms / 1000) % 60;
    let ms = total_ms % 1000;
    format!("{hh:02}:{mm:02}:{ss:02}.{ms:03}")
}

/// Converts a sample count to seconds at the given sample rate.
pub fn samples_to_seconds(samples: u64, rate: u32) -> f64 {
    samples as f64 / f64::from(rate)
}

/// Converts seconds to a sample count at the given rate, rounded to nearest.
/// Negative input saturates at zero.
pub fn seconds_to_samples(secs: f64, rate: u32) -> u64 {
    (secs * f64::from(rate)).round().max(0.0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_zero() {
        assert_eq!(samples_to_timecode(0, 48_000), "00:00:00.000");
    }

    #[test]
    fn timecode_one_second() {
        assert_eq!(samples_to_timecode(48_000, 48_000), "00:00:01.000");
    }

    #[test]
    fn timecode_one_two_three_half() {
        // 1 h 2 min 3.5 s at 1000 Hz.
        assert_eq!(samples_to_timecode(3_723_500, 1000), "01:02:03.500");
    }

    #[test]
    fn timecode_milliseconds_round_down() {
        // 1.5 samples at 1000 Hz = 1.5 ms, floor to 1 ms.
        assert_eq!(samples_to_timecode(1, 1_000), "00:00:00.001");
        assert_eq!(samples_to_timecode(2, 1_000), "00:00:00.002");
    }

    #[test]
    fn seconds_round_trip() {
        let secs = samples_to_seconds(96_000, 48_000);
        assert_eq!(secs, 2.0);
        assert_eq!(seconds_to_samples(secs, 48_000), 96_000);
        // 0.5 s at 44_100 Hz is 22_050 exactly; 1/3 s rounds to nearest.
        assert_eq!(seconds_to_samples(0.5, 44_100), 22_050);
        assert_eq!(seconds_to_samples(1.0 / 3.0, 30_000), 10_000);
    }

    #[test]
    fn negative_seconds_saturate_at_zero() {
        assert_eq!(seconds_to_samples(-1.0, 48_000), 0);
    }
}
