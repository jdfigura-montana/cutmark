//! The MKV muxer: copies encoder packets into a Matroska file that stays
//! playable if the process dies mid-take.

use std::path::Path;

use ffmpeg_next::Rational;
use ffmpeg_next::Rescale;
use ffmpeg_next::codec::Parameters;
use ffmpeg_next::packet::{Flags, Packet};

use crate::MediaError;
use crate::encode::EncodedPacket;

/// The stream time base for Matroska tracks: milliseconds.
const MKV_TIME_BASE: Rational = Rational(1, 1000);

/// The output container to write. MP4 and MOV arrive in phase 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Matroska, via FFmpeg's matroska muxer.
    Mkv,
}

/// The timing facts of one added stream, kept for rescaling on write.
struct StreamInfo {
    /// The time base the encoder stamps its packets in.
    encoder_time_base: Rational,
    /// One frame duration expressed as a rescale source unit.
    frame_step: Rational,
}

/// Writes encoded packets into a Matroska file and flushes on keyframes.
pub struct Muxer {
    ctx: ffmpeg_next::format::context::Output,
    streams: Vec<StreamInfo>,
    started: bool,
}

impl Muxer {
    /// Opens the output file with the muxer the container names.
    pub fn create(path: &Path, container: Container) -> Result<Self, MediaError> {
        let ctx = match container {
            Container::Mkv => ffmpeg_next::format::output_as(path, "matroska")?,
        };
        Ok(Self { ctx, streams: Vec::new(), started: false })
    }

    /// Adds one video stream that copies the encoder's codec parameters,
    /// including extradata, and returns the stream index.
    pub fn add_video(&mut self, params: Parameters, encoder_time_base: (i32, i32), fps: (i32, i32)) -> Result<usize, MediaError> {
        let encoder_time_base = Rational::new(encoder_time_base.0, encoder_time_base.1);
        let frame_step = Rational::new(fps.1, fps.0);
        let codec_id = params.id();
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(MKV_TIME_BASE);
            stream.set_rate(Rational::new(fps.0, fps.1));
            stream.set_avg_frame_rate(Rational::new(fps.0, fps.1));
            stream.set_parameters(params);
            stream.index()
        };
        self.streams.push(StreamInfo { encoder_time_base, frame_step });
        Ok(index)
    }

    /// Writes the container header; call once after all streams are added.
    pub fn start(&mut self) -> Result<(), MediaError> {
        self.ctx.write_header()?;
        self.started = true;
        Ok(())
    }

    /// Rescales one packet's timestamps into the stream's time base and
    /// interleaved-writes it, flushing the output when it is a keyframe.
    pub fn write(&mut self, stream: usize, packet: &EncodedPacket) -> Result<(), MediaError> {
        let Self { ctx, streams, started } = self;
        if !*started {
            return Err(MediaError::Null("muxer write before start"));
        }
        let Some(info) = streams.get(stream) else {
            return Err(MediaError::Null("muxer write stream index"));
        };
        let Some(data) = packet.data() else {
            return Ok(());
        };
        let stream_tb = ctx.stream(stream).map(|s| s.time_base()).ok_or(MediaError::Null("muxer write stream"))?;
        let is_key = packet.is_key();
        let mut out = Packet::copy(data);
        out.set_stream(stream);
        out.set_pts(packet.pts().map(|pts| pts.rescale(info.encoder_time_base, stream_tb)));
        out.set_dts(packet.dts().map(|dts| dts.rescale(info.encoder_time_base, stream_tb)));
        out.set_duration(1i64.rescale(info.frame_step, stream_tb));
        if is_key {
            out.set_flags(Flags::KEY);
        }
        out.write_interleaved(ctx)?;
        if is_key {
            flush_output(ctx);
        }
        Ok(())
    }

    /// Writes the trailer, which for Matroska means the cues and the duration.
    pub fn finish(mut self) -> Result<(), MediaError> {
        self.ctx.write_trailer()?;
        Ok(())
    }
}

/// Forces the AVIO layer out to the file so a killed process loses at most
/// the data after the last keyframe.
#[allow(unsafe_code)]
fn flush_output(ctx: &mut ffmpeg_next::format::context::Output) {
    // SAFETY: ctx is a live output context, so its pb field points at the
    // AVIOContext that owns the open file; avio_flush only flushes buffers.
    #[allow(unsafe_code)]
    unsafe {
        let ptr = ctx.as_mut_ptr();
        ffmpeg_next::ffi::avio_flush((*ptr).pb);
    }
}
