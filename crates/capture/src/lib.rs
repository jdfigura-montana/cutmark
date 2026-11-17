//! Cutmark suite screen capture crate.
//!
//! One shared D3D11 device plus Windows Graphics Capture (WGC) monitor capture.
//! Frames stay on the GPU as BGRA textures, and every frame is stamped with QPC
//! on arrival. See the recorder spec: Screen capture, and Sync and reliability
//! rules, Clock and frame rate.

#[cfg(windows)]
mod audio;
#[cfg(windows)]
mod convert;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod monitor;
#[cfg(windows)]
mod wgc;

#[cfg(windows)]
pub use audio::{AudioCapture, AudioDevice, AudioPacket, DeviceKind, find_device, list_audio_devices};
#[cfg(windows)]
pub use convert::Nv12Converter;
#[cfg(windows)]
pub use gpu::{AdapterInfo, Gpu, list_adapters};
#[cfg(windows)]
pub use monitor::{MonitorInfo, list_monitors};
#[cfg(windows)]
pub use wgc::{LatestFrame, MonitorCapture};

/// Error type for the capture crate.
#[cfg(windows)]
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// A Windows API call failed.
    #[error("{context}: {source}")]
    Windows {
        /// What the crate was doing when the call failed.
        context: &'static str,
        /// The underlying Windows error.
        #[source]
        source: windows::core::Error,
    },
    /// The requested DXGI adapter index does not exist.
    #[error("no DXGI adapter with index {0}")]
    AdapterIndex(usize),
    /// An expected result was missing, for example no primary monitor.
    #[error("{0}")]
    Missing(&'static str),
    /// The input texture does not match the size the converter was created for.
    #[error("input size {actual_w}x{actual_h} does not match the converter input size {expected_w}x{expected_h}; recreate the Nv12Converter")]
    InputSizeChanged {
        /// Input width the converter was created for.
        expected_w: u32,
        /// Input height the converter was created for.
        expected_h: u32,
        /// Width of the texture that arrived.
        actual_w: u32,
        /// Height of the texture that arrived.
        actual_h: u32,
    },
}

/// Placeholder error so the crate still compiles on non-Windows targets.
#[cfg(not(windows))]
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {}

/// Maps a Windows result into a CaptureError with a context string.
#[cfg(windows)]
pub(crate) fn with_context<T>(context: &'static str, result: windows::core::Result<T>) -> Result<T, CaptureError> {
    result.map_err(|source| CaptureError::Windows { context, source })
}

/// Converts a NUL terminated UTF-16 buffer into a String.
#[cfg(windows)]
pub(crate) fn utf16_to_string(chars: &[u16]) -> String {
    let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// The QPC counter value at this instant, in counter ticks.
#[cfg(windows)]
pub fn qpc_now() -> i64 {
    qpc_counter().unwrap_or(0)
}

/// The QPC counter frequency, in ticks per second.
#[cfg(windows)]
pub fn qpc_frequency() -> i64 {
    qpc_ticks_per_second().unwrap_or(0)
}

/// Converts a QPC counter value into 100 nanosecond units on the same clock.
#[cfg(windows)]
pub fn qpc_to_100ns(qpc: i64) -> i64 {
    let freq = qpc_frequency();
    if freq <= 0 {
        return 0;
    }
    // Split so the multiply cannot overflow i64 even after days of uptime.
    (qpc / freq) * 10_000_000 + ((qpc % freq) * 10_000_000) / freq
}

/// The QPC counter value at this instant, in 100 nanosecond units.
#[cfg(windows)]
pub fn qpc_now_100ns() -> i64 {
    qpc_to_100ns(qpc_now())
}

/// Reads the QPC counter.
#[cfg(windows)]
#[allow(unsafe_code)]
fn qpc_counter() -> windows::core::Result<i64> {
    let mut value = 0i64;
    // SAFETY: QueryPerformanceCounter only writes through the pointer we pass.
    unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut value) }.map(|()| value)
}

/// Reads the QPC frequency.
#[cfg(windows)]
#[allow(unsafe_code)]
fn qpc_ticks_per_second() -> windows::core::Result<i64> {
    let mut value = 0i64;
    // SAFETY: QueryPerformanceFrequency only writes through the pointer we pass.
    unsafe { windows::Win32::System::Performance::QueryPerformanceFrequency(&mut value) }.map(|()| value)
}
