//! io::resample: rubato conform of imported media to the project sample rate.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, FixedAsync, Indexing, Resampler, SincInterpolationParameters, SincInterpolationType,
    WindowFunction,
};

use super::IoError;

/// Input chunk size in frames processed per resampler call.
const CHUNK_FRAMES: usize = 1024;
/// Sinc filter length; higher is better quality but slower.
const SINC_LEN: usize = 256;
/// Intermediate points for sinc interpolation.
const OVERSAMPLING_FACTOR: usize = 256;

/// Resamples interleaved f32 audio from `from` Hz to `to` Hz with a high
/// quality sinc resampler, compensating for the resampler's output delay.
///
/// Returns exactly `round(in_frames * to / from)` frames of interleaved
/// samples, zero-padding at the end if the resampler came up short.
/// `progress` is called with a 0.0..1.0 fraction as chunks are processed.
pub fn resample(
    samples: &[f32],
    channels: u16,
    from: u32,
    to: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<f32>, IoError> {
    if from == to {
        progress(1.0);
        return Ok(samples.to_vec());
    }
    if channels == 0 {
        return Err(IoError::Decode("cannot resample zero channels".to_string()));
    }
    let ch = usize::from(channels);
    if !samples.len().is_multiple_of(ch) {
        return Err(IoError::Decode(format!(
            "sample count {} is not a multiple of {} channels",
            samples.len(),
            ch
        )));
    }
    let in_frames = samples.len() / ch;
    if in_frames == 0 {
        progress(1.0);
        return Ok(Vec::new());
    }
    // Exact rounded target length, in integer math to avoid float error.
    let target =
        ((in_frames as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from)) as usize;

    let params = SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2)
        .oversampling_factor(OVERSAMPLING_FACTOR)
        .interpolation(SincInterpolationType::Cubic);
    let mut resampler = Async::<f32>::new_sinc(
        f64::from(to) / f64::from(from),
        1.1,
        &params,
        CHUNK_FRAMES,
        ch,
        FixedAsync::Input,
    )
    .map_err(|e| IoError::Decode(format!("resampler setup failed: {e}")))?;
    let delay = resampler.output_delay();

    // Worst case produced output: the target length plus the resampler delay
    // plus the frames generated while flushing the final partial chunk.
    let flush_frames =
        (f64::from(CHUNK_FRAMES as u32) * f64::from(to) / f64::from(from)).ceil() as usize;
    let out_frames_cap = target + delay + flush_frames + CHUNK_FRAMES + 64;
    let mut out = vec![0.0_f32; out_frames_cap * ch];

    let in_adapter = InterleavedSlice::new(samples, ch, in_frames)
        .map_err(|e| IoError::Decode(format!("bad input buffer for resampler: {e}")))?;
    let mut out_adapter = InterleavedSlice::new_mut(&mut out, ch, out_frames_cap)
        .map_err(|e| IoError::Decode(format!("bad output buffer for resampler: {e}")))?;

    let mut indexing = Indexing::new();
    let mut produced = 0usize;
    let mut frames_left = in_frames;
    let mut next_in = resampler.input_frames_next();
    while frames_left >= next_in {
        let (in_used, out_written) = resampler
            .process_into_buffer(&in_adapter, &mut out_adapter, Some(&indexing))
            .map_err(|e| IoError::Decode(format!("resampling failed: {e}")))?;
        indexing.input_offset += in_used;
        indexing.output_offset += out_written;
        produced += out_written;
        frames_left -= in_used;
        next_in = resampler.input_frames_next();
        progress((in_frames - frames_left) as f32 / in_frames as f32);
    }
    // Flush the tail: partial_len treats the rest of the chunk as silence.
    indexing.partial_len = Some(frames_left);
    let (_in_used, out_written) = resampler
        .process_into_buffer(&in_adapter, &mut out_adapter, Some(&indexing))
        .map_err(|e| IoError::Decode(format!("resampling failed: {e}")))?;
    produced += out_written;
    progress(1.0);

    // The first `delay` output frames are the resampler's startup silence;
    // skip them so the audio is not shifted in time.
    let audio_frames = produced.saturating_sub(delay);
    let mut result = Vec::with_capacity(target * ch);
    for frame in 0..target {
        for channel in 0..ch {
            if frame < audio_frames {
                result.push(out[(delay + frame) * ch + channel]);
            } else {
                result.push(0.0);
            }
        }
    }
    Ok(result)
}

/// Streaming counterpart of [`resample`]: feed interleaved audio in pieces of
/// any size and get the resampled audio back as it becomes available.
///
/// Uses the same sinc filter and chunk size as `resample`. All working buffers
/// are allocated once in `new`. The filter's startup delay is dropped, so the
/// output is not shifted, and after `finish` the total output is exactly
/// `round(total_in_frames * to / from)` frames.
pub struct StreamResampler {
    resampler: Async<f32>,
    channels: usize,
    from: u32,
    to: u32,
    /// Input staging chunk: filled by `push` until it holds CHUNK_FRAMES frames.
    in_buf: Vec<f32>,
    in_filled: usize,
    /// Resampler output for one chunk.
    out_buf: Vec<f32>,
    out_cap_frames: usize,
    /// Startup-delay frames still to drop from the output.
    delay_left: usize,
    in_frames: u64,
    out_frames: u64,
}

impl StreamResampler {
    /// Creates a resampler for `channels` interleaved channels, `from` Hz to `to` Hz.
    pub fn new(channels: u16, from: u32, to: u32) -> Result<Self, IoError> {
        if channels == 0 || from == 0 || to == 0 {
            return Err(IoError::Decode(
                "cannot resample zero channels or a zero sample rate".to_string(),
            ));
        }
        let ch = usize::from(channels);
        let params = SincInterpolationParameters::new(SINC_LEN, WindowFunction::BlackmanHarris2)
            .oversampling_factor(OVERSAMPLING_FACTOR)
            .interpolation(SincInterpolationType::Cubic);
        let resampler = Async::<f32>::new_sinc(
            f64::from(to) / f64::from(from),
            1.1,
            &params,
            CHUNK_FRAMES,
            ch,
            FixedAsync::Input,
        )
        .map_err(|e| IoError::Decode(format!("resampler setup failed: {e}")))?;
        let out_cap_frames = resampler.output_frames_max();
        Ok(Self {
            delay_left: resampler.output_delay(),
            resampler,
            channels: ch,
            from,
            to,
            in_buf: vec![0.0; CHUNK_FRAMES * ch],
            in_filled: 0,
            out_buf: vec![0.0; out_cap_frames * ch],
            out_cap_frames,
            in_frames: 0,
            out_frames: 0,
        })
    }

    /// Resamples `input` (whole interleaved frames) and appends whatever output
    /// is ready to `out`. Up to one chunk of input stays buffered inside.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<(), IoError> {
        let ch = self.channels;
        if !input.len().is_multiple_of(ch) {
            return Err(IoError::Decode(format!(
                "sample count {} is not a multiple of {} channels",
                input.len(),
                ch
            )));
        }
        let mut rest = input;
        while !rest.is_empty() {
            let take = (rest.len() / ch).min(CHUNK_FRAMES - self.in_filled);
            let start = self.in_filled * ch;
            self.in_buf[start..start + take * ch].copy_from_slice(&rest[..take * ch]);
            self.in_filled += take;
            self.in_frames += take as u64;
            rest = &rest[take * ch..];
            if self.in_filled == CHUNK_FRAMES {
                self.run_chunk(out)?;
                self.in_filled = 0;
            }
        }
        Ok(())
    }

    /// Flushes the buffered tail and the filter delay into `out`, ending the
    /// stream at exactly `round(total_in_frames * to / from)` frames in total.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<(), IoError> {
        let ch = self.channels;
        let target =
            (self.in_frames * u64::from(self.to) + u64::from(self.from) / 2) / u64::from(self.from);
        let start_len = out.len();
        let emitted_before = self.out_frames;

        // The tail of the last chunk is processed as if followed by silence,
        // then more silence pushes the delayed audio out of the filter.
        if self.in_filled > 0 {
            self.in_buf[self.in_filled * ch..].fill(0.0);
            self.run_chunk(out)?;
            self.in_filled = 0;
        }
        self.in_buf.fill(0.0);
        let mut guard = 0;
        while self.out_frames < target && guard < 16 {
            self.run_chunk(out)?;
            guard += 1;
        }

        let want = target.saturating_sub(emitted_before) as usize;
        out.resize(start_len + want * ch, 0.0);
        self.out_frames = emitted_before + want as u64;
        Ok(())
    }

    /// Resamples the staging chunk and appends its output, minus any startup delay.
    fn run_chunk(&mut self, out: &mut Vec<f32>) -> Result<(), IoError> {
        let ch = self.channels;
        let in_adapter = InterleavedSlice::new(&self.in_buf, ch, CHUNK_FRAMES)
            .map_err(|e| IoError::Decode(format!("bad input buffer for resampler: {e}")))?;
        let mut out_adapter = InterleavedSlice::new_mut(&mut self.out_buf, ch, self.out_cap_frames)
            .map_err(|e| IoError::Decode(format!("bad output buffer for resampler: {e}")))?;
        let (_in_used, written) = self
            .resampler
            .process_into_buffer(&in_adapter, &mut out_adapter, None)
            .map_err(|e| IoError::Decode(format!("resampling failed: {e}")))?;

        let skip = self.delay_left.min(written);
        self.delay_left -= skip;
        out.extend_from_slice(&self.out_buf[skip * ch..written * ch]);
        self.out_frames += (written - skip) as u64;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{StreamResampler, resample};

    /// Silences the progress callback in tests.
    fn quiet(f: f32) {
        let _ = f;
    }

    /// Streams `input` through a StreamResampler in pieces of `piece` frames.
    fn stream(input: &[f32], channels: u16, from: u32, to: u32, piece: usize) -> Vec<f32> {
        let mut rs = StreamResampler::new(channels, from, to).unwrap();
        let mut out = Vec::new();
        for block in input.chunks(piece * usize::from(channels)) {
            rs.push(block, &mut out).unwrap();
        }
        rs.finish(&mut out).unwrap();
        out
    }

    #[test]
    fn stream_output_length_is_exact_for_any_piece_size() {
        for (from, to) in [
            (96_000, 48_000),
            (88_200, 44_100),
            (44_100, 48_000),
            (48_000, 44_100),
        ] {
            for frames in [0usize, 1, 100, 1023, 1024, 1025, 5000, 20_000] {
                let input = vec![0.25_f32; frames * 2];
                let want = (frames as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from);
                for piece in [1usize, 333, 1024, 4096, 50_000] {
                    let out = stream(&input, 2, from, to, piece);
                    assert_eq!(
                        out.len() as u64,
                        want * 2,
                        "{from}->{to}, {frames} frames, pieces of {piece}"
                    );
                }
            }
        }
    }

    #[test]
    fn stream_matches_batch_resample() {
        let input: Vec<f32> = (0..44_100)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let batch = resample(&input, 1, 44_100, 48_000, &mut quiet).unwrap();
        let streamed = stream(&input, 1, 44_100, 48_000, 700);
        assert_eq!(streamed.len(), batch.len());
        // Identical filter and chunking, so the two agree away from the tail,
        // where the batch version pads zeros instead of flushing the filter.
        let compare = batch.len() - 400;
        for i in 0..compare {
            assert!(
                (streamed[i] - batch[i]).abs() < 1e-5,
                "frame {i}: stream {} vs batch {}",
                streamed[i],
                batch[i]
            );
        }
    }

    #[test]
    fn stream_timing_is_not_shifted() {
        let mut input = vec![0.0_f32; 96_000];
        input[48_000] = 1.0; // 0.5 s at 96 kHz
        let out = stream(&input, 1, 96_000, 48_000, 1000);
        assert_eq!(out.len(), 48_000);
        let (peak_index, _) = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        assert!(
            (peak_index as i64 - 24_000).abs() <= 2,
            "impulse landed at {peak_index}, expected within 2 frames of 24000"
        );
    }

    #[test]
    fn stream_keeps_the_tail_of_the_signal() {
        // A burst right at the end of a length that is not a chunk multiple
        // must survive the flush, not be replaced by zeros.
        let mut input = vec![0.0_f32; 3000];
        for s in &mut input[2900..] {
            *s = 0.5;
        }
        let out = stream(&input, 1, 48_000, 44_100, 3000);
        assert_eq!(out.len(), 2756);
        let tail_peak = out[out.len() - 40..]
            .iter()
            .fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(tail_peak > 0.4, "tail peak was {tail_peak}");
    }

    #[test]
    fn stream_rejects_partial_frames_and_zero_settings() {
        let mut rs = StreamResampler::new(2, 96_000, 48_000).unwrap();
        assert!(rs.push(&[0.0; 3], &mut Vec::new()).is_err());
        assert!(StreamResampler::new(0, 96_000, 48_000).is_err());
        assert!(StreamResampler::new(2, 0, 48_000).is_err());
    }

    #[test]
    fn same_rate_returns_identical_samples() {
        let input: Vec<f32> = (0..500).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample(&input, 1, 48_000, 48_000, &mut quiet).unwrap();
        assert_eq!(out.len(), input.len());
        assert_eq!(out, input);
    }

    #[test]
    fn forty_four_one_to_forty_eight_gives_exact_frames() {
        // 1 second of stereo pseudo-noise.
        let input: Vec<f32> = (0..44_100 * 2)
            .map(|i| (i as f32 * 12.9898).sin() * 0.5)
            .collect();
        let out = resample(&input, 2, 44_100, 48_000, &mut quiet).unwrap();
        assert_eq!(out.len(), 48_000 * 2, "output must be exactly 48000 frames");
        // Stereo stays stereo: both channels carry signal.
        let [ch0_max, ch1_max] = out
            .as_chunks::<2>()
            .0
            .iter()
            .fold([0.0_f32; 2], |mut m, p| {
                m[0] = m[0].max(p[0].abs());
                m[1] = m[1].max(p[1].abs());
                m
            });
        assert!(ch0_max > 0.1 && ch1_max > 0.1);
    }

    #[test]
    fn thousand_frames_downsample_gives_exact_frames() {
        let input: Vec<f32> = vec![0.1; 1000];
        let out = resample(&input, 1, 48_000, 44_100, &mut quiet).unwrap();
        assert_eq!(out.len(), 919, "output must be exactly 919 frames");
    }

    #[test]
    fn impulse_timing_is_preserved() {
        let mut input = vec![0.0_f32; 44_100];
        input[22_050] = 1.0; // 0.5 s at 44.1 kHz
        let out = resample(&input, 1, 44_100, 48_000, &mut quiet).unwrap();
        assert_eq!(out.len(), 48_000);
        let (peak_index, _) = out
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap();
        assert!(
            (peak_index as i64 - 24_000).abs() <= 2,
            "impulse landed at {peak_index}, expected within 2 frames of 24000"
        );
    }

    #[test]
    fn sine_amplitude_is_preserved() {
        // 1 kHz sine at 0.5 amplitude, one second at 44.1 kHz.
        let input: Vec<f32> = (0..44_100)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let out = resample(&input, 1, 44_100, 48_000, &mut quiet).unwrap();
        assert_eq!(out.len(), 48_000);
        // Look well away from the filter edges.
        let middle = &out[24_000..43_000];
        let peak = middle.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(
            (0.45..=0.55).contains(&peak),
            "middle peak was {peak}, expected 0.45..0.55"
        );
    }
}
