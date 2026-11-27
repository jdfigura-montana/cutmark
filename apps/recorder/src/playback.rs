//! The review playback engine: a take's video decoder, its mixdown audio decoder and the
//! output stream that drives the shared playback clock.
//!
//! No UI: the review panels call this API every frame and pull the picture with
//! [`Playback::current_frame`]. When the file has audio and an output device exists, the
//! clock counts the stereo frames the callback actually hands to the device minus the
//! latency the device reports; without one, or when the device or the audio decoder fails,
//! it falls back to an `Instant` clock and keeps the video playing.

use std::cell::Cell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{OutputCallbackInfo, SampleFormat, Stream, StreamConfig, SupportedStreamConfig, SupportedStreamConfigRange};
use media_io::MediaError;
use media_io::player::audio::AudioDecoder;
use media_io::player::probe::probe;
use media_io::player::video::{RgbaFrame, VideoDecoder};
use thiserror::Error;

use crate::review;

/// Errors from opening a take for playback.
#[derive(Debug, Error)]
pub enum PlaybackError {
    /// The file could not be probed or its video decoded.
    #[error(transparent)]
    Media(#[from] MediaError),
}

/// Frames are scaled to at most this width for the review preview.
const PREVIEW_MAX_WIDTH: u32 = 1280;

/// How many stereo frames the callback scratch holds; the device buffer never exceeds it.
const SCRATCH_FRAMES: usize = 8_192;

/// The counters and play state the output callback shares with the playback.
struct DeviceShared {
    /// Stereo frames the callback handed to the device so far, at the decoder rate.
    handed: AtomicU64,
    /// The device latency the last callback reported, in nanoseconds.
    latency_ns: AtomicU64,
    /// Whether the callback may hand samples; false while paused.
    playing: AtomicBool,
    /// Set by the stream error callback when the device failed.
    errored: AtomicBool,
}

/// What the playback position is measured from.
enum Clock {
    /// The output callback counts the stereo frames it actually hands to the device.
    Audio(SampleClock),
    /// No usable output stream: an `Instant` that only runs while playing.
    Wall(WallClock),
}

impl Clock {
    /// The position in milliseconds; the caller clamps it to the take's length.
    fn position_ms(&self, playing: bool) -> u64 {
        match self {
            Self::Audio(clock) => clock.position_ms(),
            Self::Wall(clock) => clock.position_ms(playing),
        }
    }

    /// Sets the clock to `ms`; `latency_ms` compensates the device latency the audio clock
    /// subtracts, and `playing` restarts the wall clock only when it runs.
    fn seek(&self, ms: u64, latency_ms: u64, playing: bool) {
        match self {
            Self::Audio(clock) => clock.set_seek(ms, latency_ms),
            Self::Wall(clock) => clock.seek(ms, playing),
        }
    }
}

/// The audio clock: counts the stereo frames the output callback hands to the device.
struct SampleClock {
    /// The counters shared with the callback.
    shared: Arc<DeviceShared>,
    /// The rate the handed samples count at, in hertz.
    rate: u32,
    /// The position the clock was last set to, plus the latency it compensates.
    seek_ms: Cell<u64>,
    /// The handed counter at the last seek.
    handed_at_seek: Cell<u64>,
}

impl SampleClock {
    /// A clock at position zero for the shared counters.
    fn new(shared: Arc<DeviceShared>, rate: u32) -> Self {
        Self { shared, rate, seek_ms: Cell::new(0), handed_at_seek: Cell::new(0) }
    }

    /// Positions the clock at `ms`, compensating the device latency so the reported position
    /// lands on the seek target.
    fn set_seek(&self, ms: u64, latency_ms: u64) {
        self.seek_ms.set(ms.saturating_add(latency_ms));
        self.handed_at_seek.set(self.shared.handed.load(Ordering::Relaxed));
    }

    /// The position: the seek base plus the samples handed since, at the clock's rate, minus
    /// the device latency the last callback reported.
    fn position_ms(&self) -> u64 {
        let handed = self.shared.handed.load(Ordering::Relaxed);
        let latency_ms = self.shared.latency_ns.load(Ordering::Relaxed) / 1_000_000;
        let delta = handed.saturating_sub(self.handed_at_seek.get());
        self.seek_ms.get().saturating_add(samples_to_ms(delta, self.rate)).saturating_sub(latency_ms)
    }
}

/// Milliseconds for a count of samples at `rate`.
fn samples_to_ms(samples: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    u64::try_from(u128::from(samples) * 1000 / u128::from(rate)).unwrap_or(u64::MAX)
}

/// The fallback clock without an output stream: an `Instant` that only runs while playing.
struct WallClock {
    /// When the clock last started running, `None` while paused.
    since: Cell<Option<Instant>>,
    /// The position the clock was last set to or froze at, in milliseconds.
    base_ms: Cell<u64>,
}

impl WallClock {
    /// A clock at position zero.
    fn new() -> Self {
        Self { since: Cell::new(None), base_ms: Cell::new(0) }
    }

    /// The position: the base plus the time the clock has been running.
    fn position_ms(&self, playing: bool) -> u64 {
        let run = if playing { self.since.get().map(|since| since.elapsed().as_millis()).unwrap_or(0) } else { 0 };
        self.base_ms.get().saturating_add(u64::try_from(run).unwrap_or(u64::MAX))
    }

    /// Starts the clock from the position it froze at.
    fn play(&self) {
        self.since.set(Some(Instant::now()));
    }

    /// Freezes the clock at its current position.
    fn pause(&self) {
        if let Some(since) = self.since.take() {
            let elapsed = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
            self.base_ms.set(self.base_ms.get().saturating_add(elapsed));
        }
    }

    /// Positions the clock at `ms`, running from now while playing.
    fn seek(&self, ms: u64, playing: bool) {
        self.base_ms.set(ms);
        self.since.set(if playing { Some(Instant::now()) } else { None });
    }
}

/// One take playing back: its video decoder, its mixdown audio decoder and the output stream
/// that drives the shared clock. Dropping it stops the stream and joins both decoders.
pub struct Playback {
    /// The live output stream; dropped before the decoders, so the callback joins first.
    stream: Option<Stream>,
    /// The mixdown audio decoder, when the file has audio and an output device exists.
    audio: Option<Arc<AudioDecoder>>,
    /// The video decoder, when the file has video.
    video: Option<VideoDecoder>,
    /// The counters and play state shared with the callback.
    shared: Arc<DeviceShared>,
    /// What the position is measured from.
    clock: Clock,
    /// The length of the take in milliseconds (see [`review::take_duration`]); 0 when unknown.
    duration_ms: u64,
    /// Whether the clock runs.
    playing: bool,
    /// Whether playback already fell back to the wall clock, so a failure reports once.
    fallback: bool,
    /// The audio problem that made playback fall back to the wall clock, to report once.
    device_error: Option<String>,
}

impl Playback {
    /// Opens `path` for review: decodes video at most [`PREVIEW_MAX_WIDTH`] wide on its own
    /// thread and, when the file has audio and an output device exists, the mixdown stream
    /// on another. A dead or missing output device only falls back to the wall clock; the
    /// reason is reported once through [`Playback::take_device_error`].
    pub fn open(path: &Path) -> Result<Self, PlaybackError> {
        let info = probe(path)?;
        let video = if info.video.is_some() { Some(VideoDecoder::open(path, PREVIEW_MAX_WIDTH)?) } else { None };
        let shared = Arc::new(DeviceShared {
            handed: AtomicU64::new(0),
            latency_ns: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            errored: AtomicBool::new(false),
        });
        let mut device_error = None;
        let opened = if info.audio.is_empty() { None } else { Some(open_audio(path, &shared)) };
        let (audio, stream, rate) = match opened {
            Some(Ok((audio, stream, rate))) => (Some(audio), Some(stream), Some(rate)),
            Some(Err(reason)) => {
                log::warn!("playback falls back to the wall clock: {reason}");
                device_error = Some(reason);
                (None, None, None)
            }
            None => (None, None, None),
        };
        let clock = match (audio.as_ref(), rate) {
            (Some(_), Some(rate)) => Clock::Audio(SampleClock::new(Arc::clone(&shared), rate)),
            _ => Clock::Wall(WallClock::new()),
        };
        let duration_ms = review::take_duration(path, Some(info.duration_ms), review::load_sidecar(path).as_ref()).unwrap_or(0);
        Ok(Self { stream, audio, video, shared, clock, duration_ms, playing: false, fallback: false, device_error })
    }

    /// Resumes the clock and both decoders.
    pub fn play(&mut self) {
        self.playing = true;
        self.shared.playing.store(true, Ordering::Relaxed);
        if let Some(audio) = &self.audio {
            audio.play();
        }
        if let Some(video) = &self.video {
            video.play();
        }
        if let Clock::Wall(clock) = &self.clock {
            clock.play();
        }
    }

    /// Freezes the clock and both decoders.
    pub fn pause(&mut self) {
        self.playing = false;
        self.shared.playing.store(false, Ordering::Relaxed);
        if let Some(audio) = &self.audio {
            audio.pause();
        }
        if let Some(video) = &self.video {
            video.pause();
        }
        if let Clock::Wall(clock) = &self.clock {
            clock.pause();
        }
    }

    /// Plays or pauses.
    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Seeks both decoders and sets the clock to `ms`, clamped to the take's length.
    pub fn seek(&mut self, ms: u64) {
        let ms = if self.duration_ms > 0 { ms.min(self.duration_ms) } else { ms };
        if let Some(video) = &self.video {
            video.seek(ms);
        }
        if let Some(audio) = &self.audio {
            audio.seek(ms);
        }
        let latency_ms = self.shared.latency_ns.load(Ordering::Relaxed) / 1_000_000;
        self.clock.seek(ms, latency_ms, self.playing);
    }

    /// The position the clock reports, clamped to the take's length.
    pub fn position_ms(&self) -> u64 {
        let position = self.clock.position_ms(self.playing);
        if self.duration_ms > 0 { position.min(self.duration_ms) } else { position }
    }

    /// The take's length in milliseconds, the one the File group shows.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Whether the clock is running.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The newest video frame at or before the clock, pausing the take when it reaches its
    /// end. A lost output device or a failed audio decoder is noticed once: the clock
    /// switches to the wall clock and the reason is reported through
    /// [`Playback::take_device_error`], the video keeps going.
    pub fn current_frame(&mut self) -> Option<RgbaFrame> {
        if !self.fallback {
            if let Some(reason) = self.audio.as_ref().and_then(|audio| audio.take_error()) {
                self.fallback_to_wall_clock(&reason);
            } else if self.shared.errored.load(Ordering::Relaxed) {
                self.fallback_to_wall_clock("the output device was lost");
            }
        }
        let position = self.position_ms();
        if self.playing && end_of_file(position, self.duration_ms) {
            self.pause();
        }
        self.video.as_ref().and_then(|video| video.frame_at(position))
    }

    /// The caption for the audio problem (device or decoder) that made playback fall back to
    /// the wall clock, if it happened; reported once, `None` afterwards.
    pub fn take_device_error(&mut self) -> Option<String> {
        self.device_error.take().map(|reason| format!("Audio could not be played: {reason}"))
    }

    /// Stops playback and closes the output stream and both decoders.
    pub fn close(&mut self) {
        self.playing = false;
        self.shared.playing.store(false, Ordering::Relaxed);
        self.stream = None;
        self.audio = None;
        self.video = None;
    }

    /// Switches to the wall clock at the current position and reports `reason` once.
    fn fallback_to_wall_clock(&mut self, reason: &str) {
        self.fallback = true;
        self.shared.errored.store(false, Ordering::Relaxed);
        let position = self.clock.position_ms(self.playing);
        let clock = WallClock::new();
        clock.base_ms.set(position);
        if self.playing {
            clock.play();
        }
        self.clock = Clock::Wall(clock);
        // The stream holds the decoder too, so it goes first.
        self.stream = None;
        self.audio = None;
        if self.device_error.is_none() {
            self.device_error = Some(reason.to_owned());
        }
        log::warn!("playback fell back to the wall clock: {reason}");
    }

    /// Whether the clock is the wall clock fallback; test hook for the no-audio take.
    #[cfg(test)]
    fn runs_on_the_wall_clock(&self) -> bool {
        matches!(self.clock, Clock::Wall(_))
    }
}

/// Whether a playing take of `duration_ms` has reached its end at `position_ms`.
fn end_of_file(position_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0 && position_ms >= duration_ms
}

/// Picks an f32 output config on the default device, opens the mixdown decoder at the
/// device's rate and starts the stream. Any failure returns the reason for the fallback.
fn open_audio(path: &Path, shared: &Arc<DeviceShared>) -> Result<(Arc<AudioDecoder>, Stream, u32), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device found")?;
    let configs: Vec<SupportedStreamConfigRange> = device
        .supported_output_configs()
        .map_err(|error| format!("the output device could not be queried: {error}"))?
        .filter(|range| range.sample_format() == SampleFormat::F32)
        .collect();
    let supported = pick_config(&configs).ok_or("the output device has no f32 sample format")?;
    let rate = supported.sample_rate();
    let audio = Arc::new(AudioDecoder::open(path, rate).map_err(|error| error.to_string())?);
    let mut callback = Callback {
        audio: Arc::clone(&audio),
        shared: Arc::clone(shared),
        scratch: vec![0.0f32; SCRATCH_FRAMES * 2].into_boxed_slice(),
        channels: supported.channels(),
        rate,
    };
    let error_shared = Arc::clone(shared);
    let stream = device
        .build_output_stream(
            StreamConfig::from(supported),
            move |data: &mut [f32], info: &OutputCallbackInfo| callback.run(data, info),
            move |_error: cpal::Error| error_shared.errored.store(true, Ordering::Relaxed),
            None,
        )
        .map_err(|error| format!("the output stream could not start: {error}"))?;
    stream.play().map_err(|error| format!("the output stream could not start: {error}"))?;
    Ok((audio, stream, rate))
}

/// Picks the best f32 output config: stereo at 48 kHz first, then any at 48 kHz, then any.
fn pick_config(configs: &[SupportedStreamConfigRange]) -> Option<SupportedStreamConfig> {
    let at_rate = |range: &SupportedStreamConfigRange| range.contains_rate(48_000);
    let stereo_at_rate = configs.iter().find(|range| range.channels() == 2 && at_rate(range)).or_else(|| configs.iter().find(|range| at_rate(range)));
    let any = configs
        .iter()
        .find(|range| range.channels() == 2)
        .map(|range| range.with_max_sample_rate())
        .or_else(|| configs.first().map(|range| range.with_max_sample_rate()));
    stereo_at_rate.and_then(|range| range.try_with_sample_rate(48_000)).or(any)
}

/// The state the output callback owns: it pulls decoder samples and counts what it hands out.
struct Callback {
    /// The mixdown decoder the samples come from.
    audio: Arc<AudioDecoder>,
    /// The counters shared with the playback clock.
    shared: Arc<DeviceShared>,
    /// Preallocated stereo scratch, `SCRATCH_FRAMES * 2` samples.
    scratch: Box<[f32]>,
    /// The device channel count.
    channels: u16,
    /// The device sample rate, for the latency fallback.
    rate: u32,
}

impl Callback {
    /// One device callback: pulls the mixdown and maps it into the device layout, or silence
    /// while paused. No allocation, no locks, no panics.
    fn run(&mut self, data: &mut [f32], info: &OutputCallbackInfo) {
        let channels = usize::from(self.channels.max(1));
        let frames = data.len() / channels;
        let mut handed = 0u64;
        if self.shared.playing.load(Ordering::Relaxed) {
            let mut done = 0usize;
            while done < frames {
                let wanted = (frames - done).min(self.scratch.len() / 2);
                let stereo = &mut self.scratch[..wanted * 2];
                let pulled = self.audio.pull(stereo);
                handed += pulled as u64;
                let target = &mut data[done * channels..(done + wanted) * channels];
                fill_channels(target, stereo, self.channels, pulled);
                if pulled < wanted {
                    // The decoder ran dry; the rest of the buffer stays silent.
                    for sample in data[(done + wanted) * channels..].iter_mut() {
                        *sample = 0.0;
                    }
                    break;
                }
                done += wanted;
            }
        } else {
            for sample in data.iter_mut() {
                *sample = 0.0;
            }
        }
        self.shared.handed.fetch_add(handed, Ordering::Relaxed);
        self.shared.latency_ns.store(callback_latency_ns(info, frames, self.rate), Ordering::Relaxed);
    }
}

/// Maps `pulled` interleaved stereo frames into the device layout and silences the rest of
/// the chunk: one channel averages L and R, two copy, more zero every channel after the
/// first two.
fn fill_channels(target: &mut [f32], stereo: &[f32], channels: u16, pulled: usize) {
    let count = usize::from(channels.max(1));
    for frame in 0..target.len() / count {
        let (left, right) = if frame < pulled { (stereo[frame * 2], stereo[frame * 2 + 1]) } else { (0.0, 0.0) };
        match count {
            1 => target[frame] = (left + right) * 0.5,
            2 => {
                target[frame * 2] = left;
                target[frame * 2 + 1] = right;
            }
            n => {
                target[frame * n] = left;
                target[frame * n + 1] = right;
                for channel in 2..n {
                    target[frame * n + channel] = 0.0;
                }
            }
        }
    }
}

/// The device latency of one callback in nanoseconds: the host's playback minus callback
/// timestamp difference, or the buffer size in samples when it gave no usable timestamp.
fn callback_latency_ns(info: &OutputCallbackInfo, frames: usize, rate: u32) -> u64 {
    let stamp = info.timestamp();
    let delta = stamp.playback.saturating_duration_since(stamp.callback);
    if !delta.is_zero() {
        return u64::try_from(delta.as_nanos()).unwrap_or(u64::MAX);
    }
    if frames == 0 || rate == 0 {
        return 0;
    }
    u64::try_from(frames as u128 * 1_000_000_000 / u128::from(rate)).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::Duration;

    use super::*;

    /// A fake output sink: hands stereo frames to the shared counter the way the device
    /// callback does, so the audio clock can be driven without a real device.
    struct FakeSink {
        /// The counters the clock reads.
        shared: Arc<DeviceShared>,
    }

    impl FakeSink {
        /// Hands `frames` stereo frames to the clock.
        fn hand(&self, frames: u64) {
            self.shared.handed.fetch_add(frames, Ordering::Relaxed);
        }

        /// Reports a device latency of `ms` milliseconds.
        fn report_latency(&self, ms: u64) {
            self.shared.latency_ns.store(ms * 1_000_000, Ordering::Relaxed);
        }
    }

    /// The counters and a clock and sink at 48 kHz, the rate of the recorder's takes.
    fn fake_clock() -> (Arc<DeviceShared>, FakeSink, SampleClock) {
        let shared = Arc::new(DeviceShared {
            handed: AtomicU64::new(0),
            latency_ns: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            errored: AtomicBool::new(false),
        });
        let sink = FakeSink { shared: Arc::clone(&shared) };
        let clock = SampleClock::new(Arc::clone(&shared), 48_000);
        (shared, sink, clock)
    }

    #[test]
    fn the_audio_clock_counts_what_the_fake_sink_hands() {
        let (_, sink, clock) = fake_clock();
        // The device latency is subtracted: a seek compensates it, so the position lands on
        // the target, and handed samples advance it at the clock's rate.
        sink.report_latency(20);
        clock.set_seek(0, 20);
        assert_eq!(clock.position_ms(), 0);
        sink.hand(48_000);
        assert_eq!(clock.position_ms(), 1_000, "one second of handed samples");
        // A device that reports more latency reports less position.
        sink.report_latency(40);
        assert_eq!(clock.position_ms(), 980);
        // Pausing hands nothing, so the position freezes even as time passes.
        sink.report_latency(20);
        let frozen = clock.position_ms();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(clock.position_ms(), frozen, "paused hands nothing");
        // A seek sets the position exactly and handed samples continue from there.
        clock.set_seek(1_000, 20);
        assert_eq!(clock.position_ms(), 1_000);
        sink.hand(4_800);
        assert_eq!(clock.position_ms(), 1_100, "100 ms more of handed samples");
    }

    #[test]
    fn the_end_of_the_file_is_reached_when_the_clock_passes_the_length() {
        let (_, sink, clock) = fake_clock();
        clock.set_seek(0, 0);
        // Stereo frames at 48 kHz: 23_000 frames are 479 ms of a 500 ms take.
        sink.hand(23_000);
        assert!(!end_of_file(clock.position_ms(), 500));
        sink.hand(2_000);
        assert!(end_of_file(clock.position_ms(), 500), "the clock passed the length");
        // Without a length there is no end.
        assert!(!end_of_file(clock.position_ms(), 0));
    }

    /// The shared test take: a small video-only AV1 MKV, encoded once per process.
    pub(crate) fn test_take() -> Result<&'static PathBuf, String> {
        static TAKE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
        TAKE.get_or_init(|| {
            let path = std::env::temp_dir().join(format!("cutmark_playback_test_{}__wall.mkv", std::process::id()));
            // AMF rejects heights that are not a multiple of 16, so the clip is 640x368.
            match encode_take(&path, 640, 368, 60, 30) {
                Ok(()) => Ok(path),
                Err(error) => Err(error.to_string()),
            }
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    /// Encodes a small video-only take with the AMF encoder for the wall-clock tests.
    fn encode_take(path: &Path, width: u32, height: u32, fps: u32, frames: u32) -> Result<(), Box<dyn std::error::Error>> {
        media_io::init()?;
        let gpu = capture::Gpu::create(None)?;
        let hw_device = media_io::D3d11Device::from_device(gpu.device())?;
        let pool = media_io::D3d11FramePool::new(&hw_device, width, height, 8)?;
        let config = media_io::EncoderConfig {
            codec: media_io::Codec::Av1,
            width,
            height,
            fps_num: fps,
            fps_den: 1,
            rate_control: media_io::RateControl::ConstantQp { qp_i: 28, qp_p: 30 },
            preset: media_io::Preset::Speed,
            keyframe_secs: 2.0,
        };
        let mut encoder = media_io::AmfEncoder::open(&config, &pool)?;
        use media_io::Encoder as _;
        let mut muxer = media_io::Muxer::create(path, media_io::Container::Mkv)?;
        muxer.add_video(encoder.parameters(), encoder.time_base(), (fps as i32, 1))?;
        muxer.start()?;
        for index in 0..frames {
            let mut frame = media_io::HwFrame::from_pool(&pool)?;
            frame.set_pts(i64::from(index));
            encoder.send(frame)?;
            let mut batch = Vec::new();
            encoder.receive(&mut |packet| batch.push(packet))?;
            for packet in batch {
                muxer.write(0, &packet)?;
            }
        }
        let mut batch = Vec::new();
        encoder.finish(&mut |packet| batch.push(packet))?;
        for packet in batch {
            muxer.write(0, &packet)?;
        }
        muxer.finish()?;
        Ok(())
    }

    #[test]
    fn a_take_without_audio_runs_on_the_wall_clock() -> Result<(), Box<dyn std::error::Error>> {
        let take = match test_take() {
            Ok(take) => take,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let mut playback = Playback::open(take)?;
        assert!(playback.runs_on_the_wall_clock(), "a video-only take needs no output device");
        assert!(playback.take_device_error().is_none(), "no device was touched");
        // The wall clock runs while playing and freezes while paused.
        playback.play();
        std::thread::sleep(Duration::from_millis(150));
        assert!(playback.is_playing());
        assert!(playback.position_ms() > 0, "the wall clock advanced");
        playback.pause();
        let frozen = playback.position_ms();
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(playback.position_ms(), frozen, "paused freezes the wall clock");
        // A seek sets the position exactly.
        playback.seek(200);
        assert_eq!(playback.position_ms(), 200);
        // The take ends at its length and pauses at the last frame.
        playback.play();
        let started = std::time::Instant::now();
        let mut frame = None;
        while playback.is_playing() && started.elapsed() < Duration::from_secs(5) {
            frame = playback.current_frame();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!playback.is_playing(), "the take paused at its end");
        assert_eq!(playback.position_ms(), playback.duration_ms());
        assert!(playback.current_frame().is_some(), "the last frame stays shown");
        drop(frame);
        Ok(())
    }

    /// Writes a tenth of a second of silence as a 16-bit mono WAV.
    fn write_silent_wav(path: &Path) -> std::io::Result<()> {
        let data = [0u8; 9_600];
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        for field in [1u16, 1] {
            bytes.extend(field.to_le_bytes());
        }
        for field in [48_000u32, 96_000] {
            bytes.extend(field.to_le_bytes());
        }
        for field in [2u16, 16] {
            bytes.extend(field.to_le_bytes());
        }
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        std::fs::write(path, bytes)
    }

    #[test]
    fn a_failed_resampler_leaves_the_video_playing_on_the_wall_clock() -> Result<(), Box<dyn std::error::Error>> {
        let take = match test_take() {
            Ok(take) => take,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let wav = std::env::temp_dir().join(format!("cutmark_playback_test_{}_refused.wav", std::process::id()));
        write_silent_wav(&wav)?;
        let mut playback = Playback::open(take)?;
        // Stand in for an opened output: the audio clock, with no device handing samples, and a
        // decoder whose resampler cannot start because its output rate is zero.
        playback.audio = Some(Arc::new(AudioDecoder::open(&wav, 0)?));
        playback.clock = Clock::Audio(SampleClock::new(Arc::clone(&playback.shared), 48_000));
        playback.play();
        let started = std::time::Instant::now();
        while !playback.runs_on_the_wall_clock() && started.elapsed() < Duration::from_secs(5) {
            playback.current_frame();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(playback.runs_on_the_wall_clock(), "the failed decoder gave up the clock");
        assert!(playback.is_playing(), "the video keeps playing");
        assert!(playback.audio.is_none(), "the decoder was stopped");
        let before = playback.position_ms();
        std::thread::sleep(Duration::from_millis(100));
        assert!(playback.position_ms() > before, "the wall clock advances");
        assert!(playback.current_frame().is_some(), "the picture arrives");
        let caption = playback.take_device_error().ok_or("the failure was not reported")?;
        assert!(caption.starts_with("Audio could not be played: "), "{caption}");
        playback.current_frame();
        assert_eq!(playback.take_device_error(), None, "reported once");
        let _ = std::fs::remove_file(wav);
        Ok(())
    }
}
