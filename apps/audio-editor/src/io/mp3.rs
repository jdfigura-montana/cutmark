//! io::mp3: streaming MP3 export sink through FFmpeg's libmp3lame encoder, CBR only.
//!
//! Audio arrives at the project rate. 88.2 and 96 kHz are resampled to 44.1 and
//! 48 kHz on the fly (rubato), so nothing is buffered beyond one chunk. FFmpeg
//! (LGPL DLLs) receives PCM at the final rate, and its mp3 muxer overwrites the
//! first frame with the Xing/Info tag so decoders can trim delay and padding.

use std::path::Path;

use ffmpeg_next::format::flag::Flags as FormatFlags;
use ffmpeg_next::format::{Sample, sample::Type};
use ffmpeg_next::{Rational, channel_layout, codec, frame, log, packet};

use super::IoError;
use super::export::{AudioSink, ExportFormat, PartGuard, check_whole_frames, mp3_rate};
use super::resample::StreamResampler;

/// The muxer format forced for `.part` files, which have no known extension.
const MP3_MUXER: &str = "mp3";

/// Clamps a sample to +/-1.0 and maps non-finite values to silence.
fn clean(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// The mp3 muxer output plus the libmp3lame encoder and its reusable buffers.
struct Mp3 {
    channels: usize,
    /// Samples per encoder frame (1152 per channel for libmp3lame).
    frame_size: usize,
    /// pts of the next input frame, in samples at the output rate.
    pts: i64,
    /// Stream time base after `write_header` (the muxer may change it).
    stream_tb: Rational,
    /// Reused encoder input frame (fltp, `frame_size` samples).
    frame: frame::Audio,
    /// Reused output packet.
    pkt: packet::Packet,
    /// Interleaved samples carried over between writes, always shorter than one
    /// encoder frame after `encode` returns.
    pending: Vec<f32>,
    encoder: codec::encoder::Audio,
    // Declared last so the muxer (and its file handle) closes after the encoder.
    octx: ffmpeg_next::format::context::Output,
}

impl Mp3 {
    /// CBR libmp3lame encoder for `part` at `rate` Hz (input and output rate are
    /// the same), best quality, mono or joint stereo.
    fn new(part: &Path, channels: u16, rate: u32, kbps: u32) -> Result<Self, IoError> {
        if !matches!(kbps, 128 | 192 | 256 | 320) {
            return Err(IoError::Export(format!("unsupported MP3 bitrate: {kbps} kbps")));
        }
        ffmpeg_next::init().map_err(|e| IoError::Export(format!("MP3 encoder setup failed: {e}")))?;
        log::set_level(log::Level::Error);
        let setup = |e| IoError::Export(format!("MP3 encoder setup failed: {e}"));
        let mp3_codec = codec::encoder::find_by_name("libmp3lame")
            .ok_or_else(|| IoError::Export("libmp3lame is missing from the FFmpeg build".to_string()))?
            .audio()
            .map_err(setup)?;
        let layout =
            if channels == 1 { channel_layout::ChannelLayout::MONO } else { channel_layout::ChannelLayout::STEREO };

        // `.part` is not a known extension, so the mp3 muxer is forced. This
        // creates the file; the sink's guard deletes it when the sink goes away.
        let mut octx = ffmpeg_next::format::output_as(part, MP3_MUXER)
            .map_err(|e| IoError::Export(format!("could not create the MP3 output: {e}")))?;
        let global_header = octx.format().flags().contains(FormatFlags::GLOBAL_HEADER);
        let mut stream = octx.add_stream(mp3_codec).map_err(setup)?;
        stream.set_time_base((1, rate as i32));

        let ctx = codec::context::Context::from_parameters(stream.parameters()).map_err(setup)?;
        let mut encoder = ctx.encoder().audio().map_err(setup)?;
        if global_header {
            encoder.set_flags(codec::flag::Flags::GLOBAL_HEADER);
        }
        encoder.set_rate(rate as i32);
        encoder.set_channel_layout(layout);
        encoder.set_format(Sample::F32(Type::Planar));
        encoder.set_bit_rate(kbps as usize * 1000);
        encoder.set_time_base((1, rate as i32));

        // LAME quality "best" is compression level 0. No VBR options are set,
        // so libmp3lame encodes CBR at the requested bit rate.
        let mut options = ffmpeg_next::Dictionary::new();
        options.set("compression_level", "0");
        let encoder = encoder
            .open_as_with(mp3_codec, options)
            .map_err(|e| IoError::Export(format!("could not open the MP3 encoder: {e}")))?;
        stream.set_parameters(&encoder);
        // The muxer would otherwise prepend an ID3v2 tag; the export is an MP3
        // that starts at its first frame, like the old LAME output was.
        let mut muxer_options = ffmpeg_next::Dictionary::new();
        muxer_options.set("id3v2_version", "0");
        octx.write_header_with(muxer_options)
            .map_err(|e| IoError::Export(format!("could not write the MP3 header: {e}")))?;
        // The muxer may pick its own time base in write_header; packets must be
        // rescaled to whatever the stream ended up with.
        let stream_tb = octx.stream(0).map_or((1, rate as i32).into(), |s| s.time_base());
        let frame_size = usize::try_from(encoder.frame_size())
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| IoError::Export("MP3 encoder reported no frame size".to_string()))?;
        let mut frame = frame::Audio::new(Sample::F32(Type::Planar), frame_size, layout);
        frame.set_rate(rate);
        Ok(Self {
            channels: usize::from(channels),
            frame_size,
            pts: 0,
            stream_tb,
            frame,
            pkt: packet::Packet::empty(),
            pending: Vec::new(),
            encoder,
            octx,
        })
    }

    /// Writes one drained packet into the muxer on stream 0.
    fn write_packet(&mut self) -> Result<(), IoError> {
        self.pkt.set_stream(0);
        self.pkt.rescale_ts((1, self.encoder.rate() as i32), self.stream_tb);
        self.pkt
            .write_interleaved(&mut self.octx)
            .map_err(|e| IoError::Export(format!("MP3 write failed: {e}")))
    }

    /// Drains every packet the encoder currently has into the muxer.
    fn drain_packets(&mut self) -> Result<(), IoError> {
        while self.encoder.receive_packet(&mut self.pkt).is_ok() {
            self.write_packet()?;
        }
        Ok(())
    }

    /// Fills the frame planes from an interleaved chunk, encodes it and drains
    /// the muxer. `block` holds whole frames.
    fn send_frame_chunk(&mut self, block: &[f32]) -> Result<(), IoError> {
        let ch = self.channels;
        let frames = block.len() / ch;
        self.frame.set_samples(frames);
        self.frame.set_pts(Some(self.pts));
        self.pts += frames as i64;
        if ch == 1 {
            let plane = self.frame.plane_mut::<f32>(0);
            for (dst, &src) in plane.iter_mut().zip(block) {
                *dst = clean(src);
            }
        } else {
            {
                let left = self.frame.plane_mut::<f32>(0);
                for (dst, &src) in left.iter_mut().zip(block.iter().step_by(2)) {
                    *dst = clean(src);
                }
            }
            let right = self.frame.plane_mut::<f32>(1);
            for (dst, &src) in right.iter_mut().zip(block.iter().skip(1).step_by(2)) {
                *dst = clean(src);
            }
        }
        self.encoder
            .send_frame(&self.frame)
            .map_err(|e| IoError::Export(format!("MP3 encode failed: {e}")))?;
        self.drain_packets()
    }

    /// Encodes whole interleaved frames into the muxer.
    ///
    /// The encoder takes full `frame_size` frames plus exactly one short frame
    /// at the end of the stream, so a remainder that is not a whole encoder
    /// frame is carried over to the next call instead of being sent early.
    fn encode(&mut self, samples: &[f32]) -> Result<(), IoError> {
        let want = self.frame_size * self.channels;
        let mut chunk = std::mem::take(&mut self.pending);
        chunk.extend_from_slice(samples);
        while chunk.len() >= want {
            self.send_frame_chunk(&chunk[..want])?;
            chunk.drain(..want);
        }
        self.pending = chunk;
        Ok(())
    }

    /// Sends the carried-over remainder, which is shorter than one encoder
    /// frame. Must only run once, right before the end of stream.
    fn flush_tail(&mut self) -> Result<(), IoError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let tail = std::mem::take(&mut self.pending);
        self.send_frame_chunk(&tail)
    }

    /// Flushes the encoder (padding the last short frame), writes the trailer
    /// (which finalizes the Xing/Info tag) and closes the output file.
    fn finish(mut self) -> Result<(), IoError> {
        self.flush_tail()?;
        self.encoder
            .send_eof()
            .map_err(|e| IoError::Export(format!("MP3 flush failed: {e}")))?;
        self.drain_packets()?;
        self.octx
            .write_trailer()
            .map_err(|e| IoError::Export(format!("could not write the MP3 trailer: {e}")))
    }
}

/// Streaming MP3 export sink.
struct Mp3Sink {
    mp3: Mp3,
    channels: usize,
    /// Present when the project rate is not an MP3 rate.
    resampler: Option<StreamResampler>,
    /// Resampler output for the current `write`, reused between calls.
    resampled: Vec<f32>,
    // Declared before `guard` so the muxer's file handle is closed (fields drop
    // in order) before the guard deletes the `.part` file.
    guard: PartGuard,
}

/// Opens an MP3 sink writing `<path>.part`; used by `export::open_sink`.
pub(super) fn open_sink(path: &Path, kbps: u32, format: &ExportFormat) -> Result<Box<dyn AudioSink>, IoError> {
    let out_rate = mp3_rate(format.in_rate);
    // Created before the muxer opens the `.part` file, so a failure here leaves
    // nothing on disk.
    let resampler =
        if out_rate == format.in_rate { None } else { Some(StreamResampler::new(format.channels, format.in_rate, out_rate)?) };
    let guard = PartGuard::new(path);
    let mp3 = Mp3::new(guard.part_path(), format.channels, out_rate, kbps)?;
    let channels = usize::from(format.channels);
    Ok(Box::new(Mp3Sink { mp3, channels, resampler, resampled: Vec::new(), guard }))
}

impl AudioSink for Mp3Sink {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), IoError> {
        check_whole_frames(interleaved.len(), self.channels)?;
        match self.resampler.as_mut() {
            None => self.mp3.encode(interleaved),
            Some(resampler) => {
                self.resampled.clear();
                resampler.push(interleaved, &mut self.resampled)?;
                self.mp3.encode(&self.resampled)
            }
        }
    }

    fn finish(self: Box<Self>) -> Result<(), IoError> {
        let Mp3Sink { mut mp3, mut resampler, mut resampled, guard, .. } = *self;
        if let Some(resampler) = resampler.as_mut() {
            resampled.clear();
            resampler.finish(&mut resampled)?;
            mp3.encode(&resampled)?;
        }
        // `finish` drops the muxer, which closes the `.part` file, so the
        // rename below works on Windows.
        mp3.finish()?;
        drop(resampler);
        drop(resampled);
        guard.commit()
    }

    fn abort(self: Box<Self>) {
        // Dropping closes the FFmpeg output first, then the guard deletes the
        // `.part` file.
        drop(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::decode::decode_file;
    use crate::io::export::{ExportKind, open_sink as open_export_sink};

    /// Writes `seconds` of a 0.5-amplitude 440 Hz sine (same in every channel)
    /// through an MP3 sink in odd-sized blocks and returns the decoded file.
    fn export_sine(rate: u32, channels: u16, kbps: u32, seconds: u32) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp3");
        let format = ExportFormat { kind: ExportKind::Mp3 { kbps }, channels, in_rate: rate };
        let mut sink = open_export_sink(&path, &format).unwrap();
        let total = (rate * seconds) as usize;
        let sine: Vec<f32> = (0..total)
            .flat_map(|i| {
                let phase = std::f64::consts::TAU * 440.0 * i as f64 / f64::from(rate);
                let s = (0.5 * phase.sin()) as f32;
                std::iter::repeat_n(s, usize::from(channels))
            })
            .collect();
        for block in sine.chunks(1000 * usize::from(channels)) {
            sink.write(block).unwrap();
        }
        sink.finish().unwrap();
        (dir, path)
    }

    /// Counts sign changes in channel 0 of an interleaved buffer.
    fn zero_crossings(samples: &[f32], channels: usize) -> usize {
        let left: Vec<f32> = samples.iter().step_by(channels).copied().collect();
        left.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count()
    }

    /// Peak of channel 0 away from the first and last 2,000 frames.
    fn middle_peak(samples: &[f32], channels: usize) -> f32 {
        let frames = samples.len() / channels;
        (2000..frames - 2000).map(|i| samples[i * channels].abs()).fold(0.0, f32::max)
    }

    #[test]
    fn stereo_48k_sine_decodes_with_exact_length() {
        let (_dir, path) = export_sine(48_000, 2, 192, 1);
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert_eq!(decoded.rate, 48_000);
        assert_eq!(decoded.channels, 2);
        let frames = decoded.samples.len() / 2;
        // LAME writes the Info tag with encoder delay and padding and the
        // decoder trims them, so the length is exact, not just roughly 1 s.
        assert!((47_998..=48_002).contains(&frames), "decoded {frames} frames");

        // The signal is the right level and the right pitch.
        let peak = middle_peak(&decoded.samples, 2);
        assert!((0.4..=0.6).contains(&peak), "peak was {peak}");
        let crossings = zero_crossings(&decoded.samples, 2);
        assert!((870..=890).contains(&crossings), "crossings: {crossings}");
    }

    #[test]
    fn file_starts_with_a_lame_info_tag() {
        let (_dir, path) = export_sine(48_000, 2, 192, 1);
        let bytes = std::fs::read(&path).unwrap();
        // FFmpeg's muxer can write an ID3v2 tag first; skip it to the first
        // frame. (None is written today, but stay robust either way.)
        let start = if bytes.starts_with(b"ID3") {
            let size = ((usize::from(bytes[6]) & 0x7F) << 21)
                | ((usize::from(bytes[7]) & 0x7F) << 14)
                | ((usize::from(bytes[8]) & 0x7F) << 7)
                | (usize::from(bytes[9]) & 0x7F);
            10 + size
        } else {
            0
        };
        // MPEG sync at the first frame, then the CBR Xing/Info tag. FFmpeg
        // stamps its own encoder string there, not LAME's.
        assert_eq!(bytes[start], 0xFF);
        assert_eq!(bytes[start + 1] & 0xE0, 0xE0);
        let head = &bytes[start..start + 200];
        let tag = head.windows(4).position(|w| w == b"Info" || w == b"Xing").unwrap();
        assert_eq!(tag, 36, "Xing/Info tag sits right after the stereo side info");
    }

    #[test]
    fn stereo_96k_input_decodes_at_48k_with_exact_length() {
        let (_dir, path) = export_sine(96_000, 2, 192, 1);
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert_eq!(decoded.rate, 48_000);
        assert_eq!(decoded.channels, 2);
        let frames = decoded.samples.len() / 2;
        assert!((47_998..=48_002).contains(&frames), "decoded {frames} frames");

        // Still 440 Hz at the full level after resampling.
        let peak = middle_peak(&decoded.samples, 2);
        assert!((0.4..=0.6).contains(&peak), "peak was {peak}");
        let crossings = zero_crossings(&decoded.samples, 2);
        assert!((870..=890).contains(&crossings), "crossings: {crossings}");
    }

    #[test]
    fn stereo_88k_input_decodes_at_44k() {
        let (_dir, path) = export_sine(88_200, 2, 128, 1);
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert_eq!(decoded.rate, 44_100);
        let frames = decoded.samples.len() / 2;
        assert!((44_098..=44_102).contains(&frames), "decoded {frames} frames");
    }

    #[test]
    fn mono_decodes_with_one_channel() {
        let (_dir, path) = export_sine(48_000, 1, 128, 1);
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert_eq!(decoded.rate, 48_000);
        assert_eq!(decoded.channels, 1);
        assert!((47_998..=48_002).contains(&decoded.samples.len()), "decoded {} frames", decoded.samples.len());
        let peak = middle_peak(&decoded.samples, 1);
        assert!((0.4..=0.6).contains(&peak), "peak was {peak}");
    }

    #[test]
    fn every_bitrate_encodes() {
        for kbps in [128, 192, 256, 320] {
            let (_dir, path) = export_sine(44_100, 2, kbps, 1);
            let decoded = decode_file(&path, &mut |_| {}).unwrap();
            assert_eq!(decoded.rate, 44_100, "{kbps} kbps");
            assert_eq!(decoded.channels, 2, "{kbps} kbps");
        }
    }

    #[test]
    fn overs_and_nan_do_not_break_the_stream() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp3");
        let format = ExportFormat { kind: ExportKind::Mp3 { kbps: 192 }, channels: 2, in_rate: 48_000 };
        let mut sink = open_export_sink(&path, &format).unwrap();
        let mut block = vec![0.0_f32; 9600];
        block[10] = 4.0;
        block[11] = f32::NAN;
        block[12] = f32::INFINITY;
        sink.write(&block).unwrap();
        sink.finish().unwrap();
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert!(decoded.samples.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn empty_export_still_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.mp3");
        let format = ExportFormat { kind: ExportKind::Mp3 { kbps: 192 }, channels: 2, in_rate: 96_000 };
        open_export_sink(&path, &format).unwrap().finish().unwrap();
        assert!(path.is_file());
        assert!(!dir.path().join("out.mp3.part").exists());
    }

    #[test]
    fn ffmpeg_loads_and_has_libmp3lame() {
        ffmpeg_next::init().expect("ffmpeg init");
        assert!(ffmpeg_next::encoder::find_by_name("libmp3lame").is_some(), "libmp3lame encoder missing from FFmpeg build");
    }
}
