//! Change detection for cursor samples: a sample is sent only when something changed, or when the
//! keep-alive comes due.

use super::CursorSample;

/// The longest gap between two samples, in milliseconds.
const KEEP_ALIVE_MS: i64 = 250;

/// Decides which cursor readings become samples.
pub(crate) struct SampleFilter {
    /// The keep-alive gap in QPC ticks.
    keep_alive_ticks: i64,
    /// The last sample that was actually sent.
    last: Option<CursorSample>,
}

impl SampleFilter {
    pub(crate) fn new(qpc_frequency: i64) -> Self {
        Self { keep_alive_ticks: qpc_frequency.saturating_mul(KEEP_ALIVE_MS) / 1000, last: None }
    }

    /// Whether this reading should be sent.
    pub(crate) fn wants(&self, sample: &CursorSample) -> bool {
        let Some(last) = &self.last else { return true };
        (last.x, last.y, last.shape, last.visible) != (sample.x, sample.y, sample.shape, sample.visible)
            || sample.qpc.saturating_sub(last.qpc) >= self.keep_alive_ticks
    }

    /// Records a sample that was sent. A sample that could not be sent is not recorded, so the next
    /// reading offers it again.
    pub(crate) fn sent(&mut self, sample: CursorSample) {
        self.last = Some(sample);
    }
}

#[cfg(test)]
mod tests {
    use super::{CursorSample, SampleFilter};

    /// 1000 ticks a second, so one tick is a millisecond.
    const FREQUENCY: i64 = 1000;

    fn sample(qpc: i64) -> CursorSample {
        CursorSample { qpc, x: 10, y: 20, shape: Some(1), visible: true }
    }

    fn filter_after_first() -> SampleFilter {
        let mut filter = SampleFilter::new(FREQUENCY);
        assert!(filter.wants(&sample(0)));
        filter.sent(sample(0));
        filter
    }

    #[test]
    fn unchanged_reading_is_skipped() {
        assert!(!filter_after_first().wants(&sample(4)));
    }

    #[test]
    fn every_kind_of_change_is_sent() {
        let filter = filter_after_first();
        assert!(filter.wants(&CursorSample { x: 11, ..sample(4) }));
        assert!(filter.wants(&CursorSample { y: 21, ..sample(4) }));
        assert!(filter.wants(&CursorSample { shape: Some(2), ..sample(4) }));
        assert!(filter.wants(&CursorSample { shape: None, ..sample(4) }));
        assert!(filter.wants(&CursorSample { visible: false, ..sample(4) }));
    }

    #[test]
    fn keep_alive_comes_due_at_250_ms() {
        let filter = filter_after_first();
        assert!(!filter.wants(&sample(249)));
        assert!(filter.wants(&sample(250)));
    }

    #[test]
    fn keep_alive_counts_from_the_last_sent_sample() {
        let mut filter = filter_after_first();
        filter.sent(CursorSample { x: 11, ..sample(100) });
        assert!(!filter.wants(&CursorSample { x: 11, ..sample(349) }));
        assert!(filter.wants(&CursorSample { x: 11, ..sample(350) }));
    }

    #[test]
    fn a_sample_that_was_not_sent_is_offered_again() {
        let filter = filter_after_first();
        let moved = CursorSample { x: 11, ..sample(4) };
        assert!(filter.wants(&moved));
        assert!(filter.wants(&CursorSample { qpc: 8, ..moved }));
    }
}
