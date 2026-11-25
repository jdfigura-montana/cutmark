//! The autosave timer (spec "Library window": autosave on edit, 500 ms debounce).

use std::time::{Duration, Instant};

/// How long the text has to rest before it is saved.
pub const DELAY: Duration = Duration::from_millis(500);

/// How long until text last changed at `last_edit` is due to be saved. Zero means now.
pub fn time_left(last_edit: Instant, now: Instant) -> Duration {
    DELAY.saturating_sub(now.saturating_duration_since(last_edit))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_save_is_due_500_ms_after_the_last_edit() {
        let edit = Instant::now();
        let after = |ms| time_left(edit, edit + Duration::from_millis(ms));
        assert_eq!(after(0), Duration::from_millis(500));
        assert_eq!(after(499), Duration::from_millis(1));
        assert_eq!(after(500), Duration::ZERO);
        assert_eq!(after(9_000), Duration::ZERO);
    }

    #[test]
    fn another_edit_starts_the_wait_again() {
        let first = Instant::now();
        let second = first + Duration::from_millis(400);
        assert_eq!(time_left(second, first + Duration::from_millis(500)), Duration::from_millis(400));
        // A clock reading from before the edit waits the whole delay.
        assert_eq!(time_left(second, first), DELAY);
    }
}
