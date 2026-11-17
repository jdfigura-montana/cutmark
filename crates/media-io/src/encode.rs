//! The Encoder trait and the vendor-neutral encoder configuration types.

use ffmpeg_next::codec::Parameters;
use ffmpeg_next::packet::Packet;

use crate::HwFrame;
use crate::MediaError;

/// The video codec to encode with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// AV1.
    Av1,
    /// HEVC, also known as H.265.
    Hevc,
    /// H.264, also known as AVC.
    H264,
}

/// The speed and quality balance preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Fastest, lowest quality.
    Speed,
    /// The middle ground.
    Balanced,
    /// Slowest, best quality.
    Quality,
}

/// Vendor-neutral rate control, mapped to each backend's own options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateControl {
    /// Constant quantisation parameters for I and P frames.
    Cqp {
        /// Quantisation parameter for I frames.
        qp_i: u8,
        /// Quantisation parameter for P frames.
        qp_p: u8,
    },
    /// Constant quality variable bitrate with a ceiling.
    Qvbr {
        /// Quality level, 1 to 100.
        quality: u8,
        /// Maximum bitrate in megabits per second.
        max_mbps: u32,
    },
    /// Constant bitrate.
    Cbr {
        /// Target bitrate in megabits per second.
        mbps: u32,
    },
    /// Variable bitrate with a peak ceiling.
    VbrPeak {
        /// Target bitrate in megabits per second.
        target_mbps: u32,
        /// Maximum bitrate in megabits per second.
        max_mbps: u32,
    },
}

/// Everything needed to open one encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub keyframe_seconds: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            codec: Codec::Av1,
            width: 1920,
            height: 1080,
            fps_num: 60,
            fps_den: 1,
            rate_control: RateControl::Qvbr { quality: 23, max_mbps: 20 },
            preset: Preset::Balanced,
            keyframe_seconds: 2,
        }
    }
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
