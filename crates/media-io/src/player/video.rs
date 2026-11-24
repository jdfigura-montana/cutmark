//! One video file decoded on its own thread into RGBA frames in system memory.
//!
//! [`VideoDecoder::open`] tries D3D11VA on its own D3D11 device and falls back to
//! software decoding the way `bgvideo` does. Every decoded frame is moved into
//! system memory when needed, scaled with swscale to RGBA at most `max_width`
//! wide and queued for the caller, who pulls frames with [`VideoDecoder::frame_at`]
//! as its own clock advances. The queue holds a few frames, so memory stays flat
//! on long takes.
#![allow(unsafe_code)]

use std::collections::VecDeque;
use std::ffi::{CString, c_int};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use ffmpeg_next::ffi;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device};
use windows::core::Interface;

use crate::hw::{MediaError, ff_error};

/// How many scaled frames the queue holds while the caller has not taken them.
const QUEUE_LEN: usize = 8;

/// The swscale flag set: bilinear, a good middle ground for downscaling.
const SWS_BILINEAR: c_int = 2;

/// The AVERROR code for EAGAIN, which the decoder returns when it wants to hand out a frame first.
const AVERROR_EAGAIN: c_int = -11;

/// One decoded frame in system memory: RGBA8 bytes, top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaFrame {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The presentation time in milliseconds from the start of the file.
    pub pts_ms: u64,
    /// The RGBA bytes, `width * height * 4` of them.
    pub data: Vec<u8>,
}

/// The play state and the pending seek the decode thread reads under its lock.
struct ControlState {
    /// Whether the thread decodes ahead into the queue or waits.
    playing: bool,
    /// A seek the caller asked for, in milliseconds, not yet carried out.
    pending_seek: Option<u64>,
    /// Whether the file was read to the end, so the thread waits even while playing.
    eof: bool,
}

/// The shared play state with the condition variable the thread sleeps on.
type Control = (Mutex<ControlState>, Condvar);

/// The shared frame queue with the condition variable a full queue waits on.
type Queue = (Mutex<VecDeque<RgbaFrame>>, Condvar);

/// One video file decoded on its own thread into RGBA frames the caller pulls with its own clock.
pub struct VideoDecoder {
    /// The play state shared with the decode thread.
    control: Arc<Control>,
    /// The queue of decoded frames the caller drains with `frame_at`.
    queue: Arc<Queue>,
    /// The frame `frame_at` returned last, shown again while nothing newer arrives.
    shown: Arc<Mutex<Option<RgbaFrame>>>,
    /// Set by Drop to end the decode thread.
    stop: Arc<AtomicBool>,
    /// The decode thread, taken and joined by Drop.
    thread: Option<JoinHandle<()>>,
    /// The length of the file in milliseconds, from its headers.
    duration_ms: u64,
    /// Whether the hardware path opened; false means software decoding.
    hardware: bool,
}

impl VideoDecoder {
    /// Opens `path` on a new "video-decode" thread that decodes with D3D11VA on its own D3D11
    /// device, or in software when the hardware decoder rejects the stream, and scales every
    /// frame to RGBA at most `max_width` wide keeping the aspect; 0 keeps the source width.
    ///
    /// The file and the decoder are opened here, so a bad path or an unreadable stream fails
    /// before the thread starts.
    pub fn open(path: &Path, max_width: u32) -> Result<Self, MediaError> {
        let device = create_device();
        let session = open_session(path, max_width, device.as_ref())?;
        let (duration_ms, hardware) = (session.duration_ms, session.hardware);
        log::info!(
            "video player {}: {} decoding, {}x{} scaled to at most {} wide, {} ms long",
            path.display(),
            if hardware { "D3D11VA on its own device" } else { "software" },
            session.width,
            session.height,
            if max_width == 0 { session.width } else { max_width },
            duration_ms
        );
        let control: Arc<Control> = Arc::new((Mutex::new(ControlState { playing: false, pending_seek: None, eof: false }), Condvar::new()));
        let queue: Arc<Queue> = Arc::new((Mutex::new(VecDeque::new()), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let payload = SendSession(session);
            let (thread_control, thread_queue, thread_stop) = (Arc::clone(&control), Arc::clone(&queue), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("video-decode".to_owned())
                .spawn(move || run(payload, thread_control, thread_queue, thread_stop))
                .map_err(|_| MediaError::Null("the video decode thread"))?
        };
        Ok(Self { control, queue, shown: Arc::new(Mutex::new(None)), stop, thread: Some(thread), duration_ms, hardware })
    }

    /// Resumes decoding: the thread fills the queue ahead of the caller's clock.
    pub fn play(&self) {
        let (state_lock, signal) = &*self.control;
        let mut state = lock(state_lock);
        state.playing = true;
        signal.notify_all();
    }

    /// Stops decoding at the current position; frames already queued stay available.
    pub fn pause(&self) {
        let (state_lock, signal) = &*self.control;
        let mut state = lock(state_lock);
        state.playing = false;
        signal.notify_all();
    }

    /// Seeks backward to the keyframe at or before `ms`, decodes forward dropping the frames
    /// until the target and delivers the first frame at or after it, even while paused. The
    /// queue and the last shown frame are dropped, so `frame_at` only shows the new position.
    pub fn seek(&self, ms: u64) {
        {
            let (state_lock, signal) = &*self.control;
            let mut state = lock(state_lock);
            state.pending_seek = Some(ms);
            state.eof = false;
            signal.notify_all();
        }
        let (queue_lock, space) = &*self.queue;
        lock(queue_lock).clear();
        space.notify_all();
        *lock(&self.shown) = None;
    }

    /// The newest decoded frame with a presentation time at or before `clock_ms`, dropping the
    /// older ones; frames past the clock stay queued. `None` means nothing decoded yet.
    pub fn frame_at(&self, clock_ms: u64) -> Option<RgbaFrame> {
        let (queue_lock, _) = &*self.queue;
        let mut queue = lock(queue_lock);
        let best = newest_at_or_before(&mut queue, lock(&self.shown).take(), clock_ms);
        *lock(&self.shown) = best.clone();
        best
    }

    /// The length of the file in milliseconds, read from its headers at open.
    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    /// Whether the decoder runs on a D3D11 device; false means software decoding.
    pub fn decodes_on_gpu(&self) -> bool {
        self.hardware
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let (_, control_signal) = &*self.control;
        control_signal.notify_all();
        let (_, queue_signal) = &*self.queue;
        queue_signal.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Locks a mutex, carrying on with the data if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The newest frame at or before `clock_ms`: drops the queued frames older than the one
/// returned, leaves the frames past the clock queued and refuses a last shown frame past
/// the clock.
fn newest_at_or_before(queue: &mut VecDeque<RgbaFrame>, last: Option<RgbaFrame>, clock_ms: u64) -> Option<RgbaFrame> {
    let mut best = last.filter(|frame| frame.pts_ms <= clock_ms);
    while queue.front().is_some_and(|front| front.pts_ms <= clock_ms) {
        best = queue.pop_front().or(best);
    }
    best
}

/// Queues one decoded frame, waiting for room; a seek that arrives meanwhile makes the
/// frame stale and it is dropped. Lock order: the queue first, then the control state,
/// which `seek` never holds while taking the queue, so the two never deadlock.
fn push_frame(control: &Control, queue: &Queue, frame: RgbaFrame, stop: &AtomicBool) {
    let (queue_lock, space) = queue;
    let mut queued = lock(queue_lock);
    while queued.len() >= QUEUE_LEN && !stop.load(Ordering::Relaxed) {
        queued = space.wait(queued).unwrap_or_else(PoisonError::into_inner);
    }
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let (control_lock, _) = control;
    if lock(control_lock).pending_seek.is_some() {
        return;
    }
    queued.push_back(frame);
}

/// Marks the end of the stream so the thread waits for a play or a seek instead of spinning.
fn mark_eof(control: &Control) {
    let (state_lock, signal) = control;
    let mut state = lock(state_lock);
    if !state.eof {
        state.eof = true;
        signal.notify_all();
    }
}

/// The body of the decode thread: waits for play or a seek, decodes and fills the queue.
fn run(SendSession(mut session): SendSession, control: Arc<Control>, queue: Arc<Queue>, stop: Arc<AtomicBool>) {
    let mut seek_target: Option<u64> = None;
    loop {
        {
            let (state_lock, signal) = &*control;
            let mut state = lock(state_lock);
            while !stop.load(Ordering::Relaxed) && state.pending_seek.is_none() && seek_target.is_none() && (!state.playing || state.eof) {
                state = signal.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            if stop.load(Ordering::Relaxed) {
                return;
            }
            if let Some(target) = state.pending_seek.take() {
                match session.seek(target) {
                    Ok(()) => {
                        seek_target = Some(target);
                        state.eof = false;
                    }
                    Err(error) => log::warn!("video decode: the seek to {target} ms failed: {error}"),
                }
            }
        }
        if session.input_done {
            mark_eof(&control);
            continue;
        }
        match session.decode_next() {
            Ok(Some(frame)) => {
                if seek_target.is_none_or(|target| frame.pts_ms >= target) {
                    push_frame(&control, &queue, frame, &stop);
                    seek_target = None;
                }
            }
            Ok(None) => mark_eof(&control),
            Err(error) => {
                log::warn!("video decode: {error}");
                mark_eof(&control);
            }
        }
    }
}

/// Creates a D3D11 device of its own for the decoder's hardware path, or None when none can be made.
fn create_device() -> Option<ID3D11Device> {
    let mut device: Option<ID3D11Device> = None;
    // SAFETY: the out pointers are valid; no adapter, module or feature list is wanted.
    let created = unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    };
    if created.is_ok() { device } else { None }
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

/// An FFmpeg D3D11VA hardware device built on the decoder's own D3D11 device, kept alive while the decoder uses it.
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
            (*d3d11_ctx).device = device.as_raw().cast::<std::ffi::c_void>().cast();
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

/// The opened decoder, its input file and the scale context one decode thread works with.
struct Session {
    /// The opened input file.
    fmt: *mut ffi::AVFormatContext,
    /// The opened video decoder.
    codec: *mut ffi::AVCodecContext,
    /// The index of the video stream in the file.
    stream_index: usize,
    /// The stream time base, as (numerator, denominator).
    time_base: (u32, u32),
    /// The frame width of the stream.
    width: u32,
    /// The frame height of the stream.
    height: u32,
    /// The width and height the RGBA frames are scaled to.
    out_size: (u32, u32),
    /// The packet reused for every read.
    packet: *mut ffi::AVPacket,
    /// The frame reused for every decode.
    frame: *mut ffi::AVFrame,
    /// The system-memory frame a hardware frame is transferred into, filled on first use.
    sw_frame: *mut ffi::AVFrame,
    /// The swscale context, made on first use and rebuilt when the source format changes.
    sws: *mut ffi::SwsContext,
    /// The source format and size the sws context was built for.
    sws_src: Option<(ffi::AVPixelFormat, u32, u32)>,
    /// Whether end of input was read and the decoder was flushed.
    input_done: bool,
    /// The length of the file in milliseconds, from its headers.
    duration_ms: u64,
    /// Whether the hardware path opened; false means software decoding.
    hardware: bool,
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
        // SAFETY: every pointer is either null or a resource this session owns, and each call is
        // the documented way to release it. The decoder is freed before the hardware device,
        // whose reference it holds, so the fields above must stay in this order.
        unsafe {
            ffi::sws_freeContext(self.sws);
            ffi::av_frame_free(&mut self.sw_frame);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_packet_free(&mut self.packet);
            ffi::avcodec_free_context(&mut self.codec);
            ffi::avformat_close_input(&mut self.fmt);
        }
    }
}

impl Session {
    /// Seeks backward to the keyframe at or before `target_ms` and flushes the decoder, so the
    /// next frames come from at or before the target and can be decoded forward from there.
    fn seek(&mut self, target_ms: u64) -> Result<(), MediaError> {
        let (num, den) = self.time_base;
        let ticks = if num > 0 { (target_ms as i64 * i64::from(den)) / (i64::from(num) * 1000) } else { 0 };
        // SAFETY: the format context is live and the stream index is one of its streams; the
        // backward flag lands on the largest keyframe at or before the target.
        let code = unsafe { ffi::avformat_seek_file(self.fmt, self.stream_index as c_int, i64::MIN, ticks, ticks, ffi::AVSEEK_FLAG_BACKWARD as c_int) };
        if code < 0 {
            return Err(ff_error(code));
        }
        // SAFETY: the decoder is live; flushing drops the frames buffered across the seek.
        unsafe { ffi::avcodec_flush_buffers(self.codec) };
        self.input_done = false;
        Ok(())
    }

    /// Reads and decodes the next frame of the file, scaled to RGBA, or None at the end.
    fn decode_next(&mut self) -> Result<Option<RgbaFrame>, MediaError> {
        loop {
            // SAFETY: the decoder and the frame are live and owned by this session.
            let received = unsafe { ffi::avcodec_receive_frame(self.codec, self.frame) };
            if received >= 0 {
                return self.convert().map(Some);
            }
            if received == ffi::AVERROR_EOF || (received != AVERROR_EAGAIN && self.input_done) {
                return Ok(None);
            }
            if received != AVERROR_EAGAIN {
                return Err(ff_error(received));
            }
            // SAFETY: the format context and the packet are live and owned by this session.
            let read = unsafe { ffi::av_read_frame(self.fmt, self.packet) };
            if read < 0 {
                // SAFETY: the packet is owned by the session and is no longer needed.
                unsafe { ffi::av_packet_unref(self.packet) };
                if read == ffi::AVERROR_EOF {
                    self.input_done = true;
                    // Flush the decoder with a null packet so its buffered frames come out.
                    // SAFETY: the decoder is live; a null packet asks it to drain.
                    unsafe { ffi::avcodec_send_packet(self.codec, std::ptr::null_mut()) };
                    continue;
                }
                return Err(ff_error(read));
            }
            // SAFETY: the packet was filled by av_read_frame above.
            let ours = unsafe { (*self.packet).stream_index } == self.stream_index as c_int;
            if ours {
                // SAFETY: the decoder and the packet are live; the packet is released right after.
                let sent = unsafe { ffi::avcodec_send_packet(self.codec, self.packet) };
                // SAFETY: the packet is owned by the session and is no longer needed.
                unsafe { ffi::av_packet_unref(self.packet) };
                if sent < 0 && sent != AVERROR_EAGAIN {
                    return Err(ff_error(sent));
                }
            } else {
                // SAFETY: the packet is owned by the session and is no longer needed.
                unsafe { ffi::av_packet_unref(self.packet) };
            }
        }
    }

    /// Scales the decoded frame into a new RGBA frame, transferring it into system memory first
    /// when the decoder produced it on the D3D11 device.
    fn convert(&mut self) -> Result<RgbaFrame, MediaError> {
        // SAFETY: the frame is live and was filled by avcodec_receive_frame.
        let (raw_format, src_w, src_h) = unsafe { ((*self.frame).format, (*self.frame).width.max(0) as u32, (*self.frame).height.max(0) as u32) };
        if src_w == 0 || src_h == 0 {
            return Err(MediaError::Null("a decoded frame with no size"));
        }
        // SAFETY: the frame is live; best_effort_timestamp carries the true pts when it has one.
        let ts = unsafe {
            let best = (*self.frame).best_effort_timestamp;
            if best != ffi::AV_NOPTS_VALUE { best } else { (*self.frame).pts }
        };
        let pts_ms = pts_to_ms(ts, self.time_base);
        let hardware = raw_format == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 as c_int;
        if hardware {
            self.transfer_to_memory()?;
        }
        let source = if hardware { self.sw_frame } else { self.frame };
        // SAFETY: the source frame is live; its data pointers and strides describe src_w by src_h pixels.
        let (format_number, src_data, src_stride) = unsafe { ((*source).format, (*source).data, (*source).linesize) };
        let Some(src_format) = pixel_format(format_number) else {
            return Err(MediaError::Null("a decoded frame in an unknown pixel format"));
        };
        let sws = self.scale_context(src_format, src_w, src_h)?;
        let (out_w, out_h) = self.out_size;
        let mut data = vec![0u8; out_w as usize * out_h as usize * 4];
        let dst_data: [*mut u8; 8] = std::array::from_fn(|index| if index == 0 { data.as_mut_ptr() } else { std::ptr::null_mut() });
        let mut dst_stride: [c_int; 8] = [0; 8];
        dst_stride[0] = (out_w * 4) as c_int;
        // SAFETY: the source planes describe src_w by src_h pixels of src_format and the
        // destination buffer holds out_w by out_h RGBA pixels with the stride above.
        let written = unsafe { ffi::sws_scale(sws, src_data.as_ptr().cast(), src_stride.as_ptr(), 0, src_h as c_int, dst_data.as_ptr(), dst_stride.as_ptr()) };
        if written < 0 {
            return Err(ff_error(written));
        }
        Ok(RgbaFrame { width: out_w, height: out_h, pts_ms, data })
    }

    /// Copies a hardware decoded D3D11 frame into the reusable system-memory frame.
    fn transfer_to_memory(&mut self) -> Result<(), MediaError> {
        // SAFETY: the frame is live; its hw_frames_ctx names the pool and its software format.
        let sw_format = unsafe {
            let frames_ctx = (*self.frame).hw_frames_ctx;
            if frames_ctx.is_null() {
                ffi::AVPixelFormat::AV_PIX_FMT_NV12 as c_int
            } else {
                (*(*frames_ctx).data.cast::<ffi::AVHWFramesContext>()).sw_format as c_int
            }
        };
        // SAFETY: the frames are live and owned by this session; the target frame is (re)filled
        // to the source size in the pool's software format and the transfer copies one frame.
        let code = unsafe {
            if (*self.sw_frame).format != sw_format || (*self.sw_frame).width != (*self.frame).width || (*self.sw_frame).height != (*self.frame).height {
                ffi::av_frame_unref(self.sw_frame);
                (*self.sw_frame).format = sw_format;
                (*self.sw_frame).width = (*self.frame).width;
                (*self.sw_frame).height = (*self.frame).height;
                let allocated = ffi::av_frame_get_buffer(self.sw_frame, 0);
                if allocated < 0 {
                    return Err(ff_error(allocated));
                }
            }
            ffi::av_hwframe_transfer_data(self.sw_frame, self.frame, 0)
        };
        if code < 0 {
            return Err(ff_error(code));
        }
        Ok(())
    }

    /// The swscale context for the source format and size, made on first use and rebuilt on change.
    fn scale_context(&mut self, src_format: ffi::AVPixelFormat, src_w: u32, src_h: u32) -> Result<*mut ffi::SwsContext, MediaError> {
        let key = (src_format, src_w, src_h);
        if self.sws.is_null() || self.sws_src != Some(key) {
            if !self.sws.is_null() {
                // SAFETY: the context was made by sws_getContext and this is its last owner.
                unsafe { ffi::sws_freeContext(self.sws) };
                self.sws = std::ptr::null_mut();
            }
            let (out_w, out_h) = self.out_size;
            // SAFETY: every argument is a plain value; a null return signals failure.
            let sws = unsafe {
                ffi::sws_getContext(
                    src_w as c_int,
                    src_h as c_int,
                    src_format,
                    out_w as c_int,
                    out_h as c_int,
                    ffi::AVPixelFormat::AV_PIX_FMT_RGBA,
                    SWS_BILINEAR,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            if sws.is_null() {
                return Err(MediaError::Null("sws_getContext"));
            }
            self.sws = sws;
            self.sws_src = Some(key);
        }
        Ok(self.sws)
    }
}

/// Opens the file, the video stream and the decoder, trying D3D11VA on `device` first.
fn open_session(path: &Path, max_width: u32, device: Option<&ID3D11Device>) -> Result<Session, MediaError> {
    crate::init()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| MediaError::Null("the video path"))?;
    let mut fmt: *mut ffi::AVFormatContext = std::ptr::null_mut();
    // SAFETY: the out pointer is valid, the path is NUL terminated and no custom IO is used.
    let code = unsafe { ffi::avformat_open_input(&mut fmt, c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
    if code < 0 {
        return Err(ff_error(code));
    }
    let session = open_session_rest(fmt, max_width, device);
    if session.is_err() {
        // SAFETY: the format context was opened above and this is its last owner.
        unsafe { ffi::avformat_close_input(&mut fmt) };
    }
    session
}

/// Does the work of open_session on an already opened input file.
fn open_session_rest(fmt: *mut ffi::AVFormatContext, max_width: u32, device: Option<&ID3D11Device>) -> Result<Session, MediaError> {
    // SAFETY: the format context is live and the decoder list out pointer is not wanted.
    let stream_index = unsafe { ffi::av_find_best_stream(fmt, ffi::AVMediaType::AVMEDIA_TYPE_VIDEO, -1, -1, std::ptr::null_mut(), 0) };
    if stream_index < 0 {
        return Err(MediaError::Null("av_find_best_stream"));
    }
    // SAFETY: the format context is live and the index is one of its streams.
    let stream = unsafe { *(*fmt).streams.offset(stream_index as isize) };
    // SAFETY: the stream is live; its codec parameters, time base and the file duration are plain data.
    let (codecpar, time_base, file_duration) = unsafe { ((*stream).codecpar, (*stream).time_base, (*fmt).duration) };
    if codecpar.is_null() {
        return Err(MediaError::Null("the video stream has no codec parameters"));
    }
    // SAFETY: the codec parameters are live.
    let (codec_id, width, height) = unsafe { ((*codecpar).codec_id, (*codecpar).width.max(0) as u32, (*codecpar).height.max(0) as u32) };
    if width == 0 || height == 0 {
        return Err(MediaError::Null("the video stream has no size"));
    }
    // SAFETY: the codec id comes from the codec parameters and names a decoder of this FFmpeg build.
    let decoder = unsafe { ffi::avcodec_find_decoder(codec_id) };
    if decoder.is_null() {
        return Err(MediaError::Null("avcodec_find_decoder"));
    }
    let hw = device.and_then(|device| HwDevice::new(device).ok());
    let hw_open = hw.as_ref().and_then(|hw| open_decoder(codecpar, decoder, Some(hw)).ok());
    let (codec, hw, hardware) = match hw_open {
        Some(codec) => (codec, hw, true),
        None => (open_decoder(codecpar, decoder, None)?, None, false),
    };
    let time_base = (time_base.num.max(0) as u32, time_base.den.max(0) as u32);
    let duration_ms = if file_duration > 0 { file_duration as u64 / 1_000 } else { 0 };
    let out_w = if max_width == 0 || max_width >= width { width } else { max_width };
    let out_h = (u64::from(height) * u64::from(out_w)).div_ceil(u64::from(width)).max(1) as u32;
    let mut session = Session {
        fmt,
        codec,
        stream_index: stream_index as usize,
        time_base,
        width,
        height,
        out_size: (out_w, out_h),
        packet: std::ptr::null_mut(),
        frame: std::ptr::null_mut(),
        sw_frame: std::ptr::null_mut(),
        sws: std::ptr::null_mut(),
        sws_src: None,
        input_done: false,
        duration_ms,
        hardware,
        _hw: hw,
    };
    // SAFETY: av_packet_alloc returns an owned packet or null; on null the session drop cleans up.
    session.packet = unsafe { ffi::av_packet_alloc() };
    if session.packet.is_null() {
        return Err(MediaError::Null("av_packet_alloc"));
    }
    // SAFETY: av_frame_alloc returns an owned frame or null; on null the session drop cleans up.
    session.frame = unsafe { ffi::av_frame_alloc() };
    if session.frame.is_null() {
        return Err(MediaError::Null("av_frame_alloc"));
    }
    // SAFETY: av_frame_alloc returns an owned frame or null; on null the session drop cleans up.
    session.sw_frame = unsafe { ffi::av_frame_alloc() };
    if session.sw_frame.is_null() {
        return Err(MediaError::Null("av_frame_alloc"));
    }
    Ok(session)
}

/// Turns the pixel format number of an AVFrame into the enum the swscale functions take, or
/// None when the number names no pixel format this FFmpeg build knows.
fn pixel_format(format: c_int) -> Option<ffi::AVPixelFormat> {
    let none = ffi::AVPixelFormat::AV_PIX_FMT_NONE as c_int;
    let count = ffi::AVPixelFormat::AV_PIX_FMT_NB as c_int;
    if format != none && !(0..count).contains(&format) {
        return None;
    }
    // SAFETY: the value was checked against the enum's whole variant range above (NONE and
    // 0 to AV_PIX_FMT_NB minus one), so it names a real variant.
    Some(unsafe { std::mem::transmute::<c_int, ffi::AVPixelFormat>(format) })
}

/// A presentation timestamp in stream ticks as milliseconds from the start of the file.
fn pts_to_ms(pts: i64, time_base: (u32, u32)) -> u64 {
    if pts <= 0 || time_base.0 == 0 || time_base.1 == 0 {
        return 0;
    }
    let ms = i128::from(pts) * 1000 * i128::from(time_base.0) / i128::from(time_base.1);
    u64::try_from(ms).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use super::{RgbaFrame, VideoDecoder, newest_at_or_before};
    use crate::amf::AmfEncoder;
    use crate::encode::{Codec, Encoder, EncoderConfig, Preset, RateControl};
    use crate::hw::{D3d11Device, D3d11FramePool, HwFrame};
    use crate::mux::{Container, Muxer};
    use crate::player::probe::probe;

    /// The frame period of the shared test clip, in milliseconds.
    const CLIP_FRAME_MS: u64 = 1000 / 60 + 1;

    /// Encodes a tiny AV1 take with the AMF encoder into an MKV file.
    fn encode_clip(path: &Path, width: u32, height: u32, fps: u32, frames: u32) -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let device = super::create_device().ok_or(crate::MediaError::Null("D3D11CreateDevice"))?;
        let hw_device = D3d11Device::from_device(&device)?;
        let pool = D3d11FramePool::new(&hw_device, width, height, 8)?;
        let config = EncoderConfig {
            codec: Codec::Av1,
            width,
            height,
            fps_num: fps,
            fps_den: 1,
            rate_control: RateControl::ConstantQp { qp_i: 28, qp_p: 30 },
            preset: Preset::Speed,
            keyframe_secs: 2.0,
        };
        let mut encoder = AmfEncoder::open(&config, &pool)?;
        let mut muxer = Muxer::create(path, Container::Mkv)?;
        muxer.add_video(encoder.parameters(), encoder.time_base(), (fps as i32, 1))?;
        muxer.start()?;
        for index in 0..frames {
            let mut frame = HwFrame::from_pool(&pool)?;
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

    /// The shared test clip: 2560x1440 AV1 at 60 fps, two seconds long, encoded once per process.
    fn test_clip() -> Result<&'static PathBuf, String> {
        static CLIP: OnceLock<Result<PathBuf, String>> = OnceLock::new();
        CLIP.get_or_init(|| {
            let path = std::env::temp_dir().join(format!("cutmark_player_test_{}__1440p60.mkv", std::process::id()));
            match encode_clip(&path, 2560, 1440, 60, 120) {
                Ok(()) => Ok(path),
                Err(error) => Err(error.to_string()),
            }
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    /// A synthetic one pixel frame for the queue tests.
    fn frame_at_pts(pts_ms: u64) -> RgbaFrame {
        RgbaFrame { width: 1, height: 1, pts_ms, data: vec![0; 4] }
    }

    /// Waits until `frame_at` hands out a frame, or None after the budget is spent.
    fn wait_for_frame(decoder: &VideoDecoder, clock_ms: u64, budget: Duration) -> Option<RgbaFrame> {
        let started = Instant::now();
        loop {
            if let Some(frame) = decoder.frame_at(clock_ms) {
                return Some(frame);
            }
            if started.elapsed() >= budget {
                return None;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn the_probe_reports_the_clip_size_duration_and_codec() -> Result<(), Box<dyn std::error::Error>> {
        let clip = match test_clip() {
            Ok(clip) => clip,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let info = probe(clip)?;
        let video = info.video.clone().ok_or("the clip has no video stream")?;
        assert_eq!(video.codec, "av1");
        assert_eq!(video.width, 2560);
        assert_eq!(video.height, 1440);
        assert!((video.fps - 60.0).abs() < 0.5, "{}", video.fps);
        assert!(info.audio.is_empty());
        // Two seconds of frames at 60 fps, within one frame of the target.
        let duration = i64::try_from(info.duration_ms).unwrap_or(i64::MAX);
        assert!((2000 - duration).abs() <= i64::try_from(CLIP_FRAME_MS).unwrap_or(0), "{duration}");
        Ok(())
    }

    #[test]
    fn a_seek_to_the_middle_returns_a_frame_within_one_frame_of_the_target() -> Result<(), Box<dyn std::error::Error>> {
        let clip = match test_clip() {
            Ok(clip) => clip,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let decoder = VideoDecoder::open(clip, 1280)?;
        let target = decoder.duration_ms() / 2;
        decoder.seek(target);
        // The seek delivers while paused; the clock may be anywhere at or after the target.
        let frame = wait_for_frame(&decoder, decoder.duration_ms(), Duration::from_secs(10)).ok_or("no frame within ten seconds of the seek")?;
        assert!(frame.pts_ms + CLIP_FRAME_MS >= target, "{} < {target}", frame.pts_ms);
        assert!(frame.pts_ms <= target + CLIP_FRAME_MS, "{} > {target}", frame.pts_ms);
        // The frame is scaled to the requested width, keeping the aspect.
        assert_eq!(frame.width, 1280);
        assert_eq!(frame.height, 720);
        assert_eq!(frame.data.len(), usize::try_from(frame.width).unwrap_or(0) * usize::try_from(frame.height).unwrap_or(0) * 4);
        Ok(())
    }

    #[test]
    fn frame_at_never_returns_a_frame_newer_than_the_clock() {
        // Pure queue logic: only frames at or before the clock come out.
        let mut queue = VecDeque::new();
        for pts in [10u64, 20, 30] {
            queue.push_back(frame_at_pts(pts));
        }
        let best = newest_at_or_before(&mut queue, Some(frame_at_pts(5)), 25);
        assert_eq!(best.as_ref().map(|frame| frame.pts_ms), Some(20));
        assert_eq!(queue.front().map(|frame| frame.pts_ms), Some(30));
        let best = newest_at_or_before(&mut queue, best, 40);
        assert_eq!(best.map(|frame| frame.pts_ms), Some(30));
        assert!(queue.is_empty());
        // A shown frame past the clock never comes back.
        assert!(newest_at_or_before(&mut VecDeque::new(), Some(frame_at_pts(50)), 9).is_none());
        // A shown frame at or before the clock still shows.
        assert_eq!(newest_at_or_before(&mut VecDeque::new(), Some(frame_at_pts(7)), 9).map(|frame| frame.pts_ms), Some(7));
        // Nothing queued and nothing shown is None.
        assert!(newest_at_or_before(&mut VecDeque::new(), None, 100).is_none());
    }

    #[test]
    fn a_decoded_stream_also_respects_the_clock() -> Result<(), Box<dyn std::error::Error>> {
        let clip = match test_clip() {
            Ok(clip) => clip,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let decoder = VideoDecoder::open(clip, 640)?;
        decoder.play();
        std::thread::sleep(Duration::from_millis(60));
        decoder.pause();
        for clock in [0u64, 50, 200, 1000, 5000] {
            if let Some(frame) = decoder.frame_at(clock) {
                assert!(frame.pts_ms <= clock, "pts {} above clock {clock}", frame.pts_ms);
            }
        }
        // A frame at or before a clock keeps showing while nothing newer arrives.
        let shown = decoder.frame_at(300).ok_or("no frame at or before 300 ms after playing")?;
        assert_eq!(shown.pts_ms, decoder.frame_at(300).map(|frame| frame.pts_ms).unwrap_or(u64::MAX));
        Ok(())
    }

    #[test]
    fn a_1440p60_av1_take_reports_its_decode_path_and_seek_latency() -> Result<(), Box<dyn std::error::Error>> {
        let clip = match test_clip() {
            Ok(clip) => clip,
            Err(reason) => {
                eprintln!("skipped: {reason}");
                return Ok(());
            }
        };
        let decoder = VideoDecoder::open(clip, 1280)?;
        let path = if decoder.decodes_on_gpu() { "D3D11VA hardware" } else { "software" };
        let target = decoder.duration_ms() / 2;
        let started = Instant::now();
        decoder.seek(target);
        let mut first = None;
        while started.elapsed() < Duration::from_secs(10) {
            if let Some(frame) = decoder.frame_at(u64::MAX)
                && frame.pts_ms >= target
            {
                first = Some(frame);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let elapsed = started.elapsed();
        let frame = first.ok_or("no frame within ten seconds of the seek")?;
        assert!(frame.pts_ms <= target + CLIP_FRAME_MS, "{} > {target}", frame.pts_ms);
        let report = format!("1440p60 AV1 take: decode path {path}; seek to {target} ms delivered pts {} after {elapsed:?}", frame.pts_ms);
        eprintln!("{report}");
        let _ = std::fs::write(std::env::temp_dir().join("cutmark_r9a_player_report.txt"), report);
        Ok(())
    }
}
