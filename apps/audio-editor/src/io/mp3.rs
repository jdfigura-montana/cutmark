//! io::mp3: streaming MP3 export sink via mp3lame-encoder, CBR only.
//!
//! Audio arrives at the project rate. 88.2 and 96 kHz are resampled to 44.1 and
//! 48 kHz on the fly (rubato), so nothing is buffered beyond one chunk. When the
//! stream ends, the LAME/Info tag frame is written over the placeholder frame at
//! the start of the file so decoders can trim the encoder delay and padding.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::num::NonZeroU32;
use std::path::Path;

use mp3lame_encoder::{Bitrate, Builder, Encoder, FlushGap, InterleavedPcm, Mode, MonoPcm, Quality, VbrMode, max_required_buffer_size};

use super::IoError;
use super::export::{AudioSink, ExportFormat, PartGuard, check_whole_frames, mp3_rate};
use super::resample::StreamResampler;

/// Frames handed to LAME per call, which bounds the output buffer size.
const ENCODE_FRAMES: usize = 8192;
/// Write buffer for the .part file.
const FILE_BUFFER: usize = 64 * 1024;

/// A LAME encoder plus the reusable buffers it needs.
struct Lame {
    encoder: Encoder,
    channels: usize,
    /// Clamped copy of the current block (LAME wants +/-1.0 full scale).
    scratch: Vec<f32>,
    /// Encoded bytes for the current block.
    out: Vec<u8>,
}

impl Lame {
    /// CBR encoder at `rate` Hz (input and output rate are the same), best quality.
    fn new(channels: u16, rate: u32, kbps: u32) -> Result<Self, IoError> {
        let bitrate = match kbps {
            128 => Bitrate::Kbps128,
            192 => Bitrate::Kbps192,
            256 => Bitrate::Kbps256,
            320 => Bitrate::Kbps320,
            other => {
                return Err(IoError::Export(format!("unsupported MP3 bitrate: {other} kbps")));
            }
        };
        let mode = if channels == 1 { Mode::Mono } else { Mode::JointStereo };
        let build = |e| IoError::Export(format!("MP3 encoder setup failed: {e}"));
        let encoder = Builder::new()
            .ok_or_else(|| IoError::Export("could not create the MP3 encoder".to_string()))?
            .with_num_channels(channels as u8)
            .map_err(build)?
            .with_sample_rate(rate)
            .map_err(build)?
            .with_output_sample_rate(NonZeroU32::new(rate))
            .map_err(build)?
            .with_mode(mode)
            .map_err(build)?
            .with_vbr_mode(VbrMode::Off)
            .map_err(build)?
            .with_brate(bitrate)
            .map_err(build)?
            .with_quality(Quality::Best)
            .map_err(build)?
            .with_to_write_vbr_tag(true)
            .map_err(build)?
            .build()
            .map_err(build)?;
        let ch = usize::from(channels);
        Ok(Self { encoder, channels: ch, scratch: Vec::with_capacity(ENCODE_FRAMES * ch), out: Vec::with_capacity(max_required_buffer_size(ENCODE_FRAMES)) })
    }

    /// Encodes whole interleaved frames and writes the output to `file`.
    fn encode(&mut self, samples: &[f32], file: &mut impl Write) -> Result<(), IoError> {
        for block in samples.chunks(ENCODE_FRAMES * self.channels) {
            // Overs would only distort inside the encoder; NaN must never reach it.
            self.scratch.clear();
            self.scratch.extend(block.iter().map(|&s| if s.is_finite() { s.clamp(-1.0, 1.0) } else { 0.0 }));
            self.out.clear();
            self.out.reserve(max_required_buffer_size(block.len() / self.channels));
            let result = if self.channels == 1 {
                self.encoder.encode_to_vec(MonoPcm(self.scratch.as_slice()), &mut self.out)
            } else {
                self.encoder.encode_to_vec(InterleavedPcm(self.scratch.as_slice()), &mut self.out)
            };
            result.map_err(|e| IoError::Export(format!("MP3 encode failed: {e}")))?;
            file.write_all(&self.out)?;
        }
        Ok(())
    }

    /// Flushes the encoder's internal buffers (and pads the last frame) to `file`.
    fn flush(&mut self, file: &mut impl Write) -> Result<(), IoError> {
        self.out.clear();
        self.out.reserve(max_required_buffer_size(0));
        self.encoder.flush_to_vec::<FlushGap>(&mut self.out).map_err(|e| IoError::Export(format!("MP3 flush failed: {e}")))?;
        file.write_all(&self.out)?;
        Ok(())
    }

    /// Overwrites the placeholder first frame with the final LAME/Info tag.
    ///
    /// Call after `flush`. Does nothing when LAME wrote no tag.
    fn write_tag(&self, file: &mut File) -> Result<(), IoError> {
        if self.encoder.lame_tag_size() == 0 {
            return Ok(());
        }
        let mut tag = Vec::with_capacity(self.encoder.lame_tag_size());
        if self.encoder.lame_tag_encode_to_vec(&mut tag).is_none() {
            return Ok(());
        }
        // The placeholder follows any ID3v2 tag (there is none unless one is set).
        file.seek(SeekFrom::Start(self.encoder.id3v2_tag_size() as u64))?;
        file.write_all(&tag)?;
        Ok(())
    }
}

/// Streaming MP3 export sink.
struct Mp3Sink {
    lame: Lame,
    channels: usize,
    /// Present when the project rate is not an MP3 rate.
    resampler: Option<StreamResampler>,
    /// Resampler output for the current `write`, reused between calls.
    resampled: Vec<f32>,
    // Declared before `guard` so the file is closed before the guard deletes it.
    file: BufWriter<File>,
    guard: PartGuard,
}

/// Opens an MP3 sink writing `<path>.part`; used by `export::open_sink`.
pub(super) fn open_sink(path: &Path, kbps: u32, format: &ExportFormat) -> Result<Box<dyn AudioSink>, IoError> {
    let out_rate = mp3_rate(format.in_rate);
    let lame = Lame::new(format.channels, out_rate, kbps)?;
    let resampler = if out_rate == format.in_rate { None } else { Some(StreamResampler::new(format.channels, format.in_rate, out_rate)?) };
    let guard = PartGuard::new(path);
    let file = BufWriter::with_capacity(FILE_BUFFER, File::create(guard.part_path())?);
    let channels = usize::from(format.channels);
    Ok(Box::new(Mp3Sink { lame, channels, resampler, resampled: Vec::with_capacity(ENCODE_FRAMES * channels), file, guard }))
}

impl AudioSink for Mp3Sink {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), IoError> {
        check_whole_frames(interleaved.len(), self.channels)?;
        match self.resampler.as_mut() {
            None => self.lame.encode(interleaved, &mut self.file),
            Some(resampler) => {
                self.resampled.clear();
                resampler.push(interleaved, &mut self.resampled)?;
                self.lame.encode(&self.resampled, &mut self.file)
            }
        }
    }

    fn finish(self: Box<Self>) -> Result<(), IoError> {
        let Mp3Sink { mut lame, mut resampler, mut resampled, mut file, guard, .. } = *self;
        if let Some(resampler) = resampler.as_mut() {
            resampled.clear();
            resampler.finish(&mut resampled)?;
            lame.encode(&resampled, &mut file)?;
        }
        lame.flush(&mut file)?;
        let mut file = file.into_inner().map_err(|e| e.into_error())?;
        lame.write_tag(&mut file)?;
        file.flush()?;
        drop(file);
        guard.commit()
    }

    fn abort(self: Box<Self>) {
        // Dropping closes the file, then the guard deletes it.
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
        // MPEG sync at the very start, then the CBR "Info" tag and LAME version.
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1] & 0xE0, 0xE0);
        let head = &bytes[..200];
        let info = head.windows(4).position(|w| w == b"Info").unwrap();
        assert_eq!(info, 36, "Info tag sits right after the stereo side info");
        assert!(head.windows(4).any(|w| w == b"LAME"));
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
}
