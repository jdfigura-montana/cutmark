//! Fast file probe: reads the container headers of a media file and reports its streams.
//!
//! [`probe`] opens the file, reads what the container itself says (codec, size, frame
//! rate, duration, audio channels and titles) and closes it again. No packets are
//! read and nothing is decoded, so it is fast even on long takes.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString};
use std::path::Path;

use ffmpeg_next::ffi;

use crate::hw::{MediaError, ff_error};

/// What a media file contains, read from its headers alone.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    /// The length of the file in milliseconds, 0 when the container does not say.
    pub duration_ms: u64,
    /// The video stream, `None` when the file has none.
    pub video: Option<VideoInfo>,
    /// Every audio stream, in file order.
    pub audio: Vec<AudioInfo>,
}

/// The video stream of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    /// The decoder name of the codec, for example "av1".
    pub codec: String,
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// The frames per second, from the stream's average frame rate.
    pub fps: f64,
}

/// One audio stream of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    /// The stream index in the file.
    pub index: usize,
    /// The decoder name of the codec, for example "aac".
    pub codec: String,
    /// The number of channels.
    pub channels: u32,
    /// The sample rate in hertz.
    pub rate: u32,
    /// The stream's title metadata, when it carries one.
    pub title: Option<String>,
}

/// Reads the headers of `path` and reports its streams. No packets are read or decoded.
pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    crate::init()?;
    let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|_| MediaError::Null("the media path"))?;
    let mut fmt: *mut ffi::AVFormatContext = std::ptr::null_mut();
    // SAFETY: the out pointer is valid, the path is NUL terminated and no custom IO is used.
    let code = unsafe { ffi::avformat_open_input(&mut fmt, c_path.as_ptr(), std::ptr::null(), std::ptr::null_mut()) };
    if code < 0 {
        return Err(ff_error(code));
    }
    let info = read_info(fmt);
    // SAFETY: the format context was opened above and this is its last owner.
    unsafe { ffi::avformat_close_input(&mut fmt) };
    info
}

/// Reads the streams out of an opened format context, headers only.
#[allow(unsafe_code)]
fn read_info(fmt: *mut ffi::AVFormatContext) -> Result<MediaInfo, MediaError> {
    // SAFETY: the context is live; its duration is in AV_TIME_BASE microseconds.
    let mut duration_ms = unsafe {
        let duration = (*fmt).duration;
        if duration > 0 { duration as u64 / 1_000 } else { 0 }
    };
    // SAFETY: the context is live and nb_streams counts its streams.
    let count = unsafe { (*fmt).nb_streams as usize };
    let mut video = None;
    let mut audio = Vec::new();
    for index in 0..count {
        // SAFETY: the index is below nb_streams, so the stream is live.
        let stream = unsafe { *(*fmt).streams.add(index) };
        // SAFETY: the stream is live and owns its codec parameters.
        let par = unsafe { (*stream).codecpar };
        if par.is_null() {
            continue;
        }
        // SAFETY: the stream and its codec parameters are live.
        let (kind, id) = unsafe { ((*par).codec_type, (*par).codec_id) };
        // SAFETY: the stream is live; its duration and time base are plain data.
        let (stream_duration, time_base) = unsafe { ((*stream).duration, (*stream).time_base) };
        if stream_duration > 0 && time_base.num > 0 && time_base.den > 0 {
            let ms = stream_duration * 1000 * i64::from(time_base.num) / i64::from(time_base.den);
            duration_ms = duration_ms.max(u64::try_from(ms).unwrap_or(0));
        }
        match kind {
            ffi::AVMediaType::AVMEDIA_TYPE_VIDEO if video.is_none() => {
                // SAFETY: the codec parameters are live.
                let (width, height) = unsafe { ((*par).width.max(0) as u32, (*par).height.max(0) as u32) };
                if width == 0 || height == 0 {
                    continue;
                }
                // SAFETY: the stream is live; the average frame rate is preferred and the real one is the fallback.
                let rate = unsafe {
                    let avg = (*stream).avg_frame_rate;
                    if avg.num > 0 && avg.den > 0 { avg } else { (*stream).r_frame_rate }
                };
                let fps = if rate.den > 0 { f64::from(rate.num) / f64::from(rate.den) } else { 0.0 };
                video = Some(VideoInfo { codec: codec_name(id), width, height, fps });
            }
            ffi::AVMediaType::AVMEDIA_TYPE_AUDIO => {
                // SAFETY: the codec parameters are live; ch_layout counts the channels.
                let (channels, rate) = unsafe { ((*par).ch_layout.nb_channels.max(0) as u32, (*par).sample_rate.max(0) as u32) };
                audio.push(AudioInfo { index, codec: codec_name(id), channels, rate, title: stream_title(stream) });
            }
            _ => {}
        }
    }
    if video.is_none() && audio.is_empty() {
        return Err(MediaError::Null("the file has no video or audio stream"));
    }
    Ok(MediaInfo { duration_ms, video, audio })
}

/// The decoder name of a codec id, for example "av1".
#[allow(unsafe_code)]
fn codec_name(id: ffi::AVCodecID) -> String {
    // SAFETY: avcodec_get_name returns a static NUL terminated string for every id.
    let name = unsafe { ffi::avcodec_get_name(id) };
    if name.is_null() {
        return "unknown".to_owned();
    }
    // SAFETY: the string is static and NUL terminated.
    unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned()
}

/// The title metadata tag of one stream, when it carries one.
#[allow(unsafe_code)]
fn stream_title(stream: *mut ffi::AVStream) -> Option<String> {
    // SAFETY: the stream is live; av_dict_get finds the title tag or returns null.
    let entry = unsafe { ffi::av_dict_get((*stream).metadata, c"title".as_ptr(), std::ptr::null(), 0) };
    if entry.is_null() {
        return None;
    }
    // SAFETY: the entry is live and its value is a NUL terminated string.
    let value = unsafe { (*entry).value };
    if value.is_null() {
        return None;
    }
    // SAFETY: the value is NUL terminated.
    Some(unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned())
}
