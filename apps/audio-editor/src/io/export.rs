//! io::export: streaming export sinks for WAV and MP3.
//!
//! A sink takes interleaved f32 blocks as the mixer renders them, so a whole
//! export never has to sit in memory. Every sink writes to `<path>.part` in the
//! target folder; `finish` renames it over the target and `abort` (or just
//! dropping the sink) deletes it.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use super::{IoError, mp3, wav};
use crate::model::project::BitDepth;

/// What kind of file to export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    /// WAV at the project rate with the given sample format.
    Wav(BitDepth),
    /// CBR MP3 (`kbps` is 128, 192, 256 or 320).
    Mp3 {
        /// Constant bitrate in kilobits per second.
        kbps: u32,
    },
}

/// Everything a sink needs to know about the audio it will be fed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportFormat {
    /// File kind and its settings.
    pub kind: ExportKind,
    /// Channels in the interleaved blocks passed to `write` (1 or 2).
    pub channels: u16,
    /// Sample rate of the blocks passed to `write` (the project rate).
    pub in_rate: u32,
}

/// Where audio goes during an export.
pub trait AudioSink: Send {
    /// Appends whole interleaved frames (a multiple of the channel count).
    fn write(&mut self, interleaved: &[f32]) -> Result<(), IoError>;
    /// Flushes everything and renames the `.part` file over the target path.
    fn finish(self: Box<Self>) -> Result<(), IoError>;
    /// Discards the export: closes and deletes the `.part` file.
    fn abort(self: Box<Self>);
}

/// MP3 output rate for a project rate: 44.1 and 48 kHz stay, 88.2 and 96 kHz
/// halve. Any other rate falls back to 48 kHz.
pub fn mp3_rate(project_rate: u32) -> u32 {
    match project_rate {
        44_100 | 88_200 => 44_100,
        _ => 48_000,
    }
}

/// Opens a sink that writes `format` audio to `path` via `<path>.part`.
pub fn open_sink(path: &Path, format: &ExportFormat) -> Result<Box<dyn AudioSink>, IoError> {
    if !(1..=2).contains(&format.channels) {
        return Err(IoError::Export(format!("unsupported channel count: {}", format.channels)));
    }
    if format.in_rate == 0 {
        return Err(IoError::Export("sample rate must be above 0".to_string()));
    }
    match format.kind {
        ExportKind::Wav(depth) => wav::open_sink(path, depth, format),
        ExportKind::Mp3 { kbps } => mp3::open_sink(path, kbps, format),
    }
}

/// Fails unless `samples` holds a whole number of frames of `channels`.
pub(super) fn check_whole_frames(samples: usize, channels: usize) -> Result<(), IoError> {
    if samples.is_multiple_of(channels) {
        Ok(())
    } else {
        Err(IoError::Export(format!("{samples} samples is not a whole number of {channels}-channel frames")))
    }
}

/// Owns the `<path>.part` temp file: deletes it on drop unless `commit` ran.
///
/// A sink must declare its file handle before its guard, so the handle is
/// closed (fields drop in order) before the guard deletes the file.
pub(super) struct PartGuard {
    part: PathBuf,
    target: PathBuf,
    armed: bool,
}

impl PartGuard {
    /// Guard for the export of `target`; touches nothing on disk yet.
    pub(super) fn new(target: &Path) -> Self {
        let mut part: OsString = target.as_os_str().to_owned();
        part.push(".part");
        Self { part: PathBuf::from(part), target: target.to_path_buf(), armed: true }
    }

    /// Path of the temp file to create and write.
    pub(super) fn part_path(&self) -> &Path {
        &self.part
    }

    /// Renames the temp file over the target. The file handle must be closed.
    pub(super) fn commit(mut self) -> Result<(), IoError> {
        fs::rename(&self.part, &self.target)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for PartGuard {
    fn drop(&mut self) {
        if self.armed {
            // Best effort: nothing useful to do if the delete fails.
            let _ = fs::remove_file(&self.part);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_format(depth: BitDepth, channels: u16) -> ExportFormat {
        ExportFormat { kind: ExportKind::Wav(depth), channels, in_rate: 48_000 }
    }

    fn mp3_format(channels: u16) -> ExportFormat {
        ExportFormat { kind: ExportKind::Mp3 { kbps: 192 }, channels, in_rate: 48_000 }
    }

    fn part_of(path: &Path) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(".part");
        PathBuf::from(s)
    }

    /// One second of a quiet 440 Hz stereo sine, as interleaved samples.
    fn tone_block(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = 0.25 * (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin();
                [s, s]
            })
            .collect()
    }

    #[test]
    fn mp3_rate_maps_project_rates() {
        assert_eq!(mp3_rate(44_100), 44_100);
        assert_eq!(mp3_rate(48_000), 48_000);
        assert_eq!(mp3_rate(88_200), 44_100);
        assert_eq!(mp3_rate(96_000), 48_000);
    }

    #[test]
    fn finish_leaves_no_part_file() {
        for format in [wav_format(BitDepth::Int24, 2), mp3_format(2)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("out.bin");
            let mut sink = open_sink(&path, &format).unwrap();
            // The temp file exists while the export is running.
            assert!(part_of(&path).exists());
            assert!(!path.exists());
            sink.write(&tone_block(4800)).unwrap();
            sink.finish().unwrap();

            assert!(path.is_file());
            assert!(!part_of(&path).exists());
        }
    }

    #[test]
    fn abort_leaves_no_file_at_all() {
        for format in [wav_format(BitDepth::Int16, 2), mp3_format(2)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("out.bin");
            let mut sink = open_sink(&path, &format).unwrap();
            sink.write(&tone_block(4800)).unwrap();
            sink.abort();

            assert!(!path.exists());
            assert!(!part_of(&path).exists());
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn dropping_a_sink_cleans_up_the_part_file() {
        for format in [wav_format(BitDepth::Float32, 1), mp3_format(1)] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("out.bin");
            let mut sink = open_sink(&path, &format).unwrap();
            sink.write(&[0.0; 2400]).unwrap();
            drop(sink);

            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn finish_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        fs::write(&path, b"old contents").unwrap();

        let mut sink = open_sink(&path, &wav_format(BitDepth::Int16, 2)).unwrap();
        // The old file stays untouched until finish.
        assert_eq!(fs::read(&path).unwrap(), b"old contents");
        sink.write(&tone_block(100)).unwrap();
        sink.finish().unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.len(), 200);
    }

    #[test]
    fn rejects_bad_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        for channels in [0, 3] {
            let result = open_sink(&path, &wav_format(BitDepth::Int16, channels));
            assert!(matches!(result, Err(IoError::Export(_))));
        }
        let zero_rate = ExportFormat { in_rate: 0, ..wav_format(BitDepth::Int16, 2) };
        assert!(matches!(open_sink(&path, &zero_rate), Err(IoError::Export(_))));
        let bad_kbps = ExportFormat { kind: ExportKind::Mp3 { kbps: 100 }, ..mp3_format(2) };
        assert!(matches!(open_sink(&path, &bad_kbps), Err(IoError::Export(_))));
        // Nothing was created by the failed opens.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_partial_frames() {
        let dir = tempfile::tempdir().unwrap();
        for format in [wav_format(BitDepth::Int16, 2), mp3_format(2)] {
            let path = dir.path().join("out.bin");
            let mut sink = open_sink(&path, &format).unwrap();
            assert!(matches!(sink.write(&[0.0, 0.0, 0.0]), Err(IoError::Export(_))));
            sink.abort();
        }
    }
}
