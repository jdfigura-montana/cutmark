//! A live, scaled-down preview of a monitor or a webcam for the Recorder window.
//!
//! A monitor preview runs its own [`MonitorCapture`] on its own thread, independent of any recording
//! session. A webcam preview shows the latest NV12 frame of a [`WebcamCapture`] that someone else owns.
//! Scaling happens on the GPU (D3D11 video processor, BGRA or NV12 in and BGRA out), and only the
//! small scaled frame is read back, through a ring of staging textures that is never waited on.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::webcam::WebcamCapture;
use crate::{CaptureError, Gpu, MonitorCapture, MonitorInfo, with_context};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_FLAG_DO_NOT_WAIT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255, D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice,
    ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAS_STILL_DRAWING;
use windows::core::{BOOL, Interface};

/// Smallest preview size in pixels.
const MIN_SIZE: (u32, u32) = (320, 180);

/// Largest preview size in pixels.
const MAX_SIZE: (u32, u32) = (1920, 1080);

/// Size used until the window asks for another one.
const DEFAULT_SIZE: (u32, u32) = (960, 540);

/// How many staging textures the readback ring has.
const SLOTS: usize = 3;

/// How many input views the video processor keeps, enough for a camera's whole texture ring.
const VIEW_CACHE: usize = 8;

/// The picture height from which an NV12 camera frame uses the HD colour matrix.
const HD_HEIGHT: u32 = 720;

/// The shortest time between two produced frames, a little under 1/60 s.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// How long the thread sleeps when there is nothing to do and the preview is on.
const POLL_SLEEP: Duration = Duration::from_millis(2);

/// How long the thread sleeps between checks while the preview is switched off.
const IDLE_SLEEP: Duration = Duration::from_millis(50);

/// How long the thread waits after a GPU error before it tries again.
const ERROR_BACKOFF: Duration = Duration::from_secs(1);

/// How long the thread waits after a failed capture restart before it tries again.
const RESTART_BACKOFF: Duration = Duration::from_secs(2);

/// One preview frame: RGBA pixels at the preview size.
#[derive(Debug, Clone)]
pub struct PreviewFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Straight RGBA bytes, `width * height * 4` of them.
    pub rgba: Arc<[u8]>,
    /// Increases by one for every frame the preview produces.
    pub seq: u64,
}

/// State shared between the handle and the preview thread.
struct Shared {
    /// The newest finished frame.
    latest: Mutex<Option<PreviewFrame>>,
    /// The wanted output size, width in the high half and height in the low half.
    target: AtomicU64,
    /// False while the preview is switched off.
    active: AtomicBool,
    /// True once the thread has been asked to end.
    stop: AtomicBool,
    /// Called after each new frame.
    callback: Mutex<Option<Box<dyn Fn() + Send>>>,
    /// The text of the last error, cleared by the next good frame.
    error: Mutex<Option<String>>,
}

/// Locks a mutex, carrying on with the data if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Packs a size into one atomic word.
fn pack(size: (u32, u32)) -> u64 {
    (u64::from(size.0) << 32) | u64::from(size.1)
}

/// Unpacks a size packed by [`pack`].
fn unpack(word: u64) -> (u32, u32) {
    ((word >> 32) as u32, (word & 0xFFFF_FFFF) as u32)
}

/// Where a preview gets its pictures.
enum Source {
    /// A monitor that the preview captures by itself.
    Monitor {
        /// The monitor to capture.
        monitor: MonitorInfo,
        /// Whether the cursor is drawn.
        cursor: bool,
    },
    /// A running webcam capture that someone else owns.
    Webcam(Arc<WebcamCapture>),
}

/// The newest picture of a source, ready for the video processor.
struct Input {
    /// The texture to scale.
    texture: ID3D11Texture2D,
    /// Width in pixels.
    width: u32,
    /// Height in pixels.
    height: u32,
    /// The frame number of the source.
    seq: u64,
    /// True for an NV12 camera frame, false for a BGRA screen frame.
    nv12: bool,
}

/// A running preview of one monitor or one webcam.
pub struct Preview {
    shared: Arc<Shared>,
    source_size: (u32, u32),
    handle: Option<JoinHandle<()>>,
}

impl Preview {
    /// Starts the "preview" thread with its own capture of the monitor; fails when the capture cannot start.
    pub fn start(gpu: &Gpu, monitor: &MonitorInfo, cursor: bool) -> Result<Self, CaptureError> {
        let source_size = (u32::try_from(monitor.width).unwrap_or(0), u32::try_from(monitor.height).unwrap_or(0));
        Self::spawn(gpu, Source::Monitor { monitor: monitor.clone(), cursor }, source_size)
    }

    /// Starts the "preview" thread on the latest frames of a webcam capture.
    ///
    /// The capture must have been started on the same [`Gpu`]. Switching the preview off does not stop the capture.
    pub fn start_webcam(gpu: &Gpu, webcam: Arc<WebcamCapture>) -> Result<Self, CaptureError> {
        let format = webcam.format();
        Self::spawn(gpu, Source::Webcam(webcam), (format.width, format.height))
    }

    /// Starts the preview thread for a source and waits until it reports that it runs.
    fn spawn(gpu: &Gpu, source: Source, source_size: (u32, u32)) -> Result<Self, CaptureError> {
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            target: AtomicU64::new(pack(fit_target(DEFAULT_SIZE.0, DEFAULT_SIZE.1, source_size.0, source_size.1))),
            active: AtomicBool::new(true),
            stop: AtomicBool::new(false),
            callback: Mutex::new(None),
            error: Mutex::new(None),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_shared = Arc::clone(&shared);
        let gpu = gpu.clone();
        let handle = std::thread::Builder::new()
            .name("preview".to_owned())
            .spawn(move || run_thread(gpu, source, thread_shared, ready_tx))
            .map_err(|_| CaptureError::Missing("the preview thread could not be started"))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { shared, source_size, handle: Some(handle) }),
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                let _ = handle.join();
                Err(CaptureError::Missing("the preview thread ended before it started"))
            }
        }
    }

    /// The newest preview frame, or None before the first one.
    pub fn latest(&self) -> Option<PreviewFrame> {
        lock(&self.shared.latest).clone()
    }

    /// Asks for a preview that fits the given size in pixels, kept at the source's aspect ratio.
    pub fn set_target_size(&self, width: u32, height: u32) {
        let size = fit_target(width, height, self.source_size.0, self.source_size.1);
        self.shared.target.store(pack(size), Ordering::Relaxed);
    }

    /// Switches the preview on or off; while off the thread stops its own capture, if it has one, and does no GPU work.
    pub fn set_active(&self, active: bool) {
        self.shared.active.store(active, Ordering::Relaxed);
    }

    /// Sets the function that is called after each new frame.
    pub fn set_frame_callback(&self, callback: Box<dyn Fn() + Send>) {
        *lock(&self.shared.callback) = Some(callback);
    }

    /// The text of the last preview error, or None while the preview works.
    pub fn error(&self) -> Option<String> {
        lock(&self.shared.error).clone()
    }

    /// Ends the thread and waits for it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Asks the thread to end and joins it; safe to call twice.
    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && handle.join().is_err()
        {
            log::error!("the preview thread panicked");
        }
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The preview size for a requested size: clamped to the allowed range, fitted to the monitor's
/// aspect ratio inside it, and rounded down to even numbers.
fn fit_target(req_w: u32, req_h: u32, mon_w: u32, mon_h: u32) -> (u32, u32) {
    let box_w = req_w.clamp(MIN_SIZE.0, MAX_SIZE.0);
    let box_h = req_h.clamp(MIN_SIZE.1, MAX_SIZE.1);
    let (w, h) = if mon_w == 0 || mon_h == 0 {
        (box_w, box_h)
    } else if u64::from(mon_w) * u64::from(box_h) > u64::from(box_w) * u64::from(mon_h) {
        (box_w, (u64::from(box_w) * u64::from(mon_h) / u64::from(mon_w)) as u32)
    } else {
        ((u64::from(box_h) * u64::from(mon_w) / u64::from(mon_h)) as u32, box_h)
    };
    (even(w), even(h))
}

/// Rounds down to an even number, never below 2.
fn even(value: u32) -> u32 {
    (value & !1).max(2)
}

/// True when the input and output aspect ratios differ by more than one percent.
fn aspects_differ(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> bool {
    let a = u64::from(in_w) * u64::from(out_h);
    let b = u64::from(out_w) * u64::from(in_h);
    a.abs_diff(b) * 100 > a.max(b)
}

/// The largest centred rectangle with the input aspect that fits the output.
fn fitted_rect(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    let wide = u64::from(in_w) * u64::from(out_h) > u64::from(out_w) * u64::from(in_h);
    let (fit_w, fit_h) = if wide {
        (out_w, (u64::from(out_w) * u64::from(in_h) / u64::from(in_w)) as u32)
    } else {
        ((u64::from(out_h) * u64::from(in_w) / u64::from(in_h)) as u32, out_h)
    };
    let x = (out_w.saturating_sub(fit_w) / 2) as i32;
    let y = (out_h.saturating_sub(fit_h) / 2) as i32;
    RECT { left: x, top: y, right: x + fit_w as i32, bottom: y + fit_h as i32 }
}

/// Copies rows of BGRA pixels into tightly packed RGBA with opaque alpha.
fn bgra_to_rgba(src: &[u8], row_pitch: usize, width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    for row in 0..height {
        let start = row * row_pitch;
        let Some(line) = src.get(start..start + width * 4) else { break };
        for &[b, g, r, _] in line.as_chunks::<4>().0 {
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    out
}

/// The body of the "preview" thread: starts the monitor capture if there is one, reports how that went, then runs the loop.
fn run_thread(gpu: Gpu, source: Source, shared: Arc<Shared>, ready: mpsc::Sender<Result<(), CaptureError>>) {
    let capture = match &source {
        Source::Monitor { monitor, cursor } => match MonitorCapture::start(&gpu, monitor, *cursor) {
            Ok(capture) => Some(capture),
            Err(err) => {
                let _ = ready.send(Err(err));
                return;
            }
        },
        Source::Webcam(_) => None,
    };
    // A receiver that is gone means the handle was dropped; the loop then sees the stop flag.
    let _ = ready.send(Ok(()));
    let mut worker = Worker { gpu, source, shared, capture, pipeline: None, last_seq: 0, last_submit: None, out_seq: 0, retry_at: None };
    worker.run();
}

/// The state of the preview thread.
struct Worker {
    gpu: Gpu,
    source: Source,
    shared: Arc<Shared>,
    /// The running monitor capture, or None while the preview is off or when the source is a webcam.
    capture: Option<MonitorCapture>,
    /// The scaler and readback ring, or None until the first frame and after errors.
    pipeline: Option<Pipeline>,
    /// The sequence number of the last captured frame that was scaled.
    last_seq: u64,
    /// When the last frame was scaled, for the 60 per second limit.
    last_submit: Option<Instant>,
    /// The sequence number of the last published preview frame.
    out_seq: u64,
    /// Not before this time may a failed capture start again.
    retry_at: Option<Instant>,
}

impl Worker {
    /// Runs until the stop flag is set.
    fn run(&mut self) {
        while !self.shared.stop.load(Ordering::Relaxed) {
            if !self.shared.active.load(Ordering::Relaxed) {
                self.release();
                self.nap(IDLE_SLEEP);
                continue;
            }
            if !self.has_input() && !self.restart_capture() {
                self.nap(IDLE_SLEEP);
                continue;
            }
            match self.step() {
                Ok(true) => {}
                Ok(false) => self.nap(POLL_SLEEP),
                Err(err) => {
                    log::warn!("preview error: {err}");
                    *lock(&self.shared.error) = Some(err.to_string());
                    self.pipeline = None;
                    self.nap(ERROR_BACKOFF);
                }
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

    /// Stops the capture and frees the GPU objects while the preview is off.
    fn release(&mut self) {
        self.capture = None;
        self.pipeline = None;
    }

    /// True when pictures can be read: always for a webcam, and for a monitor while its capture runs.
    fn has_input(&self) -> bool {
        matches!(self.source, Source::Webcam(_)) || self.capture.is_some()
    }

    /// Starts the monitor capture again after the preview was off; false while it cannot start.
    fn restart_capture(&mut self) -> bool {
        let Source::Monitor { monitor, cursor } = &self.source else { return true };
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return false;
        }
        match MonitorCapture::start(&self.gpu, monitor, *cursor) {
            Ok(capture) => {
                self.capture = Some(capture);
                self.retry_at = None;
                self.last_seq = 0;
                self.last_submit = None;
                true
            }
            Err(err) => {
                log::warn!("the preview capture could not restart: {err}");
                *lock(&self.shared.error) = Some(err.to_string());
                self.retry_at = Some(Instant::now() + RESTART_BACKOFF);
                false
            }
        }
    }

    /// The newest picture of the source, or None before the first one.
    fn latest_input(&self) -> Option<Input> {
        match &self.source {
            Source::Monitor { .. } => self.capture.as_ref().and_then(MonitorCapture::latest).map(|frame| Input {
                texture: frame.texture.clone(),
                width: u32::try_from(frame.width).unwrap_or(0),
                height: u32::try_from(frame.height).unwrap_or(0),
                seq: frame.seq,
                nv12: false,
            }),
            Source::Webcam(webcam) => {
                let format = webcam.format();
                webcam.latest().map(|frame| Input { texture: frame.texture, width: format.width, height: format.height, seq: frame.seq, nv12: true })
            }
        }
    }

    /// Scales a new captured frame when one is due and publishes a frame that the readback finished; true when it did either.
    fn step(&mut self) -> Result<bool, CaptureError> {
        let mut worked = false;
        if let Some(frame) = self.latest_input() {
            let due = frame.seq != self.last_seq && self.last_submit.is_none_or(|at| at.elapsed() >= MIN_FRAME_INTERVAL);
            if due {
                self.submit(&frame)?;
                worked = true;
            }
        }
        if let Some(pipeline) = &mut self.pipeline
            && let Some(rgba) = pipeline.poll()?
        {
            let (width, height) = (pipeline.out_w, pipeline.out_h);
            self.publish(width, height, rgba);
            worked = true;
        }
        Ok(worked)
    }

    /// Scales one captured frame into the next ring slot, rebuilding the pipeline when a size changed.
    fn submit(&mut self, frame: &Input) -> Result<(), CaptureError> {
        let in_size = (frame.width, frame.height);
        let target = unpack(self.shared.target.load(Ordering::Relaxed));
        let current = self.pipeline.as_ref().is_some_and(|pipeline| pipeline.matches(in_size, target, frame.nv12));
        if !current {
            self.pipeline = None;
            self.pipeline = Some(Pipeline::new(&self.gpu, in_size, target, frame.nv12)?);
        }
        // Count the frame as taken even when the scaling fails, so an error does not repeat at once.
        self.last_seq = frame.seq;
        self.last_submit = Some(Instant::now());
        match &mut self.pipeline {
            Some(pipeline) => pipeline.submit(&frame.texture, frame.seq),
            None => Ok(()),
        }
    }

    /// Makes a finished frame the newest one and tells the window.
    fn publish(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        self.out_seq += 1;
        *lock(&self.shared.latest) = Some(PreviewFrame { width, height, rgba: Arc::from(rgba), seq: self.out_seq });
        *lock(&self.shared.error) = None;
        if let Some(callback) = lock(&self.shared.callback).as_ref() {
            callback();
        }
    }
}

/// One staging texture of the readback ring.
struct Slot {
    /// The CPU readable texture.
    texture: ID3D11Texture2D,
    /// The captured frame number written into the slot and not read yet.
    seq: Option<u64>,
}

/// The video processor that scales BGRA or NV12 to BGRA, and the ring that reads the result back.
struct Pipeline {
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    /// The GPU texture the processor writes the scaled frame into.
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    /// The ring of staging textures, written in turn.
    slots: [Slot; SLOTS],
    /// The slot the next frame goes into.
    next: usize,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    /// True when the input textures are NV12 camera frames.
    nv12: bool,
    /// Cached input views, each with the texture it was created from.
    input_views: Vec<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
}

impl Pipeline {
    /// Creates the processor, the output texture and the staging ring for one input and output size.
    fn new(gpu: &Gpu, (in_w, in_h): (u32, u32), (out_w, out_h): (u32, u32), nv12: bool) -> Result<Self, CaptureError> {
        if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 {
            return Err(CaptureError::Missing("the preview sizes must be greater than zero"));
        }
        let device = gpu.device();
        let context = gpu.context().clone();
        let video_device: ID3D11VideoDevice = with_context("failed to query ID3D11VideoDevice", device.cast())?;
        let video_context: ID3D11VideoContext = with_context("failed to query ID3D11VideoContext", context.cast())?;
        let (enumerator, processor) = create_processor(&video_device, in_w, in_h, out_w, out_h)?;
        configure(&video_context, &processor, (in_w, in_h), (out_w, out_h), nv12);
        let output = create_texture(device, out_w, out_h, false)?;
        let output_view = create_output_view(&video_device, &enumerator, &output)?;
        let slots = [
            Slot { texture: create_texture(device, out_w, out_h, true)?, seq: None },
            Slot { texture: create_texture(device, out_w, out_h, true)?, seq: None },
            Slot { texture: create_texture(device, out_w, out_h, true)?, seq: None },
        ];
        Ok(Self {
            context,
            video_device,
            video_context,
            enumerator,
            processor,
            output,
            output_view,
            slots,
            next: 0,
            in_w,
            in_h,
            out_w,
            out_h,
            nv12,
            input_views: Vec::new(),
        })
    }

    /// True when the pipeline was built for these sizes and this kind of input.
    fn matches(&self, input: (u32, u32), output: (u32, u32), nv12: bool) -> bool {
        (self.in_w, self.in_h) == input && (self.out_w, self.out_h) == output && self.nv12 == nv12
    }

    /// Scales the captured texture into the output texture and copies it into the next ring slot, without waiting.
    #[allow(unsafe_code)]
    fn submit(&mut self, input: &ID3D11Texture2D, seq: u64) -> Result<(), CaptureError> {
        let cached = self.input_views.iter().position(|(texture, _)| std::ptr::eq(texture.as_raw(), input.as_raw()));
        let view_index = match cached {
            Some(position) => position,
            None => {
                let view = create_input_view(&self.video_device, &self.enumerator, input)?;
                if self.input_views.len() >= VIEW_CACHE {
                    self.input_views.remove(0);
                }
                self.input_views.push((input.clone(), view));
                self.input_views.len() - 1
            }
        };
        let (_, input_view) = self.input_views.get(view_index).ok_or(CaptureError::Missing("the cached preview input view is gone"))?;
        blt(&self.video_context, &self.processor, input_view, &self.output_view)?;
        let index = self.next;
        self.next = (self.next + 1) % SLOTS;
        let Some(slot) = self.slots.get_mut(index) else { return Ok(()) };
        // SAFETY: both textures are live, on the same device, with the same size and format.
        unsafe { self.context.CopyResource(&slot.texture, &self.output) };
        slot.seq = Some(seq);
        // SAFETY: the context is a live COM object; Flush only submits queued work.
        unsafe { self.context.Flush() };
        Ok(())
    }

    /// Reads the oldest unread slot if the GPU has finished with it; None when it is not ready.
    fn poll(&mut self) -> Result<Option<Vec<u8>>, CaptureError> {
        let oldest = self.slots.iter().enumerate().filter_map(|(index, slot)| slot.seq.map(|seq| (seq, index))).min();
        let Some((_, index)) = oldest else { return Ok(None) };
        let Some(slot) = self.slots.get(index) else { return Ok(None) };
        let rgba = read_slot(&self.context, &slot.texture, self.out_w, self.out_h)?;
        if rgba.is_some()
            && let Some(slot) = self.slots.get_mut(index)
        {
            slot.seq = None;
        }
        Ok(rgba)
    }
}

/// Maps a staging texture without waiting and turns its pixels into RGBA; None while the GPU is still writing it.
#[allow(unsafe_code)]
fn read_slot(context: &ID3D11DeviceContext, texture: &ID3D11Texture2D, width: u32, height: u32) -> Result<Option<Vec<u8>>, CaptureError> {
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: texture is a live staging texture with CPU read access and mapped is a valid out pointer.
    let result = unsafe { context.Map(texture, 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut mapped)) };
    match result {
        Ok(()) => {}
        Err(err) if err.code() == DXGI_ERROR_WAS_STILL_DRAWING => return Ok(None),
        Err(source) => return Err(CaptureError::Windows { context: "failed to map the preview staging texture", source }),
    }
    let (w, h, pitch) = (width as usize, height as usize, mapped.RowPitch as usize);
    let rgba = if mapped.pData.is_null() || h == 0 {
        Vec::new()
    } else {
        // SAFETY: a successful Map gives pData pointing at RowPitch bytes for each of the h rows, and the last row is read only up to w * 4 bytes; the memory stays valid until Unmap below.
        let bytes = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), pitch * (h - 1) + w * 4) };
        bgra_to_rgba(bytes, pitch, w, h)
    };
    // SAFETY: the texture was mapped above and the slice is no longer used.
    unsafe { context.Unmap(texture, 0) };
    if rgba.len() == w * h * 4 { Ok(Some(rgba)) } else { Err(CaptureError::Missing("the preview readback returned a short frame")) }
}

/// The colour space bits of a video processor stream.
///
/// Full range RGB holds only Nominal_Range 0 to 255 in bits 4 to 5. An NV12 camera frame is limited range
/// (16 to 235) YCbCr, and bit 2 picks the BT.709 matrix for HD pictures and BT.601 below.
fn color_space_bits(nv12: bool, height: u32) -> u32 {
    if nv12 {
        let matrix = u32::from(height >= HD_HEIGHT);
        ((D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235.0 as u32) << 4) | (matrix << 2)
    } else {
        (D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255.0 as u32) << 4
    }
}

/// Sets the colour spaces and letterboxes the picture when the input and output aspects differ.
#[allow(unsafe_code)]
fn configure(video_context: &ID3D11VideoContext, processor: &ID3D11VideoProcessor, (in_w, in_h): (u32, u32), (out_w, out_h): (u32, u32), nv12: bool) {
    let input_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: color_space_bits(nv12, in_h) };
    let output_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: color_space_bits(false, out_h) };
    // SAFETY: the processor is a live COM object; the descriptors are plain values.
    unsafe {
        video_context.VideoProcessorSetStreamColorSpace(processor, 0, &input_space);
        video_context.VideoProcessorSetOutputColorSpace(processor, &output_space);
        video_context.VideoProcessorSetStreamAutoProcessingMode(processor, 0, false);
    }
    if aspects_differ(in_w, in_h, out_w, out_h) {
        let rect = fitted_rect(in_w, in_h, out_w, out_h);
        let background = D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } } };
        // SAFETY: the processor is a live COM object; the rect and colour are plain values.
        unsafe {
            video_context.VideoProcessorSetOutputTargetRect(processor, true, Some(&rect));
            video_context.VideoProcessorSetStreamDestRect(processor, 0, true, Some(&rect));
            video_context.VideoProcessorSetOutputBackgroundColor(processor, false, &background);
        }
    }
}

/// Creates the video processor enumerator and processor for the given sizes.
#[allow(unsafe_code)]
fn create_processor(
    video_device: &ID3D11VideoDevice,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
) -> Result<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor), CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
        InputWidth: in_w,
        InputHeight: in_h,
        OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
        OutputWidth: out_w,
        OutputHeight: out_h,
        Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    };
    // SAFETY: desc is a plain value; the call returns an owned enumerator reference.
    let enumerator = with_context("failed to create the preview video processor enumerator", unsafe { video_device.CreateVideoProcessorEnumerator(&desc) })?;
    // SAFETY: enumerator is a live COM object; rate conversion 0 is the plain path.
    let processor = with_context("failed to create the preview video processor", unsafe { video_device.CreateVideoProcessor(&enumerator, 0) })?;
    Ok((enumerator, processor))
}

/// Creates a BGRA texture: a GPU render target for the processor output, or a CPU readable staging texture.
#[allow(unsafe_code)]
fn create_texture(device: &ID3D11Device, width: u32, height: u32, staging: bool) -> Result<ID3D11Texture2D, CaptureError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: if staging { D3D11_USAGE_STAGING } else { D3D11_USAGE_DEFAULT },
        BindFlags: if staging { 0 } else { D3D11_BIND_RENDER_TARGET.0 as u32 },
        CPUAccessFlags: if staging { D3D11_CPU_ACCESS_READ.0 as u32 } else { 0 },
        MiscFlags: 0,
    };
    let mut created: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is a plain value and created is a valid out pointer.
    with_context("failed to create a preview texture", unsafe { device.CreateTexture2D(&desc, None, Some(&mut created)) })?;
    created.ok_or(CaptureError::Missing("CreateTexture2D returned no preview texture"))
}

/// Creates the video processor input view for one BGRA input texture.
#[allow(unsafe_code)]
fn create_input_view(
    video_device: &ID3D11VideoDevice,
    enumerator: &ID3D11VideoProcessorEnumerator,
    input: &ID3D11Texture2D,
) -> Result<ID3D11VideoProcessorInputView, CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
        FourCC: 0,
        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
    };
    let mut view: Option<ID3D11VideoProcessorInputView> = None;
    // SAFETY: all arguments are live COM objects and view is a valid out pointer.
    with_context("failed to create the preview input view", unsafe { video_device.CreateVideoProcessorInputView(input, enumerator, &desc, Some(&mut view)) })?;
    view.ok_or(CaptureError::Missing("CreateVideoProcessorInputView returned no preview view"))
}

/// Creates the video processor output view for the BGRA output texture.
#[allow(unsafe_code)]
fn create_output_view(
    video_device: &ID3D11VideoDevice,
    enumerator: &ID3D11VideoProcessorEnumerator,
    output: &ID3D11Texture2D,
) -> Result<ID3D11VideoProcessorOutputView, CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
    };
    let mut view: Option<ID3D11VideoProcessorOutputView> = None;
    // SAFETY: all arguments are live COM objects and view is a valid out pointer.
    with_context("failed to create the preview output view", unsafe {
        video_device.CreateVideoProcessorOutputView(output, enumerator, &desc, Some(&mut view))
    })?;
    view.ok_or(CaptureError::Missing("CreateVideoProcessorOutputView returned no preview view"))
}

/// Runs one VideoProcessorBlt from the input view into the output view.
#[allow(unsafe_code)]
fn blt(
    video_context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
    input_view: &ID3D11VideoProcessorInputView,
    output_view: &ID3D11VideoProcessorOutputView,
) -> Result<(), CaptureError> {
    let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
        Enable: BOOL(1),
        OutputIndex: 0,
        InputFrameOrField: 0,
        PastFrames: 0,
        FutureFrames: 0,
        ppPastSurfaces: std::ptr::null_mut(),
        pInputSurface: std::mem::ManuallyDrop::new(Some(input_view.clone())),
        ppFutureSurfaces: std::ptr::null_mut(),
        ppPastSurfacesRight: std::ptr::null_mut(),
        pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
        ppFutureSurfacesRight: std::ptr::null_mut(),
    };
    // SAFETY: processor and views are live COM objects on the same device.
    let result = unsafe { video_context.VideoProcessorBlt(processor, output_view, 0, std::slice::from_ref(&stream)) };
    // SAFETY: we drop the one reference we placed in the stream, exactly once.
    unsafe { std::mem::ManuallyDrop::drop(&mut stream.pInputSurface) };
    with_context("preview VideoProcessorBlt failed", result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_requests_clamp_up_to_the_minimum() {
        assert_eq!(fit_target(100, 100, 1920, 1080), (320, 180));
    }

    #[test]
    fn large_requests_clamp_down_to_the_maximum() {
        assert_eq!(fit_target(4000, 3000, 1920, 1080), (1920, 1080));
        assert_eq!(fit_target(3840, 2160, 3840, 2160), (1920, 1080));
    }

    #[test]
    fn sizes_round_down_to_even() {
        // 641x361 fits 16:9 at 641x360.56, which becomes 640x360.
        assert_eq!(fit_target(641, 361, 1920, 1080), (640, 360));
        let (w, h) = fit_target(777, 555, 2560, 1440);
        assert_eq!((w % 2, h % 2), (0, 0));
    }

    #[test]
    fn the_monitor_aspect_ratio_is_kept() {
        // Ultrawide in a 16:9 box: width limits.
        assert_eq!(fit_target(1280, 720, 3440, 1440), (1280, 534));
        // A wide box and a tall monitor: height limits.
        assert_eq!(fit_target(1920, 1080, 1080, 1920), (606, 1080));
        // A short, wide request keeps the width limit.
        assert_eq!(fit_target(500, 1000, 1920, 1080), (500, 280));
    }

    #[test]
    fn an_unknown_monitor_size_uses_the_clamped_box() {
        assert_eq!(fit_target(801, 451, 0, 0), (800, 450));
    }

    #[test]
    fn aspect_differences_under_one_percent_are_ignored() {
        assert!(!aspects_differ(1920, 1080, 640, 360));
        assert!(!aspects_differ(1920, 1080, 640, 358));
        assert!(aspects_differ(3440, 1440, 640, 360));
    }

    #[test]
    fn the_fitted_rect_is_centred() {
        let rect = fitted_rect(3440, 1440, 640, 360);
        assert_eq!((rect.left, rect.right), (0, 640));
        assert!((rect.top - (360 - rect.bottom)).abs() <= 1);
    }

    #[test]
    fn bgra_rows_become_tight_rgba() {
        // Two pixels per row, padded to a 12 byte pitch.
        let src = [1, 2, 3, 0, 4, 5, 6, 0, 99, 99, 99, 99, 7, 8, 9, 0, 10, 11, 12, 0, 99, 99, 99, 99];
        let out = bgra_to_rgba(&src, 12, 2, 2);
        assert_eq!(out, vec![3, 2, 1, 255, 6, 5, 4, 255, 9, 8, 7, 255, 12, 11, 10, 255]);
    }

    #[test]
    fn colour_space_bits_pick_range_and_matrix() {
        // Full range RGB: Nominal_Range 2 in bits 4 to 5.
        assert_eq!(color_space_bits(false, 1080), 0b10_0000);
        // Limited range YCbCr with the HD matrix, and with the SD matrix.
        assert_eq!(color_space_bits(true, 1080), 0b01_0100);
        assert_eq!(color_space_bits(true, 480), 0b01_0000);
    }

    #[test]
    fn a_camera_keeps_its_aspect_ratio_in_the_preview() {
        assert_eq!(fit_target(640, 360, 1280, 960), (480, 360));
    }

    #[test]
    fn pack_round_trips() {
        assert_eq!(unpack(pack((1920, 1080))), (1920, 1080));
    }
}
