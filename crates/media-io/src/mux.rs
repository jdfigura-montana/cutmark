//! The MKV muxer: copies encoder packets into a Matroska file that stays
//! playable if the process dies mid-take.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;

use ffmpeg_next::Dictionary;
use ffmpeg_next::Rational;
use ffmpeg_next::Rescale;
use ffmpeg_next::codec::Parameters;
use ffmpeg_next::ffi;
use ffmpeg_next::packet::{Flags, Packet};

use crate::MediaError;
use crate::encode::EncodedPacket;

/// The stream time base for Matroska tracks: milliseconds.
const MKV_TIME_BASE: Rational = Rational(1, 1000);

/// The stream time base for MP4 and MOV tracks: the 90 kHz clock, fine enough for any frame rate.
const MOV_TIME_BASE: Rational = Rational(1, 90_000);

/// The movflags that make a killed process leave a playable fragmented file.
const FRAGMENT_FLAGS: &str = "frag_keyframe+empty_moov+default_base_moof";

pub use crate::encode::Container;

/// The timing facts of one added stream, kept for rescaling on write.
struct StreamInfo {
    /// The time base the encoder stamps its packets in.
    encoder_time_base: Rational,
    /// One frame duration expressed as a rescale source unit, video only.
    frame_step: Option<Rational>,
}

/// Writes encoded packets into a Matroska file and flushes on keyframes.
pub struct Muxer {
    ctx: ffmpeg_next::format::context::Output,
    /// The output file, read back for its final size after the trailer.
    path: PathBuf,
    /// The container being written.
    container: Container,
    streams: Vec<StreamInfo>,
    started: bool,
    /// The chapters added just before the trailer, as start ms, end ms and title.
    chapters: Vec<(u64, u64, String)>,
}

impl Muxer {
    /// Opens the output file with the muxer the container names.
    pub fn create(path: &Path, container: Container) -> Result<Self, MediaError> {
        let ctx = match container {
            Container::Mkv => ffmpeg_next::format::output_as(path, "matroska")?,
            Container::Mp4 => ffmpeg_next::format::output_as(path, "mp4")?,
            Container::Mov => ffmpeg_next::format::output_as(path, "mov")?,
        };
        Ok(Self { ctx, path: path.to_path_buf(), container, streams: Vec::new(), started: false, chapters: Vec::new() })
    }

    /// The time base the container's streams are created with.
    fn stream_time_base(&self) -> Rational {
        match self.container {
            Container::Mkv => MKV_TIME_BASE,
            Container::Mp4 | Container::Mov => MOV_TIME_BASE,
        }
    }

    /// Sets the chapters, as start ms, end ms and title, that are written just before the trailer; MP4 and MOV ignore them.
    pub fn set_chapters(&mut self, chapters: Vec<(u64, u64, String)>) {
        if self.container == Container::Mkv {
            self.chapters = chapters;
        }
    }

    /// Adds one video stream that copies the encoder's codec parameters,
    /// including extradata, and returns the stream index.
    pub fn add_video(&mut self, params: Parameters, encoder_time_base: (i32, i32), fps: (i32, i32)) -> Result<usize, MediaError> {
        let encoder_time_base = Rational::new(encoder_time_base.0, encoder_time_base.1);
        let frame_step = Rational::new(fps.1, fps.0);
        let codec_id = params.id();
        let time_base = self.stream_time_base();
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(time_base);
            stream.set_rate(Rational::new(fps.0, fps.1));
            stream.set_avg_frame_rate(Rational::new(fps.0, fps.1));
            stream.set_parameters(params);
            stream.index()
        };
        self.streams.push(StreamInfo { encoder_time_base, frame_step: Some(frame_step) });
        Ok(index)
    }

    /// Adds one audio stream whose metadata title names the track and returns
    /// the stream index; the default flag sets the default disposition.
    pub fn add_audio(&mut self, params: Parameters, encoder_time_base: (i32, i32), title: &str, default: bool) -> Result<usize, MediaError> {
        let encoder_time_base = Rational::new(encoder_time_base.0, encoder_time_base.1);
        let codec_id = params.id();
        let time_base = self.stream_time_base();
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(time_base);
            stream.set_parameters(params);
            let mut metadata = Dictionary::new();
            metadata.set("title", title);
            stream.set_metadata(metadata);
            if default {
                set_default_disposition(&mut stream);
            }
            stream.index()
        };
        self.streams.push(StreamInfo { encoder_time_base, frame_step: None });
        Ok(index)
    }

    /// Writes the container header; call once after all streams are added.
    pub fn start(&mut self) -> Result<(), MediaError> {
        // Matroska cannot store negative timestamps, so tell the muxer to
        // shift the AAC priming gap instead of failing on the first packet.
        // SAFETY: ctx is a live output context; setting avoid_negative_ts
        // before write_header is what avformat's own callers do.
        #[allow(unsafe_code)]
        unsafe {
            (*self.ctx.as_mut_ptr()).avoid_negative_ts = ffi::AVFMT_AVOID_NEG_TS_MAKE_ZERO as _;
        }
        match self.container {
            Container::Mkv => {
                self.ctx.write_header()?;
            }
            Container::Mp4 | Container::Mov => {
                let mut options = Dictionary::new();
                options.set("movflags", FRAGMENT_FLAGS);
                self.ctx.write_header_with(options)?;
            }
        }
        self.started = true;
        Ok(())
    }

    /// Rescales one packet's timestamps into the stream's time base and
    /// interleaved-writes it, flushing the output when it is a keyframe.
    pub fn write(&mut self, stream: usize, packet: &EncodedPacket) -> Result<(), MediaError> {
        let Self { ctx, streams, started, .. } = self;
        if !*started {
            return Err(MediaError::Null("muxer write before start"));
        }
        let Some(info) = streams.get(stream) else {
            return Err(MediaError::Null("muxer write stream index"));
        };
        let Some(data) = packet.data() else {
            return Ok(());
        };
        let stream_tb = ctx.stream(stream).map(|s| s.time_base()).ok_or(MediaError::Null("muxer write stream"))?;
        let is_key = packet.is_key();
        let mut out = Packet::copy(data);
        out.set_stream(stream);
        out.set_pts(packet.pts().map(|pts| pts.rescale(info.encoder_time_base, stream_tb)));
        out.set_dts(packet.dts().map(|dts| dts.rescale(info.encoder_time_base, stream_tb)));
        if let Some(frame_step) = info.frame_step {
            out.set_duration(1i64.rescale(frame_step, stream_tb));
        }
        if is_key {
            out.set_flags(Flags::KEY);
        }
        out.write_interleaved(ctx)?;
        // Only video flushes on keyframes; audio packets interleave freely.
        if is_key && info.frame_step.is_some() {
            flush_output(ctx);
        }
        Ok(())
    }

    /// Writes the trailer, which for Matroska means the cues and the duration.
    pub fn finish(mut self) -> Result<(), MediaError> {
        self.write_trailer()?;
        Ok(())
    }

    /// Adds the chapters, then writes the Matroska trailer.
    fn write_trailer(&mut self) -> Result<(), MediaError> {
        for (index, (start_ms, end_ms, title)) in std::mem::take(&mut self.chapters).into_iter().enumerate() {
            let id = i64::try_from(index + 1).unwrap_or(i64::MAX);
            let start = i64::try_from(start_ms).unwrap_or(i64::MAX);
            let end = i64::try_from(end_ms).unwrap_or(i64::MAX).max(start);
            self.ctx.add_chapter(id, MKV_TIME_BASE, start, end, &title)?;
        }
        self.ctx.write_trailer()?;
        Ok(())
    }

    /// The size of the finished file, read from the file system after the
    /// trailer was written.
    fn file_size(&self) -> u64 {
        std::fs::metadata(&self.path).map(|meta| meta.len()).unwrap_or(0)
    }

    /// The number of streams added so far.
    fn stream_count(&self) -> usize {
        self.streams.len()
    }
}

/// Marks one stream as the default one through the raw disposition field,
/// which the stream wrapper does not expose.
#[allow(unsafe_code)]
fn set_default_disposition(stream: &mut ffmpeg_next::format::stream::StreamMut<'_>) {
    // SAFETY: the stream belongs to the live output context; writing the
    // disposition flag set is what avformat's own helpers do.
    unsafe {
        (*stream.as_mut_ptr()).disposition = ffi::AV_DISPOSITION_DEFAULT as _;
    }
}

/// The result of a finished mux thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuxSummary {
    /// The size of the finished file.
    pub bytes_written: u64,
    /// Packets written, one entry per stream index.
    pub packets: Vec<u64>,
    /// How often a send had to wait because the queue was full.
    pub queue_full_waits: u64,
}

/// Codec parameters that may be handed from the thread that owns the encoder
/// to the thread that owns the muxer.
pub struct SendParameters(pub Parameters);

// SAFETY: the wrapper owns its AVCodecParameters copy outright and nothing
// aliases it, so moving it to the mux owner thread is sound.
#[allow(unsafe_code)]
unsafe impl Send for SendParameters {}

/// Live counters the mux thread shares with every sender handle.
#[derive(Default)]
struct MuxCounters {
    /// Packet bytes the mux thread has written so far.
    bytes: AtomicU64,
    /// Packets queued and not yet taken by the mux thread.
    queued: AtomicUsize,
}

/// The first error the mux thread hit, shared with every sender handle.
type SharedFailure = Arc<Mutex<Option<&'static str>>>;

/// A clonable handle that queues packets into a MuxThread; hand one to every
/// stream thread.
#[derive(Clone)]
pub struct MuxSender {
    /// The bounded queue feeding the mux thread.
    sender: SyncSender<(usize, EncodedPacket)>,
    /// Counter for sends that had to wait for queue space.
    queue_full: Arc<AtomicU64>,
    /// The byte and queue-depth counters the mux thread keeps.
    counters: Arc<MuxCounters>,
    /// The stored first failure of the mux thread, if it died.
    failure: SharedFailure,
}

impl MuxSender {
    /// Queues one packet for its stream, blocking until there is room and
    /// counting how often that wait happened.
    pub fn send(&self, stream: usize, packet: EncodedPacket) -> Result<(), MediaError> {
        self.counters.queued.fetch_add(1, Ordering::Relaxed);
        let result = match self.sender.try_send((stream, packet)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full((stream, packet))) => {
                self.queue_full.fetch_add(1, Ordering::Relaxed);
                self.sender.send((stream, packet)).map_err(|_| self.closed_error())
            }
            Err(TrySendError::Disconnected(_)) => Err(self.closed_error()),
        };
        if result.is_err() {
            self.counters.queued.fetch_sub(1, Ordering::Relaxed);
        }
        result
    }

    /// Packet bytes the mux thread has written so far.
    pub fn bytes_written(&self) -> u64 {
        self.counters.bytes.load(Ordering::Relaxed)
    }

    /// How many packets wait in the queue for the mux thread.
    pub fn queue_depth(&self) -> usize {
        self.counters.queued.load(Ordering::Relaxed)
    }

    /// The stored first mux failure if the thread died, else the generic text.
    fn closed_error(&self) -> MediaError {
        let stored = self.failure.lock().ok().and_then(|slot| *slot);
        MediaError::Null(stored.unwrap_or("mux thread closed"))
    }

    /// How often a send had to wait because the queue was full.
    fn queue_full_waits(&self) -> u64 {
        self.queue_full.load(Ordering::Relaxed)
    }
}

/// The chapters the mux thread adds before its trailer.
type SharedChapters = Arc<Mutex<Vec<(u64, u64, String)>>>;

/// A background thread that owns the muxer and writes queued packets.
pub struct MuxThread {
    /// The queue handle the owner and the stream threads share.
    sender: MuxSender,
    /// The chapters handed over by `set_chapters`, read by the thread before the trailer.
    chapters: SharedChapters,
    /// The mux thread's join handle, which returns its summary.
    join: JoinHandle<Result<MuxSummary, MediaError>>,
}

impl MuxThread {
    /// Starts the thread that owns the muxer; the muxer must have been
    /// started, after every stream was added. The queue holds 1024 packets.
    pub fn start(muxer: Muxer) -> Result<Self, MediaError> {
        let (sender, receiver) = sync_channel(1024);
        let queue_full = Arc::new(AtomicU64::new(0));
        let counters = Arc::new(MuxCounters::default());
        let thread_counters = counters.clone();
        let failure: SharedFailure = Arc::new(Mutex::new(None));
        let thread_failure = failure.clone();
        let chapters: SharedChapters = Arc::new(Mutex::new(Vec::new()));
        let thread_chapters = chapters.clone();
        let join = std::thread::Builder::new()
            .name("mux".to_string())
            .spawn(move || mux_loop(muxer, receiver, thread_failure, thread_chapters, thread_counters))
            .map_err(|_| MediaError::Null("mux thread spawn"))?;
        Ok(Self { sender: MuxSender { sender, queue_full, counters, failure }, chapters, join })
    }

    /// Sets the chapters, as start ms, end ms and title, that the thread adds just before the trailer; call before `finish`.
    pub fn set_chapters(&self, chapters: Vec<(u64, u64, String)>) {
        if let Ok(mut slot) = self.chapters.lock() {
            *slot = chapters;
        }
    }

    /// Packet bytes the mux thread has written so far.
    pub fn bytes_written(&self) -> u64 {
        self.sender.bytes_written()
    }

    /// How many packets wait in the queue for the mux thread.
    pub fn queue_depth(&self) -> usize {
        self.sender.queue_depth()
    }

    /// A handle to the queue; every stream thread sends through one.
    pub fn sender(&self) -> MuxSender {
        self.sender.clone()
    }

    /// Queues one packet for its stream, blocking until there is room.
    pub fn send(&self, stream: usize, packet: EncodedPacket) -> Result<(), MediaError> {
        self.sender.send(stream, packet)
    }

    /// Closes the queue, waits for the trailer and returns the summary.
    pub fn finish(self) -> Result<MuxSummary, MediaError> {
        let queue_full_waits = self.sender.queue_full_waits();
        drop(self.sender);
        let mut summary = self.join.join().map_err(|_| MediaError::Null("mux thread panicked"))??;
        summary.queue_full_waits = queue_full_waits;
        Ok(summary)
    }
}

/// Writes queued packets until the queue closes, then the trailer.
fn mux_loop(
    mut muxer: Muxer,
    receiver: Receiver<(usize, EncodedPacket)>,
    failure: SharedFailure,
    chapters: SharedChapters,
    counters: Arc<MuxCounters>,
) -> Result<MuxSummary, MediaError> {
    let mut packets = vec![0u64; muxer.stream_count()];
    while let Ok((stream, packet)) = receiver.recv() {
        counters.queued.fetch_sub(1, Ordering::Relaxed);
        let outcome = match packets.get_mut(stream) {
            Some(slot) => muxer.write(stream, &packet).map(|()| {
                *slot += 1;
                counters.bytes.fetch_add(packet.data().map_or(0, |data| data.len() as u64), Ordering::Relaxed);
            }),
            None => Err(MediaError::Null("mux thread stream index")),
        };
        if let Err(err) = outcome {
            return Err(store_failure(&failure, stream, &packet, err));
        }
    }
    if let Ok(mut slot) = chapters.lock() {
        muxer.set_chapters(std::mem::take(&mut slot));
    }
    muxer.write_trailer()?;
    let bytes_written = muxer.file_size();
    Ok(MuxSummary { bytes_written, packets, queue_full_waits: 0 })
}

/// Logs the first mux failure and keeps it, with the stream index, the
/// packet's encoder-side timestamps and the FFmpeg error text.
fn store_failure(failure: &SharedFailure, stream: usize, packet: &EncodedPacket, err: MediaError) -> MediaError {
    let message: &'static str = Box::leak(format!("mux write failed on stream {stream} pts {:?} dts {:?}: {err}", packet.pts(), packet.dts()).into_boxed_str());
    log::error!("{message}");
    if let Ok(mut slot) = failure.lock()
        && slot.is_none()
    {
        *slot = Some(message);
    }
    MediaError::Null(message)
}

/// Forces the AVIO layer out to the file so a killed process loses at most
/// the data after the last keyframe.
#[allow(unsafe_code)]
fn flush_output(ctx: &mut ffmpeg_next::format::context::Output) {
    // SAFETY: ctx is a live output context, so its pb field points at the
    // AVIOContext that owns the open file; avio_flush only flushes buffers.
    #[allow(unsafe_code)]
    unsafe {
        let ptr = ctx.as_mut_ptr();
        ffmpeg_next::ffi::avio_flush((*ptr).pb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Codec parameters for a tiny MJPEG video stream, which needs no encoder to describe.
    fn mjpeg_params() -> Parameters {
        let mut params = Parameters::new();
        // SAFETY: params owns a fresh AVCodecParameters, so writing its plain fields is sound.
        #[allow(unsafe_code)]
        unsafe {
            let raw = params.as_mut_ptr();
            (*raw).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            (*raw).codec_id = ffi::AVCodecID::AV_CODEC_ID_MJPEG;
            (*raw).width = 16;
            (*raw).height = 16;
        }
        params
    }

    /// Writes three tiny key frames straight into the output so the file has something to probe.
    fn write_frames(muxer: &mut Muxer) -> Result<(), ffmpeg_next::Error> {
        // One frame at 30 fps, in the time base the muxer settled on for the stream.
        let time_base = muxer.ctx.stream(0).map(|stream| stream.time_base()).ok_or(ffmpeg_next::Error::StreamNotFound)?;
        let step = i64::from(time_base.denominator()) / i64::from(time_base.numerator()) / 30;
        for pts in [0, step, 2 * step] {
            let mut packet = Packet::copy(&[0xFF, 0xD8, 0xFF, 0xD9]);
            packet.set_stream(0);
            packet.set_pts(Some(pts));
            packet.set_dts(Some(pts));
            packet.set_duration(step);
            packet.set_flags(Flags::KEY);
            packet.write_interleaved(&mut muxer.ctx)?;
        }
        Ok(())
    }

    #[test]
    fn chapters_are_written_and_read_back() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("cutmark_mux_chapters_{}_{nanos}.mkv", std::process::id()));
        let mut muxer = Muxer::create(&path, Container::Mkv)?;
        muxer.add_video(mjpeg_params(), (1, 1000), (30, 1))?;
        muxer.start()?;
        write_frames(&mut muxer)?;
        muxer.set_chapters(vec![(0, 1500, "Note".to_owned()), (1500, 4000, "Highlight: intro".to_owned())]);
        muxer.finish()?;

        let input = ffmpeg_next::format::input(&path)?;
        let found: Vec<(i64, i64, Option<String>)> = input
            .chapters()
            .map(|chapter| {
                // The demuxer reports chapters in its own time base, so bring them back to milliseconds.
                let (start, end) = (chapter.start().rescale(chapter.time_base(), MKV_TIME_BASE), chapter.end().rescale(chapter.time_base(), MKV_TIME_BASE));
                (start, end, chapter.metadata().get("title").map(str::to_owned))
            })
            .collect();
        let _ = std::fs::remove_file(&path);
        assert_eq!(found, vec![(0, 1500, Some("Note".to_owned())), (1500, 4000, Some("Highlight: intro".to_owned()))]);
        Ok(())
    }

    #[test]
    fn fragmented_mp4_and_mov_reopen_with_one_stream_and_no_chapters() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        for container in [Container::Mp4, Container::Mov] {
            let path = std::env::temp_dir().join(format!("cutmark_mux_frag_{}_{nanos}.{}", std::process::id(), container.extension()));
            let mut muxer = Muxer::create(&path, container)?;
            muxer.add_video(mjpeg_params(), (1, 90_000), (30, 1))?;
            muxer.start()?;
            write_frames(&mut muxer)?;
            muxer.set_chapters(vec![(0, 1500, "Note".to_owned())]);
            muxer.finish()?;
            let input = ffmpeg_next::format::input(&path)?;
            let (streams, chapters) = (input.streams().count(), input.chapters().count());
            drop(input);
            let _ = std::fs::remove_file(&path);
            assert_eq!(streams, 1, "{container:?}");
            assert_eq!(chapters, 0, "{container:?}");
        }
        Ok(())
    }

    #[test]
    fn an_empty_chapter_list_adds_none() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("cutmark_mux_nochapters_{}_{nanos}.mkv", std::process::id()));
        let mut muxer = Muxer::create(&path, Container::Mkv)?;
        muxer.add_video(mjpeg_params(), (1, 1000), (30, 1))?;
        muxer.start()?;
        write_frames(&mut muxer)?;
        muxer.set_chapters(Vec::new());
        muxer.finish()?;
        let input = ffmpeg_next::format::input(&path)?;
        let count = input.chapters().count();
        let _ = std::fs::remove_file(&path);
        assert_eq!(count, 0);
        Ok(())
    }
}
