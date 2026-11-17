//! Cutmark suite media IO crate.
//!
//! Recorder phase 2: hands the suite's shared D3D11 device to FFmpeg and
//! manages a pool of D3D11 frames that NV12 textures are copied into with no
//! CPU round trip. A later phase feeds those hardware frames straight to an
//! FFmpeg encoder. See the recorder spec: Screen capture, and Encode.

#[cfg(windows)]
mod hw;

#[cfg(windows)]
pub use hw::{D3d11Device, D3d11FramePool, HwFrame, MediaError};

/// Initialises FFmpeg and sets its log level to Error.
#[cfg(windows)]
pub fn init() -> Result<(), MediaError> {
    ffmpeg_next::init()?;
    set_ffmpeg_log_level();
    Ok(())
}

/// Sets the FFmpeg log level to Error.
#[cfg(windows)]
#[allow(unsafe_code)]
fn set_ffmpeg_log_level() {
    // SAFETY: av_log_set_level only stores the level in a global.
    unsafe {
        ffmpeg_next::ffi::av_log_set_level(ffmpeg_next::ffi::AV_LOG_ERROR);
    }
}
