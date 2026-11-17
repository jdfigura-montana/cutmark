//! The software audio encoders: FFmpeg's native AAC and 24-bit PCM.
//!
//! Never Opus: the recorder spec fixes AAC-LC for compressed tracks and
//! pcm_s24le for lossless MKV and MOV takes.

use ffmpeg_next::ChannelLayout;
use ffmpeg_next::Rational;
use ffmpeg_next::codec::Parameters;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::codec::{Context, Flags};
use ffmpeg_next::ffi;
use ffmpeg_next::format::Sample;
use ffmpeg_next::frame;
use ffmpeg_next::packet::Packet;
use ffmpeg_next::util::format::sample::Type;

use crate::MediaError;
use crate::encode::EncodedPacket;

/// The audio codec to encode with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodec {
    /// AAC-LC at a fixed bitrate, through FFmpeg's native "aac" encoder.
    Aac {
        /// Target bitrate in kilobits per second.
        kbps: u32,
    },
    /// Uncompressed 24-bit little-endian PCM, through "pcm_s24le".
    Pcm24,
}

/// Feeds interleaved f32 samples into one opened audio encoder.
pub struct AudioEncoder {
    /// The opened FFmpeg audio encoder.
    encoder: encoder::Audio,
    /// The sample rate in Hz.
    sample_rate: u32,
    /// The channel count, used for the interleaved sample layout.
    channels: u16,
    /// The encoder's sample format.
    sample_format: Sample,
    /// The channel layout handed to the encoder.
    layout: ChannelLayout,
    /// The packet time base, as (numerator, denominator).
    time_base: (i32, i32),
    /// Samples per channel per encoded frame.
    frame_size: usize,
    /// Interleaved samples waiting for a full frame.
    pending: Vec<f32>,
    /// The next frame's pts, counted in samples.
    next_pts: i64,
    /// Whether end of stream has been sent.
    finished: bool,
}
impl AudioEncoder {
    /// Opens one audio encoder; the channel count picks the layout, so two
    /// channels give stereo.
    pub fn open(codec: AudioCodec, sample_rate: u32, channels: u16) -> Result<Self, MediaError> {
        let rate = i32::try_from(sample_rate).map_err(|_| MediaError::Null("audio sample rate out of range"))?;
        let (name, sample_format, bit_rate) = match codec {
            AudioCodec::Aac { kbps } => ("aac", Sample::F32(Type::Planar), Some(i64::from(kbps) * 1000)),
            AudioCodec::Pcm24 => ("pcm_s24le", Sample::I32(Type::Packed), None),
        };
        let found = encoder::find_by_name(name).ok_or_else(|| MediaError::EncoderMissing(name.to_string()))?;
        let mut context = Context::new_with_codec(found);
        // MKV and MP4 both want the extradata in the container, not in-band.
        context.set_flags(Flags::GLOBAL_HEADER);
        let mut builder = context.encoder().audio().map_err(MediaError::Init)?;
        builder.set_rate(rate);
        builder.set_format(sample_format);
        builder.set_channel_layout(ChannelLayout::default(i32::from(channels)));
        builder.set_time_base(Rational::new(1, rate));
        if let Some(bits_per_second) = bit_rate {
            builder.set_bit_rate(bits_per_second as usize);
        }
        let encoder = builder.open().map_err(MediaError::Init)?;
        // pcm_s24le reports no fixed frame size, so chunk it like AAC.
        let reported = encoder.frame_size() as usize;
        let frame_size = if reported == 0 { 1024 } else { reported };
        Ok(Self {
            encoder,
            sample_rate,
            channels,
            sample_format,
            layout: ChannelLayout::default(i32::from(channels)),
            time_base: (1, rate),
            frame_size,
            pending: Vec::with_capacity(frame_size * usize::from(channels)),
            next_pts: 0,
            finished: false,
        })
    }

    /// Buffers interleaved f32 samples and feeds full frames to the encoder.
    pub fn send(&mut self, interleaved: &[f32]) -> Result<(), MediaError> {
        if self.finished {
            return Err(MediaError::Null("audio encoder send after finish"));
        }
        self.pending.extend_from_slice(interleaved);
        let frame_len = self.frame_size * usize::from(self.channels);
        while self.pending.len() >= frame_len {
            let chunk: Vec<f32> = self.pending.drain(..frame_len).collect();
            self.encode_frame(&chunk)?;
        }
        Ok(())
    }

    /// Hands every ready packet to the callback and returns once the encoder has none left.
    pub fn receive(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError> {
        self.drain(out, false)
    }

    /// Pads the last partial frame with silence, sends end of stream and drains.
    pub fn finish(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError> {
        if self.finished {
            return Err(MediaError::Null("audio encoder finished twice"));
        }
        self.finished = true;
        let frame_len = self.frame_size * usize::from(self.channels);
        if !self.pending.is_empty() {
            self.pending.resize(frame_len, 0.0);
            let chunk = std::mem::take(&mut self.pending);
            self.encode_frame(&chunk)?;
        }
        self.encoder.send_eof().map_err(MediaError::Init)?;
        self.drain(out, true)
    }

    /// The packet time base, as (numerator, denominator).
    pub fn time_base(&self) -> (i32, i32) {
        self.time_base
    }

    /// The encoder's stream parameters, for the muxer to build a stream from.
    pub fn parameters(&self) -> Parameters {
        Parameters::from(&self.encoder)
    }

    /// Converts one full chunk of interleaved f32 into the encoder's format
    /// and sends it, with pts counted in samples.
    fn encode_frame(&mut self, chunk: &[f32]) -> Result<(), MediaError> {
        let mut frame = frame::Audio::new(self.sample_format, self.frame_size, self.layout);
        frame.set_rate(self.sample_rate);
        frame.set_pts(Some(self.next_pts));
        self.next_pts += self.frame_size as i64;
        match self.sample_format {
            Sample::F32(Type::Planar) => {
                let left = frame.plane_mut::<f32>(0);
                for (dst, src) in left.iter_mut().zip(chunk.as_chunks::<2>().0) {
                    *dst = src[0];
                }
                let right = frame.plane_mut::<f32>(1);
                for (dst, src) in right.iter_mut().zip(chunk.as_chunks::<2>().0) {
                    *dst = src[1];
                }
            }
            Sample::I32(Type::Packed) => {
                let (packed, _) = frame.plane_mut::<i32>(0).as_chunks_mut::<2>();
                for (dst, src) in packed.iter_mut().zip(chunk.as_chunks::<2>().0) {
                    dst[0] = f32_to_s32(src[0]);
                    dst[1] = f32_to_s32(src[1]);
                }
            }
            _ => return Err(MediaError::Null("audio encoder sample format")),
        }
        self.encoder.send_frame(&frame).map_err(MediaError::Init)
    }

    /// Receives packets until the encoder reports EAGAIN, and end of stream
    /// while draining.
    fn drain(&mut self, out: &mut dyn FnMut(EncodedPacket), allow_eof: bool) -> Result<(), MediaError> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => out(EncodedPacket::new(packet)),
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => return Ok(()),
                Err(ffmpeg_next::Error::Eof) if allow_eof => return Ok(()),
                Err(err) => return Err(MediaError::Init(err)),
            }
        }
    }
}

/// Converts one f32 sample to s32 with the value in the top bits, the form
/// pcm_s24le reads.
fn f32_to_s32(sample: f32) -> i32 {
    (sample.clamp(-1.0, 1.0) * 2_147_483_647.0) as i32
}
