//! io::limits: hard bounds on untrusted audio and cache files, in one place.

/// Lowest sample rate an imported file may declare, in Hz.
pub const MIN_SOURCE_RATE: u32 = 8_000;
/// Highest sample rate an imported file may declare, in Hz.
pub const MAX_SOURCE_RATE: u32 = 384_000;
/// Longest media item that is imported or loaded, in seconds (2 hours).
pub const MAX_ITEM_SECONDS: u64 = 7_200;

/// Most channels a peaks cache file may declare.
pub const MAX_PEAK_CHANNELS: u16 = 2;
/// Most levels a peaks cache file may declare.
pub const MAX_PEAK_LEVELS: u32 = 8;

/// Most frames a file at `rate` Hz may hold when it may last `max_seconds`.
pub fn max_frames(rate: u32, max_seconds: u64) -> u64 {
    max_seconds.saturating_mul(u64::from(rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_hours_at_48k_is_345_6_million_frames() {
        assert_eq!(max_frames(48_000, MAX_ITEM_SECONDS), 345_600_000);
    }

    #[test]
    fn max_frames_saturates_instead_of_overflowing() {
        assert_eq!(max_frames(u32::MAX, u64::MAX), u64::MAX);
    }
}
