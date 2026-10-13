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

#[cfg(test)]
mod tests {
    use super::resample;

    /// Silences the progress callback in tests.
    fn quiet(f: f32) {
        let _ = f;
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
