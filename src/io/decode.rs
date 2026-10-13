//! io::decode: symphonia decoding of WAV and MP3 into interleaved f32 samples.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::{CodecParameters, audio::AudioDecoderOptions};
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::IoError;

/// Decoded audio: interleaved samples (all of channel 0, then channel 1, ...), f32.
#[derive(Debug)]
pub struct Decoded {
    /// Interleaved f32 samples.
    pub samples: Vec<f32>,
    /// Number of channels (1 or 2).
    pub channels: u16,
    /// Sample rate in Hz.
    pub rate: u32,
}

/// Message for files with more than 2 channels.
const TOO_MANY_CHANNELS: &str = "Files with more than 2 channels aren't supported";

/// Decodes a WAV or MP3 file into interleaved f32 samples.
///
/// Gapless decoding is enabled so MP3 encoder delay and padding are trimmed.
/// `progress` is called with values in 0.0..=1.0 while decoding when the total
/// frame count is known; otherwise it is not called.
pub fn decode_file(path: &Path, progress: &mut dyn FnMut(f32)) -> Result<Decoded, IoError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(
        Box::new(file),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| IoError::UnsupportedAudio(format!("{e}")))?;

    // Pick the first audio track and clone its parameters so the format can
    // still be borrowed mutably during the decode loop.
    let track = format
        .tracks()
        .iter()
        .find(|t| matches!(t.codec_params, Some(CodecParameters::Audio(_))))
        .ok_or_else(|| IoError::UnsupportedAudio("no audio track found".to_string()))?;
    let params = match &track.codec_params {
        Some(CodecParameters::Audio(params)) => params.clone(),
        _ => unreachable!("track was selected by audio codec params"),
    };

    let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(0);
    if channels > 2 {
        return Err(IoError::Decode(TOO_MANY_CHANNELS.to_string()));
    }
    let rate = params.sample_rate.unwrap_or(0);
    if channels == 0 || rate == 0 {
        return Err(IoError::UnsupportedAudio(
            "missing channel count or sample rate".to_string(),
        ));
    }

    // Gapless decoding is on by default in AudioDecoderOptions, which trims
    // MP3 encoder delay and padding frames.
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| IoError::UnsupportedAudio(format!("{e}")))?;

    let total_frames = track.num_frames;
    let mut samples: Vec<f32> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    let mut frames_done: u64 = 0;
    let mut reported = false;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            // Clean end of stream.
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(IoError::Decode(format!("{e}"))),
        };

        match decoder.decode(&packet) {
            Ok(buf) => {
                buf.copy_to_vec_interleaved::<f32>(&mut scratch);
                frames_done += buf.frames() as u64;
                samples.extend_from_slice(&scratch);
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            // Skip undecodable packets and keep going.
            Err(_) => continue,
        }

        if let Some(total) = total_frames
            && total > 0
        {
            reported = true;
            progress((frames_done as f32 / total as f32).min(1.0));
        }
    }

    if reported {
        progress(1.0);
    }

    Ok(Decoded {
        samples,
        channels: channels as u16,
        rate,
    })
}

/// Returns true if the path has a .wav or .mp3 extension, case-insensitive.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "wav" | "mp3"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use hound::{SampleFormat, WavSpec, WavWriter};

    /// Writes a 16-bit int WAV with one sample per channel per frame.
    fn write_i16_wav(
        dir: &std::path::Path,
        name: &str,
        channels: u16,
        rate: u32,
        frames: &[Vec<i16>],
    ) -> PathBuf {
        let path = dir.join(name);
        let spec = WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        for frame in frames {
            for &s in frame {
                writer.write_sample(s).unwrap();
            }
        }
        writer.finalize().unwrap();
        path
    }

    #[test]
    fn decodes_16bit_stereo() {
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Vec<i16>> = (0..1000)
            .map(|i| {
                let v = ((i % 251) as i16 - 125) * 100;
                vec![v, -v]
            })
            .collect();
        let path = write_i16_wav(dir.path(), "stereo.wav", 2, 48000, &frames);

        let mut calls = 0;
        let decoded = decode_file(&path, &mut |_| calls += 1).unwrap();
        assert_eq!(decoded.samples.len(), 2000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.rate, 48000);
        for (i, &s) in decoded.samples.iter().enumerate() {
            let half = (i / 2 % 251) as i16 - 125;
            let expected = if i % 2 == 0 {
                half * 100
            } else {
                -(half * 100)
            };
            assert!(
                (s - f32::from(expected) / 32768.0).abs() < 1e-3,
                "sample {i}: {s}"
            );
        }
        // Total frame count is known for WAV, so progress must fire.
        assert!(calls > 0);
    }

    #[test]
    fn decodes_24bit_mono() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mono24.wav");
        let spec = WavSpec {
            channels: 1,
            sample_rate: 44100,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(&path, spec).unwrap();
        let values = [0i32, 1 << 22, -(1 << 22), (1 << 23) - 1, -(1 << 23)];
        for &v in &values {
            writer.write_sample(v).unwrap();
        }
        writer.finalize().unwrap();

        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert_eq!(decoded.channels, 1);
        assert_eq!(decoded.rate, 44100);
        assert_eq!(decoded.samples.len(), values.len());
        for (&got, &want) in decoded.samples.iter().zip(values.iter()) {
            assert!(
                (got - want as f32 / 8388608.0).abs() < 1e-3,
                "got {got}, want {want}"
            );
        }
    }

    #[test]
    fn rejects_4_channel() {
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<Vec<i16>> = (0..10).map(|i| vec![i as i16; 4]).collect();
        let path = write_i16_wav(dir.path(), "quad.wav", 4, 48000, &frames);
        let err = decode_file(&path, &mut |_| {}).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("could not decode audio: {TOO_MANY_CHANNELS}")
        );
    }

    #[test]
    fn is_supported_checks_extension() {
        assert!(is_supported(Path::new("a.WAV")));
        assert!(is_supported(Path::new("b.mp3")));
        assert!(!is_supported(Path::new("c.flac")));
        assert!(!is_supported(Path::new("noext")));
    }

    #[test]
    fn decodes_mp3_fixture_if_present() {
        let path = Path::new("tests/fixtures/tone_44k.mp3");
        if !path.is_file() {
            println!("note: tests/fixtures/tone_44k.mp3 not present; skipping MP3 test");
            return;
        }
        let decoded = decode_file(path, &mut |_| {}).unwrap();
        assert_eq!(decoded.rate, 44100);
        assert_eq!(decoded.channels, 2);
        let frames = decoded.samples.len() / 2;
        let seconds = frames as f64 / 44100.0;
        assert!(
            (3.0..=5.0).contains(&seconds),
            "unexpected duration: {seconds} s ({frames} frames)"
        );
    }
}
