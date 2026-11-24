//! Cutmark suite media IO crate.
//!
//! Recorder phase 2: hands the suite's shared D3D11 device to FFmpeg and
//! manages a pool of D3D11 frames that NV12 textures are copied into with no
//! CPU round trip. Phase 3 adds the software audio encoders and the mux
//! thread. See the recorder spec: Screen capture, Encode, and Audio capture
//! and tracks.

#[cfg(windows)]
mod hw;

#[cfg(windows)]
pub mod bgvideo;

#[cfg(windows)]
mod amf;

#[cfg(windows)]
pub mod audio;

#[cfg(windows)]
pub mod encode;

#[cfg(windows)]
pub mod mux;

#[cfg(windows)]
pub mod sync;

#[cfg(windows)]
pub mod player;

#[cfg(windows)]
pub use amf::AmfEncoder;

#[cfg(windows)]
pub use audio::{AudioCodec, AudioEncoder};

#[cfg(windows)]
pub use encode::{Codec, EncodedPacket, Encoder, EncoderConfig, Preset, RateControl};

#[cfg(windows)]
pub use mux::{Container, MuxSummary, MuxThread, Muxer};

#[cfg(windows)]
pub use sync::{SyncError, SyncReport, measure_offset};

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
