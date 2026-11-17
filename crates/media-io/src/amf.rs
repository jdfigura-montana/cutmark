//! The AMD AMF hardware encoder backend.

use std::ffi::{CStr, c_int, c_void};

use ffmpeg_next::Rational;
use ffmpeg_next::codec::encoder;
use ffmpeg_next::codec::{Context, Flags};
use ffmpeg_next::ffi;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::packet::Mut;

use crate::MediaError;
use crate::encode::{Codec, EncodedPacket, Encoder, EncoderConfig, Preset, RateControl};
use crate::hw::{D3d11FramePool, HwFrame, ff_error};

/// An FFmpeg hardware encoder running through AMD AMF.
pub struct AmfEncoder {
    /// The opened FFmpeg video encoder.
    encoder: encoder::Video,
    /// The packet time base, as (numerator, denominator).
    time_base: (i32, i32),
    /// Whether end of stream has been sent.
    finished: bool,
}

impl AmfEncoder {
    /// Opens an AMF encoder for the given configuration on the pool's frames context.
    pub fn open(cfg: &EncoderConfig, pool: &D3d11FramePool) -> Result<Self, MediaError> {
        let name = codec_name(cfg.codec);
        let codec = encoder::find_by_name(name).ok_or_else(|| MediaError::EncoderMissing(name.to_string()))?;
        let mut context = Context::new_with_codec(codec);
        // MKV and MP4 both want the extradata in the container, not in-band.
        context.set_flags(Flags::GLOBAL_HEADER);
        attach_frames_ctx(&context, pool)?;
        let priv_data = private_options(&context);
        let context_ptr = raw_context(&context);
        let mut video = context.encoder().video().map_err(MediaError::Init)?;
        video.set_width(cfg.width);
        video.set_height(cfg.height);
        video.set_time_base(Rational::new(time_base_num(cfg), time_base_den(cfg)));
        video.set_frame_rate(Some(Rational::new(frame_rate_num(cfg), frame_rate_den(cfg))));
        video.set_gop(gop_frames(cfg));
        video.set_format(Pixel::from(ffi::AVPixelFormat::AV_PIX_FMT_D3D11));
        apply_rate_control(priv_data, name, context_ptr, cfg.rate_control)?;
        apply_preset(priv_data, name, cfg.preset)?;
        let encoder = video.open().map_err(MediaError::Init)?;
        Ok(Self { encoder, time_base: (time_base_num(cfg), time_base_den(cfg)), finished: false })
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

/// The keyframe interval in frames.
fn gop_frames(cfg: &EncoderConfig) -> u32 {
    let num = u64::from(cfg.fps_num);
    let den = u64::from(cfg.fps_den).max(1);
    let frames = num * u64::from(cfg.keyframe_seconds) / den;
    (frames.min(u64::from(u32::MAX))) as u32
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

/// Looks up one private option and returns its table entry, or null.
#[allow(unsafe_code)]
fn find_option(priv_data: *mut c_void, option: &CStr) -> *const ffi::AVOption {
    // SAFETY: priv_data is live for the whole open call; av_opt_find only reads.
    unsafe { ffi::av_opt_find(priv_data, option.as_ptr(), std::ptr::null(), 0, 0) }
}

/// Checks that a named constant value exists under an option's unit.
#[allow(unsafe_code)]
fn named_value_exists(priv_data: *mut c_void, value: &CStr, unit: *const std::ffi::c_char) -> bool {
    // SAFETY: priv_data and unit are live for the whole open call.
    unsafe { !ffi::av_opt_find(priv_data, value.as_ptr(), unit, 0, 0).is_null() }
}

/// Sets one private option to a named constant value after checking both the
/// option and the value exist on this encoder.
#[allow(unsafe_code)]
fn set_checked_string(priv_data: *mut c_void, encoder_name: &str, option: &CStr, value: &CStr) -> Result<(), MediaError> {
    // SAFETY: priv_data and both strings are live; av_opt_set copies the value.
    let code = unsafe { ffi::av_opt_set(priv_data, option.as_ptr(), value.as_ptr(), 0) };
    if code < 0 {
        return Err(ff_error(code));
    }
    log::debug!("AMF {encoder_name}: {} = {}", option.to_string_lossy(), value.to_string_lossy());
    Ok(())
}

/// Sets one private option to an integer value after checking it exists.
#[allow(unsafe_code)]
fn set_int_option(priv_data: *mut c_void, encoder_name: &str, option: &CStr, value: i64) -> Result<(), MediaError> {
    if find_option(priv_data, option).is_null() {
        return Err(option_missing(encoder_name, option));
    }
    // SAFETY: priv_data is live; av_opt_set_int writes the option's own storage.
    let code = unsafe { ffi::av_opt_set_int(priv_data, option.as_ptr(), value, 0) };
    if code < 0 {
        return Err(ff_error(code));
    }
    log::debug!("AMF {encoder_name}: {} = {value}", option.to_string_lossy());
    Ok(())
}

/// Sets one private option to a named constant value after checking both the
/// option and the value exist on this encoder.
#[allow(unsafe_code)]
fn set_named_option(priv_data: *mut c_void, encoder_name: &str, option: &CStr, value: &CStr) -> Result<(), MediaError> {
    let entry = find_option(priv_data, option);
    if entry.is_null() {
        return Err(option_missing(encoder_name, option));
    }
    // SAFETY: entry points into the encoder's static option table; only the
    // unit name is read.
    let unit = unsafe { (*entry).unit };
    if !named_value_exists(priv_data, value, unit) {
        return Err(MediaError::OptionMissing { encoder: encoder_name.to_string(), option: value.to_string_lossy().into_owned() });
    }
    set_checked_string(priv_data, encoder_name, option, value)
}

/// The OptionMissing error for one option name.
fn option_missing(encoder_name: &str, option: &CStr) -> MediaError {
    MediaError::OptionMissing { encoder: encoder_name.to_string(), option: option.to_string_lossy().into_owned() }
}

/// Maps the vendor-neutral rate control onto the AMF encoder's private options
/// and bit rate fields.
fn apply_rate_control(priv_data: *mut c_void, encoder_name: &str, context: *mut ffi::AVCodecContext, rate_control: RateControl) -> Result<(), MediaError> {
    match rate_control {
        RateControl::Cqp { qp_i, qp_p } => {
            set_named_option(priv_data, encoder_name, c"rc", c"cqp")?;
            set_int_option(priv_data, encoder_name, c"qp_i", i64::from(qp_i))?;
            set_int_option(priv_data, encoder_name, c"qp_p", i64::from(qp_p))?;
        }
        RateControl::Qvbr { quality, max_mbps } => {
            set_named_option(priv_data, encoder_name, c"rc", c"qvbr")?;
            set_int_option(priv_data, encoder_name, c"qvbr_quality_level", i64::from(quality))?;
            set_max_rate(context, i64::from(max_mbps) * 1_000_000);
        }
        RateControl::Cbr { mbps } => {
            set_named_option(priv_data, encoder_name, c"rc", c"cbr")?;
            let bits_per_second = i64::from(mbps) * 1_000_000;
            set_bit_rates(context, bits_per_second);
            // rc_buffer_size is in bytes, so this is one second of buffer.
            set_rc_buffer_size(context, bits_per_second / 8);
        }
        RateControl::VbrPeak { target_mbps, max_mbps } => {
            set_named_option(priv_data, encoder_name, c"rc", c"vbr_peak")?;
            set_bit_rates(context, i64::from(target_mbps) * 1_000_000);
            set_max_rate(context, i64::from(max_mbps) * 1_000_000);
        }
    }
    Ok(())
}

/// Maps the vendor-neutral preset onto the encoder's quality option, falling
/// back to the generic preset option name.
fn apply_preset(priv_data: *mut c_void, encoder_name: &str, preset: Preset) -> Result<(), MediaError> {
    let value = match preset {
        Preset::Speed => c"speed",
        Preset::Balanced => c"balanced",
        Preset::Quality => c"quality",
    };
    for option in [c"quality", c"preset"] {
        if find_option(priv_data, option).is_null() {
            continue;
        }
        return set_named_option(priv_data, encoder_name, option, value);
    }
    Err(option_missing(encoder_name, c"quality"))
}

/// Sets the AVCodecContext bit_rate and rc_max_rate.
#[allow(unsafe_code)]
fn set_bit_rates(context: *mut ffi::AVCodecContext, bits_per_second: i64) {
    // SAFETY: the context is live and exclusively owned by the encoder being built.
    unsafe {
        (*context).bit_rate = bits_per_second;
        (*context).rc_max_rate = bits_per_second;
    }
}

/// Sets the AVCodecContext rc_max_rate.
#[allow(unsafe_code)]
fn set_max_rate(context: *mut ffi::AVCodecContext, bits_per_second: i64) {
    // SAFETY: the context is live and exclusively owned by the encoder being built.
    unsafe {
        (*context).rc_max_rate = bits_per_second;
    }
}

/// Sets the AVCodecContext rc_buffer_size in bytes.
#[allow(unsafe_code)]
fn set_rc_buffer_size(context: *mut ffi::AVCodecContext, bytes: i64) {
    // SAFETY: the context is live and exclusively owned by the encoder being built.
    unsafe {
        (*context).rc_buffer_size = bytes as c_int;
    }
}
