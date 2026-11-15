//! io::wav: WAV reading, the cache WAV writer, and the streaming export sink, via hound.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::IoError;
use super::decode::Decoded;
use super::export::{AudioSink, ExportFormat, PartGuard, check_whole_frames};
use crate::model::project::BitDepth;

/// Writes interleaved f32 samples as a 32-bit float WAV.
///
/// The file is written to a temp file in the same folder, then renamed over
/// the target path.
pub fn write_f32(path: &Path, samples: &[f32], channels: u16, rate: u32) -> Result<(), IoError> {
    let tmp = path.with_extension("wav.tmp");
    let spec = WavSpec { channels, sample_rate: rate, bits_per_sample: 32, sample_format: SampleFormat::Float };
    let result = (|| -> Result<(), hound::Error> {
        let mut writer = WavWriter::create(&tmp, spec)?;
        for &s in samples {
            writer.write_sample(s)?;
        }
        writer.finalize()?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            fs::rename(&tmp, path)?;
            Ok(())
        }
        Err(e) => {
            // Best effort cleanup of the temp file.
            let _ = fs::remove_file(&tmp);
            Err(IoError::Decode(format!("WAV write failed: {e}")))
        }
    }
}

/// Reads a 32-bit float WAV into interleaved f32 samples.
pub fn read_f32(path: &Path) -> Result<Decoded, IoError> {
    let reader = WavReader::open(path).map_err(|e| IoError::Decode(format!("cannot open WAV: {e}")))?;
    let spec = reader.spec();
    if spec.sample_format != SampleFormat::Float || spec.bits_per_sample != 32 {
        return Err(IoError::Decode(format!("expected a 32-bit float WAV, found {} bits {:?}", spec.bits_per_sample, spec.sample_format)));
    }
    let mut samples = Vec::new();
    for sample in reader.into_samples::<f32>() {
        samples.push(sample.map_err(|e| IoError::Decode(format!("bad WAV data: {e}")))?);
    }
    Ok(Decoded { samples, channels: spec.channels, rate: spec.sample_rate })
}

/// Full scale of 16-bit samples (2^15 - 1, the same rule as 24-bit).
const I16_SCALE: f32 = 32_767.0;
/// Full scale of 24-bit samples.
const I24_SCALE: f32 = 8_388_607.0;

/// Small xorshift64 PRNG for dither noise; a crate would be overkill here.
struct Xorshift64(u64);

impl Xorshift64 {
    /// Next uniform value in 0.0..1.0 (24 random bits, exact in f32).
    fn next_unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 40) as f32 / 16_777_216.0
    }
}

/// Streaming WAV export sink at the project rate.
struct WavSink {
    // Declared before `guard` so the file is closed before the guard deletes it.
    writer: WavWriter<BufWriter<File>>,
    depth: BitDepth,
    channels: usize,
    rng: Xorshift64,
    guard: PartGuard,
}

/// Opens a WAV sink writing `<path>.part`; used by `export::open_sink`.
pub(super) fn open_sink(path: &Path, depth: BitDepth, format: &ExportFormat) -> Result<Box<dyn AudioSink>, IoError> {
    let (bits_per_sample, sample_format) = match depth {
        BitDepth::Int16 => (16, SampleFormat::Int),
        BitDepth::Int24 => (24, SampleFormat::Int),
        BitDepth::Float32 => (32, SampleFormat::Float),
    };
    let spec = WavSpec { channels: format.channels, sample_rate: format.in_rate, bits_per_sample, sample_format };
    let guard = PartGuard::new(path);
    let writer = WavWriter::create(guard.part_path(), spec).map_err(wav_error)?;
    Ok(Box::new(WavSink { writer, depth, channels: usize::from(format.channels), rng: Xorshift64(0x9E37_79B9_7F4A_7C15), guard }))
}

/// Maps a hound error to an IoError, keeping real I/O errors as such.
pub(super) fn wav_error(e: hound::Error) -> IoError {
    match e {
        hound::Error::IoError(e) => IoError::Io(e),
        other => IoError::Export(format!("WAV write failed: {other}")),
    }
}

impl WavSink {
    /// 16-bit with TPDF dither: adds (r1 - r2) LSB, then rounds and clamps.
    fn write_i16(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &s in samples {
            let dither = self.rng.next_unit() - self.rng.next_unit();
            let v = (s * I16_SCALE + dither).round().clamp(-32_768.0, 32_767.0);
            self.writer.write_sample(v as i16)?;
        }
        Ok(())
    }

    /// 24-bit: scaled, rounded and clamped, no dither.
    fn write_i24(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &s in samples {
            let v = (s * I24_SCALE).round().clamp(-8_388_608.0, 8_388_607.0);
            self.writer.write_sample(v as i32)?;
        }
        Ok(())
    }

    /// 32-bit float: written as-is, no clamping.
    fn write_f32(&mut self, samples: &[f32]) -> Result<(), hound::Error> {
        for &s in samples {
            self.writer.write_sample(s)?;
        }
        Ok(())
    }
}

impl AudioSink for WavSink {
    fn write(&mut self, interleaved: &[f32]) -> Result<(), IoError> {
        check_whole_frames(interleaved.len(), self.channels)?;
        match self.depth {
            BitDepth::Int16 => self.write_i16(interleaved),
            BitDepth::Int24 => self.write_i24(interleaved),
            BitDepth::Float32 => self.write_f32(interleaved),
        }
        .map_err(wav_error)
    }

    fn finish(self: Box<Self>) -> Result<(), IoError> {
        let WavSink { writer, guard, .. } = *self;
        writer.finalize().map_err(wav_error)?;
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
    use crate::io::export::{ExportKind, open_sink as open_export_sink};

    /// Opens a WAV sink at 48 kHz for the tests below.
    fn open_wav(path: &Path, depth: BitDepth, channels: u16) -> Box<dyn AudioSink> {
        let format = ExportFormat { kind: ExportKind::Wav(depth), channels, in_rate: 48_000 };
        open_export_sink(path, &format).unwrap()
    }

    /// Writes `samples` through a WAV sink and returns the finished file's path.
    fn export_wav(dir: &Path, depth: BitDepth, channels: u16, samples: &[f32]) -> std::path::PathBuf {
        let path = dir.join("out.wav");
        let mut sink = open_wav(&path, depth, channels);
        sink.write(samples).unwrap();
        sink.finish().unwrap();
        path
    }

    #[test]
    fn int16_stereo_is_within_one_lsb() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_wav(dir.path(), BitDepth::Int16, 2, &[0.5; 2000]);

        let mut reader = WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 2);
        assert_eq!(spec.sample_rate, 48_000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(spec.sample_format, SampleFormat::Int);
        assert_eq!(reader.len(), 2000);
        for s in reader.samples::<i16>() {
            let v = i32::from(s.unwrap());
            assert!((v - 16_384).abs() <= 1, "got {v}");
        }
    }

    #[test]
    fn int16_dithers_silence_with_zero_mean() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_wav(dir.path(), BitDepth::Int16, 1, &[0.0; 20_000]);

        let values: Vec<i32> = WavReader::open(&path).unwrap().samples::<i16>().map(|s| i32::from(s.unwrap())).collect();
        assert_eq!(values.len(), 20_000);
        assert!(values.iter().all(|v| (-1..=1).contains(v)));
        // Dither must actually be present, not just rounded silence.
        assert!(values.iter().any(|&v| v != 0));
        let mean = values.iter().sum::<i32>() as f64 / values.len() as f64;
        assert!(mean.abs() < 0.05, "mean was {mean}");
    }

    #[test]
    fn int16_clamps_overs() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_wav(dir.path(), BitDepth::Int16, 1, &[1.5, -1.5]);

        let values: Vec<i16> = WavReader::open(&path).unwrap().samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(values, vec![32_767, -32_768]);
    }

    #[test]
    fn int24_rounds_exactly_without_dither() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_wav(dir.path(), BitDepth::Int24, 2, &[0.5, -0.5, 1.5, -1.5, 0.0, 1.0]);

        let mut reader = WavReader::open(&path).unwrap();
        assert_eq!(reader.spec().bits_per_sample, 24);
        let values: Vec<i32> = reader.samples::<i32>().map(|s| s.unwrap()).collect();
        assert_eq!(values, vec![4_194_304, -4_194_304, 8_388_607, -8_388_608, 0, 8_388_607]);
    }

    #[test]
    fn float32_round_trips_exactly_without_clamping() {
        let dir = tempfile::tempdir().unwrap();
        let samples = [0.0, 0.5, -0.25, 1.5, -2.0, 1.0e-7, 0.123_456_79];
        let path = export_wav(dir.path(), BitDepth::Float32, 1, &samples);

        let mut reader = WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.bits_per_sample, 32);
        assert_eq!(spec.sample_format, SampleFormat::Float);
        let values: Vec<f32> = reader.samples::<f32>().map(|s| s.unwrap()).collect();
        assert_eq!(values, samples);
    }

    #[test]
    fn many_small_writes_make_one_continuous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        let mut sink = open_wav(&path, BitDepth::Float32, 2);
        let all: Vec<f32> = (0..2000).map(|i| i as f32 / 2000.0).collect();
        for block in all.chunks(300) {
            sink.write(block).unwrap();
        }
        sink.finish().unwrap();

        let values: Vec<f32> = WavReader::open(&path).unwrap().samples::<f32>().map(|s| s.unwrap()).collect();
        assert_eq!(values, all);
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.f32.wav");
        let samples: Vec<f32> = (0..3000).map(|i| (i as f32 / 3000.0) * 2.0 - 1.0).collect();
        write_f32(&path, &samples, 2, 48000).unwrap();

        // No stray temp file left behind.
        assert!(!path.with_extension("wav.tmp").exists());

        let decoded = read_f32(&path).unwrap();
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.rate, 48000);
        assert_eq!(decoded.samples.len(), samples.len());
        for (&got, &want) in decoded.samples.iter().zip(samples.iter()) {
            assert_eq!(got, want);
        }
    }
}
