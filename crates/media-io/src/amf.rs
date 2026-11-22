//! The AMD AMF hardware encoder backend.

use std::ffi::{CStr, CString, c_int, c_void};

use ffmpeg_next::Rational;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::codec::{Context, Flags};
use ffmpeg_next::ffi;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::packet::Mut;
use windows::Win32::Graphics::Direct3D11::ID3D11Device;
use windows::core::Interface;

use crate::MediaError;
use crate::encode::{Codec, ConfigError, EncodedPacket, Encoder, EncoderConfig, EncoderError, Preset, RateControl};
use crate::hw::{D3d11Device, D3d11FramePool, HwFrame, ff_error};

/// The av_opt search flag that also looks through an object's children, so one
/// call reaches both the generic codec options and the private AMF ones.
const SEARCH_CHILDREN: c_int = 1;

/// The codecs the AMF probe covers, in display order.
const PROBE_CODECS: [Codec; 3] = [Codec::Av1, Codec::Hevc, Codec::H264];

/// An FFmpeg hardware encoder running through AMD AMF.
pub struct AmfEncoder {
    /// The opened FFmpeg video encoder.
    encoder: encoder::Video,
    /// The packet time base, as (numerator, denominator).
    time_base: (i32, i32),
    /// Whether end of stream has been sent.
    finished: bool,
    /// The applied options as read back from the opened encoder.
    read_back: Vec<(String, String)>,
}

/// One numeric encoder option: its name on this encoder and the range it allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericOption {
    /// The option name as this encoder spells it.
    pub name: String,
    /// The smallest usable value. A reported minimum of -1 means "unset" and is replaced by the floor.
    pub min: i64,
    /// The largest value the encoder's option table allows.
    pub max: i64,
}

impl NumericOption {
    /// Builds an option from the bounds an option table reports. The table's -1
    /// sentinel (and anything else below the floor) becomes the floor, the
    /// lowest value that really selects something.
    pub fn from_reported(name: &str, min: i64, max: i64, floor: i64) -> Self {
        Self { name: name.to_string(), min: min.max(floor), max }
    }
}

/// What one encoder's option table offers, read with av_opt_find at open time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OptionRanges {
    /// The QVBR quality level option, when the encoder has one.
    pub quality_level: Option<NumericOption>,
    /// The I frame quantiser option, when the encoder has one.
    pub qp_i: Option<NumericOption>,
    /// The P frame quantiser option, when the encoder has one.
    pub qp_p: Option<NumericOption>,
    /// The named values of the rate control option.
    pub rc_modes: Vec<String>,
    /// The name of the preset option ("quality" or "preset"), when the encoder has one.
    pub preset_option: Option<String>,
    /// The named values of the preset option.
    pub presets: Vec<String>,
    /// Whether the encoder has a filler_data option.
    pub has_filler_data: bool,
    /// Whether the encoder has an enforce_hrd option.
    pub has_enforce_hrd: bool,
}

/// The outcome of probing one AMF codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecSupport {
    /// The codec that was probed.
    pub codec: Codec,
    /// Ok when the encoder opened, else the reason it did not.
    pub result: Result<(), String>,
    /// The option ranges read from the encoder; empty when it could not be created.
    pub ranges: OptionRanges,
}

impl AmfEncoder {
    /// Opens an AMF encoder for the given configuration on the pool's frames context.
    pub fn open(cfg: &EncoderConfig, pool: &D3d11FramePool) -> Result<Self, EncoderError> {
        Self::open_reporting(cfg, pool).1
    }

    /// Opens an encoder and also returns the option ranges read from it, even when opening fails.
    fn open_reporting(cfg: &EncoderConfig, pool: &D3d11FramePool) -> (OptionRanges, Result<Self, EncoderError>) {
        let mut ranges = OptionRanges::default();
        let result = Self::open_into(cfg, pool, &mut ranges);
        (ranges, result)
    }

    /// Opens an encoder, storing the option ranges as soon as they are readable.
    fn open_into(cfg: &EncoderConfig, pool: &D3d11FramePool, ranges_out: &mut OptionRanges) -> Result<Self, EncoderError> {
        cfg.validate()?;
        let name = codec_name(cfg.codec);
        let codec = encoder::find_by_name(name).ok_or_else(|| MediaError::EncoderMissing(name.to_string()))?;
        let mut context = Context::new_with_codec(codec);
        // MKV and MP4 both want the extradata in the container, not in-band.
        context.set_flags(Flags::GLOBAL_HEADER);
        attach_frames_ctx(&context, pool)?;
        let priv_data = private_options(&context);
        let context_ptr = raw_context(&context);
        *ranges_out = read_option_ranges(priv_data);
        let options = amf_options(cfg.codec, cfg, ranges_out)?;
        let mut video = context.encoder().video().map_err(MediaError::Init)?;
        video.set_width(cfg.width);
        video.set_height(cfg.height);
        video.set_time_base(Rational::new(time_base_num(cfg), time_base_den(cfg)));
        video.set_frame_rate(Some(Rational::new(frame_rate_num(cfg), frame_rate_den(cfg))));
        video.set_format(Pixel::from(ffi::AVPixelFormat::AV_PIX_FMT_D3D11));
        apply_options(context_ptr, priv_data, name, &options)?;
        let encoder = video.open().map_err(MediaError::Init)?;
        let mut opened = Self { encoder, time_base: (time_base_num(cfg), time_base_den(cfg)), finished: false, read_back: Vec::new() };
        opened.read_back = read_back_options(opened.context_ptr(), name, &options)?;
        log::info!("AMF {name}: codec header of {} bytes after open (0 means the muxer reads it from the first packet)", opened.header_bytes());
        Ok(opened)
    }

    /// How many bytes of codec header (extradata) the opened encoder holds; the flag asking for
    /// global headers is set for MKV, MP4 and MOV alike, which all have AVFMT_GLOBALHEADER.
    #[allow(unsafe_code)]
    fn header_bytes(&mut self) -> usize {
        // SAFETY: the encoder owns a live AVCodecContext; extradata_size is plain data.
        let size = unsafe { (*self.context_ptr()).extradata_size };
        usize::try_from(size).unwrap_or(0)
    }

    /// Every applied option as the opened encoder reports it, in the order they were set.
    pub fn read_back(&self) -> &[(String, String)] {
        &self.read_back
    }

    /// Hands every ready packet to the callback, stopping at EAGAIN, and also
    /// at end of stream when draining.
    #[allow(unsafe_code)]
    fn drain(&mut self, out: &mut dyn FnMut(EncodedPacket), allow_eof: bool) -> Result<(), MediaError> {
        loop {
            let mut packet = ffmpeg_next::packet::Packet::empty();
            // SAFETY: the encoder context and the packet are live;
            // receive_packet fills the packet or returns a negative code.
            let code = unsafe { ffi::avcodec_receive_packet(self.context_ptr(), packet.as_mut_ptr()) };
            if code < 0 {
                match ffmpeg_next::Error::from(code) {
                    ffmpeg_next::Error::Other { errno } if errno == ffi::EAGAIN => return Ok(()),
                    ffmpeg_next::Error::Eof if allow_eof => return Ok(()),
                    err => return Err(MediaError::Init(err)),
                }
            }
            out(EncodedPacket::new(packet));
        }
    }
}

impl Encoder for AmfEncoder {
    #[allow(unsafe_code)]
    fn send(&mut self, frame: HwFrame) -> Result<(), MediaError> {
        // SAFETY: the encoder context and the frame are live; avcodec_send_frame
        // takes only a reference during the call, and the frame is dropped after.
        let code = unsafe { ffi::avcodec_send_frame(self.context_ptr(), frame.frame_ptr()) };
        if code < 0 {
            return Err(ff_error(code));
        }
        Ok(())
    }

    fn receive(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError> {
        self.drain(out, false)
    }

    #[allow(unsafe_code)]
    fn finish(&mut self, out: &mut dyn FnMut(EncodedPacket)) -> Result<(), MediaError> {
        if !self.finished {
            // SAFETY: the encoder context is live; a null frame tells the
            // encoder to enter draining mode.
            let code = unsafe { ffi::avcodec_send_frame(self.context_ptr(), std::ptr::null_mut()) };
            if code < 0 {
                return Err(ff_error(code));
            }
            self.finished = true;
        }
        self.drain(out, true)
    }

    fn time_base(&self) -> (i32, i32) {
        self.time_base
    }

    fn parameters(&self) -> ffmpeg_next::codec::Parameters {
        ffmpeg_next::codec::Parameters::from(&self.encoder)
    }
}

impl AmfEncoder {
    /// The raw encoder context pointer, valid until the encoder is dropped.
    #[allow(unsafe_code)]
    fn context_ptr(&mut self) -> *mut ffi::AVCodecContext {
        // SAFETY: the encoder owns a live AVCodecContext.
        unsafe { self.encoder.as_mut().as_mut_ptr() }
    }
}

/// Opens each AMF codec briefly on a D3D11 frames context and reports what each offers.
///
/// The probe device is the one FFmpeg creates on the default hardware adapter.
pub fn probe_amf(width: u32, height: u32, fps: u32) -> Vec<CodecSupport> {
    match probe_all(width, height, fps) {
        Ok(support) => support,
        Err(reason) => PROBE_CODECS.iter().map(|&codec| CodecSupport { codec, result: Err(reason.clone()), ranges: OptionRanges::default() }).collect(),
    }
}

/// Builds the probe device and frame pool, then opens every AMF codec on it.
fn probe_all(width: u32, height: u32, fps: u32) -> Result<Vec<CodecSupport>, String> {
    let probe_device = ProbeDevice::create()?;
    let hw_device = D3d11Device::from_device(&probe_device.device).map_err(|err| err.to_string())?;
    let pool = D3d11FramePool::new(&hw_device, width, height, 4).map_err(|err| err.to_string())?;
    Ok(PROBE_CODECS
        .iter()
        .map(|&codec| {
            let cfg = EncoderConfig { codec, width, height, fps_num: fps.max(1), fps_den: 1, ..EncoderConfig::default() };
            let (ranges, result) = AmfEncoder::open_reporting(&cfg, &pool);
            CodecSupport { codec, result: result.map(drop).map_err(|err| err.to_string()), ranges }
        })
        .collect())
}

/// A D3D11 device created by FFmpeg for probing, kept alive by its buffer reference.
struct ProbeDevice {
    /// The FFmpeg hardware device reference that owns the device.
    buffer: *mut ffi::AVBufferRef,
    /// A counted reference to the D3D11 device inside it.
    device: ID3D11Device,
}

impl ProbeDevice {
    /// Creates a D3D11 device with video support on the default hardware adapter.
    #[allow(unsafe_code)]
    fn create() -> Result<Self, String> {
        let mut buffer: *mut ffi::AVBufferRef = std::ptr::null_mut();
        // SAFETY: the out pointer is a live local; a null device name and options
        // select the default adapter and no extra settings.
        let code =
            unsafe { ffi::av_hwdevice_ctx_create(&mut buffer, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA, std::ptr::null(), std::ptr::null_mut(), 0) };
        if code < 0 {
            return Err(format!("D3D11 device creation failed: {}", ff_error(code)));
        }
        // SAFETY: on success buffer holds an AVHWDeviceContext whose D3D11VA
        // hwctx starts with the ID3D11Device pointer; the borrowed reference is
        // cloned, which adds a reference of its own.
        let device = unsafe {
            let context = (*buffer).data.cast::<ffi::AVHWDeviceContext>();
            let raw = (*context).hwctx.cast::<*mut c_void>().read();
            ID3D11Device::from_raw_borrowed(&raw).cloned()
        };
        match device {
            Some(device) => Ok(Self { buffer, device }),
            None => {
                // SAFETY: buffer is the live reference created above.
                unsafe { ffi::av_buffer_unref(&mut buffer) };
                Err("FFmpeg created a D3D11 device context without a device".to_string())
            }
        }
    }
}

impl Drop for ProbeDevice {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: buffer is the live reference this struct owns.
        unsafe { ffi::av_buffer_unref(&mut self.buffer) };
    }
}

/// The AMF encoder name for each codec.
fn codec_name(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "av1_amf",
        Codec::Hevc => "hevc_amf",
        Codec::H264 => "h264_amf",
    }
}

/// The frame rate numerator as an i32.
fn frame_rate_num(cfg: &EncoderConfig) -> i32 {
    i32::try_from(cfg.fps_num).unwrap_or(i32::MAX)
}

/// The frame rate denominator as a positive i32.
fn frame_rate_den(cfg: &EncoderConfig) -> i32 {
    i32::try_from(cfg.fps_den.max(1)).unwrap_or(1)
}

/// The packet time base numerator: the frame rate denominator.
fn time_base_num(cfg: &EncoderConfig) -> i32 {
    frame_rate_den(cfg)
}

/// The packet time base denominator: the frame rate numerator.
fn time_base_den(cfg: &EncoderConfig) -> i32 {
    frame_rate_num(cfg)
}

/// The keyframe interval in frames: keyframe_secs times the frame rate, rounded, at least 1.
fn gop_frames(cfg: &EncoderConfig) -> u32 {
    let fps = f64::from(cfg.fps_num) / f64::from(cfg.fps_den.max(1));
    let frames = (f64::from(cfg.keyframe_secs) * fps).round().max(1.0);
    // The saturating float to integer cast is the intended clamp.
    frames as u32
}

/// Adds a reference of the pool's frames context to the encoder context.
#[allow(unsafe_code)]
fn attach_frames_ctx(context: &Context, pool: &D3d11FramePool) -> Result<(), MediaError> {
    // SAFETY: the context is live and exclusively owned; av_buffer_ref adds a
    // reference the encoder context then owns and releases when it is freed.
    let frames_ctx = unsafe {
        let ptr = context.as_ptr().cast_mut();
        (*ptr).hw_frames_ctx = ffi::av_buffer_ref(pool.frames_ref_ptr());
        (*ptr).hw_frames_ctx
    };
    if frames_ctx.is_null() {
        return Err(MediaError::Null("av_buffer_ref"));
    }
    Ok(())
}

/// The encoder context's private option table object.
#[allow(unsafe_code)]
fn private_options(context: &Context) -> *mut c_void {
    // SAFETY: the context is live; priv_data stays allocated until the context
    // is freed, which happens only when the encoder built from it is dropped.
    unsafe { (*context.as_ptr()).priv_data }
}

/// The raw AVCodecContext pointer, valid until the encoder context is freed.
#[allow(unsafe_code)]
fn raw_context(context: &Context) -> *mut ffi::AVCodecContext {
    // SAFETY: the context is live and exclusively owned by this function's caller.
    unsafe { context.as_ptr() }.cast_mut()
}

/// Looks up one private option and returns its table entry, or null. Named
/// constants that share the option's name are not options and count as absent.
#[allow(unsafe_code)]
fn find_option(priv_data: *mut c_void, option: &CStr) -> *const ffi::AVOption {
    // SAFETY: priv_data is live for the whole open call; av_opt_find only reads.
    let entry = unsafe { ffi::av_opt_find(priv_data, option.as_ptr(), std::ptr::null(), 0, 0) };
    // SAFETY: a non-null entry points into the encoder's static option table.
    if !entry.is_null() && unsafe { (*entry).type_ } == ffi::AVOptionType::AV_OPT_TYPE_CONST {
        return std::ptr::null();
    }
    entry
}

/// Reads the numeric range of the first of the candidate option names the encoder has,
/// raising an unset minimum to the floor.
#[allow(unsafe_code)]
fn numeric_option(priv_data: *mut c_void, candidates: &[&CStr], floor: i64) -> Option<NumericOption> {
    for candidate in candidates {
        let entry = find_option(priv_data, candidate);
        if entry.is_null() {
            continue;
        }
        // SAFETY: entry points into the static option table; min and max are plain numbers.
        let (min, max) = unsafe { ((*entry).min, (*entry).max) };
        // The saturating float to integer casts are the intended clamp.
        return Some(NumericOption::from_reported(&candidate.to_string_lossy(), min as i64, max as i64, floor));
    }
    None
}

/// Lists the named constants that belong to one option's unit.
#[allow(unsafe_code)]
fn named_values(priv_data: *mut c_void, option: &CStr) -> Vec<String> {
    let entry = find_option(priv_data, option);
    if entry.is_null() {
        return Vec::new();
    }
    // SAFETY: entry points into the static option table; unit is null or a static string.
    let unit = unsafe { (*entry).unit };
    if unit.is_null() {
        return Vec::new();
    }
    let mut values = Vec::new();
    let mut prev: *const ffi::AVOption = std::ptr::null();
    loop {
        // SAFETY: priv_data starts with its AVClass pointer, which is what
        // av_opt_next reads; the table is static.
        let next = unsafe { ffi::av_opt_next(priv_data.cast_const(), prev) };
        if next.is_null() {
            break;
        }
        prev = next;
        // SAFETY: next points into the static option table; every field read is
        // a static string or a plain integer.
        unsafe {
            let same_unit = !(*next).unit.is_null() && CStr::from_ptr((*next).unit) == CStr::from_ptr(unit);
            if (*next).type_ == ffi::AVOptionType::AV_OPT_TYPE_CONST && same_unit {
                values.push(CStr::from_ptr((*next).name).to_string_lossy().into_owned());
            }
        }
    }
    values
}

/// Reads everything amf_options needs from the encoder's private option table.
fn read_option_ranges(priv_data: *mut c_void) -> OptionRanges {
    let preset_name = [c"quality", c"preset"].into_iter().find(|name| !find_option(priv_data, name).is_null());
    OptionRanges {
        quality_level: numeric_option(priv_data, &[c"qvbr_quality_level"], 1),
        qp_i: numeric_option(priv_data, &[c"qp_i", c"qp_I"], 0),
        qp_p: numeric_option(priv_data, &[c"qp_p", c"qp_P"], 0),
        rc_modes: named_values(priv_data, c"rc"),
        preset_option: preset_name.map(|name| name.to_string_lossy().into_owned()),
        presets: preset_name.map(|name| named_values(priv_data, name)).unwrap_or_default(),
        has_filler_data: !find_option(priv_data, c"filler_data").is_null(),
        has_enforce_hrd: !find_option(priv_data, c"enforce_hrd").is_null(),
    }
}

/// FFmpeg's AVERROR_OPTION_NOT_FOUND, which the bindings do not export.
const AVERROR_OPTION_NOT_FOUND: c_int = -1_414_549_496;

/// The generic AVCodecContext options amf_options emits; every other option is private to the encoder.
const CONTEXT_OPTIONS: [&str; 4] = ["b", "maxrate", "bufsize", "g"];

/// Where one option is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionTarget {
    /// The generic AVCodecContext.
    Context,
    /// The encoder's private data (priv_data).
    Private,
}

/// Which object an option belongs to.
pub fn option_target(name: &str) -> OptionTarget {
    if CONTEXT_OPTIONS.contains(&name) { OptionTarget::Context } else { OptionTarget::Private }
}

/// A list of option names and values.
pub type OptionList = Vec<(String, String)>;

/// Splits a mapped option list into the context options and the private ones, keeping order.
pub fn split_options(options: &[(String, String)]) -> (OptionList, OptionList) {
    options.iter().cloned().partition(|(name, _)| option_target(name) == OptionTarget::Context)
}

/// The error for one option FFmpeg refused: a missing option or a rejected value.
fn option_error(encoder: &str, name: &str, value: &str, code: c_int) -> ConfigError {
    if code == AVERROR_OPTION_NOT_FOUND {
        missing(encoder, name)
    } else {
        out_of_range(encoder, name, value, &format!("a value FFmpeg accepts (it said: {})", ff_error(code)))
    }
}

/// Sets each option on the object it belongs to: generic ones on the codec context,
/// the rest on the encoder's private data. Any option FFmpeg does not take is an error naming it.
#[allow(unsafe_code)]
fn apply_options(context: *mut ffi::AVCodecContext, priv_data: *mut c_void, encoder_name: &str, options: &[(String, String)]) -> Result<(), EncoderError> {
    if priv_data.is_null() {
        return Err(MediaError::Null("priv_data").into());
    }
    let (generic, private) = split_options(options);
    let targets = generic.iter().map(|option| (context.cast::<c_void>(), option)).chain(private.iter().map(|option| (priv_data, option)));
    for (target, (name, value)) in targets {
        let (Ok(cname), Ok(cvalue)) = (CString::new(name.as_str()), CString::new(value.as_str())) else {
            return Err(MediaError::Null("option string").into());
        };
        // SAFETY: the context and its private data are live and exclusively owned by
        // the encoder being built; both strings outlive the call and av_opt_set copies
        // the value. Flag 0 searches only the named object, so nothing can land elsewhere.
        let code = unsafe { ffi::av_opt_set(target, cname.as_ptr(), cvalue.as_ptr(), 0) };
        if code < 0 {
            return Err(option_error(encoder_name, name, value, code).into());
        }
        log::debug!("AMF {encoder_name}: {name} = {value} ({:?})", option_target(name));
    }
    Ok(())
}

/// Reads every planned option back from the opened encoder, logs it, and warns when it differs.
/// Returns (name, value) pairs; named values read as "name (number)".
#[allow(unsafe_code)]
fn read_back_options(context: *mut ffi::AVCodecContext, encoder_name: &str, options: &[(String, String)]) -> Result<Vec<(String, String)>, EncoderError> {
    // SAFETY: the context is live; priv_data stays allocated while it is.
    let priv_data = unsafe { (*context).priv_data };
    let canon = |text: &str| match text {
        "true" => "1".to_string(),
        "false" => "0".to_string(),
        other => other.to_string(),
    };
    let mut read = Vec::with_capacity(options.len());
    for (name, planned) in options {
        let Ok(cname) = CString::new(name.as_str()) else {
            return Err(MediaError::Null("option string").into());
        };
        let mut raw: *mut u8 = std::ptr::null_mut();
        // SAFETY: the context is live, the name outlives the call and raw is a live
        // out pointer; on success FFmpeg hands over an allocated string.
        let code = unsafe { ffi::av_opt_get(context.cast::<c_void>(), cname.as_ptr(), SEARCH_CHILDREN, &mut raw) };
        if code < 0 || raw.is_null() {
            return Err(option_error(encoder_name, name, planned, if code < 0 { code } else { AVERROR_OPTION_NOT_FOUND }).into());
        }
        // SAFETY: raw is a NUL terminated string allocated by av_opt_get; it is freed once, after the copy.
        let text = unsafe {
            let text = CStr::from_ptr(raw.cast()).to_string_lossy().into_owned();
            ffi::av_free(raw.cast::<c_void>());
            text
        };
        let shown = value_label(priv_data, name, &text);
        if canon(&text) == *planned || shown.starts_with(&format!("{planned} (")) {
            log::info!("AMF {encoder_name}: {name} = {shown}");
            read.push((name.clone(), shown));
        } else {
            log::warn!("AMF {encoder_name}: {name} planned {planned} but the encoder reports {shown}");
            read.push((name.clone(), format!("{shown}  <-- MISMATCH, planned {planned}")));
        }
    }
    Ok(read)
}

/// Adds the name of a named constant to a numeric read-back value, when the option has one.
#[allow(unsafe_code)]
fn value_label(priv_data: *mut c_void, name: &str, raw: &str) -> String {
    let (Ok(cname), Ok(number)) = (CString::new(name), raw.parse::<i64>()) else {
        return raw.to_string();
    };
    if priv_data.is_null() {
        return raw.to_string();
    }
    let entry = find_option(priv_data, &cname);
    // SAFETY: a non-null entry points into the encoder's static option table.
    let unit = if entry.is_null() { std::ptr::null() } else { unsafe { (*entry).unit } };
    if unit.is_null() {
        return raw.to_string();
    }
    let mut prev: *const ffi::AVOption = std::ptr::null();
    loop {
        // SAFETY: priv_data starts with its AVClass pointer, which is what av_opt_next reads; the table is static.
        let next = unsafe { ffi::av_opt_next(priv_data.cast_const(), prev) };
        if next.is_null() {
            return raw.to_string();
        }
        prev = next;
        // SAFETY: next points into the static option table; every field read is a
        // static string or a plain integer.
        unsafe {
            let same_unit = !(*next).unit.is_null() && CStr::from_ptr((*next).unit) == CStr::from_ptr(unit);
            if (*next).type_ == ffi::AVOptionType::AV_OPT_TYPE_CONST && same_unit && (*next).default_val.i64_ == number {
                return format!("{} ({raw})", CStr::from_ptr((*next).name).to_string_lossy());
            }
        }
    }
}

/// Maps the vendor-neutral configuration onto the option names and values of one AMF encoder.
///
/// Pure: the ranges come from the encoder's option table, so no GPU is needed.
/// Generic codec options (b, maxrate, bufsize, g) are included alongside the private ones.
pub fn amf_options(codec: Codec, cfg: &EncoderConfig, ranges: &OptionRanges) -> Result<Vec<(String, String)>, ConfigError> {
    cfg.validate()?;
    let encoder = codec_name(codec);
    let mut options: Vec<(String, String)> = Vec::new();
    match cfg.rate_control {
        RateControl::ConstantQuality { quality, max_mbps } => {
            push(&mut options, "rc", rc_mode(encoder, ranges, &["qvbr"])?);
            let level = ranges.quality_level.as_ref().ok_or_else(|| missing(encoder, "qvbr_quality_level"))?;
            push(&mut options, &level.name, map_quality(encoder, level, quality)?.to_string());
            // QVBR on AMF takes its target bitrate from bit_rate, so the ceiling doubles as the target.
            let ceiling = bits_per_second(max_mbps);
            push_rates(&mut options, ceiling, ceiling, ceiling);
        }
        RateControl::ConstantQp { qp_i, qp_p } => {
            push(&mut options, "rc", rc_mode(encoder, ranges, &["cqp"])?);
            let option_i = ranges.qp_i.as_ref().ok_or_else(|| missing(encoder, "qp_i"))?;
            let option_p = ranges.qp_p.as_ref().ok_or_else(|| missing(encoder, "qp_p"))?;
            push(&mut options, &option_i.name, check_qp(encoder, option_i, qp_i)?.to_string());
            push(&mut options, &option_p.name, check_qp(encoder, option_p, qp_p)?.to_string());
        }
        RateControl::Cbr { mbps } => {
            push(&mut options, "rc", rc_mode(encoder, ranges, &["hqcbr", "cbr"])?);
            let rate = bits_per_second(mbps);
            push_rates(&mut options, rate, rate, rate);
            // Filler data pads the stream so the bitrate really is constant.
            if ranges.has_filler_data {
                push(&mut options, "filler_data", "1".to_string());
            }
            push_enforce_hrd(&mut options, ranges);
        }
        RateControl::VbrPeak { target_mbps, max_mbps } => {
            push(&mut options, "rc", rc_mode(encoder, ranges, &["hqvbr", "vbr_peak"])?);
            let ceiling = bits_per_second(max_mbps);
            push_rates(&mut options, bits_per_second(target_mbps), ceiling, ceiling);
            push_enforce_hrd(&mut options, ranges);
        }
    }
    let preset_option = ranges.preset_option.as_deref().ok_or_else(|| missing(encoder, "quality"))?;
    let preset_value = match cfg.preset {
        Preset::Speed => "speed",
        Preset::Balanced => "balanced",
        Preset::Quality => "quality",
    };
    if !ranges.presets.iter().any(|value| value == preset_value) {
        return Err(out_of_range(encoder, preset_option, preset_value, &format!("one of {}", ranges.presets.join(", "))));
    }
    push(&mut options, preset_option, preset_value.to_string());
    push(&mut options, "g", gop_frames(cfg).to_string());
    Ok(options)
}

/// Appends one name and value pair.
fn push(options: &mut Vec<(String, String)>, name: &str, value: String) {
    options.push((name.to_string(), value));
}

/// Appends the bit_rate, maxrate and bufsize options, all in bits.
fn push_rates(options: &mut Vec<(String, String)>, bit_rate: i64, max_rate: i64, buffer_bits: i64) {
    push(options, "b", bit_rate.to_string());
    push(options, "maxrate", max_rate.to_string());
    push(options, "bufsize", buffer_bits.to_string());
}

/// Turns on enforce_hrd, when the encoder has it, so maxrate and bufsize are binding.
fn push_enforce_hrd(options: &mut Vec<(String, String)>, ranges: &OptionRanges) {
    if ranges.has_enforce_hrd {
        push(options, "enforce_hrd", "1".to_string());
    }
}

/// Megabits per second as whole bits per second.
fn bits_per_second(mbps: f32) -> i64 {
    // validate() has bounded the value to 500 Mbps, so the cast cannot overflow.
    (f64::from(mbps) * 1_000_000.0).round() as i64
}

/// Picks the first wanted rate control mode the encoder exposes.
fn rc_mode(encoder: &str, ranges: &OptionRanges, wanted: &[&str]) -> Result<String, ConfigError> {
    if ranges.rc_modes.is_empty() {
        return Err(missing(encoder, "rc"));
    }
    wanted
        .iter()
        .find(|mode| ranges.rc_modes.iter().any(|offered| offered == **mode))
        .map(|mode| (*mode).to_string())
        .ok_or_else(|| out_of_range(encoder, "rc", &wanted.join(" or "), &format!("one of {}", ranges.rc_modes.join(", "))))
}

/// Maps the 1 to 100 quality (100 is best) linearly onto the encoder's quality level
/// range. AMF's QVBR level is higher for a better picture, so quality 1 is the lowest
/// usable level and 100 the highest. A sentinel minimum below 1 is raised to 1.
fn map_quality(encoder: &str, option: &NumericOption, quality: u8) -> Result<i64, ConfigError> {
    let low = option.min.max(1);
    let high = option.max;
    if high < low {
        return Err(out_of_range(encoder, &option.name, &quality.to_string(), &format!("range {} to {}", option.min, option.max)));
    }
    let steps = i64::from(quality.saturating_sub(1));
    // Rounded steps * span / 99, in integers.
    let offset = (steps * (high - low) * 2 + 99) / 198;
    Ok(low + offset)
}

/// Checks a quantiser against the range the encoder allows for it.
fn check_qp(encoder: &str, option: &NumericOption, qp: u8) -> Result<i64, ConfigError> {
    let value = i64::from(qp);
    if value < option.min.max(0) || value > option.max {
        return Err(out_of_range(encoder, &option.name, &qp.to_string(), &format!("range {} to {}", option.min.max(0), option.max)));
    }
    Ok(value)
}

/// The OptionMissing error for one option name.
fn missing(encoder: &str, option: &str) -> ConfigError {
    ConfigError::OptionMissing { encoder: encoder.to_string(), option: option.to_string() }
}

/// The OutOfRange error for one option.
fn out_of_range(encoder: &str, option: &str, value: &str, allowed: &str) -> ConfigError {
    ConfigError::OutOfRange { encoder: encoder.to_string(), option: option.to_string(), value: value.to_string(), allowed: allowed.to_string() }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    const ALL_CODECS: [Codec; 3] = [Codec::Av1, Codec::Hevc, Codec::H264];

    /// Fake option ranges shaped like each encoder's real table.
    fn fake_ranges(codec: Codec) -> OptionRanges {
        let qp_max = if codec == Codec::Av1 { 255 } else { 51 };
        OptionRanges {
            quality_level: Some(NumericOption::from_reported("qvbr_quality_level", -1, 51, 1)),
            qp_i: Some(NumericOption { name: "qp_i".into(), min: 0, max: qp_max }),
            qp_p: Some(NumericOption { name: "qp_p".into(), min: 0, max: qp_max }),
            rc_modes: ["cqp", "cbr", "vbr_peak", "vbr_latency", "qvbr", "hqvbr", "hqcbr"].map(String::from).to_vec(),
            preset_option: Some("quality".into()),
            presets: ["speed", "balanced", "quality"].map(String::from).to_vec(),
            has_filler_data: true,
            has_enforce_hrd: true,
        }
    }

    fn config(codec: Codec, rate_control: RateControl) -> EncoderConfig {
        EncoderConfig { codec, rate_control, ..EncoderConfig::default() }
    }

    fn value<'a>(options: &'a [(String, String)], name: &str) -> &'a str {
        options.iter().find(|(key, _)| key == name).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("option {name} not set in {options:?}"))
    }

    #[test]
    fn constant_quality_on_every_codec() {
        for codec in ALL_CODECS {
            let cfg = config(codec, RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 });
            let options = amf_options(codec, &cfg, &fake_ranges(codec)).unwrap();
            assert_eq!(value(&options, "rc"), "qvbr");
            assert_eq!(value(&options, "b"), "40000000");
            assert_eq!(value(&options, "maxrate"), "40000000");
            assert_eq!(value(&options, "bufsize"), "40000000");
            assert_eq!(value(&options, "quality"), "balanced");
            assert_eq!(value(&options, "g"), "120");
            assert!(options.iter().any(|(key, _)| key == "qvbr_quality_level"));
        }
    }

    #[test]
    fn quality_maps_linearly_onto_the_level_range_with_higher_meaning_better() {
        let expected = [(1u8, "1"), (50, "26"), (100, "51")];
        for (quality, level) in expected {
            let cfg = config(Codec::Av1, RateControl::ConstantQuality { quality, max_mbps: 40.0 });
            let options = amf_options(Codec::Av1, &cfg, &fake_ranges(Codec::Av1)).unwrap();
            assert_eq!(value(&options, "qvbr_quality_level"), level, "quality {quality}");
        }
    }

    #[test]
    fn a_reported_minimum_of_minus_one_is_unset_and_raised_to_the_floor() {
        let level = NumericOption::from_reported("qvbr_quality_level", -1, 51, 1);
        assert_eq!((level.min, level.max), (1, 51));
        let qp = NumericOption::from_reported("qp_i", -1, 255, 0);
        assert_eq!((qp.min, qp.max), (0, 255));
        // A real minimum above the floor is kept, and QP ranges stay as read.
        let kept = NumericOption::from_reported("qp_i", 0, 51, 0);
        assert_eq!((kept.min, kept.max), (0, 51));
        // Even a raw -1 handed straight to the mapper still maps onto 1..max.
        let raw = NumericOption { name: "qvbr_quality_level".into(), min: -1, max: 51 };
        assert_eq!(map_quality("av1_amf", &raw, 1).unwrap(), 1);
        assert_eq!(map_quality("av1_amf", &raw, 100).unwrap(), 51);
    }

    #[test]
    fn cbr_enables_filler_data_only_when_the_encoder_has_it() {
        let cfg = config(Codec::Hevc, RateControl::Cbr { mbps: 20.0 });
        let options = amf_options(Codec::Hevc, &cfg, &fake_ranges(Codec::Hevc)).unwrap();
        assert_eq!(value(&options, "filler_data"), "1");
        let mut ranges = fake_ranges(Codec::Hevc);
        ranges.has_filler_data = false;
        let options = amf_options(Codec::Hevc, &cfg, &ranges).unwrap();
        assert!(!options.iter().any(|(key, _)| key == "filler_data"));
        // Other modes never set it.
        let vbr = config(Codec::Hevc, RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 });
        let options = amf_options(Codec::Hevc, &vbr, &fake_ranges(Codec::Hevc)).unwrap();
        assert!(!options.iter().any(|(key, _)| key == "filler_data"));
    }

    #[test]
    fn constant_qp_uses_each_codecs_own_range_and_names() {
        for codec in ALL_CODECS {
            let mut ranges = fake_ranges(codec);
            ranges.qp_i = ranges.qp_i.map(|option| NumericOption { name: "qp_I".into(), ..option });
            let cfg = config(codec, RateControl::ConstantQp { qp_i: 24, qp_p: 26 });
            let options = amf_options(codec, &cfg, &ranges).unwrap();
            assert_eq!(value(&options, "rc"), "cqp");
            assert_eq!(value(&options, "qp_I"), "24");
            assert_eq!(value(&options, "qp_p"), "26");
        }
    }

    #[test]
    fn qp_outside_the_encoder_range_is_an_error_naming_option_and_range() {
        let cfg = config(Codec::H264, RateControl::ConstantQp { qp_i: 52, qp_p: 26 });
        let err = amf_options(Codec::H264, &cfg, &fake_ranges(Codec::H264)).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("qp_i") && text.contains("52") && text.contains("0 to 51"), "{text}");
        // The same value is fine on AV1, whose range is wider.
        let cfg = config(Codec::Av1, RateControl::ConstantQp { qp_i: 52, qp_p: 26 });
        assert!(amf_options(Codec::Av1, &cfg, &fake_ranges(Codec::Av1)).is_ok());
    }

    #[test]
    fn cbr_prefers_hq_and_falls_back_to_plain_cbr() {
        for codec in ALL_CODECS {
            let cfg = config(codec, RateControl::Cbr { mbps: 20.0 });
            let options = amf_options(codec, &cfg, &fake_ranges(codec)).unwrap();
            assert_eq!(value(&options, "rc"), "hqcbr");
            assert_eq!(value(&options, "b"), "20000000");
            assert_eq!(value(&options, "maxrate"), "20000000");
            assert_eq!(value(&options, "bufsize"), "20000000");
            let mut ranges = fake_ranges(codec);
            ranges.rc_modes.retain(|mode| mode != "hqcbr");
            let options = amf_options(codec, &cfg, &ranges).unwrap();
            assert_eq!(value(&options, "rc"), "cbr");
        }
    }

    #[test]
    fn vbr_peak_prefers_hq_and_falls_back_to_peak_constrained() {
        for codec in ALL_CODECS {
            let cfg = config(codec, RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 });
            let options = amf_options(codec, &cfg, &fake_ranges(codec)).unwrap();
            assert_eq!(value(&options, "rc"), "hqvbr");
            assert_eq!(value(&options, "b"), "20000000");
            assert_eq!(value(&options, "maxrate"), "40000000");
            assert_eq!(value(&options, "bufsize"), "40000000");
            let mut ranges = fake_ranges(codec);
            ranges.rc_modes.retain(|mode| mode != "hqvbr");
            let options = amf_options(codec, &cfg, &ranges).unwrap();
            assert_eq!(value(&options, "rc"), "vbr_peak");
        }
    }

    /// The names in an option list, in order.
    fn names(options: &[(String, String)]) -> Vec<&str> {
        options.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn options_split_between_the_context_and_private_data_for_every_mode() {
        let modes = [
            (RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 }, vec!["b", "maxrate", "bufsize", "g"], vec!["rc", "qvbr_quality_level", "quality"]),
            (RateControl::ConstantQp { qp_i: 24, qp_p: 26 }, vec!["g"], vec!["rc", "qp_i", "qp_p", "quality"]),
            (RateControl::Cbr { mbps: 20.0 }, vec!["b", "maxrate", "bufsize", "g"], vec!["rc", "filler_data", "enforce_hrd", "quality"]),
            (RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 }, vec!["b", "maxrate", "bufsize", "g"], vec!["rc", "enforce_hrd", "quality"]),
        ];
        for codec in ALL_CODECS {
            for (rate_control, context_names, private_names) in &modes {
                let options = amf_options(codec, &config(codec, *rate_control), &fake_ranges(codec)).unwrap();
                let (context, private) = split_options(&options);
                let mut got_context = names(&context);
                let mut got_private = names(&private);
                let mut want_context = context_names.clone();
                let mut want_private = private_names.clone();
                got_context.sort_unstable();
                got_private.sort_unstable();
                want_context.sort_unstable();
                want_private.sort_unstable();
                assert_eq!(got_context, want_context, "{codec:?} {rate_control:?} context");
                assert_eq!(got_private, want_private, "{codec:?} {rate_control:?} private");
                assert_eq!(context.len() + private.len(), options.len());
            }
        }
    }

    #[test]
    fn enforce_hrd_is_set_for_cbr_and_vbr_only_when_the_encoder_has_it() {
        let cbr = config(Codec::Hevc, RateControl::Cbr { mbps: 20.0 });
        let vbr = config(Codec::Hevc, RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 });
        for cfg in [&cbr, &vbr] {
            let options = amf_options(Codec::Hevc, cfg, &fake_ranges(Codec::Hevc)).unwrap();
            assert_eq!(value(&options, "enforce_hrd"), "1");
            let mut ranges = fake_ranges(Codec::Hevc);
            ranges.has_enforce_hrd = false;
            let options = amf_options(Codec::Hevc, cfg, &ranges).unwrap();
            assert!(!options.iter().any(|(key, _)| key == "enforce_hrd"));
        }
        let quality = config(Codec::Hevc, RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 });
        let options = amf_options(Codec::Hevc, &quality, &fake_ranges(Codec::Hevc)).unwrap();
        assert!(!options.iter().any(|(key, _)| key == "enforce_hrd"));
    }

    #[test]
    fn presets_map_onto_the_preset_option() {
        for (preset, name) in [(Preset::Speed, "speed"), (Preset::Balanced, "balanced"), (Preset::Quality, "quality")] {
            let cfg = EncoderConfig { preset, ..EncoderConfig::default() };
            let options = amf_options(Codec::Av1, &cfg, &fake_ranges(Codec::Av1)).unwrap();
            assert_eq!(value(&options, "quality"), name);
        }
        let mut ranges = fake_ranges(Codec::Av1);
        ranges.preset_option = Some("preset".into());
        let options = amf_options(Codec::Av1, &EncoderConfig::default(), &ranges).unwrap();
        assert_eq!(value(&options, "preset"), "balanced");
    }

    #[test]
    fn missing_options_and_values_are_errors_not_silent() {
        let cfg = EncoderConfig::default();

        let mut ranges = fake_ranges(Codec::Av1);
        ranges.quality_level = None;
        let err = amf_options(Codec::Av1, &cfg, &ranges).unwrap_err();
        assert!(matches!(&err, ConfigError::OptionMissing { option, .. } if option == "qvbr_quality_level"), "{err}");

        let mut ranges = fake_ranges(Codec::Av1);
        ranges.preset_option = None;
        assert!(matches!(amf_options(Codec::Av1, &cfg, &ranges), Err(ConfigError::OptionMissing { .. })));

        let mut ranges = fake_ranges(Codec::Av1);
        ranges.presets.retain(|preset| preset != "quality");
        let quality = EncoderConfig { preset: Preset::Quality, ..EncoderConfig::default() };
        let err = amf_options(Codec::Av1, &quality, &ranges).unwrap_err();
        assert!(err.to_string().contains("quality") && err.to_string().contains("speed, balanced"), "{err}");

        let mut ranges = fake_ranges(Codec::Av1);
        ranges.rc_modes.retain(|mode| mode != "qvbr");
        let err = amf_options(Codec::Av1, &cfg, &ranges).unwrap_err();
        assert!(err.to_string().contains("rc") && err.to_string().contains("qvbr"), "{err}");

        let mut ranges = fake_ranges(Codec::Av1);
        ranges.rc_modes.clear();
        assert!(matches!(amf_options(Codec::Av1, &cfg, &ranges), Err(ConfigError::OptionMissing { .. })));

        let mut ranges = fake_ranges(Codec::Hevc);
        ranges.qp_p = None;
        let cqp = config(Codec::Hevc, RateControl::ConstantQp { qp_i: 20, qp_p: 20 });
        assert!(matches!(amf_options(Codec::Hevc, &cqp, &ranges), Err(ConfigError::OptionMissing { option, .. }) if option == "qp_p"));
    }

    #[test]
    fn gop_is_keyframe_seconds_times_fps_rounded() {
        let cases = [(2.0, 60, 1, 120), (0.5, 30_000, 1001, 15), (1.5, 25, 1, 38), (2.5, 24, 1, 60), (10.0, 144, 1, 1440)];
        for (secs, num, den, gop) in cases {
            let cfg = EncoderConfig { keyframe_secs: secs, fps_num: num, fps_den: den, ..EncoderConfig::default() };
            let options = amf_options(Codec::Hevc, &cfg, &fake_ranges(Codec::Hevc)).unwrap();
            assert_eq!(value(&options, "g"), gop.to_string(), "{secs} s at {num}/{den}");
        }
    }

    #[test]
    fn an_invalid_config_is_rejected_before_mapping() {
        let cfg = config(Codec::Av1, RateControl::Cbr { mbps: 0.0 });
        assert!(matches!(amf_options(Codec::Av1, &cfg, &fake_ranges(Codec::Av1)), Err(ConfigError::Invalid { .. })));
    }
}
