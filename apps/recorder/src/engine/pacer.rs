//! The constant frame rate pacer: pure QPC time to frame index logic, with no
//! GPU, no capture, no egui and no FFmpeg.

use std::ops::Range;

/// Turns QPC timestamps into exact constant frame rate frame indexes.
///
/// A take of exactly T seconds yields exactly T times the frame rate frames:
/// a frame is due once its tick time is strictly before the QPC now value.
/// The recording loop that drives it arrives in a later phase 2 prompt.
#[allow(dead_code)]
pub struct Pacer {
    /// The QPC frequency, in ticks per second.
    qpc_freq: i64,
    /// Frame rate numerator.
    fps_num: u32,
    /// Frame rate denominator.
    fps_den: u32,
    /// The QPC value that recording time 0 maps to, shifted by pauses.
    start_qpc: i64,
    /// The next frame index that has not been reported as due yet.
    next_frame: u64,
    /// The source sequence number of the last recorded frame.
    last_source_seq: Option<u64>,
    /// Total frames recorded so far.
    frames: u64,
    /// Total recorded frames that repeated the previous source frame.
    duplicated: u64,
    /// Total recorded frames that never reached the encoder.
    dropped: u64,
    /// The QPC value when pause was called, if currently paused.
    paused_at: Option<i64>,
}

#[allow(dead_code)]
impl Pacer {
    /// Creates a pacer that ticks at the exact given frame rate from start_qpc.
    pub fn new(qpc_freq: i64, fps_num: u32, fps_den: u32, start_qpc: i64) -> Self {
        Self { qpc_freq, fps_num, fps_den, start_qpc, next_frame: 0, last_source_seq: None, frames: 0, duplicated: 0, dropped: 0, paused_at: None }
    }

    /// The frame indexes whose tick time has passed since the last call.
    ///
    /// Tick time for frame n is start_qpc plus n times qpc_freq times fps_den
    /// divided by fps_num, all in i128 integer math, so there is no drift over
    /// hours. Nothing is due while paused.
    pub fn due_frames(&mut self, now_qpc: i64) -> Range<u64> {
        let empty = self.next_frame..self.next_frame;
        if self.paused_at.is_some() {
            return empty;
        }
        let elapsed = (now_qpc - self.start_qpc) as i128;
        if elapsed <= 0 {
            return empty;
        }
        // last is the largest n with n * qpc_freq * fps_den < elapsed * fps_num,
        // which is ceil(elapsed * fps_num / (qpc_freq * fps_den)) minus one.
        let numer = elapsed * i128::from(self.fps_num);
        let denom = i128::from(self.qpc_freq) * i128::from(self.fps_den);
        let last = ((numer - 1) / denom) as u64;
        if last < self.next_frame {
            return empty;
        }
        let end = last + 1;
        let range = self.next_frame..end;
        self.next_frame = end;
        range
    }

    /// The encoder time base is fps_den over fps_num, so the pts is just n.
    pub fn pts_for(&self, n: u64) -> i64 {
        n as i64
    }

    /// Counts one sent or dropped frame; a repeated source_seq is a duplicate.
    pub fn record(&mut self, _n: u64, source_seq: u64, sent: bool) {
        if self.last_source_seq == Some(source_seq) {
            self.duplicated += 1;
        }
        self.last_source_seq = Some(source_seq);
        if !sent {
            self.dropped += 1;
        }
        self.frames += 1;
    }

    /// Stops the clock: nothing is due until resume is called.
    pub fn pause(&mut self, now_qpc: i64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now_qpc);
        }
    }

    /// Continues from the same frame index with no gap by shifting the start.
    pub fn resume(&mut self, now_qpc: i64) {
        if let Some(at) = self.paused_at.take() {
            self.start_qpc += now_qpc - at;
        }
    }

    /// Total frames recorded so far.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Total recorded frames that repeated the previous source frame.
    pub fn duplicated(&self) -> u64 {
        self.duplicated
    }

    /// Total recorded frames that never reached the encoder.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real QPC frequency: 100 ns ticks.
    const QPC_FREQ: i64 = 10_000_000;

    /// Irregular step sizes, in QPC ticks, well over and under one frame.
    const STEPS: [i64; 5] = [16_516, 199_233, 41_717, 1_002_119, 7_331];

    /// Drives the pacer from 0 to end in irregular steps, clamping the last
    /// step so the final now lands exactly on end, and collects every index.
    fn drive_irregular(pacer: &mut Pacer, end: i64) -> Vec<u64> {
        let mut now = 0i64;
        let mut seen = Vec::new();
        let mut step_index = 0;
        while now < end {
            now = (now + STEPS[step_index % STEPS.len()]).min(end);
            step_index += 1;
            seen.extend(pacer.due_frames(now));
        }
        seen
    }

    #[test]
    fn ten_minutes_at_60fps_yields_exactly_36000_frames() {
        let mut pacer = Pacer::new(QPC_FREQ, 60, 1, 0);
        let seen = drive_irregular(&mut pacer, 600 * QPC_FREQ);
        assert_eq!(seen, (0..36_000).collect::<Vec<u64>>());
        for n in 0..36_000 {
            pacer.record(n, n, true);
        }
        assert_eq!(pacer.frames(), 36_000);
        assert_eq!(pacer.duplicated(), 0);
        assert_eq!(pacer.dropped(), 0);
        assert_eq!(pacer.pts_for(35_999), 35_999);
    }

    #[test]
    fn one_hour_at_30000_1001_yields_the_exact_rational_count() {
        // 3600 seconds times 30000/1001: ticks strictly before 3600 s are
        // indexes 0 to 107892, so the exact rational count is 107893.
        let mut pacer = Pacer::new(QPC_FREQ, 30_000, 1_001, 0);
        let seen = drive_irregular(&mut pacer, 3600 * QPC_FREQ);
        assert_eq!(seen.len(), 107_893);
        assert_eq!(seen.first(), Some(&0));
        assert_eq!(seen.last(), Some(&107_892));
        for n in 0..107_893 {
            pacer.record(n, n, true);
        }
        assert_eq!(pacer.frames(), 107_893);
    }

    #[test]
    fn repeated_source_sequences_count_as_duplicated() {
        let mut pacer = Pacer::new(QPC_FREQ, 60, 1, 0);
        pacer.record(0, 100, true);
        pacer.record(1, 100, true);
        assert_eq!(pacer.duplicated(), 1);
        pacer.record(2, 101, true);
        assert_eq!(pacer.duplicated(), 1);
        pacer.record(3, 101, false);
        assert_eq!(pacer.duplicated(), 2);
        assert_eq!(pacer.dropped(), 1);
        assert_eq!(pacer.frames(), 4);
    }

    #[test]
    fn a_five_second_pause_adds_no_frames_and_no_gap() {
        let mut pacer = Pacer::new(QPC_FREQ, 60, 1, 0);
        let before: Vec<u64> = pacer.due_frames(2 * QPC_FREQ).collect();
        assert_eq!(before.len(), 120);
        pacer.pause(2 * QPC_FREQ);
        // Five seconds of wall time pass while paused: nothing is due.
        assert!(pacer.due_frames(7 * QPC_FREQ).is_empty());
        pacer.resume(7 * QPC_FREQ);
        let after: Vec<u64> = pacer.due_frames(9 * QPC_FREQ).collect();
        assert_eq!(after.len(), 120);
        assert_eq!(after[0], 120);
        let mut all = before;
        all.extend(after);
        assert_eq!(all, (0..240).collect::<Vec<u64>>());
        for n in all {
            pacer.record(n, n, true);
        }
        assert_eq!(pacer.frames(), 240);
        assert_eq!(pacer.pts_for(239), 239);
    }

    #[test]
    fn due_frames_never_returns_an_index_twice() {
        let mut pacer = Pacer::new(QPC_FREQ, 60, 1, 0);
        let mut seen = Vec::new();
        // One QPC tick at a time is the densest possible drive.
        for t in 1..=10 * QPC_FREQ {
            seen.extend(pacer.due_frames(t));
        }
        for pair in seen.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert_eq!(seen, (0..600).collect::<Vec<u64>>());
    }
}
