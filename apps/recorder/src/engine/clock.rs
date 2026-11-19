//! The media clock: turns QPC times into recording time with paused time removed.

/// Maps QPC values onto media time: time since the start, minus every pause.
#[derive(Debug, Clone)]
pub struct MediaClock {
    start_qpc: i64,
    qpc_freq: i64,
    /// Finished pauses as `(pause_qpc, resume_qpc)`.
    paused: Vec<(i64, i64)>,
    /// When the clock is paused right now, the QPC time the pause began.
    paused_at: Option<i64>,
}

impl MediaClock {
    /// A clock whose media time 0 is `start_qpc`, on a counter that ticks `qpc_freq` times a second.
    pub fn new(start_qpc: i64, qpc_freq: i64) -> Self {
        Self { start_qpc, qpc_freq: qpc_freq.max(1), paused: Vec::new(), paused_at: None }
    }

    /// Starts a pause at `qpc`; does nothing when already paused.
    pub fn pause(&mut self, qpc: i64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(qpc);
        }
    }

    /// Ends the pause at `qpc`; does nothing when not paused.
    pub fn resume(&mut self, qpc: i64) {
        if let Some(at) = self.paused_at.take() {
            self.paused.push((at, qpc.max(at)));
        }
    }

    /// The media time in milliseconds at `qpc`, with paused time removed and clamped at 0.
    ///
    /// A time inside a pause maps to the moment the pause began.
    pub fn media_ms(&self, qpc: i64) -> u64 {
        let mut paused: i128 = self.paused.iter().map(|&(from, to)| overlap(from, to, qpc)).sum();
        if let Some(from) = self.paused_at {
            paused += overlap(from, i64::MAX, qpc);
        }
        let ticks = (i128::from(qpc) - i128::from(self.start_qpc) - paused).max(0);
        u64::try_from(ticks * 1000 / i128::from(self.qpc_freq)).unwrap_or(u64::MAX)
    }
}

/// How many ticks of the pause from `from` to `to` lie before `qpc`.
fn overlap(from: i64, to: i64, qpc: i64) -> i128 {
    (i128::from(to.min(qpc)) - i128::from(from)).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FREQ: i64 = 10_000_000;
    const START: i64 = 1_000_000_000;

    fn at_seconds(seconds: i64) -> i64 {
        START + seconds * FREQ
    }

    #[test]
    fn no_pause_is_plain_elapsed_time() {
        let clock = MediaClock::new(START, FREQ);
        assert_eq!(clock.media_ms(at_seconds(3)), 3000);
        assert_eq!(clock.media_ms(START), 0);
    }

    #[test]
    fn before_the_start_clamps_to_zero() {
        let clock = MediaClock::new(START, FREQ);
        assert_eq!(clock.media_ms(START - 5 * FREQ), 0);
    }

    #[test]
    fn a_five_second_pause_is_removed() {
        let mut clock = MediaClock::new(START, FREQ);
        clock.pause(at_seconds(2));
        clock.resume(at_seconds(7));
        assert_eq!(clock.media_ms(at_seconds(10)), 5000);
        assert_eq!(clock.media_ms(at_seconds(1)), 1000);
    }

    #[test]
    fn a_press_inside_a_pause_maps_to_the_pause_point() {
        let mut clock = MediaClock::new(START, FREQ);
        clock.pause(at_seconds(2));
        // While the pause is still open.
        assert_eq!(clock.media_ms(at_seconds(4)), 2000);
        clock.resume(at_seconds(7));
        // After it closed, a press that was made inside it.
        assert_eq!(clock.media_ms(at_seconds(4)), 2000);
        assert_eq!(clock.media_ms(at_seconds(7)), 2000);
    }

    #[test]
    fn ten_hours_have_no_rounding_drift() {
        let freq = 10_000_019;
        let clock = MediaClock::new(START, freq);
        let ten_hours = START + 36_000 * freq;
        assert_eq!(clock.media_ms(ten_hours), 36_000_000);
        assert_eq!(clock.media_ms(ten_hours + freq), 36_001_000);
        assert_eq!(clock.media_ms(ten_hours - 1), 35_999_999);
    }

    #[test]
    fn ten_hours_with_a_pause_stay_exact() {
        let freq = 10_000_019;
        let mut clock = MediaClock::new(START, freq);
        clock.pause(START + 100 * freq);
        clock.resume(START + 160 * freq);
        assert_eq!(clock.media_ms(START + 36_000 * freq), 35_940_000);
    }
}
