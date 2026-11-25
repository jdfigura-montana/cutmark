//! The "compose" thread: the screen and the camera meet here, at the profile's frame rate.
//!
//! The thread owns the screen capture and the [`capture::Compositor`], takes the latest screen frame and the
//! latest camera frame each tick, composes them at the profile's output size and keeps the newest result.
//! The preview and the recording's video thread both read that result, so what the window shows is what the
//! file gets. The thread runs while the window is shown and while a take is recording, and sleeps otherwise.
//!
//! The camera's frames are textures on the webcam host's device, so the thread moves to that device when the
//! layout needs the camera. A recording pins the device for the whole take.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture::{
    BackgroundSettings, CamColor, CaptureError, Compositor, Gpu, KeySettings, Layout, MonitorCapture, PipSettings, YuvMatrix, YuvRange, list_monitors,
};
use media_io::bgvideo::{LoopingVideo, VideoColor, VideoMatrix, VideoRange};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::core::Interface;

use crate::config::{CompositeMode, Profile};
use crate::webcam::{self, CamShare};

/// How long the thread sleeps between checks while it is switched off.
const IDLE_SLEEP: Duration = Duration::from_millis(50);

/// How long the thread waits for the first screen frame before it looks again.
const POLL_SLEEP: Duration = Duration::from_millis(5);

/// How long the thread waits after a failed capture or compositor start.
const RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// How long the thread waits after a failed frame.
const ERROR_BACKOFF: Duration = Duration::from_millis(500);

/// Everything the thread needs to know about the picture it composes.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposeConfig {
    /// Output width in pixels; 0 means the size of the captured monitor.
    pub width: u32,
    /// Output height in pixels; 0 means the size of the captured monitor.
    pub height: u32,
    /// How many frames are composed per second.
    pub fps: u32,
    /// Which pictures the composite shows.
    pub layout: Layout,
    /// Whether a take records in Clean mode, so its encoder gets a frame without the camera; the preview
    /// always shows `layout` in full.
    pub clean: bool,
    /// The camera's place and look in the picture in picture layout.
    pub pip: PipSettings,
    /// What the composite draws behind the camera.
    pub bg: BackgroundSettings,
    /// The chroma key of the camera.
    pub key: KeySettings,
    /// Whether the keyed camera shows its matte; only the window's preview ever sets this, never a take.
    pub matte: bool,
    /// Index into `capture::list_monitors()`.
    pub monitor: usize,
    /// Whether the mouse cursor is drawn.
    pub cursor: bool,
    /// The path of the background video, set only when the background kind is Video.
    pub video_path: String,
}

impl Default for ComposeConfig {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            fps: 60,
            layout: Layout::ScreenOnly,
            clean: false,
            pip: PipSettings::default(),
            bg: BackgroundSettings::default(),
            key: KeySettings::default(),
            matte: false,
            monitor: 0,
            cursor: true,
            video_path: String::new(),
        }
    }
}

/// Turns a decoded video's colour into the colour the compositor's NV12 path converts with.
fn cam_color(color: VideoColor) -> CamColor {
    CamColor {
        matrix: match color.matrix {
            VideoMatrix::Bt601 => YuvMatrix::Bt601,
            VideoMatrix::Bt709 => YuvMatrix::Bt709,
            VideoMatrix::Bt2020 => YuvMatrix::Bt2020,
        },
        range: match color.range {
            VideoRange::Limited => YuvRange::Limited,
            VideoRange::Full => YuvRange::Full,
        },
    }
}

impl ComposeConfig {
    /// The configuration of a profile, with the matte view off; a profile with the Source size gets `source_size` rounded down to even numbers.
    pub fn from_profile(profile: &Profile, source_size: (u32, u32), monitor: usize, cursor: bool) -> Self {
        let (width, height) = if profile.is_source_size() { (source_size.0 & !1, source_size.1 & !1) } else { (profile.width, profile.height) };
        let key = profile.webcam.as_ref().map(|webcam| KeySettings::from(&webcam.key)).unwrap_or_default();
        Self {
            width,
            height,
            fps: profile.fps.max(1),
            layout: profile.layout.into(),
            clean: profile.effective_composite_mode() == CompositeMode::Clean,
            pip: (&profile.pip).into(),
            bg: (&profile.background).into(),
            key,
            matte: false,
            monitor,
            cursor,
            video_path: if profile.background.kind == crate::config::BackgroundKind::Video { profile.background.video_path.clone() } else { String::new() },
        }
    }

    /// The layout of the second frame a take's encoder needs, or None when it records the composite itself.
    ///
    /// Clean mode records the screen as the Screen only layout draws it, with no camera layer.
    pub fn encoder_layout(&self) -> Option<Layout> {
        self.clean.then_some(Layout::ScreenOnly)
    }
}

/// One composed frame and the device it lives on.
#[derive(Clone)]
pub struct Composite {
    /// The device the texture lives on.
    pub gpu: Gpu,
    /// The BGRA texture, one of a small ring that is reused as newer frames are composed.
    pub texture: ID3D11Texture2D,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The frame number; it grows by one for every composed frame.
    pub seq: u64,
    /// Changes whenever the screen or the camera delivered a new picture, so the pacer can tell a repeat.
    pub content_seq: u64,
    /// How many screen frames the capture has delivered so far.
    pub screen_received: u64,
    /// The separate frame for a Clean take's encoder; None otherwise.
    encoder: Option<EncoderFrame>,
}

/// The frame a Clean take records in place of the composite.
#[derive(Clone)]
struct EncoderFrame {
    /// The BGRA texture, from a ring of its own.
    texture: ID3D11Texture2D,
    /// Changes only when the screen delivered a new picture; the camera is not in this frame.
    content_seq: u64,
}

impl Composite {
    /// The frame as a take's encoder sees it. A Clean take gets the separate encoder frame, and nothing
    /// from a composite that has none, so the camera can never slip into its main video.
    fn for_encoder(mut self, clean: bool) -> Option<Self> {
        if clean {
            let frame = self.encoder.take()?;
            self.texture = frame.texture;
            self.content_seq = frame.content_seq;
        }
        Some(self)
    }
}

/// What the handle and the thread share.
struct Shared {
    /// The picture the thread composes.
    config: Mutex<ComposeConfig>,
    /// The newest composite, or None before the first and after the thread let go of its device.
    latest: Mutex<Option<Composite>>,
    /// The device the thread works on right now.
    gpu: Mutex<Gpu>,
    /// The device a recording has pinned, or None while no take runs.
    pinned: Mutex<Option<Gpu>>,
    /// False while the window is hidden or the preview is off.
    active: AtomicBool,
    /// True once the thread has been asked to end.
    stop: AtomicBool,
    /// The text of the last error, cleared by the next good frame.
    error: Mutex<Option<String>>,
    /// The text of the last background warning, cleared when the background works again.
    warning: Mutex<Option<String>>,
}

/// Locks a mutex, carrying on with the data if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// True when both handles share one D3D11 device.
fn same_device(a: &Gpu, b: &Gpu) -> bool {
    std::ptr::eq(a.device().as_raw(), b.device().as_raw())
}

/// A cheap handle to the compose thread, for the window, the preview and the recording.
#[derive(Clone)]
pub struct CompositorHandle {
    shared: Arc<Shared>,
}

impl CompositorHandle {
    /// Sets the picture to compose; ignored while a recording holds the compositor, whose profile rules then.
    pub fn set_config(&self, config: ComposeConfig) {
        if lock(&self.shared.pinned).is_some() {
            return;
        }
        store_config(&self.shared, config);
    }

    /// Switches the thread on or off; a recording keeps it running either way.
    pub fn set_active(&self, active: bool) {
        self.shared.active.store(active, Ordering::Relaxed);
    }

    /// The newest composite, or None before the first.
    pub fn latest(&self) -> Option<Composite> {
        lock(&self.shared.latest).clone()
    }

    /// The device the thread composes on right now.
    pub fn gpu(&self) -> Gpu {
        lock(&self.shared.gpu).clone()
    }

    /// The text of the last error, or None while the thread works.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.error).clone()
    }

    /// The text of the last background warning, or None while the background is fine.
    pub fn warning(&self) -> Option<String> {
        lock(&self.shared.warning).clone()
    }

    /// Holds the compositor for a recording: the thread keeps running and stays on one device until the guard is dropped.
    ///
    /// A layout that shows the camera pins the camera's device when a camera is open, so its frames can be drawn.
    pub fn pin(&self, layout: Layout) -> Result<PinGuard, String> {
        let mut pinned = lock(&self.shared.pinned);
        if pinned.is_some() {
            return Err("another recording already holds the compositor".to_owned());
        }
        let current = lock(&self.shared.gpu).clone();
        let gpu = match (layout, webcam::shared()) {
            (Layout::ScreenOnly, _) | (_, None) => current,
            (_, Some(share)) => share.gpu,
        };
        *pinned = Some(gpu.clone());
        // A matte frame must never reach a take, even if the window had the matte view on a moment ago.
        lock(&self.shared.config).matte = false;
        Ok(PinGuard { handle: self.clone(), gpu })
    }
}

/// Proof that a recording holds the compositor; dropping it lets the window steer the compositor again.
pub struct PinGuard {
    handle: CompositorHandle,
    gpu: Gpu,
}

impl PinGuard {
    /// The device every frame of the take lives on.
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// Sets the picture to compose for the take.
    pub fn set_config(&self, config: ComposeConfig) {
        store_config(&self.handle.shared, config);
    }

    /// The newest frame to encode on the pinned device, or None before the first.
    ///
    /// This is the composite, except in Clean mode, where it is the screen without the camera. The
    /// preview reads [`CompositorHandle::latest`] and always gets the full composite.
    pub fn latest(&self) -> Option<Composite> {
        let clean = lock(&self.handle.shared.config).encoder_layout().is_some();
        self.handle.latest().filter(|frame| same_device(&frame.gpu, &self.gpu)).and_then(|frame| frame.for_encoder(clean))
    }

    /// The text of the compositor's last error, or None while it works.
    pub fn error(&self) -> Option<String> {
        self.handle.error()
    }
}

impl Drop for PinGuard {
    fn drop(&mut self) {
        *lock(&self.handle.shared.pinned) = None;
    }
}

/// Replaces the configuration when it differs.
fn store_config(shared: &Shared, config: ComposeConfig) {
    let mut current = lock(&shared.config);
    if *current != config {
        *current = config;
    }
}

/// The compositor the window has started, for a recording to find.
static SHARED: Mutex<Option<CompositorHandle>> = Mutex::new(None);

/// The compositor that the window runs, or None when there is none; a recording then starts its own.
pub fn shared() -> Option<CompositorHandle> {
    lock(&SHARED).clone()
}

/// The running "compose" thread; dropping it ends the thread.
pub struct CompositorThread {
    handle: CompositorHandle,
    thread: Option<JoinHandle<()>>,
    registered: bool,
}

impl CompositorThread {
    /// Starts the thread on a new device of the given adapter; it stays asleep until it is switched on or a recording pins it.
    pub fn start(adapter: usize) -> Result<Self, CaptureError> {
        let gpu = Gpu::create(Some(adapter))?;
        let shared = Arc::new(Shared {
            config: Mutex::new(ComposeConfig::default()),
            latest: Mutex::new(None),
            gpu: Mutex::new(gpu.clone()),
            pinned: Mutex::new(None),
            active: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
            warning: Mutex::new(None),
        });
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("compose".to_owned())
            .spawn(move || Worker::new(thread_shared, gpu).run())
            .map_err(|_| CaptureError::Missing("the compose thread could not be started"))?;
        Ok(Self { handle: CompositorHandle { shared }, thread: Some(thread), registered: false })
    }

    /// The handle to the thread.
    pub fn handle(&self) -> &CompositorHandle {
        &self.handle
    }

    /// Makes this thread the one that [`shared`] hands to recordings, until it is dropped.
    pub fn register(&mut self) {
        *lock(&SHARED) = Some(self.handle.clone());
        self.registered = true;
    }
}

impl Drop for CompositorThread {
    fn drop(&mut self) {
        self.handle.shared.stop.store(true, Ordering::Relaxed);
        if self.registered {
            let mut published = lock(&SHARED);
            if published.as_ref().is_some_and(|handle| Arc::ptr_eq(&handle.shared, &self.handle.shared)) {
                *published = None;
            }
        }
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            log::error!("the compose thread panicked");
        }
    }
}

/// The screen capture the thread runs with the settings it was started for.
struct Capture {
    capture: MonitorCapture,
    monitor: usize,
    cursor: bool,
}

/// The background video the worker has open.
struct ActiveVideo {
    /// The path it was opened for; a different path reopens it.
    path: String,
    /// The looping decoder thread.
    video: LoopingVideo,
}

/// The state of the compose thread.
struct Worker {
    shared: Arc<Shared>,
    /// The device everything below lives on.
    gpu: Gpu,
    capture: Option<Capture>,
    compositor: Option<Compositor>,
    /// Draws the encoder frame of a Clean take; its own ring keeps both frames valid as long as a
    /// single one. None while no Clean take runs.
    encoder_compositor: Option<Compositor>,
    /// When the next frame is due.
    next_tick: Instant,
    /// The number of the last published frame.
    seq: u64,
    /// The open background video, kept while the configured path stays.
    bg_video: Option<ActiveVideo>,
    /// The path of the last background video that failed to open, so a bad file is not reopened every frame.
    bg_video_failed: Option<String>,
}

impl Worker {
    /// A worker on the given device with nothing running yet.
    fn new(shared: Arc<Shared>, gpu: Gpu) -> Self {
        Self {
            shared,
            gpu,
            capture: None,
            compositor: None,
            encoder_compositor: None,
            next_tick: Instant::now(),
            seq: 0,
            bg_video: None,
            bg_video_failed: None,
        }
    }

    /// Runs until the stop flag is set.
    fn run(&mut self) {
        while !self.shared.stop.load(Ordering::Relaxed) {
            let config = lock(&self.shared.config).clone();
            let pinned = lock(&self.shared.pinned).clone();
            if !self.shared.active.load(Ordering::Relaxed) && pinned.is_none() {
                self.release();
                self.next_tick = Instant::now();
                self.nap(IDLE_SLEEP);
                continue;
            }
            let cam = if config.layout == Layout::ScreenOnly { None } else { webcam::shared() };
            let wanted = pick_gpu(pinned.as_ref(), cam.as_ref(), &self.gpu);
            if !same_device(&wanted, &self.gpu) {
                self.switch_gpu(wanted);
            }
            let cam = cam.filter(|share| same_device(&share.gpu, &self.gpu));
            match self.prepare(&config) {
                Ok(true) => {}
                Ok(false) => {
                    self.nap(POLL_SLEEP);
                    continue;
                }
                Err(message) => {
                    self.fail(&message);
                    self.nap(RETRY_BACKOFF);
                    continue;
                }
            }
            self.wait_for_tick(config.fps);
            if let Err(message) = self.tick(&config, cam.as_ref(), pinned.is_some()) {
                self.fail(&message);
                self.nap(ERROR_BACKOFF);
            }
        }
    }

    /// Sleeps for the given time, in short slices so a stop request is noticed quickly.
    fn nap(&self, total: Duration) {
        let mut left = total;
        while !left.is_zero() && !self.shared.stop.load(Ordering::Relaxed) {
            let slice = left.min(IDLE_SLEEP);
            std::thread::sleep(slice);
            left -= slice;
        }
    }

    /// Sleeps until the next frame is due, and does not hurry to catch up after a late one.
    fn wait_for_tick(&mut self, fps: u32) {
        let period = tick_period(fps);
        let now = Instant::now();
        if self.next_tick + period < now {
            self.next_tick = now;
        }
        if self.next_tick > now {
            self.nap(self.next_tick - now);
        }
        self.next_tick += period;
    }

    /// Stops the capture and frees the GPU objects while the thread is switched off.
    fn release(&mut self) {
        if self.capture.is_some() || self.compositor.is_some() || self.bg_video.is_some() {
            self.capture = None;
            self.compositor = None;
            self.encoder_compositor = None;
            self.bg_video = None;
            self.bg_video_failed = None;
            *lock(&self.shared.latest) = None;
        }
    }

    /// Moves everything to another device.
    fn switch_gpu(&mut self, gpu: Gpu) {
        self.release();
        *lock(&self.shared.gpu) = gpu.clone();
        self.gpu = gpu;
    }

    /// Records an error: the first time a message appears it is also logged.
    fn fail(&self, message: &str) {
        let mut error = lock(&self.shared.error);
        if error.as_deref() != Some(message) {
            log::warn!("compositor: {message}");
            *error = Some(message.to_owned());
        }
    }

    /// Records a background warning: the first time a message appears it is also logged.
    fn warn(shared: &Shared, message: &str) {
        let mut warning = lock(&shared.warning);
        if warning.as_deref() != Some(message) {
            log::warn!("compositor: {message}");
            *warning = Some(message.to_owned());
        }
    }

    /// Makes sure the capture and the compositor match the configuration; false while the output size is not known yet.
    fn prepare(&mut self, config: &ComposeConfig) -> Result<bool, String> {
        let capture_current = self.capture.as_ref().is_some_and(|running| running.monitor == config.monitor && running.cursor == config.cursor);
        if !capture_current {
            // The old capture ends before the new one starts.
            self.capture = None;
            let monitor = list_monitors()
                .map_err(|err| err.to_string())?
                .into_iter()
                .nth(config.monitor)
                .ok_or_else(|| format!("monitor {} was not found", config.monitor))?;
            let capture = MonitorCapture::start(&self.gpu, &monitor, config.cursor).map_err(|err| format!("could not start the screen capture: {err}"))?;
            self.capture = Some(Capture { capture, monitor: config.monitor, cursor: config.cursor });
        }
        let size = if config.width == 0 || config.height == 0 {
            let Some(frame) = self.capture.as_ref().and_then(|running| running.capture.latest()) else { return Ok(false) };
            (u32::try_from(frame.width).unwrap_or(0) & !1, u32::try_from(frame.height).unwrap_or(0) & !1)
        } else {
            (config.width, config.height)
        };
        if self.compositor.as_ref().map(Compositor::output_size) != Some(size) {
            self.compositor = None;
            self.encoder_compositor = None;
            let compositor = Compositor::new(&self.gpu, size.0, size.1).map_err(|err| format!("could not start the compositor: {err}"))?;
            self.compositor = Some(compositor);
        }
        Ok(true)
    }

    /// Composes one frame from the latest pictures and publishes it; while a Clean take records, the
    /// encoder's frame is composed and published with it.
    fn tick(&mut self, config: &ComposeConfig, cam: Option<&CamShare>, recording: bool) -> Result<(), String> {
        let (Some(running), Some(compositor)) = (&self.capture, &mut self.compositor) else { return Ok(()) };
        let screen = running.capture.latest();
        let cam_frame = cam.and_then(|share| share.capture.latest());
        // A screen layout waits for the first screen frame instead of publishing a black one.
        if screen.is_none() && config.layout != Layout::CamOnly {
            return Ok(());
        }
        if let Some(share) = cam {
            compositor.set_cam_color(share.capture.color());
        }
        // The background video is opened here, on the compose thread and device, and kept while the path stays.
        let mut video_frame = None;
        let mut video_error = None;
        if config.video_path.is_empty() {
            self.bg_video = None;
            self.bg_video_failed = None;
        } else {
            let open = self.bg_video.as_ref().map(|active| active.path.as_str()) != Some(config.video_path.as_str())
                && self.bg_video_failed.as_deref() != Some(config.video_path.as_str());
            if open {
                self.bg_video = None;
                match LoopingVideo::open(self.gpu.device(), std::path::Path::new(&config.video_path)) {
                    Ok(video) => self.bg_video = Some(ActiveVideo { path: config.video_path.clone(), video }),
                    Err(err) => {
                        self.bg_video_failed = Some(config.video_path.clone());
                        video_error = Some(format!("the background video could not be opened, using the colour instead: {err}"));
                    }
                }
            }
            if let Some(active) = &self.bg_video {
                video_frame = active.video.latest().map(|frame| (frame.texture.clone(), frame.width, frame.height, cam_color(frame.color)));
            }
        }
        compositor.set_background_video(video_frame.as_ref().map(|(texture, width, height, color)| (texture, *width, *height, *color)));
        // The background image is loaded here, on the compose thread and device, and cached by path.
        match (video_error, compositor.set_background(&config.bg)) {
            (Some(message), _) => Worker::warn(&self.shared, &message),
            (None, Ok(())) => *lock(&self.shared.warning) = None,
            (None, Err(err)) => Worker::warn(&self.shared, &format!("the background image could not be loaded, using the colour instead: {err}")),
        }
        let composed = compositor
            .compose(screen.as_ref().map(|frame| &frame.texture), cam_frame.as_ref(), &config.layout, &config.pip, &config.key, config.matte)
            .map_err(|err| err.to_string())?;
        let (width, height) = compositor.output_size();
        let screen_seq = screen.as_ref().map_or(0, |frame| frame.seq);
        let encoder = match config.encoder_layout().filter(|_| recording) {
            Some(layout) => {
                let second = match &mut self.encoder_compositor {
                    Some(second) => second,
                    empty => empty.insert(Compositor::new(&self.gpu, width, height).map_err(|err| format!("could not start the encoder compositor: {err}"))?),
                };
                let frame = second
                    .compose(screen.as_ref().map(|frame| &frame.texture), None, &layout, &config.pip, &config.key, false)
                    .map_err(|err| err.to_string())?;
                Some(EncoderFrame { texture: frame.texture, content_seq: screen_seq })
            }
            None => {
                self.encoder_compositor = None;
                None
            }
        };
        self.seq += 1;
        let content_seq = screen_seq.wrapping_add(cam_frame.as_ref().map_or(0, |frame| frame.seq));
        let published = Composite {
            gpu: self.gpu.clone(),
            texture: composed.texture,
            width,
            height,
            seq: self.seq,
            content_seq,
            screen_received: running.capture.frames_received(),
            encoder,
        };
        *lock(&self.shared.latest) = Some(published);
        *lock(&self.shared.error) = None;
        Ok(())
    }
}

/// The device the thread should work on: a recording's pinned one, else the camera's when there is a camera to draw, else the current one.
fn pick_gpu(pinned: Option<&Gpu>, cam: Option<&CamShare>, current: &Gpu) -> Gpu {
    match (pinned, cam) {
        (Some(gpu), _) => gpu.clone(),
        (None, Some(share)) => share.gpu.clone(),
        (None, None) => current.clone(),
    }
}

/// The time between two composed frames.
fn tick_period(fps: u32) -> Duration {
    Duration::from_secs_f64(1.0 / f64::from(fps.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_with_the_source_size_composes_at_the_monitor_size_rounded_to_even() {
        let profile = Profile { width: 0, height: 0, ..Profile::premiere_hevc_1080p60() };
        let config = ComposeConfig::from_profile(&profile, (2561, 1441), 0, true);
        assert_eq!((config.width, config.height), (2560, 1440));
    }

    #[test]
    fn a_profile_with_a_fixed_size_composes_at_that_size() {
        let config = ComposeConfig::from_profile(&Profile::tutorial_av1_1440p60(), (1920, 1080), 1, false);
        assert_eq!((config.width, config.height, config.fps), (2560, 1440, 60));
        assert_eq!((config.monitor, config.cursor), (1, false));
    }

    #[test]
    fn the_layout_and_the_pip_come_from_the_profile() {
        let mut profile = Profile::premiere_hevc_1080p60();
        profile.layout = crate::config::LayoutKind::Pip;
        profile.pip.scale_pct = 40.0;
        profile.background = crate::config::BackgroundConfig {
            kind: crate::config::BackgroundKind::Image,
            color: [1, 2, 3],
            image_path: "bg.png".to_owned(),
            video_path: String::new(),
            fit: crate::config::BackgroundFit::Stretch,
        };
        let config = ComposeConfig::from_profile(&profile, (1920, 1080), 0, true);
        assert_eq!(config.layout, Layout::Pip);
        assert_eq!(config.pip.scale_pct, 40.0);
        assert_eq!(config.bg.color, [1, 2, 3]);
        assert_eq!(config.bg.image_path, "bg.png");
        assert_eq!(config.bg.fit, capture::BackgroundFit::Stretch);
        assert_eq!(config.bg.kind, capture::BackgroundKind::Image);
    }

    #[test]
    fn the_key_comes_from_the_profile_webcam_and_the_matte_starts_off() {
        let mut profile = Profile::premiere_hevc_1080p60();
        assert_eq!(ComposeConfig::from_profile(&profile, (1920, 1080), 0, true).key, KeySettings::default());
        let mut webcam = crate::config::WebcamSettings::default();
        webcam.key.enabled = true;
        webcam.key.similarity = 0.55;
        profile.webcam = Some(webcam);
        let config = ComposeConfig::from_profile(&profile, (1920, 1080), 0, true);
        assert!(config.key.enabled);
        assert_eq!(config.key.similarity, 0.55);
        assert!(!config.matte);
    }

    #[test]
    fn a_clean_pip_config_keeps_the_camera_in_the_preview_and_out_of_the_encoder_frame() {
        let profile = Profile {
            layout: crate::config::LayoutKind::Pip,
            webcam: Some(crate::config::WebcamSettings::default()),
            composite_mode: CompositeMode::Clean,
            ..Profile::premiere_hevc_1080p60()
        };
        let config = ComposeConfig::from_profile(&profile, (1920, 1080), 0, true);
        // The preview composes `layout`, which has the camera layer.
        assert_eq!(config.layout, Layout::Pip);
        // The encoder gets its own frame in the Screen only layout, which has none.
        assert_eq!(config.encoder_layout(), Some(Layout::ScreenOnly));

        let baked = Profile { composite_mode: CompositeMode::Baked, ..profile };
        let config = ComposeConfig::from_profile(&baked, (1920, 1080), 0, true);
        assert_eq!((config.layout, config.encoder_layout()), (Layout::Pip, None));
    }

    #[test]
    fn the_tick_period_follows_the_frame_rate() {
        assert!((tick_period(60).as_secs_f64() - 1.0 / 60.0).abs() < 1e-9);
        assert!((tick_period(30).as_secs_f64() - 1.0 / 30.0).abs() < 1e-9);
        // A frame rate of 0 does not divide by zero.
        assert_eq!(tick_period(0), Duration::from_secs(1));
    }
}
