//! The MKV muxer: copies encoder packets into a Matroska file that stays
//! playable if the process dies mid-take.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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

/// The output container to write. MP4 and MOV arrive in phase 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// Matroska, via FFmpeg's matroska muxer.
    Mkv,
}

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
    streams: Vec<StreamInfo>,
    started: bool,
}

impl Muxer {
    /// Opens the output file with the muxer the container names.
    pub fn create(path: &Path, container: Container) -> Result<Self, MediaError> {
        let ctx = match container {
            Container::Mkv => ffmpeg_next::format::output_as(path, "matroska")?,
        };
        Ok(Self { ctx, streams: Vec::new(), started: false })
    }

    /// Adds one video stream that copies the encoder's codec parameters,
    /// including extradata, and returns the stream index.
    pub fn add_video(&mut self, params: Parameters, encoder_time_base: (i32, i32), fps: (i32, i32)) -> Result<usize, MediaError> {
        let encoder_time_base = Rational::new(encoder_time_base.0, encoder_time_base.1);
        let frame_step = Rational::new(fps.1, fps.0);
        let codec_id = params.id();
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(MKV_TIME_BASE);
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
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(MKV_TIME_BASE);
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
        self.ctx.write_header()?;
        self.started = true;
        Ok(())
    }

    /// Rescales one packet's timestamps into the stream's time base and
    /// interleaved-writes it, flushing the output when it is a keyframe.
    pub fn write(&mut self, stream: usize, packet: &EncodedPacket) -> Result<(), MediaError> {
        let Self { ctx, streams, started } = self;
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

    /// Writes the Matroska trailer.
    fn write_trailer(&mut self) -> Result<(), MediaError> {
        self.ctx.write_trailer()?;
        Ok(())
    }

    /// The bytes the AVIO layer has written out so far.
    #[allow(unsafe_code)]
    fn bytes_written(&mut self) -> u64 {
        // SAFETY: ctx is a live output context whose pb points at the
        // AVIOContext that owns the open file; avio_tell only reads the
        // current position.
        unsafe {
            let pb = (*self.ctx.as_mut_ptr()).pb;
            let written = (*pb).bytes_written;
            u64::try_from(written).unwrap_or(0)
        }
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
    /// Bytes the muxer wrote into the file.
    pub bytes_written: u64,
    /// Packets written, one entry per stream index.
    pub packets: Vec<u64>,
    /// How often a send had to wait because the queue was full.
    pub queue_full_waits: u64,
}

/// A background thread that owns the muxer and writes queued packets.
pub struct MuxThread {
    /// The bounded queue feeding the mux thread.
    sender: SyncSender<(usize, EncodedPacket)>,
    /// The mux thread's join handle, which returns its summary.
    join: JoinHandle<Result<MuxSummary, MediaError>>,
    /// Counter for sends that had to wait for queue space.
    queue_full: Arc<AtomicU64>,
}

impl MuxThread {
    /// Starts the thread that owns the muxer; the muxer must have been
    /// started, after every stream was added. The queue holds 1024 packets.
    pub fn start(muxer: Muxer) -> Result<Self, MediaError> {
        let (sender, receiver) = sync_channel(1024);
        let queue_full = Arc::new(AtomicU64::new(0));
        let join =
            std::thread::Builder::new().name("mux".to_string()).spawn(move || mux_loop(muxer, receiver)).map_err(|_| MediaError::Null("mux thread spawn"))?;
        Ok(Self { sender, join, queue_full })
    }

    /// Queues one packet for its stream, blocking until there is room and
    /// counting how often that wait happened.
    pub fn send(&self, stream: usize, packet: EncodedPacket) -> Result<(), MediaError> {
        match self.sender.try_send((stream, packet)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full((stream, packet))) => {
                self.queue_full.fetch_add(1, Ordering::Relaxed);
                self.sender.send((stream, packet)).map_err(|_| MediaError::Null("mux thread closed"))?;
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(MediaError::Null("mux thread closed")),
        }
    }

    /// Closes the queue, waits for the trailer and returns the summary.
    pub fn finish(self) -> Result<MuxSummary, MediaError> {
        drop(self.sender);
        let mut summary = self.join.join().map_err(|_| MediaError::Null("mux thread panicked"))??;
        summary.queue_full_waits = self.queue_full.load(Ordering::Relaxed);
        Ok(summary)
    }
}

/// Writes queued packets until the queue closes, then the trailer.
fn mux_loop(mut muxer: Muxer, receiver: Receiver<(usize, EncodedPacket)>) -> Result<MuxSummary, MediaError> {
    let mut packets = vec![0u64; muxer.stream_count()];
    while let Ok((stream, packet)) = receiver.recv() {
        let Some(slot) = packets.get_mut(stream) else {
            return Err(MediaError::Null("mux thread stream index"));
        };
        muxer.write(stream, &packet)?;
        *slot += 1;
    }
    let bytes_written = muxer.bytes_written();
    muxer.write_trailer()?;
    Ok(MuxSummary { bytes_written, packets, queue_full_waits: 0 })
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
