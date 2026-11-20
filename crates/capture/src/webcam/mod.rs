//! Webcam capture through Media Foundation.
//!
//! The camera is read on its own thread by an IMFSourceReader that decodes to
//! NV12 straight into D3D11 textures on the shared device. Each frame is copied
//! into a texture we own and stamped with the sample time converted to QPC. See
//! the recorder spec: Webcam, chroma key, compositor, Camera.

mod devices;
mod reader;

pub use devices::{CameraFormat, CameraInfo, CameraSubtype, list_cameras};
pub use reader::{CamFrame, WebcamCapture, WebcamPath, WebcamStats};

use std::sync::{Mutex, PoisonError};

use crate::{CaptureError, with_context};
use windows::Win32::Media::MediaFoundation::{MF_VERSION, MFSTARTUP_FULL, MFShutdown, MFStartup};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// Media Foundation sample times and durations count in units of 100 nanoseconds.
const HNS_PER_SECOND: i64 = 10_000_000;

/// How many guards are alive; MFStartup runs on the first and MFShutdown on the last.
static MF_USERS: Mutex<usize> = Mutex::new(0);

/// Keeps Media Foundation started for as long as any guard is alive.
pub(crate) struct MfGuard;

impl MfGuard {
    /// Starts Media Foundation when this is the first guard in the process.
    #[allow(unsafe_code)]
    pub(crate) fn acquire() -> Result<Self, CaptureError> {
        let mut users = MF_USERS.lock().unwrap_or_else(PoisonError::into_inner);
        if *users == 0 {
            // SAFETY: MFStartup takes plain values and is balanced by the MFShutdown in Drop.
            with_context("failed to start Media Foundation", unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) })?;
        }
        *users += 1;
        Ok(Self)
    }
}

impl Drop for MfGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        let mut users = MF_USERS.lock().unwrap_or_else(PoisonError::into_inner);
        *users = users.saturating_sub(1);
        if *users == 0 {
            // SAFETY: the last guard is gone, so every start call has its matching shutdown here.
            if let Err(error) = unsafe { MFShutdown() } {
                log::warn!("MFShutdown failed: {error}");
            }
        }
    }
}

/// Initializes COM on the current thread and undoes it on drop when this call did the work.
pub(crate) struct ComGuard {
    initialized: bool,
}

impl ComGuard {
    /// Joins the multithreaded apartment, or accepts the apartment the thread already has.
    #[allow(unsafe_code)]
    pub(crate) fn init() -> Self {
        // SAFETY: CoInitializeEx takes plain values. S_OK and S_FALSE both count a
        // reference that Drop releases; RPC_E_CHANGED_MODE means COM is already
        // initialized in another mode, which Media Foundation tolerates.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        Self { initialized: result.is_ok() }
    }
}

impl Drop for ComGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx in init on this same thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Converts a sample time in 100 nanosecond units into QPC ticks at the given frequency.
///
/// Camera drivers stamp samples from the QPC clock, so this puts them on the
/// same timeline as every other capture source.
pub(crate) fn hns_to_qpc(hns: i64, freq: i64) -> i64 {
    // Split so the multiply cannot overflow i64 even after days of uptime.
    (hns / HNS_PER_SECOND) * freq + ((hns % HNS_PER_SECOND) * freq) / HNS_PER_SECOND
}

/// Converts a QPC tick difference into milliseconds.
pub(crate) fn qpc_ticks_to_ms(ticks: i64, freq: i64) -> f64 {
    if freq <= 0 {
        return 0.0;
    }
    ticks as f64 * 1000.0 / freq as f64
}

/// Running mean, standard deviation and largest absolute value of a stream of samples.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct RunningStats {
    count: u64,
    mean: f64,
    m2: f64,
    max_abs: f64,
}

impl RunningStats {
    /// Adds one sample (Welford's update).
    pub(crate) fn push(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        self.m2 += delta * (value - self.mean);
        self.max_abs = self.max_abs.max(value.abs());
    }

    /// The mean of the samples so far, or 0 with no samples.
    pub(crate) fn mean(&self) -> f64 {
        self.mean
    }

    /// The population standard deviation, or 0 with fewer than two samples.
    pub(crate) fn std_dev(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        (self.m2 / self.count as f64).sqrt()
    }

    /// The largest absolute value seen so far.
    pub(crate) fn max_abs(&self) -> f64 {
        self.max_abs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hns_to_qpc_is_identity_at_ten_mhz() {
        assert_eq!(hns_to_qpc(123_456_789, 10_000_000), 123_456_789);
    }

    #[test]
    fn hns_to_qpc_scales_to_other_frequencies() {
        // One second is 3,000,000 ticks at 3 MHz; half a second is 1,500,000.
        assert_eq!(hns_to_qpc(10_000_000, 3_000_000), 3_000_000);
        assert_eq!(hns_to_qpc(5_000_000, 3_000_000), 1_500_000);
        assert_eq!(hns_to_qpc(0, 3_000_000), 0);
    }

    #[test]
    fn hns_to_qpc_does_not_overflow_after_weeks_of_uptime() {
        let thirty_days_hns = 30 * 24 * 3600 * HNS_PER_SECOND;
        let freq = 1_000_000_000;
        assert_eq!(hns_to_qpc(thirty_days_hns, freq), 30 * 24 * 3600 * freq);
    }

    #[test]
    fn qpc_ticks_convert_to_milliseconds() {
        assert!((qpc_ticks_to_ms(15_000, 10_000_000) - 1.5).abs() < 1e-9);
        assert!((qpc_ticks_to_ms(-10_000_000, 10_000_000) + 1000.0).abs() < 1e-9);
        assert_eq!(qpc_ticks_to_ms(100, 0), 0.0);
    }

    #[test]
    fn running_stats_match_a_hand_computed_series() {
        let mut stats = RunningStats::default();
        for value in [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            stats.push(value);
        }
        assert!((stats.mean() - 5.0).abs() < 1e-9);
        assert!((stats.std_dev() - 2.0).abs() < 1e-9);
        assert!((stats.max_abs() - 9.0).abs() < 1e-9);
    }

    #[test]
    fn running_stats_track_the_largest_magnitude_of_either_sign() {
        let mut stats = RunningStats::default();
        stats.push(3.0);
        stats.push(-8.0);
        assert!((stats.max_abs() - 8.0).abs() < 1e-9);
        assert_eq!(RunningStats::default().std_dev(), 0.0);
    }
}
