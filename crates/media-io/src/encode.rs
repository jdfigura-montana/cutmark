//! The Encoder trait and the vendor-neutral encoder configuration types.

use ffmpeg_next::codec::Parameters;
use ffmpeg_next::packet::Packet;
use serde::{Deserialize, Serialize};

pub use crate::amf::{CodecSupport, NumericOption, OptionRanges, amf_options, probe_amf};

use crate::HwFrame;
use crate::MediaError;

/// The largest bitrate any rate control mode accepts, in megabits per second.
pub const MAX_MBPS: f32 = 500.0;

/// The video codec to encode with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    /// AV1.
    Av1,
    /// HEVC, also known as H.265.
    Hevc,
    /// H.264, also known as AVC.
    H264,
}

/// The speed and quality balance preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preset {
    /// Fastest, lowest quality.
    Speed,
    /// The middle ground.
    Balanced,
    /// Slowest, best quality.
    Quality,
}

/// Vendor-neutral rate control, mapped to each backend's own options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum RateControl {
    /// Constant quality with a bitrate ceiling.
    ConstantQuality {
        /// Quality, 1 (smallest file) to 100 (best picture).
        quality: u8,
        /// Maximum bitrate in megabits per second.
        max_mbps: f32,
    },
    /// Constant quantisation parameters for I and P frames.
    ConstantQp {
        /// Quantisation parameter for I frames.
        qp_i: u8,
        /// Quantisation parameter for P frames.
        qp_p: u8,
    },
    /// Constant bitrate.
    Cbr {
        /// Bitrate in megabits per second.
        mbps: f32,
    },
    /// Variable bitrate with a peak ceiling.
    VbrPeak {
        /// Target bitrate in megabits per second.
        target_mbps: f32,
        /// Maximum bitrate in megabits per second.
        max_mbps: f32,
    },
}

/// Everything needed to open one encoder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncoderConfig {
    /// The codec to encode with.
    pub codec: Codec,
    /// Output width in pixels.
    pub width: u32,
    /// Output height in pixels.
    pub height: u32,
    /// Frame rate numerator.
    pub fps_num: u32,
    /// Frame rate denominator.
    pub fps_den: u32,
    /// The rate control mode.
    pub rate_control: RateControl,
    /// The speed and quality balance preset.
    pub preset: Preset,
    /// Seconds between keyframes.
    pub keyframe_secs: f32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            codec: Codec::Av1,
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            rate_control: RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 },
            preset: Preset::Balanced,
            keyframe_secs: 2.0,
        }
    }
}

impl EncoderConfig {
    /// Checks every value against the vendor-neutral limits.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.width == 0 || self.height == 0 {
            return Err(invalid("size", format!("{}x{} must be above zero", self.width, self.height)));
        }
        if self.fps_num == 0 || self.fps_den == 0 {
            return Err(invalid("frame rate", format!("{}/{} must be above zero", self.fps_num, self.fps_den)));
        }
        if !(0.5..=10.0).contains(&self.keyframe_secs) {
            return Err(invalid("keyframe_secs", format!("{} is outside 0.5 to 10 seconds", self.keyframe_secs)));
        }
        match self.rate_control {
            RateControl::ConstantQuality { quality, max_mbps } => {
                if !(1..=100).contains(&quality) {
                    return Err(invalid("quality", format!("{quality} is outside 1 to 100")));
                }
                check_mbps("max_mbps", max_mbps)?;
            }
            RateControl::ConstantQp { .. } => {}
            RateControl::Cbr { mbps } => check_mbps("mbps", mbps)?,
            RateControl::VbrPeak { target_mbps, max_mbps } => {
                check_mbps("target_mbps", target_mbps)?;
                check_mbps("max_mbps", max_mbps)?;
                if target_mbps > max_mbps {
                    return Err(invalid("target_mbps", format!("{target_mbps} is above max_mbps {max_mbps}")));
                }
            }
        }
        Ok(())
    }
}

/// Builds an Invalid error for one field.
fn invalid(field: &'static str, reason: String) -> ConfigError {
    ConfigError::Invalid { field, reason }
}

/// Checks that a bitrate is above zero and at most the global ceiling.
fn check_mbps(field: &'static str, mbps: f32) -> Result<(), ConfigError> {
    if mbps.is_finite() && mbps > 0.0 && mbps <= MAX_MBPS { Ok(()) } else { Err(invalid(field, format!("{mbps} is outside 0 (exclusive) to {MAX_MBPS} Mbps"))) }
}

/// A configuration that the vendor-neutral limits or an encoder's own option table reject.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ConfigError {
    /// A value is outside the vendor-neutral limits.
    #[error("invalid {field}: {reason}")]
    Invalid {
        /// The offending field.
        field: &'static str,
        /// What is wrong with it.
        reason: String,
    },
    /// The encoder has no option with the needed name.
    #[error("encoder {encoder} has no option {option}")]
    OptionMissing {
        /// The encoder name.
        encoder: String,
        /// The option that was looked up.
        option: String,
    },
    /// A value is outside what the encoder allows for an option.
    #[error("encoder {encoder} option {option}: {value} is outside the allowed {allowed}")]
    OutOfRange {
        /// The encoder name.
        encoder: String,
        /// The option the value was meant for.
        option: String,
        /// The requested value.
        value: String,
        /// The allowed range or values.
        allowed: String,
    },
}

/// Why an encoder could not be opened.
#[derive(Debug, thiserror::Error)]
pub enum EncoderError {
    /// FFmpeg or the hardware layer failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The configuration was rejected before opening.
    #[error(transparent)]
    Config(#[from] ConfigError),
}

/// One encoded packet straight from the encoder.
pub struct EncodedPacket {
    packet: Packet,
}

impl EncodedPacket {
    /// Wraps one received packet.
    pub(crate) fn new(packet: Packet) -> Self {
        Self { packet }
    }

    /// The presentation timestamp, in the encoder's time base.
    pub fn pts(&self) -> Option<i64> {
        self.packet.pts()
    }

    /// The decode timestamp, in the encoder's time base.
    pub fn dts(&self) -> Option<i64> {
        self.packet.dts()
    }

    /// Whether this packet starts a keyframe.
    pub fn is_key(&self) -> bool {
        self.packet.is_key()
    }

    /// The payload size in bytes.
    pub fn size(&self) -> usize {
        self.packet.size()
    }

    /// The payload bytes.
    pub fn data(&self) -> Option<&[u8]> {
        self.packet.data()
    }
}

/// One video encoder backend.
pub trait Encoder: Send {
    /// Feeds one hardware frame to the encoder; the frame is consumed.
    fn send(&mut self, frame: HwFrame) -> Result<(), MediaError>;
    /// Hands every ready packet to the callback and returns once the encoder has none left.
    fn receive(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError>;
    /// Signals end of stream and drains every remaining packet.
    fn finish(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError>;
    /// The packet time base, as (numerator, denominator).
    fn time_base(&self) -> (i32, i32);
    /// The encoder's stream parameters, for the muxer to build a stream from.
    fn parameters(&self) -> Parameters;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(rate_control: RateControl) -> EncoderConfig {
        EncoderConfig { rate_control, ..EncoderConfig::default() }
    }

    #[test]
    fn the_default_config_is_valid() {
        assert_eq!(EncoderConfig::default().validate(), Ok(()));
        assert_eq!(EncoderConfig::default().keyframe_secs, 2.0);
    }

    #[test]
    fn quality_must_be_1_to_100() {
        for quality in [1u8, 50, 100] {
            assert_eq!(with(RateControl::ConstantQuality { quality, max_mbps: 40.0 }).validate(), Ok(()));
        }
        for quality in [0u8, 101] {
            let err = with(RateControl::ConstantQuality { quality, max_mbps: 40.0 }).validate();
            assert!(matches!(err, Err(ConfigError::Invalid { field: "quality", .. })), "{err:?}");
        }
    }

    #[test]
    fn bitrates_are_above_zero_and_at_most_500_mbps() {
        for mbps in [0.1f32, 20.0, 500.0] {
            assert_eq!(with(RateControl::Cbr { mbps }).validate(), Ok(()));
        }
        for mbps in [0.0f32, -1.0, 500.1, f32::NAN, f32::INFINITY] {
            assert!(with(RateControl::Cbr { mbps }).validate().is_err(), "{mbps}");
            assert!(with(RateControl::ConstantQuality { quality: 70, max_mbps: mbps }).validate().is_err(), "{mbps}");
            assert!(with(RateControl::VbrPeak { target_mbps: mbps, max_mbps: 40.0 }).validate().is_err(), "{mbps}");
        }
    }

    #[test]
    fn vbr_target_may_not_exceed_max() {
        assert_eq!(with(RateControl::VbrPeak { target_mbps: 40.0, max_mbps: 40.0 }).validate(), Ok(()));
        let err = with(RateControl::VbrPeak { target_mbps: 41.0, max_mbps: 40.0 }).validate();
        assert!(matches!(err, Err(ConfigError::Invalid { field: "target_mbps", .. })), "{err:?}");
    }

    #[test]
    fn keyframe_interval_is_half_a_second_to_ten() {
        for keyframe_secs in [0.5f32, 2.0, 10.0] {
            assert_eq!(EncoderConfig { keyframe_secs, ..EncoderConfig::default() }.validate(), Ok(()));
        }
        for keyframe_secs in [0.4f32, 10.1, 0.0, f32::NAN] {
            let err = EncoderConfig { keyframe_secs, ..EncoderConfig::default() }.validate();
            assert!(matches!(err, Err(ConfigError::Invalid { field: "keyframe_secs", .. })), "{keyframe_secs}: {err:?}");
        }
    }

    #[test]
    fn constant_qp_has_no_vendor_neutral_limit() {
        assert_eq!(with(RateControl::ConstantQp { qp_i: 0, qp_p: 255 }).validate(), Ok(()));
    }

    #[test]
    fn size_and_frame_rate_must_be_nonzero() {
        assert!(EncoderConfig { width: 0, ..EncoderConfig::default() }.validate().is_err());
        assert!(EncoderConfig { fps_den: 0, ..EncoderConfig::default() }.validate().is_err());
    }
}
