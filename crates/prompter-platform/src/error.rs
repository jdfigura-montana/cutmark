//! The platform layer's error type.

/// Why a platform call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlatformError {
    /// A Win32 call returned an error.
    #[error("{call} failed with Win32 error {code}")]
    Win32 {
        /// The function that failed.
        call: &'static str,
        /// The Win32 error code, as `GetLastError` reports it.
        code: u32,
    },
    /// Windows accepted the capture exclusion, but the window reads back something else.
    #[error("capture exclusion did not stick: the window's display affinity is {affinity:#x}")]
    ExclusionNotApplied {
        /// The display affinity the window reported.
        affinity: u32,
    },
    /// This platform has no backend for the call.
    #[error("not supported on this platform")]
    Unsupported,
}

/// The result of a platform call.
pub type Result<T> = std::result::Result<T, PlatformError>;
