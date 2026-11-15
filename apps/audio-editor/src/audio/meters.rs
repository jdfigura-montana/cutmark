//! audio::meters: per-track and master peak/RMS written to atomics.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use uuid::Uuid;

/// One channel's meter reading: peak, sum of squares, frames, clipped.
pub type MeterReading = (f32, f32, u32, bool);

/// One channel's lock-free meter accumulators; f32 values stored as bits.
#[derive(Debug)]
pub struct ChannelMeter {
    /// Largest absolute sample of the accumulated blocks (f32 bits).
    peak: AtomicU32,
    /// Sum of squared samples of the accumulated blocks (f32 bits).
    sum_sq: AtomicU32,
    /// Total frames of the accumulated blocks.
    frames: AtomicU32,
    /// True once any accumulated block hit 0 dBFS.
    clipped: AtomicBool,
}

impl ChannelMeter {
    /// A zeroed meter.
    pub fn new() -> ChannelMeter {
        ChannelMeter { peak: AtomicU32::new(0), sum_sq: AtomicU32::new(0), frames: AtomicU32::new(0), clipped: AtomicBool::new(false) }
    }

    /// Audio thread: accumulates one block's peak, sum of squares, frame count
    /// and clip flag. Lock-free, no allocation, no locks.
    pub fn add(&self, peak: f32, sum_sq: f32, frames: u32, clipped: bool) {
        // Peak keeps the maximum; compare-exchange loop on the f32 bits.
        let bits = peak.to_bits();
        let mut cur = self.peak.load(Ordering::Relaxed);
        while f32::from_bits(cur) < peak {
            match self.peak.compare_exchange_weak(cur, bits, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
        // Sum of squares adds; the addition happens on the f32 value.
        let mut cur = self.sum_sq.load(Ordering::Relaxed);
        loop {
            let next = (f32::from_bits(cur) + sum_sq).to_bits();
            match self.sum_sq.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(actual) => cur = actual,
            }
        }
        self.frames.fetch_add(frames, Ordering::Relaxed);
        if clipped {
            self.clipped.store(true, Ordering::Relaxed);
        }
    }

    /// UI thread: swaps all four accumulators to zero and returns what was there.
    pub fn take(&self) -> (f32, f32, u32, bool) {
        (
            f32::from_bits(self.peak.swap(0, Ordering::Relaxed)),
            f32::from_bits(self.sum_sq.swap(0, Ordering::Relaxed)),
            self.frames.swap(0, Ordering::Relaxed),
            self.clipped.swap(false, Ordering::Relaxed),
        )
    }
}

impl Default for ChannelMeter {
    fn default() -> ChannelMeter {
        ChannelMeter::new()
    }
}

/// Left and right meters for one track or the master.
#[derive(Debug)]
pub struct StereoMeter {
    /// Left channel meter.
    pub l: ChannelMeter,
    /// Right channel meter.
    pub r: ChannelMeter,
}

impl StereoMeter {
    /// A zeroed stereo meter.
    pub fn new() -> StereoMeter {
        StereoMeter { l: ChannelMeter::new(), r: ChannelMeter::new() }
    }
}

impl Default for StereoMeter {
    fn default() -> StereoMeter {
        StereoMeter::new()
    }
}

/// One meter pair per track, in the same order as the mix plan's tracks.
#[derive(Debug)]
pub struct MeterBank {
    /// Track id per meter pair.
    pub track_ids: Vec<Uuid>,
    /// One stereo meter per track, same order.
    pub tracks: Vec<StereoMeter>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_twice_then_take_gives_max_and_sums() {
        let m = ChannelMeter::new();
        m.add(0.25, 0.0625, 100, false);
        m.add(0.5, 0.0625, 50, true);
        let (peak, sum_sq, frames, clipped) = m.take();
        assert_eq!(peak, 0.5);
        assert!((sum_sq - 0.125).abs() < 1e-6);
        assert_eq!(frames, 150);
        assert!(clipped);
    }

    #[test]
    fn second_take_returns_zeros() {
        let m = ChannelMeter::new();
        m.add(0.5, 0.25, 10, false);
        let _ = m.take();
        let (peak, sum_sq, frames, clipped) = m.take();
        assert_eq!(peak, 0.0);
        assert_eq!(sum_sq, 0.0);
        assert_eq!(frames, 0);
        assert!(!clipped);
    }
}
