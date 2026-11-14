//! audio::engine: the cpal output stream and its real-time callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, OutputCallbackInfo, SampleFormat, Stream, StreamConfig, SupportedBufferSize,
    SupportedStreamConfig, SupportedStreamConfigRange,
};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use thiserror::Error;

use crate::audio::input::device_name;
use crate::audio::mix_plan::MixPlan;
use crate::audio::player::{Command, Player, State};
use crate::audio::resampler::DeviceResampler;

/// Name shown when the output device cannot report one.
const UNKNOWN_DEVICE: &str = "Unknown output device";
/// Capacity of the editor-to-audio command queue.
const QUEUE_CAP: usize = 64;
/// Preallocated callback scratch, in stereo frames (at least 8,192).
const SCRATCH_FRAMES: usize = 8_192;
/// Internal pull capacity of the resampler, minus its interpolation margin.
const MAX_PULL_FRAMES: usize = 16_384 - 8;

/// Errors from starting or running the audio engine.
#[derive(Debug, Error)]
pub enum AudioError {
    /// No default output device is available.
    #[error("no output device found")]
    NoDevice,
    /// The device offers no f32 output format.
    #[error("output device \"{0}\" has no f32 sample format")]
    NoF32(String),
    /// Querying or configuring the device failed.
    #[error("output device error: {0}")]
    Device(String),
    /// The stream could not be built or started.
    #[error("failed to start the output stream: {0}")]
    Stream(#[from] cpal::Error),
    /// No input device is available.
    #[error("no input device found")]
    NoInputDevice,
    /// The input device offers no f32, i16 or u16 sample format.
    #[error("input device \"{0}\" has no supported sample format (needs f32, i16 or u16)")]
    NoInputFormat(String),
    /// The chosen input needs more channels than the device has.
    #[error("input device \"{device}\" has {have} input channel(s), but this input needs {need}")]
    InputChannels {
        /// Display name of the device.
        device: String,
        /// Channels the device offers.
        have: u16,
        /// Channels the chosen input needs.
        need: u16,
    },
    /// The input stream could not be built or started.
    #[error("failed to start the input stream: {0}")]
    InputStream(cpal::Error),
}

/// Transport state the audio thread publishes for the UI to read.
pub struct Shared {
    /// Timeline playhead in project samples.
    pub playhead: AtomicU64,
    /// Preview playhead in preview samples.
    pub preview_pos: AtomicU64,
    /// Transport state code: 0 stopped, 1 playing, 2 paused, 3 previewing, 4 scrubbing.
    pub state: AtomicU8,
}

/// The cpal output stream plus the shared state the audio callback reads.
///
/// Dropping the `Engine` (that is, the `Editor`) stops the stream: cpal
/// joins the callback when the `Stream` is dropped.
pub struct Engine {
    /// The live output stream; dropping it stops playback.
    _stream: Stream,
    /// Bounded command queue into the callback; commands are dropped when full.
    cmd_tx: Sender<Command>,
    /// Latest timeline mix plan.
    timeline: Arc<ArcSwap<MixPlan>>,
    /// Latest bin-preview mix plan.
    preview: Arc<ArcSwap<MixPlan>>,
    /// Transport state published to the UI.
    shared: Arc<Shared>,
    /// Human-readable device summary, e.g. `"48000 Hz"`.
    device_info: String,
    /// Set by the stream error callback instead of logging.
    errored: Arc<AtomicBool>,
}

/// Name of the default output device, for the start screen status bar.
pub fn default_output_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|device| device_name(&device))
}

impl Engine {
    /// Opens the default output device and starts the stream.
    ///
    /// Prefers an f32 config at `project_rate` with 2 channels. When the
    /// device cannot run at the project rate it runs at its own default rate
    /// and a [`DeviceResampler`] converts the mix. The callback scratch is
    /// fully allocated before the stream starts.
    pub fn start(project_rate: u32) -> Result<Engine, AudioError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(AudioError::NoDevice)?;
        let configs: Vec<SupportedStreamConfigRange> = device
            .supported_output_configs()
            .map_err(|e| AudioError::Device(e.to_string()))?
            .filter(|c| c.sample_format() == SampleFormat::F32)
            .collect();
        let name = device_name(&device).unwrap_or_else(|| UNKNOWN_DEVICE.to_string());
        // An f32 config at the project rate, else fall back to the device's
        // own default rate and resample the mix into it.
        let (supported, resampler, device_rate) = match pick_config(&configs, project_rate) {
            Some(config) => {
                let rate = config.sample_rate();
                (config, None, rate)
            }
            None => {
                let default = device
                    .default_output_config()
                    .map_err(|e| AudioError::Device(e.to_string()))?;
                let (supported, device_rate) = if default.sample_format() == SampleFormat::F32 {
                    (default, default.sample_rate())
                } else {
                    let range = configs
                        .first()
                        .copied()
                        .ok_or_else(|| AudioError::NoF32(name.clone()))?;
                    (range.with_max_sample_rate(), range.max_sample_rate())
                };
                let resampler = DeviceResampler::new(project_rate, device_rate);
                (supported, Some(resampler), device_rate)
            }
        };
        let ratio = project_rate as f64 / device_rate.max(1) as f64;
        Self::open(
            device,
            supported,
            resampler,
            ratio,
            device_rate,
            project_rate,
        )
    }

    /// Builds the stream for the chosen device config and assembles the engine.
    fn open(
        device: cpal::Device,
        supported: SupportedStreamConfig,
        resampler: Option<DeviceResampler>,
        ratio: f64,
        device_rate: u32,
        project_rate: u32,
    ) -> Result<Engine, AudioError> {
        // Fixed buffer of about 20 ms when the device's supported range allows it.
        let mut stream_config = StreamConfig::from(supported);
        let want = ((device_rate as f64 * 0.02) as u32).max(1);
        if let SupportedBufferSize::Range { min, max } = *supported.buffer_size()
            && want >= min
            && want <= max
        {
            stream_config.buffer_size = BufferSize::Fixed(want);
        }

        let shared = Arc::new(Shared {
            playhead: AtomicU64::new(0),
            preview_pos: AtomicU64::new(0),
            state: AtomicU8::new(state_code(State::Stopped)),
        });
        let errored = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = bounded::<Command>(QUEUE_CAP);
        let timeline = Arc::new(ArcSwap::from(Arc::new(MixPlan::empty(project_rate))));
        let preview = Arc::new(ArcSwap::from(Arc::new(MixPlan::empty(project_rate))));

        // Every scratch buffer is allocated before the stream starts.
        let channels = supported.channels();
        let scratch = vec![0.0f32; SCRATCH_FRAMES * 2];
        let player = Player::new(project_rate);
        let mut callback = Callback {
            player,
            scratch,
            resampler,
            ratio,
            cmd_rx,
            timeline: Arc::clone(&timeline),
            preview: Arc::clone(&preview),
            shared: Arc::clone(&shared),
            channels,
        };

        let errored_for_cb = Arc::clone(&errored);
        let error_callback = move |_err: cpal::Error| {
            errored_for_cb.store(true, Ordering::Relaxed);
        };
        // Debug builds warn on stderr about any allocation inside the callback;
        // in release builds assert_no_alloc is a plain call (disable_release).
        let stream = device.build_output_stream(
            stream_config,
            move |data: &mut [f32], _info: &OutputCallbackInfo| {
                assert_no_alloc::assert_no_alloc(|| callback.run(data));
            },
            error_callback,
            None,
        )?;
        stream.play()?;

        let device_info = if device_rate == project_rate {
            format!("{project_rate} Hz")
        } else {
            format!("{device_rate} Hz device, resampling from {project_rate} Hz")
        };
        Ok(Engine {
            _stream: stream,
            cmd_tx,
            timeline,
            preview,
            shared,
            device_info,
            errored,
        })
    }

    /// Sends a transport command; it is dropped when the queue is full.
    pub fn send(&self, cmd: Command) {
        if let Err(TrySendError::Full(_)) = self.cmd_tx.try_send(cmd) {
            // A full queue means the audio thread is stalled; dropping the
            // command beats blocking the UI thread.
        }
    }

    /// Publishes a new timeline plan and returns the retired one.
    pub fn publish_timeline(&self, plan: Arc<MixPlan>) -> Arc<MixPlan> {
        self.timeline.swap(plan)
    }

    /// Publishes a new preview plan and returns the retired one.
    pub fn publish_preview(&self, plan: Arc<MixPlan>) -> Arc<MixPlan> {
        self.preview.swap(plan)
    }

    /// Transport state shared with the UI.
    pub fn shared(&self) -> Arc<Shared> {
        Arc::clone(&self.shared)
    }

    /// Human-readable device summary, e.g. `"48000 Hz"`.
    pub fn device_info(&self) -> &str {
        &self.device_info
    }

    /// True once the stream error callback has fired.
    pub fn errored(&self) -> bool {
        self.errored.load(Ordering::Relaxed)
    }
}

/// Picks the best f32 output config: 2 channels at the project rate first,
/// then any channel count at the project rate, else `None`.
fn pick_config(
    configs: &[SupportedStreamConfigRange],
    project_rate: u32,
) -> Option<SupportedStreamConfig> {
    let at_rate = |c: &SupportedStreamConfigRange| c.contains_rate(project_rate);
    configs
        .iter()
        .find(|c| c.channels() == 2 && at_rate(c))
        .or_else(|| configs.iter().find(|c| at_rate(c)))
        .and_then(|c| c.try_with_sample_rate(project_rate))
}

/// Maps a transport state to the `Shared::state` code.
fn state_code(state: State) -> u8 {
    match state {
        State::Stopped => 0,
        State::Playing => 1,
        State::Paused => 2,
        State::Previewing => 3,
        State::Scrubbing => 4,
    }
}

/// State owned by the audio callback, created on the UI thread but moved
/// into the closure before the stream starts.
struct Callback {
    /// Transport state machine and renderer.
    player: Player,
    /// Preallocated stereo scratch, `SCRATCH_FRAMES * 2` samples.
    scratch: Vec<f32>,
    /// Device-rate resampler when the device runs at its own rate.
    resampler: Option<DeviceResampler>,
    /// Conversion ratio `project_rate / device_rate` (1.0 when direct).
    ratio: f64,
    /// Command queue drained each callback.
    cmd_rx: Receiver<Command>,
    /// Latest timeline plan.
    timeline: Arc<ArcSwap<MixPlan>>,
    /// Latest preview plan.
    preview: Arc<ArcSwap<MixPlan>>,
    /// Transport state written each callback.
    shared: Arc<Shared>,
    /// Device channel count.
    channels: u16,
}

impl Callback {
    /// One device callback: no allocation, no locks, no panics.
    fn run(&mut self, data: &mut [f32]) {
        // Drain the command queue without blocking.
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            self.player.handle(cmd);
        }
        // Load both plans with a guard; the UI keeps retired plans alive for
        // 2 s so the last reference is never released here.
        let timeline = self.timeline.load();
        let preview = self.preview.load();
        let channels = usize::from(self.channels).max(1);
        let frames = data.len() / channels;
        let chunk_cap = self.chunk_frames();
        let mut done = 0usize;
        while done < frames {
            let n = (frames - done).min(chunk_cap);
            let stereo = &mut self.scratch[..n * 2];
            match &mut self.resampler {
                None => self.player.process(&timeline, &preview, stereo),
                Some(res) => res.fill(stereo, &mut |buf: &mut [f32]| {
                    self.player.process(&timeline, &preview, buf);
                }),
            }
            let target = &mut data[done * channels..(done + n) * channels];
            write_channels(target, stereo, self.channels);
            done += n;
        }
        self.shared
            .playhead
            .store(self.player.position(), Ordering::Relaxed);
        self.shared
            .preview_pos
            .store(self.player.preview_position(), Ordering::Relaxed);
        self.shared
            .state
            .store(state_code(self.player.state()), Ordering::Relaxed);
    }

    /// Largest callback chunk the scratch can serve without overflowing the
    /// resampler's internal pull buffer, in frames.
    fn chunk_frames(&self) -> usize {
        let stereo_cap = self.scratch.len() / 2;
        match self.resampler {
            None => stereo_cap,
            Some(_) => stereo_cap
                .min(((MAX_PULL_FRAMES as f64) / self.ratio.max(1.0)) as usize)
                .max(1),
        }
    }
}

/// Maps interleaved stereo into the device layout: `1` channel averages L and
/// R, more than `2` channels zero every channel after the first two.
fn write_channels(target: &mut [f32], stereo: &[f32], channels: u16) {
    let frames = stereo.len() / 2;
    match channels {
        0 => {}
        1 => {
            for i in 0..frames {
                target[i] = (stereo[i * 2] + stereo[i * 2 + 1]) * 0.5;
            }
        }
        2 => {
            target[..frames * 2].copy_from_slice(stereo);
        }
        n => {
            let n = usize::from(n);
            for i in 0..frames {
                target[i * n] = stereo[i * 2];
                target[i * n + 1] = stereo[i * 2 + 1];
                for ch in 2..n {
                    target[i * n + ch] = 0.0;
                }
            }
        }
    }
}
