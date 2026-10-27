//! io::decode: symphonia decoding of WAV and MP3 into interleaved f32 samples.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::{CodecParameters, audio::AudioDecoderOptions};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::well_known::{FORMAT_ID_MP3, FORMAT_ID_WAVE};
use symphonia::core::formats::{FormatId, FormatOptions};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use super::IoError;
use super::limits::{MAX_ITEM_SECONDS, MAX_SOURCE_RATE, MIN_SOURCE_RATE, max_frames};

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

/// Message for files longer than the item cap.
const TOO_LONG: &str = "File is too long (max 2 hours)";

/// The container a file extension promises, or `None` for an extension the
/// app does not decode.
fn expected_container(ext: &str) -> Option<FormatId> {
    match ext.to_ascii_lowercase().as_str() {
        "wav" => Some(FORMAT_ID_WAVE),
        "mp3" => Some(FORMAT_ID_MP3),
        _ => None,
    }
}

/// Adds `frames` to the running decoded total, failing once it passes `cap`.
fn add_frames(done: u64, frames: u64, cap: u64) -> Result<u64, IoError> {
    match done.checked_add(frames) {
        Some(total) if total <= cap => Ok(total),
        _ => Err(IoError::UnsupportedAudio(TOO_LONG.to_string())),
    }
}

/// Decodes a WAV or MP3 file into interleaved f32 samples.
///
/// Gapless decoding is enabled so MP3 encoder delay and padding are trimmed.
/// `progress` is called with values in 0.0..=1.0 while decoding when the total
/// frame count is known; otherwise it is not called.
///
/// Untrusted headers are bounded before any packet is decoded: the container
/// must match the extension, the sample rate must be in
/// `MIN_SOURCE_RATE..=MAX_SOURCE_RATE`, and the audio may not last longer than
/// `MAX_ITEM_SECONDS` (checked up front when the file reports its length, and
/// again while decoding, so the sample buffer never grows past the cap).
pub fn decode_file(path: &Path, progress: &mut dyn FnMut(f32)) -> Result<Decoded, IoError> {
    decode_with_cap(path, progress, MAX_ITEM_SECONDS)
}

/// `decode_file` with the length cap as a parameter, so tests can use a small one.
fn decode_with_cap(
    path: &Path,
    progress: &mut dyn FnMut(f32),
    max_seconds: u64,
) -> Result<Decoded, IoError> {
    let file = File::open(path)?;
    // Only .wav and .mp3 are decoded; nothing else reaches the parsers.
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let expected = expected_container(ext).ok_or_else(|| {
        IoError::UnsupportedAudio("Only .wav and .mp3 files are supported".to_string())
    })?;
    let mss = MediaSourceStream::new(
        Box::new(file),
        symphonia::core::io::MediaSourceStreamOptions::default(),
    );
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| IoError::UnsupportedAudio(format!("{e}")))?;

    // Probing goes by content, so a renamed file must not be accepted under
    // the wrong extension.
    if format.format_info().format != expected {
        return Err(IoError::UnsupportedAudio(format!(
            "File contents do not match its .{ext} extension"
        )));
    }

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
    let rate = params
        .sample_rate
        .filter(|_| channels != 0)
        .ok_or_else(|| {
            IoError::UnsupportedAudio("missing channel count or sample rate".to_string())
        })?;
    if !(MIN_SOURCE_RATE..=MAX_SOURCE_RATE).contains(&rate) {
        return Err(IoError::UnsupportedAudio(format!(
            "Unsupported sample rate: {rate} Hz"
        )));
    }

    // Reject on the reported length before decoding a single packet.
    let frame_cap = max_frames(rate, max_seconds);
    let total_frames = track.num_frames;
    if total_frames.is_some_and(|total| total > frame_cap) {
        return Err(IoError::UnsupportedAudio(TOO_LONG.to_string()));
    }

    // Gapless decoding is on by default in AudioDecoderOptions, which trims
    // MP3 encoder delay and padding frames.
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| IoError::UnsupportedAudio(format!("{e}")))?;

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
                // The reported length can be wrong or missing, so the cap is
                // enforced on what is really decoded, before the Vec grows.
                frames_done = add_frames(frames_done, buf.frames() as u64, frame_cap)?;
                buf.copy_to_vec_interleaved::<f32>(&mut scratch);
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
    use std::fs;
    use std::path::PathBuf;

    use hound::{SampleFormat, WavSpec, WavWriter};

    use crate::io::export::{ExportFormat, ExportKind, open_sink};

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

    /// Writes a mono 16-bit silent WAV of `frames` frames whose header says `rate` Hz.
    fn write_mono_wav(dir: &Path, name: &str, rate: u32, frames: usize) -> PathBuf {
        let frames: Vec<Vec<i16>> = vec![vec![0]; frames];
        write_i16_wav(dir, name, 1, rate, &frames)
    }

    fn message(err: IoError) -> String {
        match err {
            IoError::UnsupportedAudio(text) | IoError::Decode(text) => text,
            other => panic!("unexpected error kind: {other:?}"),
        }
    }

    #[test]
    fn rejects_a_1_hz_header_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_mono_wav(dir.path(), "one_hz.wav", 1, 100);
        let mut progress_calls = 0;
        let err = decode_file(&path, &mut |_| progress_calls += 1).unwrap_err();
        assert_eq!(message(err), "Unsupported sample rate: 1 Hz");
        // No packet was decoded, so there was nothing to report.
        assert_eq!(progress_calls, 0);
    }

    #[test]
    fn sample_rate_window_is_8000_to_384000_inclusive() {
        let dir = tempfile::tempdir().unwrap();
        for rate in [8_000, 384_000] {
            let path = write_mono_wav(dir.path(), &format!("ok_{rate}.wav"), rate, 20);
            let decoded = decode_file(&path, &mut |_| {}).unwrap();
            assert_eq!(decoded.rate, rate);
            assert_eq!(decoded.samples.len(), 20);
        }
        for rate in [1, 7_999, 384_001, 400_000] {
            let path = write_mono_wav(dir.path(), &format!("bad_{rate}.wav"), rate, 20);
            let err = decode_file(&path, &mut |_| {}).unwrap_err();
            assert_eq!(message(err), format!("Unsupported sample rate: {rate} Hz"));
        }
    }

    #[test]
    fn a_header_rate_of_zero_or_u32_max_is_rejected() {
        // hound cannot write these, so patch the rate field (offset 24) of a
        // normal WAV. Whether symphonia or our check refuses them, they fail.
        let dir = tempfile::tempdir().unwrap();
        let normal = fs::read(write_mono_wav(dir.path(), "normal.wav", 48_000, 20)).unwrap();
        for rate in [0u32, u32::MAX] {
            let mut bytes = normal.clone();
            bytes[24..28].copy_from_slice(&rate.to_le_bytes());
            let path = write_bytes(dir.path(), &format!("patched_{rate}.wav"), &bytes);
            let result = decode_file(&path, &mut |_| {});
            assert!(
                matches!(result, Err(IoError::UnsupportedAudio(_))),
                "{rate}: {result:?}"
            );
        }
    }

    #[test]
    fn a_file_over_the_length_cap_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // 8 kHz with a 1 second cap: 8,000 frames fit exactly, 8,001 do not.
        let fits = write_mono_wav(dir.path(), "fits.wav", 8_000, 8_000);
        let long = write_mono_wav(dir.path(), "long.wav", 8_000, 8_001);
        let decoded = decode_with_cap(&fits, &mut |_| {}, 1).unwrap();
        assert_eq!(decoded.samples.len(), 8_000);
        let err = decode_with_cap(&long, &mut |_| {}, 1).unwrap_err();
        assert_eq!(message(err), "File is too long (max 2 hours)");
        // The shipped cap is the 2 hour limit, so the same file is fine there.
        assert!(decode_file(&long, &mut |_| {}).is_ok());
    }

    #[test]
    fn add_frames_stops_exactly_at_the_cap() {
        assert_eq!(add_frames(0, 100, 100).unwrap(), 100);
        assert_eq!(add_frames(60, 40, 100).unwrap(), 100);
        assert!(add_frames(60, 41, 100).is_err());
        assert!(add_frames(u64::MAX, 1, u64::MAX).is_err());
        assert!(add_frames(1, u64::MAX, u64::MAX).is_err());
    }

    /// An MP3 of `seconds` of sine written through the app's own MP3 sink.
    fn mp3_bytes(rate: u32, channels: u16, seconds: u32) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sine.mp3");
        let format = ExportFormat {
            kind: ExportKind::Mp3 { kbps: 192 },
            channels,
            in_rate: rate,
        };
        let mut sink = open_sink(&path, &format).unwrap();
        let frames = (rate * seconds) as usize;
        let sine: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let phase = std::f64::consts::TAU * 440.0 * i as f64 / f64::from(rate);
                std::iter::repeat_n((0.5 * phase.sin()) as f32, usize::from(channels))
            })
            .collect();
        for block in sine.chunks(1000 * usize::from(channels)) {
            sink.write(block).unwrap();
        }
        sink.finish().unwrap();
        fs::read(&path).unwrap()
    }

    /// Syncsafe 28-bit integer, as used in ID3v2 headers.
    fn syncsafe(n: usize) -> [u8; 4] {
        [
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ]
    }

    /// An ID3v2.3 or v2.4 tag holding one TIT2 frame plus `padding` zero bytes.
    fn id3v2_tag(major: u8, title: &str, padding: usize) -> Vec<u8> {
        let text_len = 1 + title.len();
        let mut body = b"TIT2".to_vec();
        if major == 4 {
            body.extend_from_slice(&syncsafe(text_len));
        } else {
            body.extend_from_slice(&(text_len as u32).to_be_bytes());
        }
        body.extend_from_slice(&[0, 0, 0]); // frame flags, then ISO-8859-1 text
        body.extend_from_slice(title.as_bytes());
        body.extend(std::iter::repeat_n(0u8, padding));
        let mut tag = vec![b'I', b'D', b'3', major, 0, 0];
        tag.extend_from_slice(&syncsafe(body.len()));
        tag.extend_from_slice(&body);
        tag
    }

    /// A 128 byte ID3v1 tag, which sits at the END of an MP3 file.
    fn id3v1_tag() -> Vec<u8> {
        let mut tag = b"TAG".to_vec();
        tag.extend_from_slice(b"Some Title");
        tag.resize(128, 0);
        tag
    }

    fn write_bytes(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn an_mp3_with_an_id3_tag_still_decodes() {
        let dir = tempfile::tempdir().unwrap();
        let plain = mp3_bytes(48_000, 2, 1);
        let reference =
            decode_file(&write_bytes(dir.path(), "plain.mp3", &plain), &mut |_| {}).unwrap();
        assert_eq!(reference.rate, 48_000);
        assert_eq!(reference.channels, 2);

        let v23 = [id3v2_tag(3, "Test title", 0), plain.clone()].concat();
        let v24 = [id3v2_tag(4, "Test title", 2_048), plain.clone()].concat();
        // A big tag, like one carrying cover art.
        let big = [id3v2_tag(3, "Cover", 200_000), plain.clone()].concat();
        let v1 = [plain.clone(), id3v1_tag()].concat();
        let both = [id3v2_tag(4, "Both", 64), plain.clone(), id3v1_tag()].concat();
        for (name, bytes) in [
            ("v23.mp3", v23),
            ("v24.mp3", v24),
            ("big.mp3", big),
            ("v1.mp3", v1),
            ("both.mp3", both),
        ] {
            let path = write_bytes(dir.path(), name, &bytes);
            let decoded = decode_file(&path, &mut |_| {}).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(decoded.rate, 48_000, "{name}");
            assert_eq!(decoded.channels, 2, "{name}");
            assert_eq!(decoded.samples.len(), reference.samples.len(), "{name}");
            assert_eq!(decoded.samples, reference.samples, "{name}");
        }
    }

    /// Splits an MP3 into its first MPEG-1 Layer III frame (LAME's Info tag
    /// frame, which carries the length) and the audio frames after it.
    fn split_info_frame(mp3: &[u8]) -> (&[u8], &[u8]) {
        const KBPS: [u32; 16] = [
            0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
        ];
        const RATES: [u32; 4] = [44_100, 48_000, 32_000, 0];
        assert_eq!(mp3[0], 0xFF, "file must start at an MPEG frame");
        let bitrate = KBPS[usize::from(mp3[2] >> 4)] * 1000;
        let rate = RATES[usize::from((mp3[2] >> 2) & 3)];
        let padding = u32::from((mp3[2] >> 1) & 1);
        mp3.split_at((144 * bitrate / rate + padding) as usize)
    }

    #[test]
    fn an_mp3_without_a_length_header_is_capped_by_its_estimated_length() {
        let dir = tempfile::tempdir().unwrap();
        let mp3 = mp3_bytes(48_000, 2, 3);
        let bare = split_info_frame(&mp3).1;
        let path = write_bytes(dir.path(), "bare.mp3", bare);
        // Three seconds against a one second cap.
        let err = decode_with_cap(&path, &mut |_| {}, 1).unwrap_err();
        assert_eq!(message(err), "File is too long (max 2 hours)");
        // And it decodes fully under the real cap.
        let decoded = decode_file(&path, &mut |_| {}).unwrap();
        assert!(decoded.samples.len() / 2 > 3 * 48_000 - 5_000);
    }

    #[test]
    fn a_streaming_wav_with_no_reported_length_is_capped_while_decoding() {
        // A data chunk length of 0xFFFFFFFF means "streaming, length unknown":
        // symphonia then reports no frame count, so the up-front check cannot
        // fire and only the cap on really decoded frames stops the read.
        let dir = tempfile::tempdir().unwrap();
        let normal = fs::read(write_mono_wav(dir.path(), "src.wav", 8_000, 9_000)).unwrap();
        assert_eq!(&normal[36..40], b"data");
        let mut streaming = normal.clone();
        streaming[4..8].copy_from_slice(&u32::MAX.to_le_bytes()); // RIFF length
        streaming[40..44].copy_from_slice(&u32::MAX.to_le_bytes()); // data length
        let path = write_bytes(dir.path(), "streaming.wav", &streaming);

        // 9,000 frames at 8 kHz against a 1 second (8,000 frame) cap.
        let err = decode_with_cap(&path, &mut |_| {}, 1).unwrap_err();
        assert_eq!(message(err), "File is too long (max 2 hours)");
        // Under a cap that fits, it reads to the end of the file.
        let decoded = decode_with_cap(&path, &mut |_| {}, 2).unwrap();
        assert_eq!(decoded.samples.len(), 9_000);
    }

    #[test]
    fn contents_must_match_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let wav = fs::read(write_mono_wav(dir.path(), "src.wav", 48_000, 100)).unwrap();
        let mp3 = mp3_bytes(48_000, 2, 1);

        let mp3_as_wav = write_bytes(dir.path(), "really_mp3.wav", &mp3);
        let err = decode_file(&mp3_as_wav, &mut |_| {}).unwrap_err();
        assert_eq!(
            message(err),
            "File contents do not match its .wav extension"
        );

        let wav_as_mp3 = write_bytes(dir.path(), "really_wav.mp3", &wav);
        let err = decode_file(&wav_as_mp3, &mut |_| {}).unwrap_err();
        assert_eq!(
            message(err),
            "File contents do not match its .mp3 extension"
        );

        // The check ignores case, and matching files pass.
        let upper = write_bytes(dir.path(), "LOUD.WAV", &wav);
        assert!(decode_file(&upper, &mut |_| {}).is_ok());
        let upper_mp3 = write_bytes(dir.path(), "LOUD.MP3", &mp3);
        assert!(decode_file(&upper_mp3, &mut |_| {}).is_ok());
    }

    #[test]
    fn a_file_with_no_supported_extension_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let wav = fs::read(write_mono_wav(dir.path(), "src.wav", 48_000, 100)).unwrap();
        for name in ["noext", "data.bin", "tone.flac"] {
            let path = write_bytes(dir.path(), name, &wav);
            let err = decode_file(&path, &mut |_| {}).unwrap_err();
            assert_eq!(message(err), "Only .wav and .mp3 files are supported");
        }
    }

    /// A FLAC stream header: "fLaC" plus a STREAMINFO block. There is no FLAC
    /// encoder in the dependency tree, so this fixture has no audio frames.
    fn flac_header() -> Vec<u8> {
        let mut bytes = b"fLaC".to_vec();
        bytes.extend_from_slice(&[0x80, 0, 0, 34]); // last block, STREAMINFO, 34 bytes
        bytes.extend_from_slice(&4096u16.to_be_bytes()); // min block size
        bytes.extend_from_slice(&4096u16.to_be_bytes()); // max block size
        bytes.extend_from_slice(&[0; 6]); // min and max frame size unknown
        // 20 bit rate, 3 bit channels - 1, 5 bit bits per sample - 1, 36 bit length.
        let packed: u64 = (44_100 << 44) | (1 << 41) | (15 << 36) | 44_100;
        bytes.extend_from_slice(&packed.to_be_bytes());
        bytes.extend_from_slice(&[0; 16]); // MD5 of the audio
        bytes
    }

    /// An Ogg page carrying a Vorbis identification header; no Ogg encoder is
    /// available, so this fixture has no audio packets.
    fn ogg_header() -> Vec<u8> {
        let mut payload = vec![0x01];
        payload.extend_from_slice(b"vorbis");
        payload.extend_from_slice(&0u32.to_le_bytes()); // version
        payload.push(2); // channels
        payload.extend_from_slice(&44_100u32.to_le_bytes());
        payload.extend_from_slice(&[0; 12]); // bitrates
        payload.push(0xB8); // block sizes
        payload.push(1); // framing
        let mut page = b"OggS".to_vec();
        page.extend_from_slice(&[0, 0x02]); // version, begin-of-stream
        page.extend_from_slice(&[0; 8]); // granule position
        page.extend_from_slice(&[0; 12]); // serial, sequence, checksum
        page.push(1); // one segment
        page.push(payload.len() as u8);
        page.extend_from_slice(&payload);
        page
    }

    #[test]
    fn flac_and_ogg_renamed_to_wav_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("flac_as.wav", flac_header()),
            ("ogg_as.wav", ogg_header()),
            ("flac_as.mp3", flac_header()),
            ("ogg_as.mp3", ogg_header()),
        ] {
            let path = write_bytes(dir.path(), name, &bytes);
            let result = decode_file(&path, &mut |_| {});
            assert!(
                matches!(result, Err(IoError::UnsupportedAudio(_))),
                "{name}: {result:?}"
            );
        }
    }
}
