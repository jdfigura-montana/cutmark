//! One audio file decoded on its own thread into stereo f32 samples in a ring buffer.
//!
//! [`AudioDecoder::open`] picks the stream whose metadata names "mixdown" and otherwise
//! the first audio stream, decodes it on an "audio-decode" thread, resamples every frame
//! with swresample to the caller's rate as interleaved stereo f32 and fills a ring buffer.
//! The caller drains the ring with [`AudioDecoder::pull`] without taking a lock, so it can
//! be called straight from an output stream callback. The first decode or resample error ends
//! the thread; [`AudioDecoder::take_error`] reports its reason once.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_int};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ffmpeg_next::ffi;

use crate::hw::{MediaError, ff_error};

/// How many stereo frames (sample pairs) the ring buffer holds; about 2.7 s at 48 kHz.
const RING_FRAMES: u64 = 1 << 17;

/// How many interleaved samples the ring buffer holds.
const RING_SAMPLES: u64 = RING_FRAMES * 2;

/// How long the decode thread waits for space or a flush before it rechecks.
const WAIT: Duration = Duration::from_millis(2);

/// How long a seek waits for the consumer to acknowledge the flush before it gives up waiting.
const FLUSH_ACK_BUDGET: Duration = Duration::from_secs(1);

/// The AVERROR code for EAGAIN, which the decoder returns when it wants to hand out a frame first.
const AVERROR_EAGAIN: c_int = -11;

/// One batch of decoded samples and the presentation time of its first sample in milliseconds.
struct Batch {
    /// Interleaved stereo f32 samples.
    samples: Vec<f32>,
    /// The presentation time of the first sample in milliseconds.
    pts_ms: u64,
}

/// The shared ring buffer: the decode thread fills it, the output callback drains it.
///
/// Soundness: exactly one thread writes samples (the decode thread) and exactly one thread
/// reads them (the output callback). `written` and `read` are monotonic sample counts whose
/// release/acquire pairing publishes every slot before it is read, so the two threads never
/// touch the same slot. A seek never rewinds either cursor: the consumer drops everything
/// unread when it acknowledges the flush generation.
struct Ring {
    /// The interleaved samples, `RING_SAMPLES` of them, as raw f32 bits.
    slots: Box<[AtomicU32]>,
    /// One past the last written sample.
    written: AtomicU64,
    /// The next sample to hand out.
    read: AtomicU64,
    /// The flush generation a seek asked for.
    flush: AtomicU64,
    /// The flush generation the last `pull` acknowledged.
    acked: AtomicU64,
    /// The sample count whose media time is `origin_ms`.
    origin: AtomicU64,
    /// The media time in milliseconds of the sample at `origin`.
    origin_ms: AtomicU64,
    /// Whether `origin` maps a real sample; false right after a flush, until the first batch.
    origin_valid: AtomicBool,
}

impl Ring {
    /// An empty ring whose first samples report position zero.
    fn new() -> Self {
        Self {
            slots: (0..RING_SAMPLES as usize).map(|_| AtomicU32::new(0)).collect::<Vec<_>>().into_boxed_slice(),
            written: AtomicU64::new(0),
            read: AtomicU64::new(0),
            flush: AtomicU64::new(0),
            acked: AtomicU64::new(0),
            origin: AtomicU64::new(0),
            origin_ms: AtomicU64::new(0),
            origin_valid: AtomicBool::new(false),
        }
    }

    /// Asks the consumer to drop everything it has not read yet and reports `target_ms` as
    /// the position of the next sample until the first one after the flush arrives.
    fn request_flush(&self, target_ms: u64) {
        self.flush.fetch_add(1, Ordering::Release);
        self.origin_ms.store(target_ms, Ordering::Relaxed);
        self.origin_valid.store(false, Ordering::Relaxed);
    }

    /// Waits until the consumer acknowledged the flush, or the budget runs out; false on stop.
    fn wait_flush_ack(&self, stop: &AtomicBool) -> bool {
        let wanted = self.flush.load(Ordering::Acquire);
        let deadline = Instant::now() + FLUSH_ACK_BUDGET;
        while self.acked.load(Ordering::Relaxed) != wanted {
            if stop.load(Ordering::Relaxed) {
                return false;
            }
            if Instant::now() >= deadline {
                return true;
            }
            std::thread::sleep(WAIT);
        }
        true
    }

    /// Pushes `samples` into the ring, waiting for space; on stop, the samples are dropped.
    fn push(&self, samples: &[f32], stop: &AtomicBool) {
        let mut written = self.written.load(Ordering::Relaxed);
        while written + samples.len() as u64 > self.read.load(Ordering::Acquire) + RING_SAMPLES {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(WAIT);
            written = self.written.load(Ordering::Relaxed);
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        for (offset, sample) in samples.iter().enumerate() {
            self.slots[((written + offset as u64) & (RING_SAMPLES - 1)) as usize].store(sample.to_bits(), Ordering::Relaxed);
        }
        self.written.store(written + samples.len() as u64, Ordering::Release);
    }

    /// Copies up to `out.len() / 2` stereo frames out of the ring and returns how many it
    /// handed out. A pending flush is acknowledged first: everything unread is dropped.
    /// No locks and no panics: safe on an output stream callback.
    fn pull(&self, out: &mut [f32]) -> usize {
        let flush = self.flush.load(Ordering::Acquire);
        if self.acked.load(Ordering::Relaxed) != flush {
            self.read.store(self.written.load(Ordering::Acquire), Ordering::Release);
            self.acked.store(flush, Ordering::Release);
        }
        let written = self.written.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        let frames = (out.len() / 2).min(written.saturating_sub(read) as usize / 2);
        let mask = RING_SAMPLES - 1;
        for (index, sample) in out.iter_mut().take(frames * 2).enumerate() {
            *sample = f32::from_bits(self.slots[((read + index as u64) & mask) as usize].load(Ordering::Relaxed));
        }
        self.read.store(read + frames as u64 * 2, Ordering::Release);
        frames
    }

    /// The media time in milliseconds of the sample `pull` hands out next.
    fn next_pts_ms(&self, rate: u32) -> u64 {
        if !self.origin_valid.load(Ordering::Relaxed) {
            return self.origin_ms.load(Ordering::Relaxed);
        }
        let read = self.read.load(Ordering::Relaxed);
        // The ring counts interleaved stereo samples; the media time advances per frame.
        self.origin_ms.load(Ordering::Relaxed).saturating_add(samples_to_ms(read.saturating_sub(self.origin.load(Ordering::Relaxed)) / 2, rate))
    }
}

/// Pushes one decoded batch, fixing the pts origin to its first sample when the ring had
/// just been flushed.
fn push_batch(ring: &Ring, batch: &Batch, stop: &AtomicBool) {
    if batch.samples.is_empty() {
        return;
    }
    if !ring.origin_valid.load(Ordering::Relaxed) {
        ring.origin.store(ring.read.load(Ordering::Acquire), Ordering::Relaxed);
        ring.origin_ms.store(batch.pts_ms, Ordering::Relaxed);
        ring.origin_valid.store(true, Ordering::Relaxed);
    }
    ring.push(&batch.samples, stop);
}

/// Milliseconds for a count of samples at `rate`.
fn samples_to_ms(samples: u64, rate: u32) -> u64 {
    if rate == 0 {
        return 0;
    }
    u64::try_from(u128::from(samples) * 1000 / u128::from(rate)).unwrap_or(u64::MAX)
}

/// One audio file decoded on its own thread into stereo f32 samples the caller pulls without locks.
pub struct AudioDecoder {
    /// The ring buffer shared with the decode thread and the caller.
    ring: Arc<Ring>,
    /// The play state shared with the decode thread.
    control: Arc<Control>,
    /// Set by Drop to end the decode thread.
    stop: Arc<AtomicBool>,
    /// The decode thread, taken and joined by Drop.
    thread: Option<JoinHandle<()>>,
    /// Why the decode thread gave up, until the caller takes it.
    failure: Failure,
    /// The output sample rate in hertz.
    rate: u32,
}

impl AudioDecoder {
    /// Opens `path` on a new "audio-decode" thread that decodes the stream whose metadata
    /// names "mixdown", or the first audio stream when none does, and resamples every frame
    /// with swresample to `rate` as interleaved stereo f32.
    ///
    /// The file and the decoder are opened here, so a bad path or an unreadable stream fails
    /// before the thread starts.
    pub fn open(path: &Path, rate: u32) -> Result<Self, MediaError> {
        let session = open_session(path, rate)?;
        log::info!("audio player {}: stream {} at {} Hz resampled to stereo {} Hz", path.display(), session.stream_index, session.in_rate, rate);
        let control: Arc<Control> = Arc::new((Mutex::new(ControlState { playing: false, pending_seek: None, eof: false }), Condvar::new()));
        let ring = Arc::new(Ring::new());
        let stop = Arc::new(AtomicBool::new(false));
        let failure = Failure::default();
        let thread = {
            let payload = SendSession(session);
            let (thread_control, thread_ring, thread_stop) = (Arc::clone(&control), Arc::clone(&ring), Arc::clone(&stop));
            let thread_failure = Arc::clone(&failure);
            std::thread::Builder::new()
                .name("audio-decode".to_owned())
                .spawn(move || {
                    if let Err(error) = run(payload, thread_control, thread_ring, thread_stop) {
                        log::warn!("audio decode stopped: {error}");
                        *lock(&thread_failure) = Some(error.to_string());
                    }
                })
                .map_err(|_| MediaError::Null("the audio decode thread"))?
        };
        Ok(Self { ring, control, stop, thread: Some(thread), failure, rate })
    }

    /// Why decoding stopped for good, if it did; reported once, `None` afterwards. Locks a
    /// mutex, so it is not for the output stream callback.
    pub fn take_error(&self) -> Option<String> {
        lock(&self.failure).take()
    }

    /// Resumes decoding into the ring buffer.
    pub fn play(&self) {
        let (state_lock, signal) = &*self.control;
        let mut state = lock(state_lock);
        state.playing = true;
        signal.notify_all();
    }

    /// Stops decoding at the current position; samples already pushed stay available.
    pub fn pause(&self) {
        let (state_lock, signal) = &*self.control;
        let mut state = lock(state_lock);
        state.playing = false;
        signal.notify_all();
    }

    /// Seeks backward to the keyframe at or before `ms`, flushes the decoder and drops every
    /// sample the caller has not pulled yet, so the next sample reports `ms` or later.
    pub fn seek(&self, ms: u64) {
        self.ring.request_flush(ms);
        let (state_lock, signal) = &*self.control;
        let mut state = lock(state_lock);
        state.pending_seek = Some(ms);
        state.eof = false;
        signal.notify_all();
    }

    /// Copies up to `out.len() / 2` stereo frames from the ring into `out` and returns how
    /// many it handed out; zero while a seek is flushing. Takes no locks, so it is safe to
    /// call from an output stream callback.
    pub fn pull(&self, out: &mut [f32]) -> usize {
        self.ring.pull(out)
    }

    /// The media time in milliseconds of the next sample `pull` hands out.
    pub fn next_pts_ms(&self) -> u64 {
        self.ring.next_pts_ms(self.rate)
    }

    /// The sample rate the samples are resampled to, in hertz.
    pub fn rate(&self) -> u32 {
        self.rate
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let (_, signal) = &*self.control;
        signal.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The reason the decode thread gave up, shared with the caller.
type Failure = Arc<Mutex<Option<String>>>;

/// The play state and the pending seek the decode thread reads under its lock.
struct ControlState {
    /// Whether the thread decodes ahead into the ring or waits.
    playing: bool,
    /// A seek the caller asked for, in milliseconds, not yet carried out.
    pending_seek: Option<u64>,
    /// Whether the file was read to the end, so the thread waits even while playing.
    eof: bool,
}

/// The shared play state with the condition variable the thread sleeps on.
type Control = (Mutex<ControlState>, Condvar);

/// Locks a mutex, carrying on with the data if a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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

/// The body of the decode thread: waits for play or a seek, decodes and fills the ring. It
/// returns on stop, or with the first error of the decoder.
fn run(SendSession(mut session): SendSession, control: Arc<Control>, ring: Arc<Ring>, stop: Arc<AtomicBool>) -> Result<(), MediaError> {
    let mut seek_target: Option<u64> = None;
    loop {
        {
            let (state_lock, signal) = &*control;
            let mut state = lock(state_lock);
            while !stop.load(Ordering::Relaxed) && state.pending_seek.is_none() && seek_target.is_none() && (!state.playing || state.eof) {
                state = signal.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Some(target) = state.pending_seek.take() {
                session.seek(target)?;
                seek_target = Some(target);
                state.eof = false;
            }
        }
        // The caller's ring must be drained before new samples with a new origin are pushed.
        if !ring.wait_flush_ack(&stop) {
            return Ok(());
        }
        if session.input_done {
            mark_eof(&control);
            continue;
        }
        match session.decode_next()? {
            Some(batch) => {
                if seek_target.is_none_or(|target| batch.pts_ms >= target) {
                    push_batch(&ring, &batch, &stop);
                    seek_target = None;
                }
            }
            None => mark_eof(&control),
        }
    }
}

/// The signature of the incoming stream the resampler was built for.
#[derive(Clone, Copy, PartialEq)]
struct InSignature {
    /// The AVSampleFormat of the decoded frames.
    format: ffi::AVSampleFormat,
    /// The sample rate in hertz.
    rate: u32,
    /// The channel count.
    channels: u32,
    /// The channel order of the layout, as its AVChannelOrder number.
    order: u32,
    /// The channel mask of the layout.
    mask: u64,
}

/// The opened decoder, its input file and the resampler one decode thread works with.
struct Session {
    /// The opened input file.
    fmt: *mut ffi::AVFormatContext,
    /// The opened audio decoder.
    codec: *mut ffi::AVCodecContext,
    /// The index of the audio stream in the file.
    stream_index: usize,
    /// The stream time base, as (numerator, denominator).
    time_base: (u32, u32),
    /// The sample rate of the audio stream, in hertz.
    in_rate: u32,
    /// The output sample rate the samples are resampled to, in hertz.
    out_rate: u32,
    /// The packet reused for every read.
    packet: *mut ffi::AVPacket,
    /// The frame reused for every decode.
    frame: *mut ffi::AVFrame,
    /// The swresample context, made on first use and rebuilt when the input changes.
    swr: *mut ffi::SwrContext,
    /// The input signature the swr context was built for.
    in_sig: Option<InSignature>,
    /// Whether end of input was read and the decoder was flushed.
    input_done: bool,
}

/// The session of the decode thread; its raw FFmpeg pointers are created, used and freed on that thread alone.
struct SendSession(Session);

// SAFETY: the session's raw pointers are only ever used on the decode thread, which is the one that
// receives the SendSession.
unsafe impl Send for SendSession {}

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: every pointer is either null or a resource this session owns, and each call is
        // the documented way to release it. The resampler is freed before the decoder it was fed
        // from, so the fields above must stay in this order.
        unsafe {
            ffi::swr_free(&mut self.swr);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_packet_free(&mut self.packet);
            ffi::avcodec_free_context(&mut self.codec);
            ffi::avformat_close_input(&mut self.fmt);
        }
    }
}

impl Session {
    /// Seeks backward to the keyframe at or before `target_ms`, flushes the decoder and drops
    /// any samples the resampler still holds, so the next frames decode from before the target.
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
        if !self.swr.is_null() {
            // SAFETY: the context was made by swr_alloc_set_opts2 and this is its last owner.
            unsafe { ffi::swr_free(&mut self.swr) };
            self.swr = std::ptr::null_mut();
            self.in_sig = None;
        }
        Ok(())
    }

    /// Reads and decodes the next frame of the file, resampled to stereo f32, or None at the end.
    fn decode_next(&mut self) -> Result<Option<Batch>, MediaError> {
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

    /// Resamples the decoded frame into interleaved stereo f32 with its presentation time.
    fn convert(&mut self) -> Result<Batch, MediaError> {
        // SAFETY: the frame is live and was filled by avcodec_receive_frame.
        let (nb_samples, format, in_rate, best, pts) = unsafe {
            (
                (*self.frame).nb_samples.max(0) as usize,
                (*self.frame).format,
                (*self.frame).sample_rate.max(0) as u32,
                (*self.frame).best_effort_timestamp,
                (*self.frame).pts,
            )
        };
        let ts = if best != ffi::AV_NOPTS_VALUE { best } else { pts };
        let pts_ms = pts_to_ms(ts, self.time_base);
        if nb_samples == 0 || in_rate == 0 {
            return Ok(Batch { samples: Vec::new(), pts_ms });
        }
        // SAFETY: the frame is live; its channel layout counts the decoded channels.
        let channels = unsafe { (*self.frame).ch_layout.nb_channels.max(0) as u32 };
        if channels == 0 {
            return Err(MediaError::Null("a decoded audio frame with no channels"));
        }
        let Some(format) = sample_format(format) else {
            return Err(MediaError::Null("a decoded frame in an unknown sample format"));
        };
        let swr = self.resampler(format, in_rate, channels)?;
        // The resampler may expand the frame, so the output gets headroom on top of the exact rate.
        let capacity = (u128::from(nb_samples as u64) * u128::from(u64::from(self.out_rate)) / u128::from(u64::from(in_rate)) + 64) as usize;
        let mut samples = vec![0.0f32; capacity * 2];
        let mut out_planes: [*mut u8; 8] = std::array::from_fn(|index| if index == 0 { samples.as_mut_ptr().cast() } else { std::ptr::null_mut() });
        // SAFETY: the resampler is live; the frame's planes hold nb_samples of `format` and the
        // output buffer holds `capacity` stereo samples of interleaved f32.
        let written = unsafe {
            let source = (*self.frame).extended_data as *const *const u8;
            ffi::swr_convert(swr, out_planes.as_mut_ptr(), capacity as c_int, source, nb_samples as c_int)
        };
        if written < 0 {
            return Err(ff_error(written));
        }
        samples.truncate(written as usize * 2);
        Ok(Batch { samples, pts_ms })
    }

    /// The initialized swresample context converting this frame's format to stereo f32 at
    /// `out_rate`, made on first use and rebuilt when the input signature changes. A frame
    /// without a channel layout gets the default one for its channel count.
    fn resampler(&mut self, format: ffi::AVSampleFormat, in_rate: u32, channels: u32) -> Result<*mut ffi::SwrContext, MediaError> {
        // SAFETY: the layout is plain data, and all zeroes is a valid unspecified layout.
        let mut default_layout: ffi::AVChannelLayout = unsafe { std::mem::zeroed() };
        // SAFETY: the frame is live and was filled by avcodec_receive_frame.
        let mut in_layout = unsafe { std::ptr::addr_of!((*self.frame).ch_layout) };
        // SAFETY: the pointer names the live frame's layout.
        if unsafe { (*in_layout).order } == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC {
            // SAFETY: the out pointer is valid.
            unsafe { ffi::av_channel_layout_default(&mut default_layout, channels as c_int) };
            in_layout = &default_layout;
        }
        // SAFETY: the pointer names the frame's layout or the default one, both live; the
        // union is only read as its bits, to compare signatures.
        let (order, mask) = unsafe { ((*in_layout).order as u32, (*in_layout).u.mask) };
        let key = InSignature { format, rate: in_rate, channels, order, mask };
        if self.swr.is_null() || self.in_sig.as_ref() != Some(&key) {
            if !self.swr.is_null() {
                // SAFETY: the context was made by swr_alloc_set_opts2 and this is its last owner.
                unsafe { ffi::swr_free(&mut self.swr) };
                self.swr = std::ptr::null_mut();
            }
            // SAFETY: the layout is filled by av_channel_layout_default, so no uninitialized
            // fields are read, and swr_alloc_set_opts2 copies it into the context.
            let mut out_layout: ffi::AVChannelLayout = unsafe { std::mem::zeroed() };
            // SAFETY: the out pointer is valid.
            unsafe { ffi::av_channel_layout_default(&mut out_layout, 2) };
            let mut swr: *mut ffi::SwrContext = std::ptr::null_mut();
            // SAFETY: the context out pointer and both layouts are valid while the call runs.
            let code = unsafe {
                ffi::swr_alloc_set_opts2(
                    &mut swr,
                    &out_layout,
                    ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT,
                    self.out_rate as c_int,
                    in_layout,
                    format,
                    in_rate as c_int,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if code < 0 {
                return Err(ff_error(code));
            }
            if swr.is_null() {
                return Err(MediaError::Null("swr_alloc_set_opts2"));
            }
            // SAFETY: the context was just allocated with its options set.
            let code = unsafe { ffi::swr_init(swr) };
            if code < 0 {
                // SAFETY: the context is owned here and this is its last owner.
                unsafe { ffi::swr_free(&mut swr) };
                return Err(ff_error(code));
            }
            self.swr = swr;
            self.in_sig = Some(key);
        }
        Ok(self.swr)
    }
}

/// Opens the file, the audio stream and the decoder for one decode thread.
fn open_session(path: &Path, out_rate: u32) -> Result<Session, MediaError> {
    crate::init()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| MediaError::Null("the audio path"))?;
    let mut fmt: *mut ffi::AVFormatContext = std::ptr::null_mut();
    // SAFETY: the out pointer is valid, the path is NUL terminated and no custom IO is used.
    let code = unsafe { ffi::avformat_open_input(&mut fmt, c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
    if code < 0 {
        return Err(ff_error(code));
    }
    let session = open_session_rest(fmt, out_rate);
    if session.is_err() {
        // SAFETY: the format context was opened above and this is its last owner.
        unsafe { ffi::avformat_close_input(&mut fmt) };
    }
    session
}

/// Does the work of open_session on an already opened input file.
fn open_session_rest(fmt: *mut ffi::AVFormatContext, out_rate: u32) -> Result<Session, MediaError> {
    let Some(stream_index) = pick_audio_stream(fmt) else {
        return Err(MediaError::Null("the file has no audio stream"));
    };
    // SAFETY: the format context is live and the index is one of its streams.
    let stream = unsafe { *(*fmt).streams.add(stream_index) };
    // SAFETY: the stream is live; its codec parameters and time base are plain data.
    let (codecpar, time_base) = unsafe { ((*stream).codecpar, (*stream).time_base) };
    if codecpar.is_null() {
        return Err(MediaError::Null("the audio stream has no codec parameters"));
    }
    // SAFETY: the codec parameters are live.
    let (codec_id, in_rate) = unsafe { ((*codecpar).codec_id, (*codecpar).sample_rate.max(0) as u32) };
    if in_rate == 0 {
        return Err(MediaError::Null("the audio stream has no sample rate"));
    }
    // SAFETY: the codec id comes from the codec parameters and names a decoder of this FFmpeg build.
    let decoder = unsafe { ffi::avcodec_find_decoder(codec_id) };
    if decoder.is_null() {
        return Err(MediaError::Null("avcodec_find_decoder"));
    }
    let codec = open_decoder(codecpar, decoder)?;
    let mut session = Session {
        fmt,
        codec,
        stream_index,
        time_base: (time_base.num.max(0) as u32, time_base.den.max(0) as u32),
        in_rate,
        out_rate,
        packet: std::ptr::null_mut(),
        frame: std::ptr::null_mut(),
        swr: std::ptr::null_mut(),
        in_sig: None,
        input_done: false,
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
    Ok(session)
}

/// The index of the audio stream to play: the first whose metadata names "mixdown", case
/// insensitively, else the first audio stream; None when the file has no audio stream.
fn pick_audio_stream(fmt: *mut ffi::AVFormatContext) -> Option<usize> {
    // SAFETY: the format context is live and nb_streams counts its streams.
    unsafe {
        let count = (*fmt).nb_streams as usize;
        let mut first_audio = None;
        let mut mixdown = None;
        for index in 0..count {
            let stream = *(*fmt).streams.add(index);
            let par = (*stream).codecpar;
            if par.is_null() || (*par).codec_type != ffi::AVMediaType::AVMEDIA_TYPE_AUDIO {
                continue;
            }
            if first_audio.is_none() {
                first_audio = Some(index);
            }
            if metadata_names_mixdown(stream) {
                mixdown = Some(index);
                break;
            }
        }
        mixdown.or(first_audio)
    }
}

/// Whether any metadata tag of the stream names "mixdown", case insensitively.
fn metadata_names_mixdown(stream: *mut ffi::AVStream) -> bool {
    // SAFETY: the stream is live; av_dict_get walks its metadata with a null key.
    unsafe {
        let mut entry = ffi::av_dict_get((*stream).metadata, std::ptr::null(), std::ptr::null(), 0);
        while !entry.is_null() {
            let value = (*entry).value;
            if !value.is_null() && CStr::from_ptr(value).to_string_lossy().to_lowercase().contains("mixdown") {
                return true;
            }
            entry = ffi::av_dict_get((*stream).metadata, std::ptr::null(), entry, 0);
        }
        false
    }
}

/// Allocates a decoder context for the stream and opens it.
fn open_decoder(codecpar: *mut ffi::AVCodecParameters, decoder: *const ffi::AVCodec) -> Result<*mut ffi::AVCodecContext, MediaError> {
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
    // SAFETY: the context and decoder are live and the context is not open yet.
    let code = unsafe { ffi::avcodec_open2(codec, decoder, std::ptr::null_mut()) };
    if code < 0 {
        // SAFETY: the context is owned here and this is its last owner.
        unsafe { ffi::avcodec_free_context(&mut codec) };
        return Err(ff_error(code));
    }
    Ok(codec)
}

/// Turns the AVSampleFormat number of an AVFrame into the enum the swresample functions take,
/// or None when the number names no sample format this FFmpeg build knows.
fn sample_format(format: c_int) -> Option<ffi::AVSampleFormat> {
    let none = ffi::AVSampleFormat::AV_SAMPLE_FMT_NONE as c_int;
    let count = ffi::AVSampleFormat::AV_SAMPLE_FMT_NB as c_int;
    if format != none && !(0..count).contains(&format) {
        return None;
    }
    // SAFETY: the value was checked against the enum's whole variant range above (NONE and
    // 0 to AV_SAMPLE_FMT_NB minus one), so it names a real variant.
    Some(unsafe { std::mem::transmute::<c_int, ffi::AVSampleFormat>(format) })
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
    use super::*;

    /// A stop flag the tests never set.
    fn running() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn the_ring_hands_what_was_pushed_and_drops_it_on_a_flush() {
        let ring = Ring::new();
        let stop = running();
        push_batch(&ring, &Batch { samples: vec![0.5, -0.5, 0.25, -0.25], pts_ms: 0 }, &stop);
        let mut out = [0.0f32; 4];
        assert_eq!(ring.pull(&mut out), 2, "two stereo frames were pushed");
        assert_eq!(out, [0.5, -0.5, 0.25, -0.25]);
        // An empty ring hands nothing.
        assert_eq!(ring.pull(&mut out), 0);
        // A flush drops everything unread: the next pull acknowledges it and hands none.
        ring.request_flush(2_000);
        assert_eq!(ring.pull(&mut out), 0, "the flush was acknowledged and dropped");
        // The decode thread waits for the ack, then the first batch after the flush fixes
        // the origin, so the next sample reports the first pushed pts.
        push_batch(&ring, &Batch { samples: vec![1.0; 4], pts_ms: 2_050 }, &stop);
        assert_eq!(ring.next_pts_ms(48_000), 2_050, "the next sample reports the first pushed pts");
        assert_eq!(ring.pull(&mut out), 2);
        assert_eq!(out, [1.0; 4]);
        assert_eq!(ring.next_pts_ms(48_000), 2_050, "everything read keeps the last pts");
    }

    #[test]
    fn the_next_pts_follows_what_the_consumer_read() {
        let ring = Ring::new();
        let stop = running();
        // 4_800 samples are 2_400 stereo frames, 50 ms at 48 kHz.
        push_batch(&ring, &Batch { samples: vec![0.0; 4_800], pts_ms: 1_000 }, &stop);
        assert_eq!(ring.next_pts_ms(48_000), 1_000);
        let mut out = [0.0f32; 4_800];
        assert_eq!(ring.pull(&mut out), 2_400);
        assert_eq!(ring.next_pts_ms(48_000), 1_050, "50 ms of samples were read");
        // An origin fixed after a flush keeps counting from the first pushed sample.
        ring.request_flush(5_000);
        assert_eq!(ring.pull(&mut out), 0, "the flush was acknowledged and dropped");
        push_batch(&ring, &Batch { samples: vec![0.0; 960], pts_ms: 5_100 }, &stop);
        let mut more = [0.0f32; 480];
        assert_eq!(ring.pull(&mut more), 240);
        assert_eq!(ring.next_pts_ms(48_000), 5_105);
    }

    /// A temporary file of this test process.
    fn temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cutmark_audio_player_{}_{name}", std::process::id()))
    }

    /// A 440 Hz tone as interleaved stereo f32, `frames` frames at 48 kHz.
    fn tone(frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|index| [(index as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.5; 2]).collect()
    }

    /// Writes two seconds of the tone as AAC in an MKV.
    fn write_aac_clip(path: &Path) -> Result<(), MediaError> {
        crate::init()?;
        let mut encoder = crate::AudioEncoder::open(crate::AudioCodec::Aac { kbps: 192 }, 48_000, 2)?;
        let mut muxer = crate::Muxer::create(path, crate::Container::Mkv)?;
        muxer.add_audio(encoder.parameters(), encoder.time_base(), "Mixdown", true)?;
        muxer.start()?;
        let mut packets = Vec::new();
        // The encoder holds only a few frames, so it is drained after every chunk.
        for chunk in tone(96_000).chunks(2_048) {
            encoder.send(chunk)?;
            encoder.receive(&mut |packet| packets.push(packet))?;
        }
        encoder.finish(&mut |packet| packets.push(packet))?;
        for packet in packets {
            muxer.write(0, &packet)?;
        }
        muxer.finish()?;
        Ok(())
    }

    /// Writes a plain 16-bit stereo WAV of one and a half seconds, which names no channel layout.
    fn write_wav(path: &Path) -> std::io::Result<()> {
        let data: Vec<u8> = tone(72_000).iter().flat_map(|sample| ((sample * 32_767.0) as i16).to_le_bytes()).collect();
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + data.len() as u32).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        for field in [1u16, 2] {
            bytes.extend(field.to_le_bytes());
        }
        for field in [48_000u32, 192_000] {
            bytes.extend(field.to_le_bytes());
        }
        for field in [4u16, 16] {
            bytes.extend(field.to_le_bytes());
        }
        bytes.extend(b"data");
        bytes.extend((data.len() as u32).to_le_bytes());
        bytes.extend(data);
        std::fs::write(path, bytes)
    }

    /// Pulls until `frames` stereo frames arrived, decoding fails or five seconds pass.
    fn pull_frames(decoder: &AudioDecoder, frames: usize) -> (Vec<f32>, Option<String>) {
        let mut samples = Vec::new();
        let mut chunk = [0.0f32; 2_048];
        let deadline = Instant::now() + Duration::from_secs(5);
        while samples.len() < frames * 2 && Instant::now() < deadline {
            let pulled = decoder.pull(&mut chunk);
            samples.extend_from_slice(&chunk[..pulled * 2]);
            if pulled == 0 {
                if let Some(error) = decoder.take_error() {
                    return (samples, Some(error));
                }
                std::thread::sleep(WAIT);
            }
        }
        (samples, None)
    }

    #[test]
    fn an_aac_clip_decodes_a_second_through_the_resampler_also_after_a_seek() -> Result<(), MediaError> {
        let path = temp_file("aac.mkv");
        write_aac_clip(&path)?;
        let decoder = AudioDecoder::open(&path, 44_100)?;
        decoder.play();
        let (samples, error) = pull_frames(&decoder, 44_100);
        assert_eq!(error, None);
        assert!(samples.len() >= 88_200, "only {} samples arrived", samples.len());
        assert!(samples.iter().any(|sample| sample.abs() > 0.2), "the tone is audible");
        // The seek drops the resampler; the next frame builds a new one.
        decoder.seek(500);
        let (samples, error) = pull_frames(&decoder, 22_050);
        assert_eq!(error, None);
        assert!(samples.len() >= 44_100, "only {} samples arrived after the seek", samples.len());
        drop(decoder);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn audio_without_a_channel_layout_decodes() -> Result<(), Box<dyn std::error::Error>> {
        let path = temp_file("plain.wav");
        write_wav(&path)?;
        let decoder = AudioDecoder::open(&path, 48_000)?;
        decoder.play();
        let (samples, error) = pull_frames(&decoder, 48_000);
        assert_eq!(error, None);
        assert!(samples.len() >= 96_000, "only {} samples arrived", samples.len());
        drop(decoder);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn a_resampler_that_cannot_start_reports_its_error_once() -> Result<(), Box<dyn std::error::Error>> {
        let path = temp_file("refused.wav");
        write_wav(&path)?;
        // No resampler can be initialized for an output rate of zero.
        let decoder = AudioDecoder::open(&path, 0)?;
        decoder.play();
        let (samples, error) = pull_frames(&decoder, 1);
        assert!(samples.is_empty());
        assert!(error.is_some(), "the failure is reported");
        // The thread is gone: a seek and a play bring no second report.
        decoder.seek(0);
        decoder.play();
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(decoder.take_error(), None);
        drop(decoder);
        let _ = std::fs::remove_file(path);
        Ok(())
    }

    #[test]
    fn the_pts_math_survives_degenerate_rates() {
        assert_eq!(samples_to_ms(48_000, 48_000), 1_000);
        assert_eq!(samples_to_ms(48_000, 0), 0);
        assert_eq!(samples_to_ms(u64::MAX, 1), u64::MAX);
    }
}
