//! io::wav: WAV writing and reading via hound.
//!
//! Export writers (MP3 export etc.) come in phase 9; this module only covers
//! reading media and reading/writing the 32-bit float conformed cache WAVs.

use std::fs;
use std::path::Path;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use super::IoError;
use super::decode::Decoded;

/// Writes interleaved f32 samples as a 32-bit float WAV.
///
/// The file is written to a temp file in the same folder, then renamed over
/// the target path.
pub fn write_f32(path: &Path, samples: &[f32], channels: u16, rate: u32) -> Result<(), IoError> {
    let tmp = path.with_extension("wav.tmp");
    let spec = WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: SampleFormat::Float,
    };
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
    let reader =
        WavReader::open(path).map_err(|e| IoError::Decode(format!("cannot open WAV: {e}")))?;
    let spec = reader.spec();
    if spec.sample_format != SampleFormat::Float || spec.bits_per_sample != 32 {
        return Err(IoError::Decode(format!(
            "expected a 32-bit float WAV, found {} bits {:?}",
            spec.bits_per_sample, spec.sample_format
        )));
    }
    let mut samples = Vec::new();
    for sample in reader.into_samples::<f32>() {
        samples.push(sample.map_err(|e| IoError::Decode(format!("bad WAV data: {e}")))?);
    }
    Ok(Decoded {
        samples,
        channels: spec.channels,
        rate: spec.sample_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
