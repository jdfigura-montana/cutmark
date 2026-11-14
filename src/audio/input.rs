//! audio::input: the cpal input stream; its callback only feeds an input meter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, ErrorKind, InputCallbackInfo, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfig, SupportedStreamConfigRange,
};
use rtrb::Producer;

use crate::audio::engine::AudioError;
use crate::audio::meters::{ChannelMeter, StereoMeter};

/// Name shown when the device cannot report one.
const UNKNOWN_DEVICE: &str = "Unknown input device";

/// Which input channel(s) of the device are metered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPick {
    /// One input channel (0-based); the meter shows it on both sides.
    Mono(u16),
    /// Input channels 0 and 1 as left and right.
    Stereo,
}

impl InputPick {
    /// The fewest channels a device needs for this pick to work.
    pub fn channels_needed(self) -> u16 {
        match self {
            InputPick::Mono(index) => index.saturating_add(1),
            InputPick::Stereo => 2,
        }
    }
}

/// A device sample type the input callback can read as f32 without allocating.
pub trait InputSample: Copy {
    /// The sample as f32 in -1.0..=1.0.
    fn to_f32(self) -> f32;
}

impl InputSample for f32 {
    fn to_f32(self) -> f32 {
        self
    }
}

impl InputSample for i16 {
    fn to_f32(self) -> f32 {
        f32::from(self) / 32768.0
    }
}

impl InputSample for u16 {
    fn to_f32(self) -> f32 {
        (f32::from(self) - 32768.0) / 32768.0
    }
}

/// One channel's totals for one block of input.
#[derive(Debug, Clone, Copy, Default)]
struct Block {
    /// Largest absolute sample.
    peak: f32,
    /// Sum of squared samples (mean square = this / frames).
    sum_sq: f32,
    /// Frames counted.
    frames: u32,
}

/// Totals of `channel` over every whole frame of interleaved `samples`.
/// Non-finite samples count as silence so one bad value cannot poison a meter.
fn channel_block<S: InputSample>(samples: &[S], channels: usize, channel: usize) -> Block {
    let mut block = Block::default();
    if channels == 0 {
        return block;
    }
    let mut frames = 0u32;
    for frame in samples.chunks_exact(channels) {
        let Some(sample) = frame.get(channel) else {
            continue;
        };
        let value = sample.to_f32();
        let value = if value.is_finite() { value } else { 0.0 };
        let magnitude = value.abs();
        if magnitude > block.peak {
            block.peak = magnitude;
        }
        block.sum_sq += value * value;
        frames = frames.saturating_add(1);
    }
    block.frames = frames;
    block
}

/// Adds one block to a channel meter; an empty block adds nothing.
fn feed(meter: &ChannelMeter, block: Block) {
    if block.frames > 0 {
        meter.add(block.peak, block.sum_sq, block.frames, block.peak >= 1.0);
    }
}

/// Meters one callback block of interleaved device samples: per picked channel
/// the block peak, sum of squares and frame count go to `meter`. A mono pick
/// feeds the same totals to both meter channels. A pick the device cannot
/// serve (channel out of range, `channels` 0) feeds nothing. No allocation, no
/// locks, no panics: safe for the input callback.
pub fn meter_block<S: InputSample>(
    samples: &[S],
    channels: u16,
    pick: InputPick,
    meter: &StereoMeter,
) {
    let channels = usize::from(channels);
    match pick {
        InputPick::Mono(index) => {
            let index = usize::from(index);
            if index >= channels {
                return;
            }
            let block = channel_block(samples, channels, index);
            feed(&meter.l, block);
            feed(&meter.r, block);
        }
        InputPick::Stereo => {
            if channels < 2 {
                return;
            }
            feed(&meter.l, channel_block(samples, channels, 0));
            feed(&meter.r, channel_block(samples, channels, 1));
        }
    }
}

/// When `recording`, pushes whole interleaved frames of `samples` (converted
/// to f32) into `producer` and returns how many whole frames did not fit and
/// were dropped; when not, pushes nothing and returns 0. Never pushes a
/// partial frame. No allocation, no locks, no panics: safe for the input
/// callback.
fn record_block<S: InputSample>(
    samples: &[S],
    channels: usize,
    recording: bool,
    producer: &mut Producer<f32>,
) -> u64 {
    if !recording || channels == 0 {
        return 0;
    }
    let frames = samples.len() / channels;
    let want = frames * channels;
    // Room for whole frames only, so a frame is never split.
    let fits = (producer.slots() / channels) * channels;
    let writable = want.min(fits);
    if writable > 0
        && let Ok(mut chunk) = producer.write_chunk(writable)
    {
        let (head, tail) = samples.split_at(writable);
        let (first, second) = chunk.as_mut_slices();
        for (dst, src) in first.iter_mut().zip(head) {
            *dst = src.to_f32();
        }
        for (dst, src) in second.iter_mut().zip(tail) {
            *dst = src.to_f32();
        }
        chunk.commit_all();
    }
    let dropped = (want - writable) / channels;
    u64::try_from(dropped).unwrap_or(u64::MAX)
}

/// True for stream errors that mean the capture has stopped. A reroute to a
/// new default device, a buffer glitch and a refused real-time priority all
/// leave the stream running, so they must not read as a lost device.
fn is_fatal(kind: ErrorKind) -> bool {
    !matches!(
        kind,
        ErrorKind::DeviceChanged | ErrorKind::Xrun | ErrorKind::RealtimeDenied
    )
}

/// Preference among the sample formats the callback can read: f32, then i16, then u16.
fn format_rank(format: SampleFormat) -> Option<u8> {
    match format {
        SampleFormat::F32 => Some(0),
        SampleFormat::I16 => Some(1),
        SampleFormat::U16 => Some(2),
        _ => None,
    }
}

/// The device's default config, or, when the device lists the same channel
/// count and rate in a format that ranks higher (f32 over i16 over u16), that
/// one. None when no readable format exists at all.
fn best_config(
    default: SupportedStreamConfig,
    others: impl IntoIterator<Item = SupportedStreamConfigRange>,
) -> Option<SupportedStreamConfig> {
    let mut best = format_rank(default.sample_format()).map(|rank| (rank, default));
    for range in others {
        if range.channels() != default.channels() {
            continue;
        }
        let Some(rank) = format_rank(range.sample_format()) else {
            continue;
        };
        let Some(config) = range.try_with_sample_rate(default.sample_rate()) else {
            continue;
        };
        if best.is_none_or(|(best_rank, _)| rank < best_rank) {
            best = Some((rank, config));
        }
    }
    best.map(|(_, config)| config)
}

/// A device's display name, or None when it cannot be read (for example
/// because it was just unplugged). `Device`'s own `Display` returns an error
/// in that case, which makes `to_string()` panic, so it is not used here.
pub(crate) fn device_name(device: &Device) -> Option<String> {
    device
        .description()
        .ok()
        .map(|description| description.name().to_string())
}

/// Display names of the input devices Windows lists, without duplicates, in
/// the order Windows lists them. Empty when none can be found.
pub fn input_device_names() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Ok(devices) = cpal::default_host().input_devices() {
        for name in devices.filter_map(|device| device_name(&device)) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Display name of the Windows default input device, if there is one.
pub fn default_input_name() -> Option<String> {
    cpal::default_host()
        .default_input_device()
        .and_then(|device| device_name(&device))
}

/// Input channel count of the device `InputStream::open(name, ..)` would use
/// (the named device, else the Windows default), or None when there is no such
/// device or it cannot report a config. Opens nothing.
pub fn device_channels(name: Option<&str>) -> Option<u16> {
    let (device, _) = choose_device(&cpal::default_host(), name).ok()?;
    device
        .default_input_config()
        .ok()
        .map(|config| config.channels())
}

/// Name, sample rate and channel count of the device [`InputStream::open`]
/// would use (the named device, else the Windows default input), resolved the
/// same way `open` resolves them, including the best-config ranking. Opens
/// nothing.
// The Editor sizes the recording ring from it (12.2).
#[allow(dead_code)]
pub fn probe_input(device: Option<&str>) -> Result<(String, u32, u16), AudioError> {
    let host = cpal::default_host();
    let (device, _) = choose_device(&host, device)?;
    let name = device_name(&device).unwrap_or_else(|| UNKNOWN_DEVICE.to_string());
    let default = device
        .default_input_config()
        .map_err(AudioError::InputStream)?;
    let others = device
        .supported_input_configs()
        .map(|ranges| ranges.collect::<Vec<_>>())
        .unwrap_or_default();
    let supported =
        best_config(default, others).ok_or_else(|| AudioError::NoInputFormat(name.clone()))?;
    Ok((name, supported.sample_rate(), supported.channels()))
}

/// The device named `wanted` (None = no preference), else the default input.
/// The flag is true when a name was asked for but not found.
fn choose_device(host: &cpal::Host, wanted: Option<&str>) -> Result<(Device, bool), AudioError> {
    if let Some(wanted) = wanted
        && let Ok(devices) = host.input_devices()
    {
        for device in devices {
            if device_name(&device).as_deref() == Some(wanted) {
                return Ok((device, false));
            }
        }
    }
    let default = host
        .default_input_device()
        .ok_or(AudioError::NoInputDevice)?;
    Ok((default, wanted.is_some()))
}

/// Builds the capture stream for sample type `S`. The data callback meters the
/// picked channels and, while the recording flag is set, pushes whole frames
/// into the ring; the error callback only sets a flag.
// The parameters mirror what cpal needs: the device, its config and everything
// the callback touches.
#[allow(clippy::too_many_arguments)]
fn build_stream<S>(
    device: &Device,
    config: StreamConfig,
    pick: InputPick,
    meter: Arc<StereoMeter>,
    mut producer: Producer<f32>,
    recording: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
    errored: Arc<AtomicBool>,
) -> Result<Stream, cpal::Error>
where
    S: InputSample + SizedSample + Send + 'static,
{
    let channels = config.channels;
    device.build_input_stream(
        config,
        move |data: &[S], _info: &InputCallbackInfo| {
            assert_no_alloc::assert_no_alloc(|| {
                meter_block(data, channels, pick, &meter);
                let dropped_frames = record_block(
                    data,
                    usize::from(channels),
                    recording.load(Ordering::Relaxed),
                    &mut producer,
                );
                if dropped_frames > 0 {
                    dropped.fetch_add(dropped_frames, Ordering::Relaxed);
                }
            });
        },
        move |error: cpal::Error| {
            if is_fatal(error.kind()) {
                errored.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}

/// An open cpal input stream whose callback feeds an input meter and, while
/// recording is on, pushes whole frames into a ring.
///
/// Dropping the `InputStream` closes the device: cpal joins the callback when
/// the `Stream` is dropped.
pub struct InputStream {
    /// The live capture stream; dropping it closes the device.
    _stream: Stream,
    /// Display name of the device actually opened.
    device_name: String,
    /// True when a named device was asked for but the default input was used.
    fell_back: bool,
    /// Device sample rate in Hz.
    device_rate: u32,
    /// Device channel count.
    device_channels: u16,
    /// True while the callback pushes frames into the ring.
    recording: Arc<AtomicBool>,
    /// Whole frames dropped because the ring was full.
    dropped: Arc<AtomicU64>,
    /// Set by the stream error callback instead of logging.
    errored: Arc<AtomicBool>,
}

impl InputStream {
    /// Opens `device` (a display name; None, or a name that is not found, means
    /// the Windows default input) and starts capturing into `meter` and, while
    /// recording is on, into `producer`.
    ///
    /// Uses the device's default config: f32 when the device offers it at that
    /// channel count and rate, otherwise i16 or u16 converted sample by sample
    /// in the callback. Fails with a clear error, without opening anything,
    /// when `pick` needs a channel the device does not have.
    pub fn open(
        device: Option<&str>,
        pick: InputPick,
        meter: Arc<StereoMeter>,
        producer: Producer<f32>,
    ) -> Result<InputStream, AudioError> {
        let host = cpal::default_host();
        let (device, fell_back) = choose_device(&host, device)?;
        let name = device_name(&device).unwrap_or_else(|| UNKNOWN_DEVICE.to_string());

        let default = device
            .default_input_config()
            .map_err(AudioError::InputStream)?;
        let others = device
            .supported_input_configs()
            .map(|ranges| ranges.collect::<Vec<_>>())
            .unwrap_or_default();
        let supported =
            best_config(default, others).ok_or_else(|| AudioError::NoInputFormat(name.clone()))?;

        let have = supported.channels();
        let need = pick.channels_needed();
        if have < need {
            return Err(AudioError::InputChannels {
                device: name,
                have,
                need,
            });
        }

        let errored = Arc::new(AtomicBool::new(false));
        let recording = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let config = supported.config();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_stream::<f32>(
                &device,
                config,
                pick,
                meter,
                producer,
                Arc::clone(&recording),
                Arc::clone(&dropped),
                Arc::clone(&errored),
            ),
            SampleFormat::I16 => build_stream::<i16>(
                &device,
                config,
                pick,
                meter,
                producer,
                Arc::clone(&recording),
                Arc::clone(&dropped),
                Arc::clone(&errored),
            ),
            SampleFormat::U16 => build_stream::<u16>(
                &device,
                config,
                pick,
                meter,
                producer,
                Arc::clone(&recording),
                Arc::clone(&dropped),
                Arc::clone(&errored),
            ),
            _ => return Err(AudioError::NoInputFormat(name)),
        }
        .map_err(AudioError::InputStream)?;
        stream.play().map_err(AudioError::InputStream)?;

        Ok(InputStream {
            _stream: stream,
            device_name: name,
            fell_back,
            device_rate: supported.sample_rate(),
            device_channels: have,
            recording,
            dropped,
            errored,
        })
    }

    /// Display name of the device actually opened.
    // The take message names the device with it (12.2).
    #[allow(dead_code)]
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// True when a named device was asked for but the default input was used.
    pub fn fell_back(&self) -> bool {
        self.fell_back
    }

    /// Device sample rate in Hz.
    // The record writer needs it to resample to the project rate (12.2).
    #[allow(dead_code)]
    pub fn device_rate(&self) -> u32 {
        self.device_rate
    }

    /// Device channel count.
    // The record writer needs it to pick the channels from the ring (12.2).
    #[allow(dead_code)]
    pub fn device_channels(&self) -> u16 {
        self.device_channels
    }

    /// Turns recording on or off: the callback starts or stops pushing whole
    /// frames into the ring.
    // The Editor sets it around a take (12.2).
    #[allow(dead_code)]
    pub fn set_recording(&self, on: bool) {
        self.recording.store(on, Ordering::Release);
    }

    /// Whole frames dropped so far because the ring was full.
    // The Editor reads it after a take for the dropped-ms message (12.2).
    #[allow(dead_code)]
    pub fn dropped_frames(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Resets the dropped-frames counter to 0 (before a new take).
    // The Editor resets it when a take starts (12.2).
    #[allow(dead_code)]
    pub fn reset_dropped(&self) {
        self.dropped.store(0, Ordering::Relaxed);
    }

    /// True once the stream error callback has reported a fatal error.
    pub fn errored(&self) -> bool {
        self.errored.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cpal::SupportedBufferSize;
    use rtrb::RingBuffer;

    #[test]
    fn record_block_off_pushes_nothing() {
        let (mut producer, consumer) = RingBuffer::new(16);
        let samples = [0.5f32, -0.5, 0.25, -0.25];
        let dropped = record_block(&samples, 2, false, &mut producer);
        assert_eq!(dropped, 0);
        assert_eq!(consumer.slots(), 0);
    }

    #[test]
    fn record_block_pushes_f32_frames_unchanged() {
        let (mut producer, mut consumer) = RingBuffer::new(16);
        let samples = [0.5f32, -0.25, 0.75, -1.0];
        let dropped = record_block(&samples, 2, true, &mut producer);
        assert_eq!(dropped, 0);
        assert_eq!(consumer.slots(), 4);
        let mut out = [0.0f32; 4];
        let (filled, _) = consumer.pop_partial_slice(&mut out);
        assert_eq!(filled, samples);
    }

    #[test]
    fn record_block_full_ring_drops_whole_frames_only() {
        // Capacity 5 slots: with 2 channels only 2 whole frames fit (4 slots);
        // the 5th slot is never filled with half a frame.
        let (mut producer, mut consumer) = RingBuffer::new(5);
        let samples = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]; // 4 frames
        let dropped = record_block(&samples, 2, true, &mut producer);
        assert_eq!(dropped, 2);
        assert_eq!(consumer.slots(), 4);
        let mut out = [0.0f32; 5];
        let (filled, _) = consumer.pop_partial_slice(&mut out);
        assert_eq!(&*filled, &samples[..4]);
    }

    #[test]
    fn record_block_never_pushes_a_partial_trailing_frame() {
        let (mut producer, mut consumer) = RingBuffer::new(16);
        let samples = [0.1f32, 0.2, 0.3]; // 1 frame + 1 stray sample
        let dropped = record_block(&samples, 2, true, &mut producer);
        assert_eq!(dropped, 0);
        assert_eq!(consumer.slots(), 2);
        let mut out = [0.0f32; 3];
        let (filled, _) = consumer.pop_partial_slice(&mut out);
        assert_eq!(&*filled, &samples[..2]);
    }

    #[test]
    fn record_block_converts_i16() {
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let samples = [16_384i16, -32_768];
        let dropped = record_block(&samples, 1, true, &mut producer);
        assert_eq!(dropped, 0);
        assert_eq!(consumer.slots(), 2);
        let mut out = [0.0f32; 2];
        let (filled, _) = consumer.pop_partial_slice(&mut out);
        assert_eq!(filled[0], 16_384.0 / 32_768.0);
        assert_eq!(filled[1], -1.0);
    }

    #[test]
    fn record_block_on_a_zero_capacity_ring_drops_everything() {
        let (mut producer, _consumer) = RingBuffer::new(0);
        let dropped = record_block(&[0.1f32, 0.2], 2, true, &mut producer);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn record_block_counts_dropped_frames_not_samples() {
        // 3-channel device: 6 slots fit 2 frames, so 9 samples (3 frames) lose 1.
        let (mut producer, consumer) = RingBuffer::new(6);
        let samples = [0.0f32; 9];
        let dropped = record_block(&samples, 3, true, &mut producer);
        assert_eq!(dropped, 1);
        assert_eq!(consumer.slots(), 6);
    }

    /// What one channel meter holds: (peak, mean square, frames, clipped).
    fn read(meter: &ChannelMeter) -> (f32, f32, u32, bool) {
        let (peak, sum_sq, frames, clipped) = meter.take();
        let mean_sq = if frames == 0 {
            0.0
        } else {
            sum_sq / frames as f32
        };
        (peak, mean_sq, frames, clipped)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn mono_pick_reads_channel_0_and_feeds_both_sides() {
        // Interleaved stereo: ch0 = 0.5, -0.25; ch1 = 0.9, 0.1.
        let samples = [0.5f32, 0.9, -0.25, 0.1];
        let meter = StereoMeter::new();
        meter_block(&samples, 2, InputPick::Mono(0), &meter);
        let (lp, lms, lf, lc) = read(&meter.l);
        let (rp, rms, rf, rc) = read(&meter.r);
        assert!(close(lp, 0.5) && close(rp, 0.5));
        assert!(close(lms, (0.25 + 0.0625) / 2.0) && close(rms, lms));
        assert_eq!((lf, rf), (2, 2));
        assert!(!lc && !rc);
    }

    #[test]
    fn mono_pick_reads_channel_1() {
        let samples = [0.5f32, 0.9, -0.25, -0.1];
        let meter = StereoMeter::new();
        meter_block(&samples, 2, InputPick::Mono(1), &meter);
        let (lp, _, lf, _) = read(&meter.l);
        let (rp, _, rf, _) = read(&meter.r);
        assert!(close(lp, 0.9) && close(rp, 0.9));
        assert_eq!((lf, rf), (2, 2));
    }

    #[test]
    fn stereo_pick_meters_left_and_right_separately() {
        let samples = [0.5f32, -0.8, -0.25, 0.1];
        let meter = StereoMeter::new();
        meter_block(&samples, 2, InputPick::Stereo, &meter);
        let (lp, lms, lf, _) = read(&meter.l);
        let (rp, rms, rf, _) = read(&meter.r);
        assert!(close(lp, 0.5), "left peak {lp}");
        assert!(close(rp, 0.8), "right peak {rp}");
        assert!(close(lms, (0.25 + 0.0625) / 2.0));
        assert!(close(rms, (0.64 + 0.01) / 2.0));
        assert_eq!((lf, rf), (2, 2));
    }

    #[test]
    fn extra_device_channels_are_skipped_by_the_pick() {
        // 3 channels: ch0 = 0.1, ch1 = 0.2, ch2 = 0.7.
        let samples = [0.1f32, 0.2, 0.7, -0.1, -0.2, -0.7];
        let meter = StereoMeter::new();
        meter_block(&samples, 3, InputPick::Stereo, &meter);
        assert!(close(read(&meter.l).0, 0.1));
        assert!(close(read(&meter.r).0, 0.2));
        meter_block(&samples, 3, InputPick::Mono(2), &meter);
        assert!(close(read(&meter.l).0, 0.7));
        assert!(close(read(&meter.r).0, 0.7));
    }

    #[test]
    fn i16_samples_are_scaled_to_f32() {
        // 16384 = 0.5, -32768 = -1.0 (full scale, so it clips).
        let samples = [16_384i16, -32_768];
        let meter = StereoMeter::new();
        meter_block(&samples, 1, InputPick::Mono(0), &meter);
        let (peak, mean_sq, frames, clipped) = read(&meter.l);
        assert!(close(peak, 1.0));
        assert!(close(mean_sq, (0.25 + 1.0) / 2.0));
        assert_eq!(frames, 2);
        assert!(clipped);
        assert_eq!(0i16.to_f32(), 0.0);
        assert_eq!(i16::MAX.to_f32(), 32_767.0 / 32_768.0);
    }

    #[test]
    fn u16_samples_are_centred_and_scaled_to_f32() {
        assert_eq!(32_768u16.to_f32(), 0.0);
        assert_eq!(49_152u16.to_f32(), 0.5);
        assert_eq!(0u16.to_f32(), -1.0);
        let samples = [49_152u16, 16_384];
        let meter = StereoMeter::new();
        meter_block(&samples, 1, InputPick::Mono(0), &meter);
        let (peak, _, frames, clipped) = read(&meter.r);
        assert!(close(peak, 0.5));
        assert_eq!(frames, 2);
        assert!(!clipped);
    }

    #[test]
    fn silence_has_no_peak_and_no_clip() {
        let meter = StereoMeter::new();
        meter_block(&[0.0f32; 64], 2, InputPick::Stereo, &meter);
        for channel in [&meter.l, &meter.r] {
            assert_eq!(read(channel), (0.0, 0.0, 32, false));
        }
        meter_block(&[0i16; 8], 1, InputPick::Mono(0), &meter);
        assert_eq!(read(&meter.l), (0.0, 0.0, 8, false));
    }

    #[test]
    fn full_scale_f32_sets_the_clip_flag() {
        let meter = StereoMeter::new();
        meter_block(&[0.2f32, 1.0, -0.3, 0.0], 2, InputPick::Stereo, &meter);
        assert!(!read(&meter.l).3);
        assert!(read(&meter.r).3);
    }

    #[test]
    fn blocks_accumulate_until_the_meter_is_taken() {
        let meter = StereoMeter::new();
        meter_block(&[0.25f32; 4], 1, InputPick::Mono(0), &meter);
        meter_block(&[0.5f32; 6], 1, InputPick::Mono(0), &meter);
        let (peak, sum_sq, frames, _) = meter.l.take();
        assert!(close(peak, 0.5));
        assert!(close(sum_sq, 4.0 * 0.0625 + 6.0 * 0.25));
        assert_eq!(frames, 10);
    }

    #[test]
    fn a_pick_the_device_cannot_serve_feeds_nothing() {
        let meter = StereoMeter::new();
        // Mono(1) and Stereo on a 1-channel device, Mono(2) on 2 channels.
        meter_block(&[0.5f32; 4], 1, InputPick::Mono(1), &meter);
        meter_block(&[0.5f32; 4], 1, InputPick::Stereo, &meter);
        meter_block(&[0.5f32; 4], 2, InputPick::Mono(2), &meter);
        // Zero channels must not divide by zero.
        meter_block(&[0.5f32; 4], 0, InputPick::Mono(0), &meter);
        meter_block(&[0.5f32; 4], 0, InputPick::Stereo, &meter);
        assert_eq!(read(&meter.l), (0.0, 0.0, 0, false));
        assert_eq!(read(&meter.r), (0.0, 0.0, 0, false));
    }

    #[test]
    fn empty_and_partial_blocks_are_safe() {
        let meter = StereoMeter::new();
        meter_block::<f32>(&[], 2, InputPick::Stereo, &meter);
        assert_eq!(read(&meter.l).2, 0);
        // 5 samples at 2 channels: the trailing half frame is ignored.
        meter_block(&[0.5f32, 0.5, 0.5, 0.5, 0.9], 2, InputPick::Mono(0), &meter);
        let (peak, _, frames, _) = read(&meter.l);
        assert!(close(peak, 0.5));
        assert_eq!(frames, 2);
    }

    #[test]
    fn non_finite_samples_count_as_silence() {
        let meter = StereoMeter::new();
        let samples = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25];
        meter_block(&samples, 1, InputPick::Mono(0), &meter);
        let (peak, sum_sq, frames, clipped) = meter.l.take();
        assert!(close(peak, 0.25));
        assert!(close(sum_sq, 0.0625));
        assert_eq!(frames, 4);
        assert!(!clipped);
    }

    #[test]
    fn pick_reports_the_channels_it_needs() {
        assert_eq!(InputPick::Mono(0).channels_needed(), 1);
        assert_eq!(InputPick::Mono(1).channels_needed(), 2);
        assert_eq!(InputPick::Stereo.channels_needed(), 2);
        assert_eq!(InputPick::Mono(u16::MAX).channels_needed(), u16::MAX);
    }

    #[test]
    fn only_a_stopped_stream_is_a_fatal_error() {
        for fatal in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::BackendError,
            ErrorKind::DeviceBusy,
            ErrorKind::Other,
        ] {
            assert!(is_fatal(fatal), "{fatal:?}");
        }
        for harmless in [
            ErrorKind::DeviceChanged,
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
        ] {
            assert!(!is_fatal(harmless), "{harmless:?}");
        }
    }

    /// A supported config with a fixed buffer range.
    fn config(channels: u16, rate: u32, format: SampleFormat) -> SupportedStreamConfig {
        SupportedStreamConfig::new(
            channels,
            rate,
            SupportedBufferSize::Range { min: 64, max: 4096 },
            format,
        )
    }

    /// A single-rate supported range.
    fn range(channels: u16, rate: u32, format: SampleFormat) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(
            channels,
            rate,
            rate,
            SupportedBufferSize::Range { min: 64, max: 4096 },
            format,
        )
    }

    #[test]
    fn best_config_keeps_an_f32_default() {
        let default = config(2, 48_000, SampleFormat::F32);
        let others = [range(2, 48_000, SampleFormat::I16)];
        assert_eq!(best_config(default, others), Some(default));
    }

    #[test]
    fn best_config_prefers_f32_then_i16_at_the_same_channels_and_rate() {
        let default = config(2, 48_000, SampleFormat::U16);
        let others = [
            range(2, 48_000, SampleFormat::I16),
            range(2, 44_100, SampleFormat::F32), // wrong rate
            range(1, 48_000, SampleFormat::F32), // wrong channel count
        ];
        let best = best_config(default, others).unwrap();
        assert_eq!(best.sample_format(), SampleFormat::I16);
        assert_eq!((best.channels(), best.sample_rate()), (2, 48_000));

        let others = [
            range(2, 48_000, SampleFormat::I16),
            range(2, 48_000, SampleFormat::F32),
        ];
        let best = best_config(default, others).unwrap();
        assert_eq!(best.sample_format(), SampleFormat::F32);
    }

    #[test]
    fn best_config_converts_an_unreadable_default_when_an_alternative_exists() {
        let default = config(2, 48_000, SampleFormat::I32);
        let best = best_config(default, [range(2, 48_000, SampleFormat::F32)]).unwrap();
        assert_eq!(best.sample_format(), SampleFormat::F32);
    }

    #[test]
    fn best_config_is_none_without_a_readable_format() {
        let default = config(2, 48_000, SampleFormat::I24);
        let others = [
            range(2, 48_000, SampleFormat::I32),
            range(2, 48_000, SampleFormat::F64),
        ];
        assert_eq!(best_config(default, others), None);
    }
}
