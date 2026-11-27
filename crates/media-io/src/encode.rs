//! The Encoder trait and the vendor-neutral encoder configuration types.

use ffmpeg_next::codec::Parameters;
use ffmpeg_next::packet::Packet;
use serde::{Deserialize, Serialize};

pub use crate::amf::{CodecSupport, NumericOption, OptionRanges, aligned_1080_value, amf_options, probe_amf};

use crate::HwFrame;
use crate::MediaError;
use crate::audio::AudioCodec;

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

/// The file container to write.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Container {
    /// Matroska, via FFmpeg's matroska muxer.
    #[default]
    Mkv,
    /// MP4, written fragmented so a crash leaves a playable file.
    Mp4,
    /// QuickTime MOV, written fragmented.
    Mov,
}

impl Container {
    /// The file extension for this container, without the dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
        }
    }

    /// The container a file extension names, ignoring case and a leading dot, or none for any other extension.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "mkv" => Some(Self::Mkv),
            "mp4" => Some(Self::Mp4),
            "mov" => Some(Self::Mov),
            _ => None,
        }
    }

    /// Whether a clean stop must rewrite the file: MP4 and MOV are written fragmented, MKV is final as written.
    pub fn needs_finalize(self) -> bool {
        matches!(self, Self::Mp4 | Self::Mov)
    }
}

/// Why a container, codec and audio codec cannot be combined.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComboError {
    /// MOV was asked to hold a codec it does not take.
    #[error("MOV takes HEVC and H.264 only. Use MKV for AV1, or pick HEVC or H.264")]
    MovCodec,
    /// AV1 in MP4 needs a height that is a multiple of 16, unless the GPU codes 1080 lines exactly.
    #[error("AV1 at {height}p needs MKV. Use MKV, or HEVC for MP4")]
    Av1Mp4Height {
        /// The output height in pixels.
        height: u32,
    },
    /// PCM audio was asked for in a container that cannot hold it.
    #[error("PCM audio needs MKV or MOV. Use AAC for MP4")]
    PcmContainer,
}

/// Checks that the container can hold the video and audio codecs at this height, using what the encoder probe
/// found out about exact 1080 line coding on this GPU (no probe yet counts as not found).
pub fn check_combo(container: Container, codec: Codec, height: u32, audio_codec: AudioCodec) -> Result<(), ComboError> {
    check_combo_with(container, codec, height, audio_codec, aligned_1080_value().is_some())
}

/// Checks that the container can hold the video and audio codecs at this height.
///
/// `aligned_1080` is what the encoder probe found: the GPU can code AV1 at exactly 1080 lines, so AV1 in MP4 is
/// fine at that height. Any other height that is not a multiple of 16 stays refused.
pub fn check_combo_with(container: Container, codec: Codec, height: u32, audio_codec: AudioCodec, aligned_1080: bool) -> Result<(), ComboError> {
    if container == Container::Mov && codec == Codec::Av1 {
        return Err(ComboError::MovCodec);
    }
    let exact_1080 = aligned_1080 && height == 1080;
    if container == Container::Mp4 && codec == Codec::Av1 && !height.is_multiple_of(16) && !exact_1080 {
        return Err(ComboError::Av1Mp4Height { height });
    }
    if container == Container::Mp4 && audio_codec == AudioCodec::Pcm24 {
        return Err(ComboError::PcmContainer);
    }
    Ok(())
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

    /// Wraps a hand-made packet so tests in other crates can feed the muxer without an encoder.
    #[doc(hidden)]
    pub fn from_raw_packet(packet: Packet) -> Self {
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
    /// Makes the next frame sent come out as a keyframe: an IDR frame for HEVC and H.264, a key frame for AV1.
    fn force_keyframe(&mut self);
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

    /// AAC at the suite's usual bitrate.
    const AAC: AudioCodec = AudioCodec::Aac { kbps: 320 };

    #[test]
    fn the_container_defaults_to_mkv_and_names_its_extension() {
        assert_eq!(Container::default(), Container::Mkv);
        assert_eq!(Container::Mkv.extension(), "mkv");
        assert_eq!(Container::Mp4.extension(), "mp4");
        assert_eq!(Container::Mov.extension(), "mov");
    }

    #[test]
    fn a_container_is_found_from_its_extension_and_only_mp4_and_mov_need_finalizing() {
        assert_eq!(Container::from_extension("MKV"), Some(Container::Mkv));
        assert_eq!(Container::from_extension(".mp4"), Some(Container::Mp4));
        assert_eq!(Container::from_extension("mov"), Some(Container::Mov));
        assert_eq!(Container::from_extension("wav"), None);
        assert!(!Container::Mkv.needs_finalize());
        assert!(Container::Mp4.needs_finalize() && Container::Mov.needs_finalize());
    }

    #[test]
    fn mov_takes_hevc_and_h264_only() {
        assert_eq!(check_combo_with(Container::Mov, Codec::Hevc, 1080, AAC, false), Ok(()));
        assert_eq!(check_combo_with(Container::Mov, Codec::H264, 1080, AAC, false), Ok(()));
        assert_eq!(check_combo_with(Container::Mov, Codec::Av1, 1088, AAC, false), Err(ComboError::MovCodec));
    }

    #[test]
    fn av1_in_mp4_needs_a_height_that_is_a_multiple_of_16() {
        assert_eq!(check_combo_with(Container::Mp4, Codec::Av1, 1440, AAC, false), Ok(()));
        assert_eq!(check_combo_with(Container::Mp4, Codec::Av1, 1088, AAC, false), Ok(()));
        let err = check_combo_with(Container::Mp4, Codec::Av1, 1080, AAC, false);
        assert_eq!(err, Err(ComboError::Av1Mp4Height { height: 1080 }));
        assert_eq!(err.map_err(|e| e.to_string()), Err("AV1 at 1080p needs MKV. Use MKV, or HEVC for MP4".to_owned()));
        assert_eq!(check_combo_with(Container::Mp4, Codec::Hevc, 1080, AAC, false), Ok(()));
    }

    #[test]
    fn av1_in_mp4_at_1080_is_allowed_when_the_probe_found_exact_1080_coding() {
        assert_eq!(check_combo_with(Container::Mp4, Codec::Av1, 1080, AAC, true), Ok(()));
        assert_eq!(check_combo_with(Container::Mp4, Codec::Av1, 1080, AAC, false), Err(ComboError::Av1Mp4Height { height: 1080 }));
        // The probe only vouches for 1080 lines, so another odd height stays refused.
        assert_eq!(check_combo_with(Container::Mp4, Codec::Av1, 900, AAC, true), Err(ComboError::Av1Mp4Height { height: 900 }));
        assert_eq!(check_combo_with(Container::Mov, Codec::Av1, 1080, AAC, true), Err(ComboError::MovCodec));
    }

    #[test]
    fn mkv_takes_every_video_codec_at_any_height() {
        for codec in [Codec::Av1, Codec::Hevc, Codec::H264] {
            assert_eq!(check_combo_with(Container::Mkv, codec, 1080, AAC, false), Ok(()));
        }
    }

    #[test]
    fn pcm_audio_is_allowed_in_mkv_and_mov_only() {
        assert_eq!(check_combo_with(Container::Mkv, Codec::Hevc, 1080, AudioCodec::Pcm24, false), Ok(()));
        assert_eq!(check_combo_with(Container::Mov, Codec::Hevc, 1080, AudioCodec::Pcm24, false), Ok(()));
        assert_eq!(check_combo_with(Container::Mp4, Codec::Hevc, 1080, AudioCodec::Pcm24, false), Err(ComboError::PcmContainer));
        assert_eq!(check_combo_with(Container::Mp4, Codec::Hevc, 1080, AAC, false), Ok(()));
    }
}
