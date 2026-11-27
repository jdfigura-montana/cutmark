//! Bake with cursor (input track spec, "Review", item 3): writes `{stem}.baked.mp4`, a copy of a take's main file
//! with the cursor drawn into every frame.
//!
//! Every video frame is decoded in software, drawn on through cursor-render, converted to NV12 by the same video
//! processor as in a take and encoded with the take's codec and rate control. The audio streams are copied. The copy
//! is written to a temp file beside the take and renamed into place only when it is whole.
//!
//! The take's input track counts time from its first video frame. So times here, and in playback, are counted from
//! the video stream's start. A take's video starts at zero, but the cursor and the audio stay in step in a file
//! where it does not.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;

use capture::{Gpu, Nv12Converter};
use cursor_render::{CaptureMap, CursorStyle};
use events::EventsReader;
use ffmpeg_next::codec::{Id, decoder, threading};
use ffmpeg_next::format::Pixel;
use ffmpeg_next::media::Type;
use ffmpeg_next::packet::Packet;
use ffmpeg_next::software::scaling;
use ffmpeg_next::{Rational, Rescale, ffi, frame};
use media_io::encode::{Codec, ComboError, Container, Encoder, EncoderConfig, EncoderError, Preset, RateControl, aligned_1080_value, check_combo, probe_amf};
use media_io::player::probe::{MediaInfo, VideoInfo, probe};
use media_io::player::video::RgbaFrame;
use media_io::{AmfEncoder, AudioCodec, D3d11Device, D3d11FramePool, EncodedPacket, HwFrame, MediaError, Muxer};
use serde::Deserialize;
use thiserror::Error;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

use super::{BAKED_SUFFIX, SESSION_SUFFIX};
use crate::config::{LayoutKind, Profile};

/// What the temp file's name adds to the baked copy's name while the copy is written.
const TEMP_SUFFIX: &str = ".tmp";

/// Why a take could not be baked.
#[derive(Debug, Error)]
pub enum BakeError {
    /// The user stopped the bake.
    #[error("the bake was cancelled")]
    Cancelled,
    /// The take has no video stream.
    #[error("the take has no video")]
    NoVideo,
    /// The take shows only the camera, so there is no place for the cursor.
    #[error("the take shows only the camera, so it has no cursor to draw")]
    NoScreen,
    /// The take's audio is PCM, which the suite does not put in MP4.
    #[error("the take has PCM audio, which cannot be copied into an MP4 file")]
    PcmAudio,
    /// The video did not decode as expected.
    #[error("the video could not be decoded: {0}")]
    Decode(&'static str),
    /// The input track could not be read.
    #[error(transparent)]
    Events(#[from] events::EventsError),
    /// FFmpeg failed while reading or decoding.
    #[error("FFmpeg failed: {0}")]
    Ffmpeg(#[from] ffmpeg_next::Error),
    /// Writing the copy failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The encoder did not open.
    #[error(transparent)]
    Encoder(#[from] EncoderError),
    /// The GPU could not be used.
    #[error(transparent)]
    Capture(#[from] capture::CaptureError),
    /// A Direct3D call failed.
    #[error("Direct3D failed: {0}")]
    Direct3d(#[from] windows::core::Error),
    /// A file could not be moved into place.
    #[error("the baked file could not be saved: {0}")]
    Io(#[from] std::io::Error),
}

/// What a bake reads.
#[derive(Debug, Clone)]
pub struct BakeRequest {
    /// The take's main file.
    pub video: PathBuf,
    /// The take's events file.
    pub events: PathBuf,
    /// How the cursor is drawn.
    pub style: CursorStyle,
}

/// A finished bake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baked {
    /// The baked copy.
    pub file: PathBuf,
    /// How many video frames it holds.
    pub frames: u64,
    /// Why the copy uses another codec than the take, when it does.
    pub note: Option<String>,
}

/// Where the baked copy of a take's main file goes: `{stem}.baked.mp4` beside it.
pub fn output_path(video: &Path) -> PathBuf {
    let stem = video.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    video.with_file_name(format!("{stem}{BAKED_SUFFIX}"))
}

/// The file a bake writes before it is renamed over [`output_path`]. Its name keeps it out of the takes list.
fn temp_path(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(TEMP_SUFFIX);
    PathBuf::from(name)
}

/// The `profile` object of the take's session.json, when the file has one.
fn session_profile(video: &Path) -> Option<serde_json::Value> {
    let stem = video.file_stem()?.to_string_lossy().into_owned();
    let bytes = fs::read(video.with_file_name(format!("{stem}{SESSION_SUFFIX}"))).ok()?;
    let mut doc: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    doc.get_mut("profile").map(serde_json::Value::take).filter(serde_json::Value::is_object)
}

/// Whether the take's video shows the screen. Only a take that records says no: a Cam only layout.
pub fn shows_screen(video: &Path) -> bool {
    let layout = session_profile(video).and_then(|profile| LayoutKind::deserialize(&profile["layout"]).ok());
    layout != Some(LayoutKind::CamOnly)
}

/// When the file's first video frame is, in microseconds from the file's start.
pub fn video_start_us(video: &Path) -> Result<i64, BakeError> {
    media_io::init()?;
    let input = ffmpeg_next::format::input(video)?;
    let stream = input.streams().best(Type::Video).ok_or(BakeError::NoVideo)?;
    Ok(start_ticks(stream.start_time()).rescale(stream.time_base(), Rational::new(1, 1_000_000)))
}

/// A stream's start in its own time base; 0 when the container does not say.
fn start_ticks(start_time: i64) -> i64 {
    if start_time == ffi::AV_NOPTS_VALUE { 0 } else { start_time }
}

/// The encoder settings of the copy: the file's own codec and size, with the frame rate, rate control, preset and
/// keyframe interval of the profile in session.json. Without that, the defaults of a new encoder.
fn take_config(video: &Path, stream: &VideoInfo) -> Result<EncoderConfig, BakeError> {
    let codec = match stream.codec.as_str() {
        "av1" => Codec::Av1,
        "hevc" => Codec::Hevc,
        "h264" => Codec::H264,
        _ => return Err(BakeError::Decode("the video codec is not AV1, HEVC or H.264")),
    };
    let defaults = EncoderConfig::default();
    let profile = session_profile(video).and_then(|profile| Profile::deserialize(&profile).ok());
    let (fps, rate_control, preset, keyframe_secs): (u32, RateControl, Preset, f32) = match profile {
        Some(profile) => (profile.fps, profile.rate_control, profile.preset, profile.keyframe_secs),
        None => (0, defaults.rate_control, defaults.preset, defaults.keyframe_secs),
    };
    let fps = if fps > 0 { fps } else { stream.fps.round() as u32 };
    if fps == 0 {
        return Err(BakeError::Decode("the video has no frame rate"));
    }
    Ok(EncoderConfig { codec, width: stream.width, height: stream.height, fps_num: fps, fps_den: 1, rate_control, preset, keyframe_secs })
}

/// Makes the settings fit an MP4 file. AV1 at a height MP4 cannot take is encoded as HEVC, as the camera file is, and
/// the note says so. PCM audio cannot be helped.
fn fit_mp4(config: &mut EncoderConfig, info: &MediaInfo) -> Result<Option<String>, BakeError> {
    let pcm = info.audio.iter().any(|stream| stream.codec.starts_with("pcm"));
    // What the GPU can do at 1080 lines is only known once it has been asked.
    if config.codec == Codec::Av1 && config.height == 1080 && aligned_1080_value().is_none() {
        probe_amf(config.width, config.height, config.fps_num);
    }
    let audio = if pcm { AudioCodec::Pcm24 } else { AudioCodec::Aac { kbps: 320 } };
    match check_combo(Container::Mp4, config.codec, config.height, audio) {
        Ok(()) => Ok(None),
        Err(ComboError::Av1Mp4Height { height }) => {
            config.codec = Codec::Hevc;
            config.rate_control = hevc_rate_control(config.rate_control);
            if pcm {
                return Err(BakeError::PcmAudio);
            }
            Ok(Some(format!("Encoded as HEVC, because AV1 at {height}p cannot go in MP4.")))
        }
        Err(ComboError::PcmContainer) => Err(BakeError::PcmAudio),
        Err(ComboError::MovCodec) => Ok(None),
    }
}

/// An AV1 take's rate control for HEVC. Only the quantisers need changing: AV1 counts them to 255 and HEVC to 51.
fn hevc_rate_control(rate_control: RateControl) -> RateControl {
    let scaled = |qp: u8| (u16::from(qp) * 51 / 255) as u8;
    match rate_control {
        RateControl::ConstantQp { qp_i, qp_p } => RateControl::ConstantQp { qp_i: scaled(qp_i), qp_p: scaled(qp_p) },
        other => other,
    }
}

/// Bakes the take. `progress` is told the share done, from 0 to 1, and `cancel` is checked between packets. When the
/// bake fails or is cancelled, no file is left behind and an earlier baked copy stays as it was.
pub fn bake(request: &BakeRequest, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<Baked, BakeError> {
    let output = output_path(&request.video);
    let temp = temp_path(&output);
    // A bake that ended with the process may have left one.
    if temp.exists() {
        fs::remove_file(&temp)?;
    }
    let written = write_copy(request, &temp, cancel, progress).and_then(|(frames, note)| {
        if cancel.load(Ordering::Relaxed) {
            return Err(BakeError::Cancelled);
        }
        fs::rename(&temp, &output)?;
        Ok(Baked { file: output, frames, note })
    });
    if written.is_err()
        && temp.exists()
        && let Err(err) = fs::remove_file(&temp)
    {
        log::warn!("could not remove {}: {err}", temp.display());
    }
    written
}

/// The video side of a bake: everything one decoded frame goes through.
struct Painter<'a> {
    events: &'a EventsReader,
    map: CaptureMap,
    style: CursorStyle,
    size: (u32, u32),
    fps: u32,
    /// The decoder's picture to RGBA, made for the first frame's format and size.
    scaler: Option<(Pixel, u32, u32, scaling::Context)>,
    rgba: frame::Video,
    gpu: Gpu,
    upload: ID3D11Texture2D,
    converter: Nv12Converter,
    nv12: ID3D11Texture2D,
    pool: D3d11FramePool,
    encoder: AmfEncoder,
    /// The frame number of the last frame sent.
    last_index: Option<i64>,
    frames: u64,
}

impl Painter<'_> {
    /// Draws on one decoded frame that is `ticks` after the video's start, and encodes it.
    fn paint(&mut self, decoded: &frame::Video, ticks: i64, time_base: Rational, muxer: &mut Muxer) -> Result<(), BakeError> {
        // The take's frames lie on the frame grid; Matroska only rounds their times to milliseconds.
        let nearest = ticks.rescale(time_base, Rational::new(1, i32::try_from(self.fps).unwrap_or(i32::MAX)));
        let index = self.last_index.map_or(nearest.max(0), |last| nearest.max(last + 1));
        self.last_index = Some(index);
        let time_us = index.rescale(Rational::new(1, i32::try_from(self.fps).unwrap_or(i32::MAX)), Rational::new(1, 1_000_000));
        let mut picture = self.rgba_picture(decoded, time_us)?;
        cursor_render::render(&mut picture, self.events, time_us, &self.map, &self.style);
        upload_picture(&self.gpu, &self.upload, self.size, &mut picture)?;
        self.converter.convert(&self.upload, &self.nv12)?;
        let mut hardware = HwFrame::from_pool(&self.pool)?;
        hardware.copy_from(self.gpu.context(), &self.nv12)?;
        hardware.set_pts(index);
        self.encoder.send(hardware)?;
        self.frames += 1;
        let mut packets = Vec::new();
        self.encoder.receive(&mut |packet| packets.push(packet))?;
        write_video(muxer, packets)
    }

    /// The decoded picture as RGBA of the video's size. An encoder may code more rows than the video has; they are cut
    /// off.
    fn rgba_picture(&mut self, decoded: &frame::Video, time_us: i64) -> Result<RgbaFrame, BakeError> {
        let (format, width, height) = (decoded.format(), decoded.width(), decoded.height());
        if width < self.size.0 || height < self.size.1 {
            return Err(BakeError::Decode("a frame is smaller than the video"));
        }
        if self.scaler.as_ref().is_none_or(|(f, w, h, _)| (*f, *w, *h) != (format, width, height)) {
            let flags = scaling::Flags::BILINEAR | scaling::Flags::ACCURATE_RND | scaling::Flags::FULL_CHR_H_INT;
            let mut scaler = scaling::Context::get(format, width, height, Pixel::RGBA, width, height, flags)?;
            use_bt709(&mut scaler)?;
            self.scaler = Some((format, width, height, scaler));
            self.rgba = frame::Video::empty();
        }
        let Some((_, _, _, scaler)) = &mut self.scaler else { return Err(BakeError::Decode("no scaler")) };
        scaler.run(decoded, &mut self.rgba)?;
        let row = self.size.0 as usize * 4;
        let mut data = Vec::with_capacity(row * self.size.1 as usize);
        for line in self.rgba.data(0).chunks(self.rgba.stride(0)).take(self.size.1 as usize) {
            data.extend_from_slice(line.get(..row).ok_or(BakeError::Decode("a converted row is too short"))?);
        }
        if data.len() != row * self.size.1 as usize {
            return Err(BakeError::Decode("a converted frame is too short"));
        }
        let pts_ms = u64::try_from(time_us / 1000).unwrap_or(0);
        Ok(RgbaFrame { width: self.size.0, height: self.size.1, pts_ms, data })
    }
}

/// Puts the picture into a BGRA texture of the given size, the form a captured frame has. The picture's bytes are
/// reordered on the way.
#[allow(unsafe_code)]
fn upload_picture(gpu: &Gpu, texture: &ID3D11Texture2D, size: (u32, u32), picture: &mut RgbaFrame) -> Result<(), BakeError> {
    for pixel in picture.data.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    if picture.data.len() != size.0 as usize * size.1 as usize * 4 {
        return Err(BakeError::Decode("a frame does not match the texture"));
    }
    // SAFETY: the texture is live, on this context's device and of this size, and the data holds exactly that many
    // rows of the pitch given, as checked above. The call copies the data before it returns.
    unsafe { gpu.context().UpdateSubresource(texture, 0, None, picture.data.as_ptr().cast(), size.0 * 4, 0) };
    Ok(())
}

/// Tells swscale the picture is BT.709 in limited range, as a take is encoded, and that the RGB out is full range.
/// Its default is BT.601, which would shift the colours of the copy.
#[allow(unsafe_code)]
fn use_bt709(scaler: &mut scaling::Context) -> Result<(), BakeError> {
    // SAFETY: the context is live and exclusively borrowed; the coefficient table is static and only read.
    let code = unsafe {
        let table = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709);
        ffi::sws_setColorspaceDetails(scaler.as_mut_ptr(), table, 0, table, 1, 0, 1 << 16, 1 << 16)
    };
    if code < 0 { Err(BakeError::Decode("swscale refused the BT.709 colour space")) } else { Ok(()) }
}

/// A BGRA texture of the given size that the video processor can read.
#[allow(unsafe_code)]
fn upload_texture(gpu: &Gpu, width: u32, height: u32) -> Result<ID3D11Texture2D, BakeError> {
    let description = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    // SAFETY: the description is a plain value and the out pointer is valid.
    unsafe { gpu.device().CreateTexture2D(&description, None, Some(&mut texture)) }?;
    texture.ok_or(BakeError::Decode("Direct3D returned no texture"))
}

fn write_video(muxer: &mut Muxer, packets: Vec<EncodedPacket>) -> Result<(), BakeError> {
    for packet in packets {
        muxer.write(0, &packet)?;
    }
    Ok(())
}

/// Takes every frame the decoder has ready through the painter.
fn drain(decoder: &mut decoder::Video, painter: &mut Painter<'_>, start: i64, time_base: Rational, muxer: &mut Muxer) -> Result<(), BakeError> {
    let mut decoded = frame::Video::empty();
    loop {
        match decoder.receive_frame(&mut decoded) {
            Ok(()) => {
                let ticks = decoded.timestamp().ok_or(BakeError::Decode("a frame has no time stamp"))?;
                painter.paint(&decoded, ticks.saturating_sub(start), time_base, muxer)?;
            }
            Err(ffmpeg_next::Error::Eof) => return Ok(()),
            Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => return Ok(()),
            Err(err) => return Err(err.into()),
        }
    }
}

/// Writes the whole copy to `temp` and returns its frame count and the codec note.
fn write_copy(request: &BakeRequest, temp: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(f32)) -> Result<(u64, Option<String>), BakeError> {
    let info = probe(&request.video)?;
    let video_info = info.video.clone().ok_or(BakeError::NoVideo)?;
    if !shows_screen(&request.video) {
        return Err(BakeError::NoScreen);
    }
    let events = EventsReader::open(&request.events)?;
    let mut config = take_config(&request.video, &video_info)?;
    let note = fit_mp4(&mut config, &info)?;

    let mut input = ffmpeg_next::format::input(&request.video)?;
    let (video_index, video_tb, start, mut video_decoder) = {
        let stream = input.streams().best(Type::Video).ok_or(BakeError::NoVideo)?;
        let id = stream.parameters().id();
        // FFmpeg's own AV1 decoder only drives hardware; dav1d decodes in software.
        let codec = if id == Id::AV1 { decoder::find_by_name("libdav1d") } else { decoder::find(id) };
        let codec = codec.ok_or(BakeError::Decode("this FFmpeg build has no decoder for the video"))?;
        let mut context = ffmpeg_next::codec::Context::from_parameters(stream.parameters())?;
        // A count of 0 lets the decoder pick the number of threads.
        context.set_threading(threading::Config::count(0));
        let opened = context.decoder().open_as(codec)?.video()?;
        (stream.index(), stream.time_base(), start_ticks(stream.start_time()), opened)
    };

    let gpu = Gpu::create(None)?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let pool = D3d11FramePool::new(&hw_device, config.width, config.height, 8)?;
    let encoder = AmfEncoder::open(&config, &pool)?;

    let mut muxer = Muxer::create(temp, Container::Mp4)?;
    // The copy is thrown away unless it finishes, and a standard file keeps the audio's offset as it is.
    muxer.use_standard_layout();
    let title = request.video.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    muxer.set_tags(&title, concat!("CutMark Recorder ", env!("CARGO_PKG_VERSION")));
    let rate = i32::try_from(config.fps_num).map_err(|_| BakeError::Decode("the frame rate is too high"))?;
    muxer.add_video(encoder.parameters(), encoder.time_base(), (rate, 1))?;
    // The muxer stream each audio stream of the take goes to.
    let mut audio: Vec<(usize, usize, Rational)> = Vec::new();
    for stream in input.streams() {
        if stream.parameters().medium() == Type::Audio {
            audio.push((stream.index(), muxer.add_copied_audio(&stream)?, stream.time_base()));
        }
    }
    muxer.start()?;

    let converter = Nv12Converter::new(&gpu, config.width, config.height, config.width, config.height, config.fps_num)?;
    let nv12 = converter.create_output()?;
    let upload = upload_texture(&gpu, config.width, config.height)?;
    let mut painter = Painter {
        events: &events,
        map: CaptureMap::new(video_info.width, video_info.height),
        style: request.style,
        size: (config.width, config.height),
        fps: config.fps_num,
        scaler: None,
        rgba: frame::Video::empty(),
        gpu,
        upload,
        converter,
        nv12,
        pool,
        encoder,
        last_index: None,
        frames: 0,
    };

    let expected = (info.duration_ms.saturating_mul(u64::from(config.fps_num)) / 1000).max(1);
    let mut packet = Packet::empty();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(BakeError::Cancelled);
        }
        match packet.read(&mut input) {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Eof) => break,
            Err(err) => return Err(err.into()),
        }
        if packet.stream() == video_index {
            video_decoder.send_packet(&packet)?;
            drain(&mut video_decoder, &mut painter, start, video_tb, &mut muxer)?;
            // The last hundredth is the encoder's rest and the file's index.
            progress((painter.frames as f32 / expected as f32).min(0.99));
        } else if let Some(&(_, target, time_base)) = audio.iter().find(|(source, _, _)| *source == packet.stream()) {
            // The copy's video starts at zero, so the audio moves by as much as the video did.
            let shift = start.rescale(video_tb, time_base);
            let mut copy = std::mem::replace(&mut packet, Packet::empty());
            copy.set_pts(copy.pts().map(|pts| pts - shift));
            copy.set_dts(copy.dts().map(|dts| dts - shift));
            muxer.write_copied(target, copy)?;
        }
    }
    video_decoder.send_eof()?;
    drain(&mut video_decoder, &mut painter, start, video_tb, &mut muxer)?;
    let mut packets = Vec::new();
    painter.encoder.finish(&mut |packet| packets.push(packet))?;
    write_video(&mut muxer, packets)?;
    if painter.frames == 0 {
        return Err(BakeError::Decode("no frame could be decoded"));
    }
    muxer.finish()?;
    progress(1.0);
    Ok((painter.frames, note))
}

/// What the job's thread and its owner share.
#[derive(Default)]
struct Shared {
    /// The share done, in thousandths.
    progress: AtomicU32,
    cancel: AtomicBool,
}

/// A bake running on its own thread. Dropping it cancels the bake and waits for it, so no partial file stays behind
/// when the app closes.
pub struct BakeJob {
    /// The name of the take being baked.
    take: String,
    shared: Arc<Shared>,
    result: Receiver<Result<Baked, String>>,
    thread: Option<JoinHandle<()>>,
}

impl BakeJob {
    /// Starts the "bake" thread. `wake` is called when the progress moves and when the bake ends.
    pub fn start(take: String, request: BakeRequest, wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let theirs = Arc::clone(&shared);
        let (sender, result) = channel();
        let thread = std::thread::Builder::new().name("bake".to_owned()).spawn(move || {
            let outcome = bake(&request, &theirs.cancel, &mut |share| {
                let thousandths = (share.clamp(0.0, 1.0) * 1000.0) as u32;
                if theirs.progress.swap(thousandths, Ordering::Relaxed) != thousandths {
                    wake();
                }
            });
            match &outcome {
                Ok(baked) => log::info!("baked {} frames into {}", baked.frames, baked.file.display()),
                Err(BakeError::Cancelled) => log::info!("the bake of {} was cancelled", request.video.display()),
                Err(err) => log::warn!("the bake of {} failed: {err}", request.video.display()),
            }
            // A cancelled bake has nothing to report.
            if !matches!(outcome, Err(BakeError::Cancelled)) {
                let _ = sender.send(outcome.map_err(|err| err.to_string()));
            }
            drop(sender);
            wake();
        })?;
        Ok(Self { take, shared, result, thread: Some(thread) })
    }

    /// The name of the take being baked.
    pub fn take(&self) -> &str {
        &self.take
    }

    /// The share done, from 0 to 1.
    pub fn progress(&self) -> f32 {
        self.shared.progress.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Asks the bake to stop; [`BakeJob::poll`] tells when it has.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// None while the bake runs. Then how it ended: Some(None) after a cancel.
    pub fn poll(&self) -> Option<Option<Result<Baked, String>>> {
        match self.result.try_recv() {
            Ok(outcome) => Some(Some(outcome)),
            Err(TryRecvError::Empty) => None,
            // The thread ended without a result: it was cancelled, or it panicked.
            Err(TryRecvError::Disconnected) if self.shared.cancel.load(Ordering::Relaxed) => Some(None),
            Err(TryRecvError::Disconnected) => Some(Some(Err("the bake stopped without a result".to_owned()))),
        }
    }
}

impl Drop for BakeJob {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
#[path = "bake_tests.rs"]
pub(crate) mod tests;
