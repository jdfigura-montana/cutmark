//! The webcam reader: an IMFSourceReader on its own thread that keeps the latest frame.
#![allow(unsafe_code)]

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::devices::{CameraFormat, CameraSubtype, native_types, pack_pair};
use super::{CameraControls, ComGuard, MfGuard, RunningStats, hns_to_qpc, qpc_ticks_to_ms};
use crate::{CaptureError, Gpu, qpc_frequency, qpc_now, with_context};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_DECODER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BOX, D3D11_CPU_ACCESS_WRITE, D3D11_FORMAT_SUPPORT,
    D3D11_FORMAT_SUPPORT_DECODER_OUTPUT, D3D11_FORMAT_SUPPORT_RENDER_TARGET, D3D11_FORMAT_SUPPORT_SHADER_SAMPLE, D3D11_FORMAT_SUPPORT_VIDEO_PROCESSOR_INPUT,
    D3D11_MAP_WRITE, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{
    IMF2DBuffer, IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaSource, IMFMediaType, IMFSample, IMFSourceReader,
    MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID, MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,
    MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_SOURCE_READERF_ENDOFSTREAM, MF_SOURCE_READERF_ERROR,
    MFCreateAttributes, MFCreateDXGIDeviceManager, MFCreateDeviceSource, MFCreateMediaType, MFCreateSourceReaderFromMediaSource, MFMediaType_Video,
    MFVideoFormat_NV12,
};
use windows::core::{HSTRING, Interface};

/// The reader stream index of the first video stream.
const VIDEO_STREAM: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
/// How many textures the reader rotates through, so a consumer can hold a frame for a while.
const RING_LEN: usize = 3;
/// How long start waits for the camera to open.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long stop waits for the reader thread before leaving it to finish alone.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
/// How many frames in a row may fail to publish before the camera is declared broken.
const MAX_PUBLISH_FAILURES: u32 = 30;

/// Where the camera frames come from on the way into D3D11.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebcamPath {
    /// Media Foundation decodes straight into D3D11 textures on our device.
    D3d11,
    /// Media Foundation hands out system memory samples that we upload.
    Cpu,
}

impl fmt::Display for WebcamPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::D3d11 => f.write_str("D3D11 (Media Foundation decodes into GPU textures)"),
            Self::Cpu => f.write_str("CPU (NV12 samples uploaded to a D3D11 texture)"),
        }
    }
}

/// One camera frame as an NV12 texture on the shared device.
#[derive(Clone)]
pub struct CamFrame {
    /// The NV12 texture, one of a small ring that is reused as newer frames arrive.
    pub texture: ID3D11Texture2D,
    /// The sample time converted to QPC ticks.
    pub qpc: i64,
    /// The frame number, counting from 1.
    pub seq: u64,
}

/// Timing statistics of a running webcam capture.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WebcamStats {
    /// How many frames were read from the camera.
    pub frames: u64,
    /// Mean time between frame arrivals, in milliseconds.
    pub interval_mean_ms: f64,
    /// Standard deviation of the time between frame arrivals, in milliseconds.
    pub interval_std_ms: f64,
    /// Mean of QPC at arrival minus the sample time as QPC, in milliseconds.
    pub offset_mean_ms: f64,
    /// Largest absolute value of QPC at arrival minus the sample time as QPC, in milliseconds.
    pub offset_max_abs_ms: f64,
}

/// What the reader thread shares with the owner.
#[derive(Default)]
struct State {
    latest: Option<CamFrame>,
    error: Option<String>,
    interval: RunningStats,
    offset: RunningStats,
    last_arrival: Option<i64>,
}

/// Shared between the owner and the reader thread.
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    received: AtomicU64,
    stop: AtomicBool,
    cpu_path: AtomicBool,
    /// The control interfaces of the open camera; cleared before the camera is reopened and when the thread ends.
    controls: Mutex<Option<CameraControls>>,
}

impl Shared {
    /// Replaces the control interfaces that the owner can fetch.
    fn set_controls(&self, controls: Option<CameraControls>) {
        *self.controls.lock().unwrap_or_else(PoisonError::into_inner) = controls;
    }

    /// Locks the state, recovering it if a thread panicked while holding the lock.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a fatal camera error; the first one wins.
    fn set_error(&self, message: String) {
        log::warn!("webcam: {message}");
        let mut state = self.lock();
        if state.error.is_none() {
            state.error = Some(message);
        }
    }
}

/// A running webcam capture on the shared D3D11 device.
pub struct WebcamCapture {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    format: CameraFormat,
}

impl WebcamCapture {
    /// Opens the camera with the given symbolic link in the given format and starts the reader thread.
    pub fn start(gpu: &Gpu, camera_id: &str, format: &CameraFormat) -> Result<Self, CaptureError> {
        let shared = Arc::new(Shared::default());
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_gpu = gpu.clone();
        let thread_id = camera_id.to_owned();
        let thread_format = *format;
        let thread_shared = shared.clone();
        let handle = std::thread::Builder::new()
            .name("webcam".to_owned())
            .spawn(move || run_thread(thread_gpu, thread_id, thread_format, thread_shared, ready_tx))
            .map_err(|error| CaptureError::Webcam(format!("failed to spawn the webcam thread: {error}")))?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { shared, thread: Some(handle), format: *format }),
            Ok(Err(error)) => {
                let _ = handle.join();
                Err(error)
            }
            Err(_) => {
                // Leave the thread to finish alone; it exits once it sees the stop flag.
                shared.stop.store(true, Ordering::SeqCst);
                Err(CaptureError::Webcam("the camera did not open in time".to_owned()))
            }
        }
    }

    /// The newest frame, or None before the first frame and after a camera error.
    pub fn latest(&self) -> Option<CamFrame> {
        let state = self.shared.lock();
        if state.error.is_some() {
            return None;
        }
        state.latest.clone()
    }

    /// The format the camera was opened in.
    pub fn format(&self) -> CameraFormat {
        self.format
    }

    /// The camera's exposure, focus, white balance, gain and brightness controls; empty when the camera has none.
    pub fn controls(&self) -> CameraControls {
        self.shared.controls.lock().unwrap_or_else(PoisonError::into_inner).clone().unwrap_or_else(CameraControls::empty)
    }

    /// How many frames were read from the camera so far.
    pub fn frames_received(&self) -> u64 {
        self.shared.received.load(Ordering::SeqCst)
    }

    /// Which path the frames take into D3D11; confirmed by the first frame.
    pub fn path_used(&self) -> WebcamPath {
        if self.shared.cpu_path.load(Ordering::SeqCst) { WebcamPath::Cpu } else { WebcamPath::D3d11 }
    }

    /// The error that stopped the capture, for example a camera that was unplugged.
    pub fn error(&self) -> Option<String> {
        self.shared.lock().error.clone()
    }

    /// Frame interval and sample time versus arrival statistics so far.
    pub fn stats(&self) -> WebcamStats {
        let state = self.shared.lock();
        WebcamStats {
            frames: self.shared.received.load(Ordering::SeqCst),
            interval_mean_ms: state.interval.mean(),
            interval_std_ms: state.interval.std_dev(),
            offset_mean_ms: state.offset.mean(),
            offset_max_abs_ms: state.offset.max_abs(),
        }
    }

    /// Stops the reader thread and releases the camera.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Signals the thread and waits briefly for it; safe to call from Drop.
    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        let Some(handle) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now() + STOP_TIMEOUT;
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            log::warn!("webcam: the reader thread is still waiting for the camera; leaving it to finish alone");
        }
    }
}

impl Drop for WebcamCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The body of the "webcam" thread: open the camera, report, then read until stopped.
fn run_thread(gpu: Gpu, camera_id: String, format: CameraFormat, shared: Arc<Shared>, ready: mpsc::Sender<Result<(), CaptureError>>) {
    // Declared first so it drops last, after every Media Foundation object is released.
    let _com = ComGuard::init();
    let _mf = match MfGuard::acquire() {
        Ok(guard) => guard,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut session = match Session::open(&gpu, &camera_id, &format) {
        Ok(session) => session,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    shared.cpu_path.store(session.path == WebcamPath::Cpu, Ordering::SeqCst);
    shared.set_controls(Some(session.controls.clone()));
    log::info!("webcam: {}x{} {} using the {} path", format.width, format.height, format.subtype, session.path);
    if ready.send(Ok(())).is_err() {
        shared.set_controls(None);
        return;
    }
    session.run(&shared);
    shared.set_controls(None);
}

/// Everything the reader thread owns.
struct Session {
    // The reader is declared before the device manager so it is released first.
    reader: Option<IMFSourceReader>,
    _manager: Option<IMFDXGIDeviceManager>,
    controls: CameraControls,
    gpu: Gpu,
    ring: Vec<ID3D11Texture2D>,
    next: usize,
    staging: Option<ID3D11Texture2D>,
    path: WebcamPath,
    camera_id: String,
    format: CameraFormat,
    freq: i64,
}

impl Session {
    /// Opens the camera, preferring the D3D11 path and falling back to the CPU path.
    fn open(gpu: &Gpu, camera_id: &str, format: &CameraFormat) -> Result<Self, CaptureError> {
        enable_multithread_protection(gpu);
        let mut ring = Vec::with_capacity(RING_LEN);
        let candidates = ring_bind_candidates(gpu);
        for _ in 0..RING_LEN {
            ring.push(create_ring_texture(gpu, format.width, format.height, &candidates)?);
        }
        let mut manager = create_device_manager(gpu);
        let mut built = None;
        if let Some(candidate) = manager.as_ref() {
            match build_reader(camera_id, format, Some(candidate)) {
                Ok(pair) => built = Some(pair),
                Err(error) => log::warn!("webcam: the D3D11 output path failed ({error}); falling back to CPU samples"),
            }
        }
        let path = if built.is_some() { WebcamPath::D3d11 } else { WebcamPath::Cpu };
        if built.is_none() {
            manager = None;
            built = Some(build_reader(camera_id, format, None)?);
        }
        let (reader, source) = built.ok_or(CaptureError::Missing("the camera reader was not built"))?;
        let controls = CameraControls::from_source(&source);
        Ok(Self {
            reader: Some(reader),
            _manager: manager,
            controls,
            gpu: gpu.clone(),
            ring,
            next: 0,
            staging: None,
            path,
            camera_id: camera_id.to_owned(),
            format: *format,
            freq: qpc_frequency(),
        })
    }

    /// Reads samples until stopped or the camera fails.
    fn run(&mut self, shared: &Shared) {
        let mut failures = 0u32;
        while !shared.stop.load(Ordering::SeqCst) {
            let Some(reader) = self.reader.clone() else {
                shared.set_error("the camera reader is gone".to_owned());
                return;
            };
            let mut flags = 0u32;
            let mut sample_time = 0i64;
            let mut sample: Option<IMFSample> = None;
            // SAFETY: the reader is live and every out pointer is valid for the call.
            let read = unsafe { reader.ReadSample(VIDEO_STREAM, 0, None, Some(&mut flags), Some(&mut sample_time), Some(&mut sample)) };
            let arrival = qpc_now();
            if shared.stop.load(Ordering::SeqCst) {
                return;
            }
            if let Err(error) = read {
                if self.path == WebcamPath::D3d11 && shared.received.load(Ordering::SeqCst) == 0 {
                    log::warn!("webcam: the first read failed on the D3D11 path ({error}); retrying with CPU samples");
                    drop(reader);
                    // The old controls keep the old camera source open, so let go of them first.
                    shared.set_controls(None);
                    match self.fall_back_to_cpu() {
                        Ok(()) => {
                            shared.cpu_path.store(true, Ordering::SeqCst);
                            shared.set_controls(Some(self.controls.clone()));
                            continue;
                        }
                        Err(fallback) => {
                            shared.set_error(format!("the camera stopped delivering frames: {fallback}"));
                            return;
                        }
                    }
                }
                shared.set_error(format!("the camera stopped delivering frames (unplugged?): {error}"));
                return;
            }
            if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
                shared.set_error("the camera reported a stream error (unplugged?)".to_owned());
                return;
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                shared.set_error("the camera stream ended (unplugged?)".to_owned());
                return;
            }
            let Some(sample) = sample else {
                continue;
            };
            match self.publish(&sample) {
                Ok(texture) => {
                    failures = 0;
                    self.record(shared, texture, sample_time, arrival);
                }
                Err(error) => {
                    failures += 1;
                    log::warn!("webcam: dropped a frame: {error}");
                    if failures >= MAX_PUBLISH_FAILURES {
                        shared.set_error(format!("{MAX_PUBLISH_FAILURES} frames in a row failed: {error}"));
                        return;
                    }
                }
            }
        }
    }

    /// Stores a copied frame as the latest one and updates the timing statistics.
    fn record(&self, shared: &Shared, texture: ID3D11Texture2D, sample_time_hns: i64, arrival: i64) {
        let sample_qpc = hns_to_qpc(sample_time_hns, self.freq);
        let seq = shared.received.fetch_add(1, Ordering::SeqCst) + 1;
        shared.cpu_path.store(self.path == WebcamPath::Cpu, Ordering::SeqCst);
        let mut state = shared.lock();
        state.offset.push(qpc_ticks_to_ms(arrival - sample_qpc, self.freq));
        if let Some(previous) = state.last_arrival {
            state.interval.push(qpc_ticks_to_ms(arrival - previous, self.freq));
        }
        state.last_arrival = Some(arrival);
        state.latest = Some(CamFrame { texture, qpc: sample_qpc, seq });
    }

    /// Replaces the reader with one that returns system memory samples.
    fn fall_back_to_cpu(&mut self) -> Result<(), CaptureError> {
        // Release the old reader first: the camera cannot be opened twice.
        self.reader = None;
        self._manager = None;
        self.controls = CameraControls::empty();
        let (reader, source) = build_reader(&self.camera_id, &self.format, None)?;
        self.controls = CameraControls::from_source(&source);
        self.reader = Some(reader);
        self.path = WebcamPath::Cpu;
        Ok(())
    }

    /// Copies one sample into the next texture of the ring and returns that texture.
    fn publish(&mut self, sample: &IMFSample) -> Result<ID3D11Texture2D, CaptureError> {
        let dest = self.ring[self.next].clone();
        // SAFETY: the sample is a live COM object; a single buffer is returned as is.
        let buffer = with_context("failed to get the sample buffer", unsafe { sample.ConvertToContiguousBuffer() })?;
        let on_gpu = if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
            self.copy_from_gpu(&dxgi, &dest)?;
            true
        } else {
            self.upload_from_cpu(&buffer, &dest)?;
            false
        };
        let path = if on_gpu { WebcamPath::D3d11 } else { WebcamPath::Cpu };
        if path != self.path {
            log::info!("webcam: frames now arrive on the {path} path");
            self.path = path;
        }
        self.next = (self.next + 1) % self.ring.len();
        Ok(dest)
    }

    /// Copies a decoder output texture into our own texture on the GPU.
    fn copy_from_gpu(&self, buffer: &IMFDXGIBuffer, dest: &ID3D11Texture2D) -> Result<(), CaptureError> {
        let mut raw = std::ptr::null_mut();
        // SAFETY: raw is a valid out pointer and the IID matches the type read back.
        with_context("failed to get the decoded texture", unsafe { buffer.GetResource(&ID3D11Texture2D::IID, &mut raw) })?;
        if raw.is_null() {
            return Err(CaptureError::Missing("the sample buffer held no texture"));
        }
        // SAFETY: GetResource returned an owned reference to an ID3D11Texture2D.
        let source = unsafe { ID3D11Texture2D::from_raw(raw) };
        // SAFETY: the buffer is a live COM object.
        let index = with_context("failed to get the texture subresource", unsafe { buffer.GetSubresourceIndex() })?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: source is a live texture; GetDesc only writes the descriptor.
        unsafe { source.GetDesc(&mut desc) };
        // Planar formats need an even box.
        let region =
            D3D11_BOX { left: 0, top: 0, front: 0, right: desc.Width.min(self.format.width) & !1, bottom: desc.Height.min(self.format.height) & !1, back: 1 };
        // SAFETY: both textures are live NV12 textures on the same device and the box lies inside both.
        unsafe { self.gpu.context().CopySubresourceRegion(dest, 0, 0, 0, 0, &source, index, Some(&region)) };
        Ok(())
    }

    /// Uploads a system memory NV12 sample through a staging texture.
    fn upload_from_cpu(&mut self, buffer: &IMFMediaBuffer, dest: &ID3D11Texture2D) -> Result<(), CaptureError> {
        let staging = match &self.staging {
            Some(texture) => texture.clone(),
            None => {
                let texture = create_nv12_texture(&self.gpu, self.format.width, self.format.height, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_WRITE.0 as u32)?;
                self.staging = Some(texture.clone());
                texture
            }
        };
        // SAFETY: the buffer is a live COM object.
        let length = with_context("failed to get the sample length", unsafe { buffer.GetCurrentLength() })? as usize;
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut pitch = self.format.width as i32;
        let two_d = buffer.cast::<IMF2DBuffer>().ok();
        if let Some(buffer_2d) = &two_d {
            // SAFETY: both out pointers are valid; the lock is released below.
            with_context("failed to lock the sample", unsafe { buffer_2d.Lock2D(&mut data, &mut pitch) })?;
        } else {
            // SAFETY: data is a valid out pointer and the null length pointers are allowed; the lock is released below.
            with_context("failed to lock the sample", unsafe { buffer.Lock(&mut data, None, None) })?;
        }
        let copied = self.copy_planes(data, pitch, length, &staging);
        if let Some(buffer_2d) = &two_d {
            // SAFETY: balances the Lock2D above.
            let _ = unsafe { buffer_2d.Unlock2D() };
        } else {
            // SAFETY: balances the Lock above.
            let _ = unsafe { buffer.Unlock() };
        }
        copied?;
        // SAFETY: both textures are live NV12 textures of the same size on the same device.
        unsafe { self.gpu.context().CopyResource(dest, &staging) };
        Ok(())
    }

    /// Copies the Y and UV planes from locked sample memory into the mapped staging texture.
    fn copy_planes(&self, data: *const u8, pitch: i32, length: usize, staging: &ID3D11Texture2D) -> Result<(), CaptureError> {
        let width = self.format.width as usize;
        let height = self.format.height as usize;
        let rows = height + height / 2;
        let src_pitch = usize::try_from(pitch).unwrap_or(0);
        let needed = src_pitch * (rows - 1) + width;
        if data.is_null() || src_pitch < width || length < needed {
            return Err(CaptureError::Webcam(format!("the NV12 sample is too small: {length} bytes with pitch {pitch}, need {needed}")));
        }
        // SAFETY: the sample is locked and its current length covers `needed` bytes (checked above).
        let src = unsafe { std::slice::from_raw_parts(data, needed) };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: staging is a live CPU-writable texture and mapped is a valid out pointer.
        with_context("failed to map the staging texture", unsafe { self.gpu.context().Map(staging, 0, D3D11_MAP_WRITE, 0, Some(&mut mapped)) })?;
        let dst_pitch = mapped.RowPitch as usize;
        let result = if mapped.pData.is_null() || dst_pitch < width {
            Err(CaptureError::Missing("the staging texture mapped without usable memory"))
        } else {
            // SAFETY: a mapped NV12 texture holds RowPitch bytes for each of its Y rows followed by its UV rows.
            let dst = unsafe { std::slice::from_raw_parts_mut(mapped.pData as *mut u8, dst_pitch * rows) };
            for row in 0..rows {
                dst[row * dst_pitch..row * dst_pitch + width].copy_from_slice(&src[row * src_pitch..row * src_pitch + width]);
            }
            Ok(())
        };
        // SAFETY: staging is the texture mapped above.
        unsafe { self.gpu.context().Unmap(staging, 0) };
        result
    }
}

/// Turns on D3D11 multithread protection: the reader thread and the recorder share one immediate context.
fn enable_multithread_protection(gpu: &Gpu) {
    match gpu.device().cast::<ID3D10Multithread>() {
        Ok(multithread) => {
            // SAFETY: the interface is live; the call only sets the protection flag and returns the old one.
            let _ = unsafe { multithread.SetMultithreadProtected(true) };
        }
        Err(error) => log::warn!("webcam: could not enable D3D11 multithread protection: {error}"),
    }
}

/// Creates a DXGI device manager bound to the shared device, or None with a logged reason.
fn create_device_manager(gpu: &Gpu) -> Option<IMFDXGIDeviceManager> {
    let mut token = 0u32;
    let mut manager: Option<IMFDXGIDeviceManager> = None;
    // SAFETY: both out pointers are valid for the call.
    if let Err(error) = unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) } {
        log::warn!("webcam: could not create the DXGI device manager: {error}");
        return None;
    }
    let manager = manager?;
    // SAFETY: the manager and the device are live; the token came from the create call.
    if let Err(error) = unsafe { manager.ResetDevice(gpu.device(), token) } {
        log::warn!("webcam: could not give the shared device to the DXGI device manager: {error}");
        return None;
    }
    Some(manager)
}

/// The bind flags to try for the NV12 ring textures, the combination the device reports support for first.
///
/// A video processor input view needs a texture with a bind flag it accepts, so there is no zero fallback.
/// The format support of the device is logged once.
fn ring_bind_candidates(gpu: &Gpu) -> Vec<u32> {
    static LOGGED: Once = Once::new();
    // SAFETY: the device is live and the call only reads the format support bits.
    let support = unsafe { gpu.device().CheckFormatSupport(DXGI_FORMAT_NV12) }.unwrap_or_else(|error| {
        log::warn!("webcam: could not query the NV12 format support: {error}");
        0
    });
    let has = |bit: D3D11_FORMAT_SUPPORT| support & bit.0 as u32 != 0;
    let decoder_ok = has(D3D11_FORMAT_SUPPORT_DECODER_OUTPUT);
    let target_ok = has(D3D11_FORMAT_SUPPORT_RENDER_TARGET) && has(D3D11_FORMAT_SUPPORT_SHADER_SAMPLE);
    LOGGED.call_once(|| {
        log::info!(
            "webcam: NV12 format support 0x{support:x}: decoder output {decoder_ok}, render target and shader sample {target_ok}, video processor input {}",
            has(D3D11_FORMAT_SUPPORT_VIDEO_PROCESSOR_INPUT)
        );
    });
    let decoder = D3D11_BIND_DECODER.0 as u32;
    let target = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32;
    if target_ok && !decoder_ok { vec![target, decoder] } else { vec![decoder, target] }
}

/// Creates one NV12 ring texture with the first bind flags that the device accepts.
fn create_ring_texture(gpu: &Gpu, width: u32, height: u32, candidates: &[u32]) -> Result<ID3D11Texture2D, CaptureError> {
    let mut last = CaptureError::Missing("no bind flags to try for the webcam texture");
    for &bind_flags in candidates {
        match create_nv12_texture(gpu, width, height, D3D11_USAGE_DEFAULT, bind_flags, 0) {
            Ok(texture) => return Ok(texture),
            Err(error) => {
                log::warn!("webcam: NV12 texture with bind flags 0x{bind_flags:x} failed: {error}");
                last = error;
            }
        }
    }
    Err(last)
}

/// Creates an NV12 texture on the shared device.
fn create_nv12_texture(
    gpu: &Gpu,
    width: u32,
    height: u32,
    usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE,
    bind_flags: u32,
    cpu_access: u32,
) -> Result<ID3D11Texture2D, CaptureError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind_flags,
        CPUAccessFlags: cpu_access,
        MiscFlags: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is fully initialized and texture is a valid out pointer.
    with_context("failed to create the webcam texture", unsafe { gpu.device().CreateTexture2D(&desc, None, Some(&mut texture)) })?;
    texture.ok_or(CaptureError::Missing("CreateTexture2D returned no webcam texture"))
}

/// Creates the camera media source from its symbolic link.
fn create_source(camera_id: &str) -> Result<IMFMediaSource, CaptureError> {
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: attributes is a valid out pointer.
    with_context("failed to create the camera attributes", unsafe { MFCreateAttributes(&mut attributes, 2) })?;
    let attributes = attributes.ok_or(CaptureError::Missing("MFCreateAttributes returned no attributes"))?;
    let link = HSTRING::from(camera_id);
    // SAFETY: attributes is live and the key pointers point at constants; link outlives the call.
    with_context(
        "failed to describe the camera",
        (|| unsafe {
            attributes.SetGUID(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE, &MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID)?;
            attributes.SetString(&MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK, &link)
        })(),
    )?;
    // SAFETY: attributes is live and fully set up.
    with_context("failed to open the camera", unsafe { MFCreateDeviceSource(&attributes) })
}

/// Builds a source reader that outputs NV12 in the given camera format.
///
/// With a device manager the reader decodes into D3D11 textures on our device.
/// Advanced video processing lets it insert the MJPEG decoder and converters.
/// The camera's media source is returned with the reader, for its controls.
fn build_reader(camera_id: &str, format: &CameraFormat, manager: Option<&IMFDXGIDeviceManager>) -> Result<(IMFSourceReader, IMFMediaSource), CaptureError> {
    let source = create_source(camera_id)?;
    let mut attributes: Option<IMFAttributes> = None;
    // SAFETY: attributes is a valid out pointer.
    with_context("failed to create the reader attributes", unsafe { MFCreateAttributes(&mut attributes, 3) })?;
    let attributes = attributes.ok_or(CaptureError::Missing("MFCreateAttributes returned no attributes"))?;
    // SAFETY: attributes is live; the keys are constants and the manager outlives the call.
    with_context(
        "failed to set the reader attributes",
        (|| unsafe {
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
            if let Some(manager) = manager {
                attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
                attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
            }
            Ok(())
        })(),
    )?;
    // SAFETY: the source and attributes are live.
    let reader = with_context("failed to create the source reader", unsafe { MFCreateSourceReaderFromMediaSource(&source, &attributes) })?;
    let native = native_types(&reader)
        .into_iter()
        .find(|(candidate, _)| candidate == format)
        .map(|(_, media_type)| media_type)
        .ok_or(CaptureError::Missing("the camera does not offer the requested format"))?;
    // SAFETY: the reader and media type are live. Selecting the native type first
    // picks the camera mode; the NV12 type then makes the reader add its decoder.
    with_context("failed to select the camera format", unsafe { reader.SetCurrentMediaType(VIDEO_STREAM, None, &native) })?;
    if format.subtype != CameraSubtype::Nv12 {
        let nv12 = nv12_type(format)?;
        // SAFETY: the reader and media type are live.
        with_context("failed to select NV12 output", unsafe { reader.SetCurrentMediaType(VIDEO_STREAM, None, &nv12) })?;
    }
    // SAFETY: the reader is live.
    with_context("failed to select the video stream", unsafe { reader.SetStreamSelection(VIDEO_STREAM, true) })?;
    Ok((reader, source))
}

/// Creates the NV12 output media type for the given camera format.
fn nv12_type(format: &CameraFormat) -> Result<IMFMediaType, CaptureError> {
    // SAFETY: the call has no inputs.
    let media_type = with_context("failed to create a media type", unsafe { MFCreateMediaType() })?;
    // SAFETY: the media type is live and the key pointers point at constants.
    with_context(
        "failed to describe the NV12 output",
        (|| unsafe {
            media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_pair(format.width, format.height))?;
            media_type.SetUINT64(&MF_MT_FRAME_RATE, pack_pair(format.fps_num, format.fps_den))
        })(),
    )?;
    Ok(media_type)
}
