//! The muxer: copies encoder packets into a Matroska, MP4 or MOV file that stays
//! playable if the process dies mid-take, and rewrites MP4 and MOV takes as standard
//! files after a clean stop.

use std::ffi::{OsStr, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use ffmpeg_next::Dictionary;
use ffmpeg_next::Rational;
use ffmpeg_next::Rescale;
use ffmpeg_next::codec::Parameters;
use ffmpeg_next::ffi;
use ffmpeg_next::media::Type;
use ffmpeg_next::packet::{Flags, Mut, Packet, Ref};

use crate::MediaError;
use crate::encode::EncodedPacket;
use crate::hw::ff_error;

/// The stream time base for Matroska tracks: milliseconds.
const MKV_TIME_BASE: Rational = Rational(1, 1000);

/// The stream time base for MP4 and MOV audio tracks: the 90 kHz clock. Video tracks use one frame as their tick instead.
const MOV_TIME_BASE: Rational = Rational(1, 90_000);

/// The movflags that make a killed process leave a playable fragmented file.
const FRAGMENT_FLAGS: &str = "frag_keyframe+empty_moov+default_base_moof";

pub use crate::encode::Container;

/// FFmpeg's AVERROR_INVALIDDATA, which the bindings do not export.
const AVERROR_INVALIDDATA: i32 = -1_094_995_529;

/// FFmpeg's AVERROR_EXTERNAL, used for failures that come from the file system.
const AVERROR_EXTERNAL: i32 = -542_398_533;

/// How many packets may wait for a missing codec header before the take is given up.
const HELD_LIMIT: usize = 4096;

/// The suffix of the temp file a finalize writes beside the take, before it replaces the take.
const FINALIZE_SUFFIX: &str = ".finalizing";

/// The samples the AAC encoder puts ahead of the first real sample (its priming gap), as one encoder frame.
const AAC_PRIMING_SAMPLES: i64 = 1024;

/// The container tags a remux must not copy, because the new muxer writes its own brands.
const BRAND_TAGS: [&str; 3] = ["major_brand", "minor_version", "compatible_brands"];

/// The timing facts of one added stream, kept for rescaling on write.
struct StreamInfo {
    /// The time base the encoder stamps its packets in.
    encoder_time_base: Rational,
    /// One frame duration expressed as a rescale source unit, video only.
    frame_step: Option<Rational>,
}

/// Writes encoded packets into a Matroska, MP4 or MOV file and flushes on keyframes.
pub struct Muxer {
    ctx: ffmpeg_next::format::context::Output,
    /// The output file, read back for its final size after the trailer.
    path: PathBuf,
    /// The container being written.
    container: Container,
    streams: Vec<StreamInfo>,
    started: bool,
    /// Whether the container header is written; false while it waits for the first video packet.
    header_written: bool,
    /// The video streams whose codec header is still missing, by stream index.
    missing_headers: Vec<usize>,
    /// Packets that arrived before the header was written, in the encoder's time stamps.
    held: Vec<(usize, Packet)>,
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
        Ok(Self {
            ctx,
            path: path.to_path_buf(),
            container,
            streams: Vec::new(),
            started: false,
            header_written: false,
            missing_headers: Vec::new(),
            held: Vec::new(),
            chapters: Vec::new(),
        })
    }

    /// Sets the container tags: the encoder name, the title and the creation time (now, in UTC); call before `start`.
    ///
    /// FFmpeg's own muxers write their name in place of the encoder tag, whatever it is set to, so
    /// the encoder tag is only handed over here; the title and the creation time do reach the file.
    pub fn set_tags(&mut self, title: &str, encoder: &str) {
        let mut metadata = Dictionary::new();
        metadata.set("encoder", encoder);
        metadata.set("title", title);
        metadata.set("creation_time", &utc_timestamp(unix_seconds_now()));
        self.ctx.set_metadata(metadata);
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
        self.add_video_titled(params, encoder_time_base, fps, "Video")
    }

    /// Adds one video stream like `add_video`, with a metadata title that names it.
    pub fn add_video_titled(&mut self, params: Parameters, encoder_time_base: (i32, i32), fps: (i32, i32), title: &str) -> Result<usize, MediaError> {
        let encoder_time_base = Rational::new(encoder_time_base.0, encoder_time_base.1);
        let frame_step = Rational::new(fps.1, fps.0);
        let codec_id = params.id();
        // MP4 and MOV count one frame per tick (the muxer scales the track up to a whole number of ticks per
        // frame), so every frame is exactly one frame long; Matroska only has milliseconds.
        let time_base = if self.container == Container::Mkv { MKV_TIME_BASE } else { frame_step };
        let index = {
            let mut stream = self.ctx.add_stream(codec_id)?;
            stream.set_time_base(time_base);
            stream.set_rate(Rational::new(fps.0, fps.1));
            stream.set_avg_frame_rate(Rational::new(fps.0, fps.1));
            stream.set_parameters(params);
            stream.set_metadata(stream_tags(title, false));
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
            stream.set_metadata(stream_tags(title, true));
            if default {
                set_default_disposition(&mut stream);
            }
            stream.index()
        };
        self.streams.push(StreamInfo { encoder_time_base, frame_step: None });
        Ok(index)
    }

    /// Writes the container header; call once after all streams are added.
    ///
    /// When a video stream the container wants a codec header for has none yet (AMF may
    /// send its headers in-band only), the header waits for the first video packet.
    pub fn start(&mut self) -> Result<(), MediaError> {
        // Matroska cannot store negative timestamps, so tell the muxer to shift the AAC priming gap
        // instead of failing on the first packet. MP4 and MOV store it as an edit of the audio track, and
        // shifting there would push the video off its first frame and stretch its first sample.
        let avoid_negative_ts = match self.container {
            Container::Mkv => ffi::AVFMT_AVOID_NEG_TS_MAKE_ZERO,
            Container::Mp4 | Container::Mov => ffi::AVFMT_AVOID_NEG_TS_DISABLED,
        };
        // SAFETY: ctx is a live output context; setting avoid_negative_ts
        // before write_header is what avformat's own callers do.
        #[allow(unsafe_code)]
        unsafe {
            (*self.ctx.as_mut_ptr()).avoid_negative_ts = avoid_negative_ts as _;
        }
        self.missing_headers = self.video_streams_without_header();
        if self.missing_headers.is_empty() {
            self.write_header()?;
        } else {
            log::info!("the encoder gave no codec header for stream(s) {:?}; the container header waits for the first video packet", self.missing_headers);
        }
        self.started = true;
        Ok(())
    }

    /// Writes the container header with the movflags the container needs.
    fn write_header(&mut self) -> Result<(), MediaError> {
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
        self.header_written = true;
        Ok(())
    }

    /// The video streams that need a codec header in the container and have none: the output
    /// format wants global headers, the codec is one that carries its parameter sets in a header,
    /// and the stream's extradata is empty.
    #[allow(unsafe_code)]
    fn video_streams_without_header(&self) -> Vec<usize> {
        // SAFETY: ctx is a live output context; oformat is set by output_as and read only here.
        let wants_global_header = unsafe {
            let format = (*self.ctx.as_ptr()).oformat;
            !format.is_null() && (*format).flags & (ffi::AVFMT_GLOBALHEADER as c_int) != 0
        };
        if !wants_global_header {
            return Vec::new();
        }
        (0..self.streams.len())
            .filter(|&index| self.streams.get(index).is_some_and(|info| info.frame_step.is_some()))
            .filter(|&index| {
                // SAFETY: index is below the stream count, so the stream and its codecpar are live.
                let (id, extradata) = unsafe {
                    let par = (*(*(*self.ctx.as_ptr()).streams.add(index))).codecpar;
                    if par.is_null() { (ffi::AVCodecID::AV_CODEC_ID_NONE, 1) } else { ((*par).codec_id, (*par).extradata_size) }
                };
                extradata <= 0 && carries_codec_header(id)
            })
            .collect()
    }

    /// Queues one packet for the header, or writes it when the header is out: it stays in the
    /// encoder's time stamps until the stream time base is final.
    pub fn write(&mut self, stream: usize, packet: &EncodedPacket) -> Result<(), MediaError> {
        if !self.started {
            return Err(MediaError::Null("muxer write before start"));
        }
        if self.streams.get(stream).is_none() {
            return Err(MediaError::Null("muxer write stream index"));
        }
        let Some(data) = packet.data() else {
            return Ok(());
        };
        let mut out = Packet::copy(data);
        out.set_stream(stream);
        out.set_pts(packet.pts());
        out.set_dts(packet.dts());
        if packet.is_key() {
            out.set_flags(Flags::KEY);
        }
        if self.header_written { self.emit(stream, out) } else { self.hold(stream, out) }
    }

    /// Rescales one packet's timestamps into the stream's time base and
    /// interleaved-writes it, flushing the output when it is a keyframe.
    fn emit(&mut self, stream: usize, mut out: Packet) -> Result<(), MediaError> {
        let Self { ctx, streams, .. } = self;
        let Some(info) = streams.get(stream) else {
            return Err(MediaError::Null("muxer write stream index"));
        };
        let stream_tb = ctx.stream(stream).map(|s| s.time_base()).ok_or(MediaError::Null("muxer write stream"))?;
        let is_key = out.is_key();
        if let Some(frame_step) = info.frame_step {
            // Video lands on the frame grid: the time stamp snaps to its whole frame number, and every frame lasts one frame.
            let on_grid = |ts: i64| ts.rescale(info.encoder_time_base, frame_step).rescale(frame_step, stream_tb);
            out.set_pts(out.pts().map(on_grid));
            out.set_dts(out.dts().map(on_grid));
            out.set_duration(1i64.rescale(frame_step, stream_tb));
        } else {
            out.set_pts(out.pts().map(|pts| pts.rescale(info.encoder_time_base, stream_tb)));
            out.set_dts(out.dts().map(|dts| dts.rescale(info.encoder_time_base, stream_tb)));
        }
        out.write_interleaved(ctx)?;
        // Only video flushes on keyframes; audio packets interleave freely.
        if is_key && info.frame_step.is_some() {
            flush_output(ctx);
        }
        Ok(())
    }

    /// Keeps a packet until the header is out. The first packet of a video stream with no codec
    /// header gives it one through FFmpeg's extract_extradata filter; once every stream has one the
    /// header is written and the kept packets follow in arrival order.
    fn hold(&mut self, stream: usize, out: Packet) -> Result<(), MediaError> {
        if self.missing_headers.contains(&stream) {
            let encoder_time_base = self.streams.get(stream).map_or(Rational::new(1, 1000), |info| info.encoder_time_base);
            let extradata = extract_extradata(self.stream_parameters(stream)?, encoder_time_base, &out)?;
            self.set_extradata(stream, &extradata)?;
            self.missing_headers.retain(|&index| index != stream);
        }
        self.held.push((stream, out));
        if self.missing_headers.is_empty() {
            self.write_header()?;
            for (stream, packet) in std::mem::take(&mut self.held) {
                self.emit(stream, packet)?;
            }
        } else if self.held.len() > HELD_LIMIT {
            return Err(no_header("the video stream sent no packet to read its codec header from"));
        }
        Ok(())
    }

    /// The codec parameters of one stream of the output.
    #[allow(unsafe_code)]
    fn stream_parameters(&self, stream: usize) -> Result<*const ffi::AVCodecParameters, MediaError> {
        if stream >= self.streams.len() {
            return Err(MediaError::Null("muxer stream index"));
        }
        // SAFETY: stream is below the stream count, so the stream and its codecpar are live.
        let par = unsafe { (*(*(*self.ctx.as_ptr()).streams.add(stream))).codecpar };
        if par.is_null() { Err(MediaError::Null("stream codecpar")) } else { Ok(par.cast_const()) }
    }

    /// Gives one stream the codec header, as extradata in its codec parameters.
    #[allow(unsafe_code)]
    fn set_extradata(&mut self, stream: usize, data: &[u8]) -> Result<(), MediaError> {
        let par = self.stream_parameters(stream)?.cast_mut();
        let size = c_int::try_from(data.len()).map_err(|_| no_header("the codec header is too large"))?;
        // SAFETY: par is the live codecpar of the stream; the new buffer is padded as FFmpeg
        // requires, filled from data before it is stored, and owned by codecpar from then on,
        // which frees it with the stream. The old extradata is empty and freed with av_free.
        unsafe {
            let buffer = ffi::av_mallocz(data.len() + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize).cast::<u8>();
            if buffer.is_null() {
                return Err(MediaError::Null("av_mallocz"));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), buffer, data.len());
            ffi::av_free((*par).extradata.cast::<c_void>());
            (*par).extradata = buffer;
            (*par).extradata_size = size;
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
        if !self.header_written {
            return Err(no_header("the take ended before the video stream sent a packet to read its codec header from"));
        }
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

/// The metadata of one stream: its title, the handler name that MP4 and MOV keep the title in,
/// and for audio the undetermined language.
fn stream_tags(title: &str, audio: bool) -> Dictionary<'static> {
    let mut metadata = Dictionary::new();
    metadata.set("title", title);
    metadata.set("handler_name", title);
    if audio {
        metadata.set("language", "und");
    }
    metadata
}

/// Whether a codec keeps its parameter sets in a codec header that the container must carry.
fn carries_codec_header(id: ffi::AVCodecID) -> bool {
    matches!(id, ffi::AVCodecID::AV_CODEC_ID_H264 | ffi::AVCodecID::AV_CODEC_ID_HEVC | ffi::AVCodecID::AV_CODEC_ID_AV1)
}

/// The error for a stream whose codec header cannot be found.
fn no_header(reason: &str) -> MediaError {
    MediaError::Ffmpeg { code: AVERROR_INVALIDDATA, message: format!("no codec header for the video stream: {reason}") }
}

/// The error for a file system step of a finalize.
fn file_failure(what: &str, err: &std::io::Error) -> MediaError {
    MediaError::Ffmpeg { code: AVERROR_EXTERNAL, message: format!("{what}: {err}") }
}

/// Reads the codec header out of the first packet of a stream with FFmpeg's extract_extradata
/// bitstream filter: the sequence header OBU for AV1, the parameter sets for H.264 and HEVC.
#[allow(unsafe_code)]
fn extract_extradata(parameters: *const ffi::AVCodecParameters, time_base: Rational, packet: &Packet) -> Result<Vec<u8>, MediaError> {
    // SAFETY: the name is a static C string; the filter pointer is read only.
    let filter = unsafe { ffi::av_bsf_get_by_name(c"extract_extradata".as_ptr()) };
    if filter.is_null() {
        return Err(MediaError::Null("av_bsf_get_by_name extract_extradata"));
    }
    let mut context: *mut ffi::AVBSFContext = std::ptr::null_mut();
    // SAFETY: filter is live and context is a live out pointer.
    let code = unsafe { ffi::av_bsf_alloc(filter, &mut context) };
    if code < 0 || context.is_null() {
        return Err(ff_error(if code < 0 { code } else { AVERROR_INVALIDDATA }));
    }
    let found = filter_for_extradata(context, parameters, time_base, packet);
    // SAFETY: context is the live filter allocated above, freed exactly once.
    unsafe { ffi::av_bsf_free(&mut context) };
    found.and_then(|data| data.ok_or_else(|| no_header("the first packet holds none")))
}

/// Runs one packet through an allocated extract_extradata filter and returns the extradata it found, if any.
#[allow(unsafe_code)]
fn filter_for_extradata(
    context: *mut ffi::AVBSFContext,
    parameters: *const ffi::AVCodecParameters,
    time_base: Rational,
    packet: &Packet,
) -> Result<Option<Vec<u8>>, MediaError> {
    // SAFETY: context is a live, not yet initialised filter and parameters is the live codecpar of
    // a stream; par_in is allocated by av_bsf_alloc and filled by the copy.
    let code = unsafe {
        (*context).time_base_in = ffi::AVRational { num: time_base.numerator(), den: time_base.denominator() };
        ffi::avcodec_parameters_copy((*context).par_in, parameters)
    };
    if code < 0 {
        return Err(ff_error(code));
    }
    // SAFETY: context holds its parameters now.
    let code = unsafe { ffi::av_bsf_init(context) };
    if code < 0 {
        return Err(ff_error(code));
    }
    // The filter takes the packet it is given, so it gets a reference of its own.
    let mut input = packet.clone();
    // SAFETY: context is initialised and input is a live packet the filter may take over.
    let code = unsafe { ffi::av_bsf_send_packet(context, input.as_mut_ptr()) };
    if code < 0 {
        return Err(ff_error(code));
    }
    let mut found = None;
    loop {
        let mut filtered = Packet::empty();
        // SAFETY: context is initialised and filtered is a clean packet.
        let code = unsafe { ffi::av_bsf_receive_packet(context, filtered.as_mut_ptr()) };
        if code < 0 {
            // EAGAIN or end of stream: the filter has nothing more for this packet.
            break;
        }
        if found.is_some() {
            continue;
        }
        let mut size = 0usize;
        // SAFETY: filtered is a live packet; the side data it returns lives as long as the packet
        // and is copied before the packet is dropped.
        found = unsafe {
            let data = ffi::av_packet_get_side_data(filtered.as_ptr(), ffi::AVPacketSideDataType::AV_PKT_DATA_NEW_EXTRADATA, &mut size);
            if data.is_null() || size == 0 { None } else { Some(std::slice::from_raw_parts(data, size).to_vec()) }
        };
    }
    Ok(found)
}

/// The current time as seconds since the Unix epoch.
fn unix_seconds_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// Formats seconds since the Unix epoch as the ISO 8601 UTC text FFmpeg reads for creation_time.
fn utc_timestamp(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Days since 1970-01-01 to a calendar date (Howard Hinnant's civil_from_days).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000000Z", rest / 3600, rest / 60 % 60, rest % 60)
}

/// Rewrites a fragmented MP4 or MOV file as a standard file with the moov atom at the front.
///
/// Every stream and every tag is copied without re-encoding into a temp file beside the original,
/// which then replaces the original. On any failure the original stays as it was. MKV needs no
/// finalize and returns at once. `progress` is called with the share done, from 0 to 1.
pub fn finalize(path: &Path, container: Container, progress: &mut dyn FnMut(f32)) -> Result<(), MediaError> {
    let format = match container {
        Container::Mkv => return Ok(()),
        Container::Mp4 => "mp4",
        Container::Mov => "mov",
    };
    let mut name = path.file_name().map(OsStr::to_os_string).ok_or(MediaError::Null("finalize file name"))?;
    name.push(FINALIZE_SUFFIX);
    let temp = path.with_file_name(name);
    if let Err(err) = remux_faststart(path, &temp, format, progress) {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    if let Err(err) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(file_failure("the finished file could not replace the take", &err));
    }
    Ok(())
}

/// Copies every video and audio stream of `source` into `target` as a standard file of the given
/// muxer, with movflags faststart, and reports the share of the source read.
fn remux_faststart(source: &Path, target: &Path, format: &str, progress: &mut dyn FnMut(f32)) -> Result<(), MediaError> {
    let total = std::fs::metadata(source).map(|meta| meta.len()).unwrap_or(0);
    let mut input = ffmpeg_next::format::input(source)?;
    let mut output = ffmpeg_next::format::output_as(target, format)?;
    output.set_metadata(copied_tags(input.metadata().iter()));
    // The output stream each input stream goes to; streams that are neither video nor audio are dropped.
    let mut mapped: Vec<Option<usize>> = Vec::new();
    // How far each source stream moves earlier, in its own time base: the AAC priming gap, which the
    // fragmented file cannot hold (it starts every track at zero) and the standard file keeps as an edit.
    let mut priming: Vec<i64> = Vec::new();
    for source_stream in input.streams() {
        if !matches!(source_stream.parameters().medium(), Type::Video | Type::Audio) {
            mapped.push(None);
            priming.push(0);
            continue;
        }
        priming.push(priming_ticks(&source_stream));
        let mut target_stream = output.add_stream(source_stream.parameters().id())?;
        target_stream.set_parameters(source_stream.parameters());
        target_stream.set_time_base(source_stream.time_base());
        target_stream.set_rate(source_stream.rate());
        target_stream.set_avg_frame_rate(source_stream.avg_frame_rate());
        target_stream.set_metadata(copied_tags(source_stream.metadata().iter()));
        carry_stream_flags(&source_stream, &mut target_stream);
        mapped.push(Some(target_stream.index()));
    }
    let mut options = Dictionary::new();
    options.set("movflags", "faststart");
    output.write_header_with(options)?;
    let mut packet = Packet::empty();
    let mut percent = -1i32;
    loop {
        match packet.read(&mut input) {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Eof) => break,
            Err(err) => return Err(err.into()),
        }
        let source_index = packet.stream();
        let Some(Some(target_index)) = mapped.get(source_index).copied() else {
            continue;
        };
        let source_tb = input.stream(source_index).map(|s| s.time_base()).ok_or(MediaError::Null("finalize source stream"))?;
        let target_tb = output.stream(target_index).map(|s| s.time_base()).ok_or(MediaError::Null("finalize target stream"))?;
        let position = packet.position();
        let gap = priming.get(source_index).copied().unwrap_or(0);
        if gap != 0 {
            packet.set_pts(packet.pts().map(|pts| pts - gap));
            packet.set_dts(packet.dts().map(|dts| dts - gap));
        }
        packet.rescale_ts(source_tb, target_tb);
        packet.set_stream(target_index);
        packet.set_position(-1);
        packet.write_interleaved(&mut output)?;
        if total > 0 && position >= 0 {
            let now = (position as f64 / total as f64 * 100.0).clamp(0.0, 99.0) as i32;
            if now != percent {
                percent = now;
                progress(now as f32 / 100.0);
            }
        }
    }
    output.write_trailer()?;
    progress(1.0);
    Ok(())
}

/// The AAC priming gap of a stream in its own time base: one encoder frame of samples for an AAC audio
/// stream, zero for every other stream.
#[allow(unsafe_code)]
fn priming_ticks(stream: &ffmpeg_next::format::stream::Stream<'_>) -> i64 {
    // SAFETY: the stream belongs to a live input context, so its codecpar is live plain data.
    let (id, rate) = unsafe {
        let par = (*stream.as_ptr()).codecpar;
        if par.is_null() { (ffi::AVCodecID::AV_CODEC_ID_NONE, 0) } else { ((*par).codec_id, (*par).sample_rate) }
    };
    if id != ffi::AVCodecID::AV_CODEC_ID_AAC || rate <= 0 {
        return 0;
    }
    AAC_PRIMING_SAMPLES.rescale(Rational::new(1, rate), stream.time_base())
}

/// Copies a tag list without the brand tags, which the new muxer writes itself.
fn copied_tags<'a>(tags: impl Iterator<Item = (&'a str, &'a str)>) -> Dictionary<'static> {
    let mut copy = Dictionary::new();
    for (key, value) in tags {
        if !BRAND_TAGS.contains(&key) {
            copy.set(key, value);
        }
    }
    copy
}

/// Carries the disposition over and clears the codec tag, so the new muxer picks the tag it needs.
#[allow(unsafe_code)]
fn carry_stream_flags(source: &ffmpeg_next::format::stream::Stream<'_>, target: &mut ffmpeg_next::format::stream::StreamMut<'_>) {
    // SAFETY: both streams belong to live contexts; their codecpar and disposition fields are plain data.
    unsafe {
        let target_ptr = target.as_mut_ptr();
        (*target_ptr).disposition = (*source.as_ptr()).disposition;
        let par = (*target_ptr).codecpar;
        if !par.is_null() {
            (*par).codec_tag = 0;
        }
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

    /// The four character types of a file's top level boxes, in order.
    fn top_level_boxes(path: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let mut types = Vec::new();
        let mut offset = 0usize;
        while offset + 8 <= bytes.len() {
            let size = u32::from_be_bytes([bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]) as usize;
            types.push(String::from_utf8_lossy(&bytes[offset + 4..offset + 8]).into_owned());
            offset += if size == 0 { bytes.len() - offset } else { size.max(8) };
        }
        Ok(types)
    }

    #[test]
    fn finalize_rewrites_a_fragmented_file_with_the_moov_first_and_keeps_the_tags() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        for container in [Container::Mp4, Container::Mov] {
            let path = std::env::temp_dir().join(format!("cutmark_mux_final_{}_{nanos}.{}", std::process::id(), container.extension()));
            let mut muxer = Muxer::create(&path, container)?;
            muxer.set_tags("Take 1", "Cutmark Recorder test");
            muxer.add_video(mjpeg_params(), (1, 90_000), (30, 1))?;
            muxer.start()?;
            write_frames(&mut muxer)?;
            muxer.finish()?;
            let before = top_level_boxes(&path)?;
            let mut last = 0.0f32;
            finalize(&path, container, &mut |share| last = share)?;
            let after = top_level_boxes(&path)?;
            let input = ffmpeg_next::format::input(&path)?;
            let (duration, title, streams) = (input.duration(), input.metadata().get("title").map(str::to_owned), input.streams().count());
            drop(input);
            let temp_left = path.with_file_name(format!("{}{FINALIZE_SUFFIX}", path.file_name().and_then(OsStr::to_str).unwrap_or_default())).exists();
            let _ = std::fs::remove_file(&path);
            assert!(before.iter().any(|kind| kind == "moof"), "{container:?} starts fragmented: {before:?}");
            assert!(!after.iter().any(|kind| kind == "moof"), "{container:?} ends standard: {after:?}");
            let (moov, mdat) = (after.iter().position(|kind| kind == "moov"), after.iter().position(|kind| kind == "mdat"));
            assert!(moov.is_some() && moov < mdat, "{container:?} moov before mdat: {after:?}");
            assert!(duration > 0, "{container:?} duration {duration}");
            assert_eq!(title.as_deref(), Some("Take 1"), "{container:?}");
            assert_eq!(streams, 1, "{container:?}");
            assert_eq!(last, 1.0, "{container:?}");
            assert!(!temp_left, "{container:?} left its temp file");
        }
        Ok(())
    }

    #[test]
    fn a_matroska_file_keeps_the_title_the_creation_time_and_the_stream_titles() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("cutmark_mux_tags_{}_{nanos}.mkv", std::process::id()));
        let mut muxer = Muxer::create(&path, Container::Mkv)?;
        muxer.set_tags("Take 2", "Cutmark Recorder test");
        muxer.add_video_titled(mjpeg_params(), (1, 1000), (30, 1), "Camera")?;
        muxer.start()?;
        write_frames(&mut muxer)?;
        muxer.finish()?;
        let input = ffmpeg_next::format::input(&path)?;
        let title = input.metadata().get("title").map(str::to_owned);
        let created = input.metadata().get("creation_time").is_some();
        let stream_title = input.stream(0).and_then(|stream| stream.metadata().get("title").map(str::to_owned));
        drop(input);
        let _ = std::fs::remove_file(&path);
        assert_eq!(title.as_deref(), Some("Take 2"));
        assert!(created, "creation_time is written");
        assert_eq!(stream_title.as_deref(), Some("Camera"));
        Ok(())
    }

    #[test]
    fn finalizing_a_file_that_is_not_there_fails_and_leaves_nothing_behind() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let path = std::env::temp_dir().join(format!("cutmark_mux_missing_{}.mp4", std::process::id()));
        assert!(finalize(&path, Container::Mp4, &mut |_| {}).is_err());
        assert!(!path.with_file_name(format!("{}{FINALIZE_SUFFIX}", path.file_name().and_then(OsStr::to_str).unwrap_or_default())).exists());
        // MKV needs nothing and succeeds even for a name that does not exist.
        assert!(finalize(&path, Container::Mkv, &mut |_| {}).is_ok());
        Ok(())
    }

    /// Codec parameters for a tiny H.264 video stream with no codec header, as AMF may deliver it.
    fn headerless_h264_params() -> Parameters {
        let mut params = Parameters::new();
        // SAFETY: params owns a fresh AVCodecParameters, so writing its plain fields is sound.
        #[allow(unsafe_code)]
        unsafe {
            let raw = params.as_mut_ptr();
            (*raw).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
            (*raw).codec_id = ffi::AVCodecID::AV_CODEC_ID_H264;
            (*raw).width = 16;
            (*raw).height = 16;
        }
        params
    }

    #[test]
    fn a_video_stream_with_no_codec_header_waits_for_one_and_is_an_error_when_none_comes() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("cutmark_mux_noheader_{}_{nanos}.mkv", std::process::id()));
        let mut muxer = Muxer::create(&path, Container::Mkv)?;
        muxer.add_video(headerless_h264_params(), (1, 1000), (30, 1))?;
        muxer.start()?;
        assert!(!muxer.header_written, "the header waits for the first video packet");
        // A lone access unit delimiter holds no parameter sets.
        let mut packet = Packet::copy(&[0, 0, 0, 1, 0x09, 0x10]);
        packet.set_pts(Some(0));
        packet.set_dts(Some(0));
        packet.set_flags(Flags::KEY);
        let err = muxer.write(0, &EncodedPacket::new(packet)).err().map(|err| err.to_string());
        drop(muxer);
        let _ = std::fs::remove_file(&path);
        assert!(err.is_some_and(|text| text.contains("codec header")));
        Ok(())
    }

    /// Codec parameters for a 48 kHz stereo AAC stream with the usual one frame priming gap and no encoder behind it.
    fn aac_params() -> Parameters {
        let mut params = Parameters::new();
        // SAFETY: params owns a fresh AVCodecParameters, so writing its plain fields is sound; the extradata
        // buffer is padded as FFmpeg requires and owned by the parameters from then on.
        #[allow(unsafe_code)]
        unsafe {
            let raw = params.as_mut_ptr();
            (*raw).codec_type = ffi::AVMediaType::AVMEDIA_TYPE_AUDIO;
            (*raw).codec_id = ffi::AVCodecID::AV_CODEC_ID_AAC;
            (*raw).sample_rate = 48_000;
            (*raw).frame_size = 1024;
            (*raw).initial_padding = 1024;
            ffi::av_channel_layout_default(&mut (*raw).ch_layout, 2);
            // The AudioSpecificConfig of AAC LC, 48 kHz, stereo.
            let buffer = ffi::av_mallocz(2 + ffi::AV_INPUT_BUFFER_PADDING_SIZE as usize).cast::<u8>();
            if !buffer.is_null() {
                buffer.write(0x11);
                buffer.add(1).write(0x90);
                (*raw).extradata = buffer;
                (*raw).extradata_size = 2;
            }
        }
        params
    }

    /// The media time of the first edit in every elst box of a file, in file order (one per track).
    fn edit_media_times(path: &Path) -> Result<Vec<i64>, Box<dyn std::error::Error>> {
        let bytes = std::fs::read(path)?;
        let mut times = Vec::new();
        for at in bytes.windows(4).enumerate().filter(|(_, tag)| *tag == b"elst").map(|(at, _)| at) {
            // After the tag: version and flags, the entry count, then segment duration and media time.
            let version = *bytes.get(at + 4).ok_or("cut off elst")?;
            let field = if version == 1 { 8 } else { 4 };
            let start = at + 4 + 4 + 4 + field;
            let raw = bytes.get(start..start + field).ok_or("cut off elst")?;
            times.push(if field == 8 { i64::from_be_bytes(raw.try_into()?) } else { i64::from(i32::from_be_bytes(raw.try_into()?)) });
        }
        Ok(times)
    }

    #[test]
    fn video_starts_at_zero_on_the_frame_grid_and_audio_keeps_only_its_priming_edit() -> Result<(), Box<dyn std::error::Error>> {
        crate::init()?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        const FRAMES: i64 = 120;
        for container in [Container::Mp4, Container::Mov] {
            let path = std::env::temp_dir().join(format!("cutmark_mux_align_{}_{nanos}.{}", std::process::id(), container.extension()));
            let mut muxer = Muxer::create(&path, container)?;
            let video = muxer.add_video(mjpeg_params(), (1, 60), (60, 1))?;
            let audio = muxer.add_audio(aac_params(), (1, 48_000), "Mixdown", true)?;
            muxer.start()?;
            // The AAC encoder stamps its first packet one frame early (the priming gap) and the video starts at frame 0.
            let audio_packets = FRAMES * 48_000 / 60 / 1024;
            let mut next_audio = 0;
            for frame in 0..FRAMES {
                let mut packet = Packet::copy(&[0xFF, 0xD8, 0xFF, 0xD9]);
                packet.set_pts(Some(frame));
                packet.set_dts(Some(frame));
                packet.set_flags(Flags::KEY);
                muxer.write(video, &EncodedPacket::new(packet))?;
                while next_audio < audio_packets && (next_audio - 1) * 1024 * 60 <= frame * 48_000 {
                    let mut packet = Packet::copy(&[0x21, 0x10, 0x04, 0x60]);
                    packet.set_pts(Some((next_audio - 1) * 1024));
                    packet.set_dts(Some((next_audio - 1) * 1024));
                    packet.set_flags(Flags::KEY);
                    muxer.write(audio, &EncodedPacket::new(packet))?;
                    next_audio += 1;
                }
            }
            muxer.finish()?;
            finalize(&path, container, &mut |_| {})?;
            let edits = edit_media_times(&path)?;
            let mut input = ffmpeg_next::format::input(&path)?;
            let starts: Vec<i64> = input.streams().map(|stream| stream.start_time()).collect();
            let video_time_base = input.stream(video).map(|stream| stream.time_base()).ok_or("no video stream")?;
            let mut stamps = Vec::new();
            for (stream, packet) in input.packets() {
                if stream.index() == video {
                    stamps.push((packet.dts().unwrap_or(i64::MIN), packet.duration()));
                }
            }
            drop(input);
            let _ = std::fs::remove_file(&path);
            let tick = i64::from(video_time_base.denominator()) / i64::from(video_time_base.numerator()) / 60;
            assert_eq!(starts.first().copied(), Some(0), "{container:?} video starts at zero: {starts:?}");
            assert_eq!(starts.get(audio).copied(), Some(0), "{container:?} audio starts with the video: {starts:?}");
            // One edit per track: the video plays from its first sample, the audio skips its priming frame.
            assert_eq!(edits, vec![0, 1024], "{container:?} edit list media times");
            assert_eq!(stamps.len(), FRAMES as usize, "{container:?}");
            for (index, (dts, duration)) in stamps.iter().enumerate() {
                assert_eq!((*dts, *duration), (index as i64 * tick, tick), "{container:?} frame {index} sits on the grid");
            }
        }
        Ok(())
    }

    #[test]
    fn creation_times_are_iso_8601_in_utc() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00.000000Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20.000000Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00.000000Z");
        assert_eq!(utc_timestamp(1_798_761_599), "2026-12-31T23:59:59.000000Z");
    }

    #[test]
    fn only_codecs_with_a_codec_header_hold_the_container_header() {
        assert!(carries_codec_header(ffi::AVCodecID::AV_CODEC_ID_AV1));
        assert!(carries_codec_header(ffi::AVCodecID::AV_CODEC_ID_HEVC));
        assert!(carries_codec_header(ffi::AVCodecID::AV_CODEC_ID_H264));
        assert!(!carries_codec_header(ffi::AVCodecID::AV_CODEC_ID_MJPEG));
        assert!(!carries_codec_header(ffi::AVCodecID::AV_CODEC_ID_AAC));
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
