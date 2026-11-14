//! io::record_writer: streams a recording ring buffer into a 24-bit WAV take.
//!
//! The input callback pushes whole interleaved device frames into an rtrb ring
//! (audio::input); this writer owns the consumer end. It picks the record
//! channels, resamples to the project rate when the rates differ, and streams
//! 24-bit samples into `<name>.wav.part`, which `finish` renames to the final
//! path and `abort` (or just dropping the writer) deletes. The Editor creates
//! the ring and wires both ends (12.2).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};
use rtrb::Consumer;

use super::IoError;
use super::config::RecordChannels;
use super::export::PartGuard;
use super::resample::StreamResampler;
use super::wav::wav_error;

/// Full scale of 24-bit samples (the same rule as io::wav).
const I24_SCALE: f32 = 8_388_607.0;

/// Output frames per min/max peak entry.
const PEAK_FRAMES: usize = 1024;

/// Staging buffer size in samples: how much one drain pop reads at most.
const IN_BUF_SAMPLES: usize = 8 * 1024;

/// Min/max of one not-yet-complete peak group.
struct PeakAcc {
    /// Smallest left sample so far.
    min_l: f32,
    /// Largest left sample so far.
    max_l: f32,
    /// Smallest right sample so far.
    min_r: f32,
    /// Largest right sample so far.
    max_r: f32,
    /// Frames accumulated.
    count: usize,
}

impl PeakAcc {
    /// An empty accumulator.
    fn new() -> Self {
        Self {
            min_l: f32::INFINITY,
            max_l: f32::NEG_INFINITY,
            min_r: f32::INFINITY,
            max_r: f32::NEG_INFINITY,
            count: 0,
        }
    }

    /// Adds one output frame; returns the finished `[minL, maxL, minR, maxR]`
    /// entry when this was frame 1,024 of a group, resetting the accumulator.
    fn add(&mut self, l: f32, r: f32) -> Option<[f32; 4]> {
        self.min_l = self.min_l.min(l);
        self.max_l = self.max_l.max(l);
        self.min_r = self.min_r.min(r);
        self.max_r = self.max_r.max(r);
        self.count += 1;
        if self.count < PEAK_FRAMES {
            return None;
        }
        let entry = [self.min_l, self.max_l, self.min_r, self.max_r];
        *self = PeakAcc::new();
        Some(entry)
    }
}

/// Scales one sample to the 24-bit range: rounded and clamped, no dither.
fn to_i24(s: f32) -> i32 {
    (s * I24_SCALE).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// Streams a recording ring buffer into a 24-bit WAV take file at the project
/// rate.
///
/// Field order matters: the writer is closed before the guard deletes the
/// `.part` file (Windows cannot delete an open file).
#[allow(dead_code)] // the Editor owns one per take (12.2)
pub struct RecordWriter {
    /// The 24-bit WAV file under construction.
    writer: WavWriter<BufWriter<File>>,
    /// Resampler to the project rate; None when the rates match.
    resampler: Option<StreamResampler>,
    /// Output channel count (1 or 2).
    out_channels: usize,
    /// Device channel count (1 or more).
    device_channels: usize,
    /// Which device channel(s) one output frame reads.
    pick: RecordChannels,
    /// Staging buffer for frames popped from the ring.
    in_buf: Vec<f32>,
    /// Channel-picked interleaved frames.
    picked: Vec<f32>,
    /// Resampler output for one pass.
    out_buf: Vec<f32>,
    /// The not-yet-complete peak group.
    peak: PeakAcc,
    /// Output frames written so far.
    out_frames: u64,
    /// Final path of the take (the `.part` suffix stripped).
    target: PathBuf,
    /// Deletes the `.part` file unless the take is finished.
    guard: PartGuard,
}

impl RecordWriter {
    /// Creates a writer for the take at `part_path` (the caller passes
    /// `<name>.wav.part`; the final name is that without the suffix). Writes
    /// a 24-bit WAV at `project_rate`: one channel for Mono1/Mono2, two for
    /// Stereo. Resamples from `device_rate` with a `StreamResampler` when the
    /// rates differ. All working buffers are allocated here.
    #[allow(dead_code)] // the Editor starts takes with it (12.2)
    pub fn create(
        part_path: &Path,
        device_rate: u32,
        device_channels: u16,
        channels: RecordChannels,
        project_rate: u32,
    ) -> Result<Self, IoError> {
        if device_channels == 0 {
            return Err(IoError::Export(
                "the input device reports 0 channels".to_string(),
            ));
        }
        let text = part_path.to_string_lossy();
        let Some(target_text) = text.strip_suffix(".part") else {
            return Err(IoError::Export(format!(
                "the take path must end in .part: {}",
                part_path.display()
            )));
        };
        let target = PathBuf::from(target_text);
        let out_channels = match channels {
            RecordChannels::Mono1 | RecordChannels::Mono2 => 1u16,
            RecordChannels::Stereo => 2,
        };
        let resampler = if device_rate == project_rate {
            None
        } else {
            Some(StreamResampler::new(
                out_channels,
                device_rate,
                project_rate,
            )?)
        };
        let guard = PartGuard::new(&target);
        let spec = WavSpec {
            channels: out_channels,
            sample_rate: project_rate,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let writer = WavWriter::create(guard.part_path(), spec).map_err(wav_error)?;
        Ok(Self {
            writer,
            resampler,
            out_channels: usize::from(out_channels),
            device_channels: usize::from(device_channels),
            pick: channels,
            in_buf: vec![0.0; IN_BUF_SAMPLES],
            picked: Vec::with_capacity(IN_BUF_SAMPLES),
            out_buf: Vec::with_capacity(IN_BUF_SAMPLES),
            peak: PeakAcc::new(),
            out_frames: 0,
            target,
            guard,
        })
    }

    /// Reads every available whole frame from `consumer`, picks the record
    /// channels, resamples and writes them, appending one
    /// `[minL, maxL, minR, maxR]` entry to `peaks` per 1,024 output frames
    /// (mono: R = L). A group still incomplete when the take ends is dropped.
    /// Returns the output frames written so far.
    #[allow(dead_code)] // the take job drains the ring with it (12.2)
    pub fn drain(
        &mut self,
        consumer: &mut Consumer<f32>,
        peaks: &mut Vec<[f32; 4]>,
    ) -> Result<u64, IoError> {
        loop {
            let filled = consumer.pop_partial_slice(&mut self.in_buf).0.len();
            if filled == 0 {
                break;
            }
            self.consume(filled, peaks)?;
        }
        Ok(self.out_frames)
    }

    /// Drains what is left in the ring, flushes the resampler, finalizes the
    /// WAV and renames the `.part` file to the final path. Returns the final
    /// path and the total output frames written.
    #[allow(dead_code)] // the Editor finishes takes with it (12.2)
    pub fn finish(mut self, consumer: &mut Consumer<f32>) -> Result<(PathBuf, u64), IoError> {
        let mut peaks = Vec::new();
        self.drain(consumer, &mut peaks)?;
        if let Some(resampler) = &mut self.resampler {
            self.out_buf.clear();
            resampler.finish(&mut self.out_buf)?;
            let out = std::mem::take(&mut self.out_buf);
            let written = self.write_out(&out, &mut peaks);
            self.out_buf = out;
            written?;
        }
        self.writer.finalize().map_err(wav_error)?;
        self.guard.commit()?;
        Ok((self.target, self.out_frames))
    }

    /// Discards the take: closes and deletes the `.part` file.
    #[allow(dead_code)] // the Editor cancels takes with it (12.2)
    pub fn abort(self) {
        // Dropping closes the writer, then the guard deletes the .part.
        drop(self);
    }

    /// Picks channels from the first `samples` staged samples, resamples and
    /// writes them, feeding the peak groups.
    fn consume(&mut self, samples: usize, peaks: &mut Vec<[f32; 4]>) -> Result<(), IoError> {
        self.picked.clear();
        for frame in self.in_buf[..samples].chunks_exact(self.device_channels) {
            match self.pick {
                RecordChannels::Mono1 => self.picked.push(frame[0]),
                RecordChannels::Mono2 => self.picked.push(frame[self.ch1()]),
                RecordChannels::Stereo => {
                    self.picked.push(frame[0]);
                    self.picked.push(frame[self.ch1()]);
                }
            }
        }
        if let Some(resampler) = &mut self.resampler {
            self.out_buf.clear();
            resampler.push(&self.picked, &mut self.out_buf)?;
            let out = std::mem::take(&mut self.out_buf);
            let written = self.write_out(&out, peaks);
            self.out_buf = out;
            written
        } else {
            let picked = std::mem::take(&mut self.picked);
            let written = self.write_out(&picked, peaks);
            self.picked = picked;
            written
        }
    }

    /// Writes `out` (whole interleaved output frames) as 24-bit samples and
    /// feeds the peak groups.
    fn write_out(&mut self, out: &[f32], peaks: &mut Vec<[f32; 4]>) -> Result<(), IoError> {
        if self.out_channels == 1 {
            for &s in out {
                let v = to_i24(s);
                self.writer.write_sample(v).map_err(wav_error)?;
                if let Some(entry) = self.peak.add(s, s) {
                    peaks.push(entry);
                }
            }
        } else {
            for pair in out.as_chunks::<2>().0 {
                let v = [to_i24(pair[0]), to_i24(pair[1])];
                self.writer.write_sample(v[0]).map_err(wav_error)?;
                self.writer.write_sample(v[1]).map_err(wav_error)?;
                if let Some(entry) = self.peak.add(pair[0], pair[1]) {
                    peaks.push(entry);
                }
            }
        }
        self.out_frames += u64::try_from(out.len() / self.out_channels).unwrap_or(u64::MAX);
        Ok(())
    }

    /// Index of the second picked device channel, clamped to what the device has.
    fn ch1(&self) -> usize {
        self.device_channels.saturating_sub(1).min(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Number of 24-bit samples a written take file holds.
    fn read_wav(path: &Path) -> Vec<i32> {
        hound::WavReader::open(path)
            .unwrap()
            .samples::<i32>()
            .map(Result::unwrap)
            .collect()
    }

    /// Writes `input` (whole interleaved device frames) through a fresh ring
    /// and writer in one drain, returning the written samples and frames.
    fn written_samples(
        dir: &Path,
        name: &str,
        pick: RecordChannels,
        input: &[f32],
        device_rate: u32,
        device_channels: u16,
        project_rate: u32,
    ) -> (Vec<i32>, u64) {
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(input.len().max(1));
        let (pushed, rest) = producer.push_partial_slice(input);
        assert!(rest.is_empty(), "ring too small for the test input");
        assert_eq!(pushed.len(), input.len());
        let mut writer = RecordWriter::create(
            &dir.join(format!("{name}.wav.part")),
            device_rate,
            device_channels,
            pick,
            project_rate,
        )
        .unwrap();
        writer.drain(&mut consumer, &mut Vec::new()).unwrap();
        let (path, frames) = writer.finish(&mut consumer).unwrap();
        assert_eq!(path, dir.join(format!("{name}.wav")));
        assert!(!path.with_extension("wav.part").exists());
        (read_wav(&path), frames)
    }

    #[test]
    fn mono1_and_mono2_pick_their_channel_and_stereo_keeps_both() {
        let dir = tempdir().unwrap();
        // Three stereo device frames: (0.1, 0.9), (-0.2, -0.8), (0.3, -0.3).
        let input = [0.1f32, 0.9, -0.2, -0.8, 0.3, -0.3];
        let want = |s: f32| (s * I24_SCALE).round() as i32;

        let (samples, frames) = written_samples(
            dir.path(),
            "mono1",
            RecordChannels::Mono1,
            &input,
            48_000,
            2,
            48_000,
        );
        assert_eq!(frames, 3);
        assert_eq!(samples, [want(0.1), want(-0.2), want(0.3)]);

        let (samples, frames) = written_samples(
            dir.path(),
            "mono2",
            RecordChannels::Mono2,
            &input,
            48_000,
            2,
            48_000,
        );
        assert_eq!(frames, 3);
        assert_eq!(samples, [want(0.9), want(-0.8), want(-0.3)]);

        let (samples, frames) = written_samples(
            dir.path(),
            "stereo",
            RecordChannels::Stereo,
            &input,
            48_000,
            2,
            48_000,
        );
        assert_eq!(frames, 3);
        assert_eq!(
            samples,
            [
                want(0.1),
                want(0.9),
                want(-0.2),
                want(-0.8),
                want(0.3),
                want(-0.3)
            ]
        );
    }

    #[test]
    fn same_rate_output_matches_input_frames_and_values() {
        let dir = tempdir().unwrap();
        // 1,000 stereo frames.
        let input: Vec<f32> = (0..2000).map(|i| ((i as f32 * 0.01).sin()) * 0.9).collect();
        let (samples, frames) = written_samples(
            dir.path(),
            "same",
            RecordChannels::Stereo,
            &input,
            48_000,
            2,
            48_000,
        );
        assert_eq!(frames, 1000);
        assert_eq!(samples.len(), 2000);
        for (&got, &want_in) in samples.iter().zip(input.iter()) {
            let want = to_i24(want_in);
            assert!(
                (got - want).abs() <= 1,
                "got {got}, want {want} (input {want_in})"
            );
        }
    }

    #[test]
    fn resampled_length_is_the_rounded_ratio() {
        let dir = tempdir().unwrap();
        // One second of stereo noise at 44.1 kHz, recorded as mono.
        let in_frames = 44_100usize;
        let input: Vec<f32> = (0..in_frames * 2)
            .map(|i| ((i as f32 * 12.9898).sin()) * 0.5)
            .collect();
        let (samples, frames) = written_samples(
            dir.path(),
            "rs",
            RecordChannels::Mono1,
            &input,
            44_100,
            2,
            48_000,
        );
        let want = (in_frames as u64 * 48_000 + 44_100 / 2) / 44_100;
        assert!(
            (frames as i64 - want as i64).abs() <= 2,
            "frames {frames}, want {want}"
        );
        assert_eq!(samples.len(), frames as usize, "mono output, one channel");
    }

    #[test]
    fn several_drains_write_the_same_file_as_one_drain() {
        let dir = tempdir().unwrap();
        let input: Vec<f32> = (0..10_000)
            .map(|i| ((i as f32 * 0.031).sin()) * 0.7)
            .collect();
        let (one, one_frames) = written_samples(
            dir.path(),
            "one",
            RecordChannels::Stereo,
            &input,
            48_000,
            2,
            48_000,
        );

        let (mut producer, mut consumer) = rtrb::RingBuffer::new(input.len());
        let mut writer = RecordWriter::create(
            &dir.path().join("many.wav.part"),
            48_000,
            2,
            RecordChannels::Stereo,
            48_000,
        )
        .unwrap();
        let mut peaks = Vec::new();
        let _ = producer.push_partial_slice(&input[..4_000]);
        writer.drain(&mut consumer, &mut peaks).unwrap();
        let _ = producer.push_partial_slice(&input[4_000..8_000]);
        writer.drain(&mut consumer, &mut peaks).unwrap();
        let _ = producer.push_partial_slice(&input[8_000..]);
        writer.drain(&mut consumer, &mut peaks).unwrap();
        let (path, frames) = writer.finish(&mut consumer).unwrap();
        assert_eq!(frames, one_frames);
        assert_eq!(frames, 5_000);
        assert_eq!(read_wav(&path), one);
    }

    #[test]
    fn peaks_are_one_entry_per_1024_output_frames() {
        let dir = tempdir().unwrap();
        // 2,048 mono frames ramping 0.0 .. 1.0 (values / 2048 are exact in f32).
        let frames = 2_048usize;
        let input: Vec<f32> = (0..frames).map(|i| i as f32 / frames as f32).collect();
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(frames);
        let _ = producer.push_partial_slice(&input);
        let mut writer = RecordWriter::create(
            &dir.path().join("p.wav.part"),
            48_000,
            1,
            RecordChannels::Mono1,
            48_000,
        )
        .unwrap();
        let mut peaks = Vec::new();
        let total = writer.drain(&mut consumer, &mut peaks).unwrap();
        assert_eq!(total, 2_048);
        assert_eq!(peaks.len(), 2);
        // Group 0 covers frames 0..1024: min 0.0, max 1023/2048.
        assert_eq!(peaks[0][0], 0.0);
        assert_eq!(peaks[0][1], 1023.0 / 2048.0);
        // Mono: R = L.
        assert_eq!(peaks[0][2], peaks[0][0]);
        assert_eq!(peaks[0][3], peaks[0][1]);
        // Group 1 covers frames 1024..2048.
        assert_eq!(peaks[1][0], 1024.0 / 2048.0);
        assert_eq!(peaks[1][1], 2047.0 / 2048.0);
        writer.finish(&mut consumer).unwrap();
    }

    #[test]
    fn abort_and_drop_delete_the_part_and_finish_renames_it() {
        let dir = tempdir().unwrap();

        let part = dir.path().join("a.wav.part");
        let writer =
            RecordWriter::create(&part, 48_000, 2, RecordChannels::Stereo, 48_000).unwrap();
        writer.abort();
        assert!(!part.exists());

        let part = dir.path().join("b.wav.part");
        let writer =
            RecordWriter::create(&part, 48_000, 2, RecordChannels::Stereo, 48_000).unwrap();
        drop(writer);
        assert!(!part.exists());

        let part = dir.path().join("c.wav.part");
        let mut writer =
            RecordWriter::create(&part, 48_000, 2, RecordChannels::Stereo, 48_000).unwrap();
        let (mut producer, mut consumer) = rtrb::RingBuffer::new(4);
        let _ = producer.push_partial_slice(&[0.0f32; 4]);
        writer.drain(&mut consumer, &mut Vec::new()).unwrap();
        let (path, frames) = writer.finish(&mut consumer).unwrap();
        assert_eq!(frames, 2);
        assert_eq!(path, dir.path().join("c.wav"));
        assert!(!part.exists());
        assert!(path.is_file());
    }

    #[test]
    fn create_rejects_a_path_without_the_part_suffix() {
        let dir = tempdir().unwrap();
        let Err(error) = RecordWriter::create(
            &dir.path().join("take.wav"),
            48_000,
            2,
            RecordChannels::Stereo,
            48_000,
        ) else {
            panic!("create must refuse a path that does not end in .part");
        };
        assert!(error.to_string().contains(".part"), "{error}");
    }
}
