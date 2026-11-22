//! A looping background video, decoded with FFmpeg straight onto the shared D3D11 device.
//!
//! [`LoopingVideo::open`] spawns a "bg-video" thread that decodes with D3D11VA built on the caller's
//! device, falls back to software decoding plus an upload when the hardware decoder rejects the stream,
//! copies every frame into an NV12 texture the compositor can sample, paces the frames at the file's own
//! frame rate and restarts from the beginning at the end of the file. Audio streams are ignored.
#![allow(unsafe_code)]

use std::ffi::{CString, c_int, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ffmpeg_next::ffi;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_CPU_ACCESS_WRITE, D3D11_MAP_WRITE, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::core::Interface;

use crate::hw::{MediaError, ff_error};

/// How many NV12 textures the decode thread rotates through.
const RING_LEN: usize = 3;

/// The longest single sleep while the decode thread waits for its next frame slot.
const WAIT_SLICE: Duration = Duration::from_millis(10);

/// The frame rate assumed when a file reports none.
const FALLBACK_FPS: f64 = 30.0;

/// The AVERROR code for EAGAIN, which the decoder returns when it wants to hand out a frame first.
const AVERROR_EAGAIN: c_int = -11;

/// The colour matrix of a video's YCbCr pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoMatrix {
    /// BT.601, the standard definition matrix.
    Bt601,
    /// BT.709, the HD matrix and the default when the file does not say.
    #[default]
    Bt709,
    /// BT.2020.
    Bt2020,
}

/// Whether a video's luma runs from 16 to 235 or over the whole 0 to 255 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoRange {
    /// 16 to 235 for luma and 16 to 240 for chroma; the default when the file does not say.
    #[default]
    Limited,
    /// 0 to 255.
    Full,
}

/// The colour matrix and range of a video's frames, taken from its codec parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VideoColor {
    /// The YCbCr to RGB matrix.
    pub matrix: VideoMatrix,
    /// The range of the YCbCr values.
    pub range: VideoRange,
}

/// One decoded frame: an NV12 texture on the shared device with its size and colour.
#[derive(Clone)]
pub struct VideoFrame {
    /// The NV12 texture, one of a small ring that is reused as newer frames are decoded.
    pub texture: ID3D11Texture2D,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// The colour matrix and range the texture's pixels are in.
    pub color: VideoColor,
}

/// A background video decoded on its own thread into NV12 textures on the shared device, looping forever.
pub struct LoopingVideo {
    /// The newest decoded frame, shared with the decode thread.
    latest: Arc<Mutex<Option<VideoFrame>>>,
    /// Set by stop and Drop to end the decode thread.
    stop: Arc<AtomicBool>,
    /// The decode thread, taken and joined by stop.
    thread: Option<JoinHandle<()>>,
}

impl LoopingVideo {
    /// Opens `path` on a new "bg-video" thread that decodes with D3D11VA on `device`, or in software with an upload when the hardware decoder rejects the stream.
    ///
    /// The file and the decoder are opened here, so a bad path or an unreadable stream fails before the thread starts.
    pub fn open(device: &ID3D11Device, path: &Path) -> Result<Self, MediaError> {
        let session = open_session(device, path)?;
        log::info!(
            "background video {}: {} decoding, {}x{} at {:.3} fps",
            path.display(),
            if session._hw.is_some() { "D3D11VA hardware" } else { "software" },
            session.width,
            session.height,
            1.0 / session.period.as_secs_f64()
        );
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_latest = Arc::clone(&latest);
        let thread_stop = Arc::clone(&stop);
        let payload = SendSession(session);
        let thread = std::thread::Builder::new()
            .name("bg-video".to_owned())
            .spawn(move || run(payload, thread_latest, thread_stop))
            .map_err(|_| MediaError::Null("the background video decode thread"))?;
        Ok(Self { latest, stop, thread: Some(thread) })
    }

    /// The newest decoded frame with its colour matrix and range, or None before the first one.
    pub fn latest(&self) -> Option<VideoFrame> {
        lock(&self.latest).clone()
    }

    /// Ends the decode thread and waits for it to finish.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for LoopingVideo {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Locks a mutex, carrying on with the data if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The instant at which frame `index` is due; the index counts from the thread start and never resets
/// across loops, so the pacing has no gap where a loop restarts.
fn due_at(start: Instant, index: u64, period: Duration) -> Instant {
    let frames = u32::try_from(index).unwrap_or(u32::MAX);
    start + period * frames
}

/// Sleeps until frame `index` is due; false when the stop flag went up while waiting.
fn wait_for_slot(start: Instant, index: u64, period: Duration, stop: &AtomicBool) -> bool {
    let mut left = due_at(start, index, period).saturating_duration_since(Instant::now());
    while !left.is_zero() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(left.min(WAIT_SLICE));
        left = due_at(start, index, period).saturating_duration_since(Instant::now());
    }
    !stop.load(Ordering::Relaxed)
}

/// An FFmpeg D3D11VA hardware device built on the shared D3D11 device, kept alive while the decoder uses it.
struct HwDevice(*mut ffi::AVBufferRef);

impl HwDevice {
    /// Wraps the given D3D11 device in an FFmpeg D3D11VA hardware device.
    fn new(device: &ID3D11Device) -> Result<Self, MediaError> {
        // SAFETY: av_hwdevice_ctx_alloc returns an owned reference or null.
        let mut device_ref = unsafe { ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA) };
        if device_ref.is_null() {
            return Err(MediaError::Null("av_hwdevice_ctx_alloc"));
        }
        // SAFETY: the reference is live; its data is an AVHWDeviceContext whose hwctx is an
        // AVD3D11VADeviceContext. FFmpeg releases the device interface when the context is
        // unreferenced, so it is handed an added reference.
        let code = unsafe {
            let hw_device_ctx = (*device_ref).data.cast::<ffi::AVHWDeviceContext>();
            let d3d11_ctx = (*hw_device_ctx).hwctx.cast::<ffi::AVD3D11VADeviceContext>();
            std::mem::forget(device.clone());
            (*d3d11_ctx).device = device.as_raw().cast::<c_void>().cast();
            ffi::av_hwdevice_ctx_init(device_ref)
        };
        if code < 0 {
            // SAFETY: the reference is owned here and this is its last owner.
            unsafe { ffi::av_buffer_unref(&mut device_ref) };
            return Err(ff_error(code));
        }
        Ok(Self(device_ref))
    }
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        // SAFETY: the reference is owned here and this is its last owner.
        unsafe { ffi::av_buffer_unref(&mut self.0) };
    }
}

/// The opened decoder, its input file and the GPU textures one decode thread works with.
struct Session {
    /// The D3D11 device every texture lives on.
    device: ID3D11Device,
    /// The device's immediate context, used for the frame copies.
    context: ID3D11DeviceContext,
    /// The opened input file.
    fmt: *mut ffi::AVFormatContext,
    /// The opened video decoder.
    codec: *mut ffi::AVCodecContext,
    /// The index of the video stream in the file.
    stream_index: usize,
    /// The time between two frames of the file.
    period: Duration,
    /// The colour matrix and range taken from the codec parameters.
    color: VideoColor,
    /// The frame width, rounded down to an even number of pixels.
    width: u32,
    /// The frame height, rounded down to an even number of pixels.
    height: u32,
    /// The packet reused for every read.
    packet: *mut ffi::AVPacket,
    /// The frame reused for every decode.
    frame: *mut ffi::AVFrame,
    /// The staging texture the software frames are converted into, None on the hardware path.
    staging: Option<ID3D11Texture2D>,
    /// The NV12 textures the decoded frames are copied into.
    ring: Vec<ID3D11Texture2D>,
    /// The ring slot the next frame goes into.
    ring_next: usize,
    /// Whether the refusal of an unsupported software pixel format was already logged.
    unsupported_logged: bool,
    /// The hardware device wrapper, kept alive while the decoder uses it.
    _hw: Option<HwDevice>,
}

/// The session of the decode thread; its raw FFmpeg pointers are created, used and freed on that thread alone.
struct SendSession(Session);

// SAFETY: the session's raw pointers are only ever used on the decode thread, which is the one that
// receives the SendSession.
unsafe impl Send for SendSession {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: every pointer is either null or a resource this session owns, and each call is the
        // documented way to release it. The decoder is freed before the hardware device, whose
        // reference it holds, so the fields above must stay in this order.
        unsafe {
            ffi::av_packet_free(&mut self.packet);
            ffi::av_frame_free(&mut self.frame);
            ffi::avcodec_free_context(&mut self.codec);
            ffi::avformat_close_input(&mut self.fmt);
        }
    }
}

impl Session {
    /// Copies one decoded frame into the next ring texture and publishes it.
    fn publish(&mut self, latest: &Mutex<Option<VideoFrame>>) -> Result<(), MediaError> {
        // SAFETY: the frame is live and was filled by avcodec_receive_frame.
        let format = unsafe { (*self.frame).format };
        let texture = self.ring.get(self.ring_next).ok_or(MediaError::Null("the texture ring"))?.clone();
        self.ring_next = (self.ring_next + 1) % self.ring.len();
        if format == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 as c_int {
            self.copy_hardware(&texture)?;
        } else {
            self.copy_software(&texture, format)?;
        }
        let frame = VideoFrame { texture, width: self.width, height: self.height, color: self.color };
        *lock(latest) = Some(frame);
        Ok(())
    }

    /// Copies a hardware decoded D3D11 texture array slice into the ring texture.
    fn copy_hardware(&self, texture: &ID3D11Texture2D) -> Result<(), MediaError> {
        // SAFETY: the frame is live; per the AV_PIX_FMT_D3D11 layout data[0] holds the texture array
        // and data[1] the array index.
        let (raw, index) = unsafe { ((*self.frame).data[0], (*self.frame).data[1] as usize as u32) };
        let raw = raw as *mut c_void;
        // SAFETY: the raw pointer is borrowed only for the copy below.
        let array = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }.ok_or(MediaError::Null("AVFrame data[0]"))?;
        // SAFETY: both textures are live on the shared device; an NV12 array slice is one subresource
        // and the copy keeps the frame's own size.
        unsafe { self.context.CopySubresourceRegion(texture, 0, 0, 0, 0, array, index, None) };
        Ok(())
    }

    /// Uploads a software decoded frame into the ring texture through a staging texture.
    ///
    /// NV12 frames are copied, planar 4:2:0 frames are interleaved into NV12; this FFmpeg build has
    /// no swscale, so any other pixel format is refused with a logged warning.
    fn copy_software(&mut self, texture: &ID3D11Texture2D, format: c_int) -> Result<(), MediaError> {
        let planar = format == ffi::AVPixelFormat::AV_PIX_FMT_YUV420P as c_int || format == ffi::AVPixelFormat::AV_PIX_FMT_YUVJ420P as c_int;
        let semi = format == ffi::AVPixelFormat::AV_PIX_FMT_NV12 as c_int;
        if !planar && !semi {
            if !self.unsupported_logged {
                self.unsupported_logged = true;
                log::warn!("bg-video: the software path has no conversion for this pixel format, the frame is skipped");
            }
            return Err(MediaError::Null("an unsupported software pixel format"));
        }
        // SAFETY: the frame is live and was filled by avcodec_receive_frame.
        let (src_w, src_h) = unsafe { ((*self.frame).width.max(0) as u32, (*self.frame).height.max(0) as u32) };
        if src_w == 0 || src_h == 0 {
            return Err(MediaError::Null("a decoded frame with no size"));
        }
        let staging = match &self.staging {
            Some(staging) => staging.clone(),
            None => {
                let staging = create_nv12(&self.device, self.width, self.height, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_WRITE.0 as u32)?;
                self.staging = Some(staging.clone());
                staging
            }
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        // SAFETY: the staging texture is live and CPU writable, and mapped is a valid out pointer.
        unsafe { self.context.Map(&staging, 0, D3D11_MAP_WRITE, 0, Some(&mut mapped)) }.map_err(|_| MediaError::Null("Map"))?;
        let result = if semi { self.upload_nv12(&mapped, src_w, src_h) } else { self.upload_planar(&mapped, src_w, src_h) };
        // SAFETY: the texture was mapped above and the upload no longer uses it.
        unsafe { self.context.Unmap(&staging, 0) };
        result?;
        // SAFETY: both textures are live on the same device with the same size and format.
        unsafe { self.context.CopyResource(texture, &staging) };
        Ok(())
    }

    /// Copies an NV12 frame's Y plane and interleaved UV plane into the mapped staging texture.
    fn upload_nv12(&self, mapped: &D3D11_MAPPED_SUBRESOURCE, src_w: u32, src_h: u32) -> Result<(), MediaError> {
        if mapped.pData.is_null() {
            return Err(MediaError::Null("a null staging map"));
        }
        // SAFETY: the frame is live; its plane pointers and line sizes describe src_w by src_h pixels.
        let (data, lines) = unsafe { ((*self.frame).data, (*self.frame).linesize) };
        let pitch = mapped.RowPitch as usize;
        let y = mapped.pData.cast::<u8>();
        // SAFETY: the UV plane of a mapped NV12 texture begins one pitch times the height in and uses the same pitch.
        let uv = unsafe { y.add(pitch * self.height as usize) };
        // SAFETY: both planes describe NV12 of the given size and the copies stay inside them.
        unsafe { copy_plane(data[0], lines[0], y, pitch, src_w, src_h) };
        unsafe { copy_plane(data[1], lines[1], uv, pitch, src_w, src_h / 2) };
        Ok(())
    }

    /// Interleaves a planar 4:2:0 frame's U and V planes into the mapped staging texture's UV plane.
    fn upload_planar(&self, mapped: &D3D11_MAPPED_SUBRESOURCE, src_w: u32, src_h: u32) -> Result<(), MediaError> {
        if mapped.pData.is_null() {
            return Err(MediaError::Null("a null staging map"));
        }
        // SAFETY: the frame is live; its plane pointers and line sizes describe src_w by src_h pixels.
        let (data, lines) = unsafe { ((*self.frame).data, (*self.frame).linesize) };
        let pitch = mapped.RowPitch as usize;
        let y = mapped.pData.cast::<u8>();
        // SAFETY: the UV plane of a mapped NV12 texture begins one pitch times the height in and uses the same pitch.
        let uv = unsafe { y.add(pitch * self.height as usize) };
        // SAFETY: the Y plane describes 4:2:0 luma of the given size and the copy stays inside it.
        unsafe { copy_plane(data[0], lines[0], y, pitch, src_w, src_h) };
        let (chroma_w, chroma_h) = ((src_w / 2) as usize, (src_h / 2) as usize);
        let (u, v) = (data[1], data[2]);
        let (u_stride, v_stride) = (lines[1].max(0) as usize, lines[2].max(0) as usize);
        for row in 0..chroma_h {
            for column in 0..chroma_w {
                // SAFETY: the U and V planes hold chroma_h rows of chroma_w samples and the UV plane
                // of the staging texture holds the same, interleaved, so every index below is in bounds.
                unsafe {
                    let sample = row * chroma_w + column;
                    *uv.add(sample * 2) = *u.add(row * u_stride + column);
                    *uv.add(sample * 2 + 1) = *v.add(row * v_stride + column);
                }
            }
        }
        Ok(())
    }
}

/// Copies one 8-bit image plane row by row from a source stride to a destination stride.
///
/// # Safety
/// `src` must point at a plane of `src_h` rows of `width` samples with at least `src_stride` bytes per
/// row, and `dst` at a plane with at least `dst_stride` bytes per row.
unsafe fn copy_plane(src: *const u8, src_stride: c_int, dst: *mut u8, dst_stride: usize, width: u32, src_h: u32) {
    if src.is_null() || dst.is_null() {
        return;
    }
    let (src_stride, width) = (src_stride.max(0) as usize, width as usize);
    for row in 0..src_h as usize {
        // SAFETY: both rows are at least width samples long per the caller's contract.
        unsafe { std::ptr::copy_nonoverlapping(src.add(row * src_stride), dst.add(row * dst_stride), width) };
    }
}

/// The AVCodecContext get_format callback of the hardware path: it accepts D3D11 frames and rejects
/// everything else, so a stream D3D11VA cannot decode makes the open fail and the caller falls back
/// to software.
extern "C" fn pick_d3d11_format(_codec: *mut ffi::AVCodecContext, formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut index = 0isize;
    // SAFETY: formats points at the NUL terminated list of pixel formats FFmpeg offers.
    unsafe {
        loop {
            let format = *formats.offset(index);
            if format == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 || format == ffi::AVPixelFormat::AV_PIX_FMT_NONE {
                return format;
            }
            index += 1;
        }
    }
}

/// Allocates a decoder context for the stream and opens it, optionally against a hardware device.
fn open_decoder(codecpar: *mut ffi::AVCodecParameters, decoder: *const ffi::AVCodec, hw: Option<&HwDevice>) -> Result<*mut ffi::AVCodecContext, MediaError> {
    // SAFETY: avcodec_alloc_context3 returns an owned context or null.
    let mut codec = unsafe { ffi::avcodec_alloc_context3(decoder) };
    if codec.is_null() {
        return Err(MediaError::Null("avcodec_alloc_context3"));
    }
    // SAFETY: both pointers are live.
    let code = unsafe { ffi::avcodec_parameters_to_context(codec, codecpar) };
    if code < 0 {
        // SAFETY: the context is owned here and this is its last owner.
        unsafe { ffi::avcodec_free_context(&mut codec) };
        return Err(ff_error(code));
    }
    if let Some(hw) = hw {
        // SAFETY: the context is live and not yet opened, and the buffer reference it takes is
        // released with the context.
        let taken = unsafe {
            (*codec).get_format = Some(pick_d3d11_format);
            (*codec).hw_device_ctx = ffi::av_buffer_ref(hw.0);
            (*codec).hw_device_ctx
        };
        if taken.is_null() {
            // SAFETY: the context is owned here and this is its last owner.
            unsafe { ffi::avcodec_free_context(&mut codec) };
            return Err(MediaError::Null("av_buffer_ref"));
        }
    }
    // SAFETY: the context and decoder are live and the context is not open yet.
    let code = unsafe { ffi::avcodec_open2(codec, decoder, std::ptr::null_mut()) };
    if code < 0 {
        // SAFETY: the context is owned here and this is its last owner.
        unsafe { ffi::avcodec_free_context(&mut codec) };
        return Err(ff_error(code));
    }
    Ok(codec)
}

/// Opens the file, the video stream and the decoder, and builds the textures of one session.
fn open_session(device: &ID3D11Device, path: &Path) -> Result<Session, MediaError> {
    crate::init()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| MediaError::Null("the video path"))?;
    let mut fmt: *mut ffi::AVFormatContext = std::ptr::null_mut();
    // SAFETY: the out pointer is valid, the path is NUL terminated and no custom IO is used.
    let code = unsafe { ffi::avformat_open_input(&mut fmt, c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
    if code < 0 {
        return Err(ff_error(code));
    }
    let session = open_session_rest(device, fmt);
    if session.is_err() {
        // SAFETY: the format context was opened above and this is its last owner.
        unsafe { ffi::avformat_close_input(&mut fmt) };
    }
    session
}

/// Does the work of open_session on an already opened input file.
fn open_session_rest(device: &ID3D11Device, fmt: *mut ffi::AVFormatContext) -> Result<Session, MediaError> {
    // SAFETY: the format context is live and the decoder list out pointer is not wanted.
    let stream_index = unsafe { ffi::av_find_best_stream(fmt, ffi::AVMediaType::AVMEDIA_TYPE_VIDEO, -1, -1, std::ptr::null_mut(), 0) };
    if stream_index < 0 {
        return Err(MediaError::Null("av_find_best_stream"));
    }
    // SAFETY: the format context is live and the index is one of its streams.
    let stream = unsafe { *(*fmt).streams.offset(stream_index as isize) };
    // SAFETY: the stream is live.
    let codecpar = unsafe { (*stream).codecpar };
    // SAFETY: the codec parameters are live.
    let (raw_w, raw_h) = unsafe { ((*codecpar).width.max(0) as u32, (*codecpar).height.max(0) as u32) };
    if raw_w == 0 || raw_h == 0 {
        return Err(MediaError::Null("the video stream has no size"));
    }
    // SAFETY: the codec parameters are live.
    let (codec_id, color_space, color_range) = unsafe { ((*codecpar).codec_id, (*codecpar).color_space, (*codecpar).color_range) };
    // SAFETY: the stream is live; the average frame rate is preferred and the real one is the fallback.
    let rate = unsafe {
        let avg = (*stream).avg_frame_rate;
        if avg.num > 0 && avg.den > 0 { avg } else { (*stream).r_frame_rate }
    };
    let period = if rate.num > 0 && rate.den > 0 {
        Duration::from_secs_f64((f64::from(rate.den) / f64::from(rate.num)).clamp(0.001, 1.0))
    } else {
        Duration::from_secs_f64(1.0 / FALLBACK_FPS)
    };
    let matrix = match color_space {
        ffi::AVColorSpace::AVCOL_SPC_BT709 => VideoMatrix::Bt709,
        ffi::AVColorSpace::AVCOL_SPC_BT2020_NCL | ffi::AVColorSpace::AVCOL_SPC_BT2020_CL => VideoMatrix::Bt2020,
        _ => VideoMatrix::Bt709,
    };
    let range = if color_range == ffi::AVColorRange::AVCOL_RANGE_JPEG { VideoRange::Full } else { VideoRange::Limited };
    // SAFETY: the codec id comes from the codec parameters and names a decoder of this FFmpeg build.
    let decoder = unsafe { ffi::avcodec_find_decoder(codec_id) };
    if decoder.is_null() {
        return Err(MediaError::Null("avcodec_find_decoder"));
    }
    let hw = HwDevice::new(device).ok();
    let hw_open = hw.as_ref().and_then(|hw| open_decoder(codecpar, decoder, Some(hw)).ok());
    let (codec, hw) = match (hw_open, hw) {
        (Some(codec), hw) => {
            log::info!("bg-video: decoding with D3D11VA on the shared device");
            (codec, hw)
        }
        (None, _) => {
            log::info!("bg-video: the hardware decoder rejected the stream, decoding in software and uploading");
            (open_decoder(codecpar, decoder, None)?, None)
        }
    };
    let context = {
        // SAFETY: the device is live; the call returns the device's immediate context or an error.
        let context = unsafe { device.GetImmediateContext() };
        context.map_err(|_| MediaError::Null("GetImmediateContext"))?
    };
    let (width, height) = (raw_w & !1, raw_h & !1);
    let mut ring = Vec::with_capacity(RING_LEN);
    for _ in 0..RING_LEN {
        ring.push(create_nv12(device, width, height, D3D11_USAGE_DEFAULT, D3D11_BIND_SHADER_RESOURCE.0 as u32, 0)?);
    }
    // SAFETY: av_packet_alloc returns an owned packet or null.
    let packet = unsafe { ffi::av_packet_alloc() };
    if packet.is_null() {
        return Err(MediaError::Null("av_packet_alloc"));
    }
    // SAFETY: av_frame_alloc returns an owned frame or null.
    let frame = unsafe { ffi::av_frame_alloc() };
    if frame.is_null() {
        return Err(MediaError::Null("av_frame_alloc"));
    }
    Ok(Session {
        device: device.clone(),
        context,
        fmt,
        codec,
        stream_index: stream_index as usize,
        period,
        color: VideoColor { matrix, range },
        width,
        height,
        packet,
        frame,
        staging: None,
        ring,
        ring_next: 0,
        unsupported_logged: false,
        _hw: hw,
    })
}

/// Creates an NV12 texture of the given size with the given usage, bindings and CPU access.
fn create_nv12(device: &ID3D11Device, width: u32, height: u32, usage: D3D11_USAGE, bind_flags: u32, cpu_flags: u32) -> Result<ID3D11Texture2D, MediaError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind_flags,
        CPUAccessFlags: cpu_flags,
        MiscFlags: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is a plain value and texture is a valid out pointer.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }.map_err(|_| MediaError::Null("CreateTexture2D"))?;
    texture.ok_or(MediaError::Null("CreateTexture2D"))
}

/// The body of the decode thread: reads, decodes, paces and publishes until the stop flag goes up.
fn run(SendSession(mut session): SendSession, latest: Arc<Mutex<Option<VideoFrame>>>, stop: Arc<AtomicBool>) {
    let start = Instant::now();
    let mut index: u64 = 0;
    let mut input_done = false;
    while !stop.load(Ordering::Relaxed) {
        if !input_done {
            // SAFETY: the format context and the packet are live and owned by this session.
            let read = unsafe { ffi::av_read_frame(session.fmt, session.packet) };
            if read < 0 {
                if read != ffi::AVERROR_EOF {
                    log::warn!("bg-video: the input could not be read: {}", ff_error(read));
                    break;
                }
                input_done = true;
            } else {
                // SAFETY: the packet was filled by av_read_frame above.
                let for_us = unsafe { (*session.packet).stream_index } == session.stream_index as c_int;
                if for_us {
                    // SAFETY: the decoder and the packet are live; the packet is released right after.
                    let sent = unsafe { ffi::avcodec_send_packet(session.codec, session.packet) };
                    // SAFETY: the packet is owned by the session and is no longer needed.
                    unsafe { ffi::av_packet_unref(session.packet) };
                    if sent < 0 && sent != AVERROR_EAGAIN {
                        log::warn!("bg-video: the decoder rejected a packet: {}", ff_error(sent));
                        break;
                    }
                } else {
                    // SAFETY: the packet is owned by the session and is no longer needed.
                    unsafe { ffi::av_packet_unref(session.packet) };
                }
            }
        } else {
            // Flush the decoder with a null packet; later calls only report EOF, so this stays cheap.
            // SAFETY: the decoder is live and a null packet asks it to drain.
            unsafe { ffi::avcodec_send_packet(session.codec, std::ptr::null_mut()) };
        }
        // Hand out every frame the decoder has ready.
        loop {
            // SAFETY: the decoder and the frame are live and owned by this session.
            let received = unsafe { ffi::avcodec_receive_frame(session.codec, session.frame) };
            if received < 0 {
                break;
            }
            if !wait_for_slot(start, index, session.period, &stop) {
                return;
            }
            if let Err(error) = session.publish(&latest) {
                log::warn!("bg-video: a frame could not be shown: {error}");
            }
            index = index.saturating_add(1);
        }
        if input_done {
            // The file ended: start over from the beginning without resetting the pacing.
            // SAFETY: the format context and decoder are live; the seek uses the default stream.
            unsafe {
                ffi::av_seek_frame(session.fmt, -1, 0, ffi::AVSEEK_FLAG_BACKWARD);
                ffi::avcodec_flush_buffers(session.codec);
            }
            input_done = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pacing_continues_across_a_loop_restart() {
        let period = Duration::from_millis(40);
        let start = Instant::now();
        // The first frame of every loop is due at its slot on one shared timeline.
        assert_eq!(due_at(start, 0, period), start);
        // The last frame of a loop and the first frame of the next stay exactly one period apart.
        let last = due_at(start, 49, period);
        let first_of_next = due_at(start, 50, period);
        assert_eq!(first_of_next - last, period);
        // Three loops in, the frame number has not reset and the slots keep their spacing.
        assert_eq!(due_at(start, 148, period), start + period * 148);
        assert_eq!(due_at(start, 149, period) - due_at(start, 148, period), period);
    }

    #[test]
    fn a_waiting_slot_is_left_when_the_stop_flag_goes_up() {
        let stop = AtomicBool::new(true);
        let start = Instant::now();
        assert!(!wait_for_slot(start, 1, Duration::from_millis(50), &stop));
        // A slot that is already due does not wait at all.
        let running = AtomicBool::new(false);
        assert!(wait_for_slot(start, 0, Duration::from_millis(50), &running));
    }
}
