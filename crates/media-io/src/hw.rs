//! Hands the suite's D3D11 device to FFmpeg and manages a pool of D3D11 frames.
//!
//! All FFmpeg pointer work goes through ffmpeg_next::ffi, with unsafe confined
//! to small private functions.

use std::ffi::{c_int, c_void};

use ffmpeg_next::ffi;
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_RENDER_TARGET, D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::core::Interface;

/// The AVHWDeviceContext wrapping our shared D3D11 device.
///
/// FFmpeg releases the device interface when the context is unreferenced, so
/// the constructor hands it an added reference. The lock callbacks are left
/// unset so FFmpeg installs its default internal mutex on init.
pub struct D3d11Device {
    device_ref: *mut ffi::AVBufferRef,
}

impl D3d11Device {
    /// Wraps the given D3D11 device in an FFmpeg D3D11VA hardware device.
    pub fn from_device(device: &ID3D11Device) -> Result<Self, MediaError> {
        alloc_device(device)
    }
}

impl Drop for D3d11Device {
    fn drop(&mut self) {
        unref_buffer(&mut self.device_ref);
    }
}

/// Which pool layout the D3D11 frame pool ended up using.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolMode {
    /// One array texture of initial_pool_size slices bound as render target.
    ArrayRenderTarget,
    /// One array texture of initial_pool_size slices with default bindings.
    ArrayDefault,
    /// One standalone NV12 texture per frame, bound as render target.
    PerFrameRenderTarget,
}

/// The NV12 D3D11 frame pool on the wrapped device.
pub struct D3d11FramePool {
    frames_ref: *mut ffi::AVBufferRef,
    mode: PoolMode,
}

impl D3d11FramePool {
    /// Creates an NV12 pool of the given size with the given initial frame count.
    pub fn new(device: &D3d11Device, width: u32, height: u32, initial_pool_size: u32) -> Result<Self, MediaError> {
        alloc_frames(device, width, height, initial_pool_size)
    }

    /// Which pool layout this pool uses.
    pub fn mode(&self) -> PoolMode {
        self.mode
    }

    /// The raw frames context reference, for handing to the encoder in P2d.
    pub fn frames_ref_ptr(&self) -> *mut ffi::AVBufferRef {
        self.frames_ref
    }
}

impl Drop for D3d11FramePool {
    fn drop(&mut self) {
        unref_buffer(&mut self.frames_ref);
    }
}

/// One hardware frame from the pool: an AVFrame whose data[0] is the
/// ID3D11Texture2D array and whose data[1] is the array index.
pub struct HwFrame {
    frame: *mut ffi::AVFrame,
}

impl HwFrame {
    /// Gets one frame from the pool.
    pub fn from_pool(pool: &D3d11FramePool) -> Result<Self, MediaError> {
        get_frame(pool)
    }

    /// Copies a full-size NV12 texture into the frame's array texture slice.
    pub fn copy_from(&mut self, context: &ID3D11DeviceContext, nv12: &ID3D11Texture2D) -> Result<(), MediaError> {
        copy_into(self, context, nv12)
    }

    /// Sets the presentation timestamp of the frame.
    pub fn set_pts(&mut self, pts: i64) {
        set_frame_pts(self.frame, pts);
    }

    /// The texture array index of this frame, from AVFrame data[1].
    pub fn array_index(&self) -> u32 {
        frame_index(self)
    }

    /// The D3D11 descriptor of the frame's array texture.
    pub fn texture_desc(&self) -> Option<D3D11_TEXTURE2D_DESC> {
        texture_desc(self)
    }

    /// The raw AVFrame, for handing to the encoder in P2d.
    pub fn frame_ptr(&self) -> *mut ffi::AVFrame {
        self.frame
    }
}

impl Drop for HwFrame {
    fn drop(&mut self) {
        free_frame(self.frame);
    }
}

/// Error type for the media-io crate.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// FFmpeg reported a failure.
    #[error("FFmpeg failed with code {code}: {message}")]
    Ffmpeg {
        /// The negative AVERROR code.
        code: i32,
        /// The av_strerror text for the code.
        message: String,
    },
    /// An FFmpeg call returned a null pointer.
    #[error("FFmpeg returned a null pointer from {0}")]
    Null(&'static str),
    /// FFmpeg initialisation failed.
    #[error("FFmpeg init failed: {0}")]
    Init(#[from] ffmpeg_next::Error),
}

/// Turns a negative AVERROR code into a MediaError with its av_strerror text.
#[allow(unsafe_code)]
fn ff_error(code: i32) -> MediaError {
    let mut buf = [0u8; 256];
    // SAFETY: av_strerror writes at most buf.len() bytes and NUL terminates.
    unsafe {
        ffi::av_strerror(code, buf.as_mut_ptr().cast::<std::ffi::c_char>(), buf.len());
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let message = String::from_utf8_lossy(&buf[..end]).into_owned();
    MediaError::Ffmpeg { code, message }
}

/// Unreferences one AVBufferRef pointer and nulls it.
#[allow(unsafe_code)]
fn unref_buffer(buffer_ref: &mut *mut ffi::AVBufferRef) {
    // SAFETY: the pointer is either null or a live reference we own.
    unsafe {
        ffi::av_buffer_unref(buffer_ref);
    }
}

/// Frees one AVFrame allocated by av_frame_alloc.
#[allow(unsafe_code)]
fn free_frame(frame: *mut ffi::AVFrame) {
    let mut frame = frame;
    // SAFETY: the frame came from av_frame_alloc and this is its last owner.
    unsafe {
        ffi::av_frame_free(&mut frame);
    }
}

/// Allocates and initialises the D3D11VA device context around our device.
#[allow(unsafe_code)]
fn alloc_device(device: &ID3D11Device) -> Result<D3d11Device, MediaError> {
    // SAFETY: av_hwdevice_ctx_alloc returns an owned reference or null.
    let mut device_ref = unsafe { ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA) };
    if device_ref.is_null() {
        return Err(MediaError::Null("av_hwdevice_ctx_alloc"));
    }
    // SAFETY: the reference is live; its data is an AVHWDeviceContext whose
    // hwctx is an AVD3D11VADeviceContext. FFmpeg releases the device interface
    // on unref, so hand it an added reference. The lock callbacks stay unset
    // so FFmpeg installs its default internal mutex on init.
    let code = unsafe {
        let hw_device_ctx = (*device_ref).data.cast::<ffi::AVHWDeviceContext>();
        let d3d11_ctx = (*hw_device_ctx).hwctx.cast::<ffi::AVD3D11VADeviceContext>();
        std::mem::forget(device.clone());
        (*d3d11_ctx).device = device.as_raw().cast::<c_void>().cast();
        ffi::av_hwdevice_ctx_init(device_ref)
    };
    if code < 0 {
        unref_buffer(&mut device_ref);
        return Err(ff_error(code));
    }
    Ok(D3d11Device { device_ref })
}

/// Allocates and initialises the NV12 D3D11 frame pool, trying the standard
/// pool layouts in turn and keeping the error of the last attempt.
#[allow(unsafe_code)]
fn alloc_frames(device: &D3d11Device, width: u32, height: u32, initial_pool_size: u32) -> Result<D3d11FramePool, MediaError> {
    let render_target = D3D11_BIND_RENDER_TARGET.0;
    let attempts: [(PoolMode, c_int, i32); 3] = [
        (PoolMode::ArrayRenderTarget, initial_pool_size as c_int, render_target),
        (PoolMode::ArrayDefault, initial_pool_size as c_int, 0),
        (PoolMode::PerFrameRenderTarget, 0, render_target),
    ];
    let mut last_err = None;
    for (mode, pool_size, bind_flags) in attempts {
        match init_frames(device, width, height, pool_size, bind_flags) {
            Ok(frames_ref) => {
                log::info!("D3D11 frame pool ready: {mode:?}, {}x{}, pool size {pool_size}", width, height);
                return Ok(D3d11FramePool { frames_ref, mode });
            }
            Err(err) => last_err = Some(err),
        }
    }
    match last_err {
        Some(err) => Err(err),
        None => Err(MediaError::Null("av_hwframe_ctx_init")),
    }
}

/// Builds one frames context candidate and initialises it.
#[allow(unsafe_code)]
fn init_frames(device: &D3d11Device, width: u32, height: u32, initial_pool_size: c_int, bind_flags: i32) -> Result<*mut ffi::AVBufferRef, MediaError> {
    // SAFETY: the device reference is live; av_hwframe_ctx_alloc returns an
    // owned reference or null.
    let mut frames_ref = unsafe { ffi::av_hwframe_ctx_alloc(device.device_ref) };
    if frames_ref.is_null() {
        return Err(MediaError::Null("av_hwframe_ctx_alloc"));
    }
    // SAFETY: the reference is live; its data is an AVHWFramesContext whose
    // fields must be set by the caller before init, and whose hwctx is an
    // AVD3D11VAFramesContext that takes the D3D11 bind flags.
    let code = unsafe {
        let frames_ctx = (*frames_ref).data.cast::<ffi::AVHWFramesContext>();
        (*frames_ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
        (*frames_ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
        (*frames_ctx).width = width as c_int;
        (*frames_ctx).height = height as c_int;
        (*frames_ctx).initial_pool_size = initial_pool_size;
        // Check what the backend will read and repair any drifted field; the
        // width and height must be the non-zero values passed in.
        if (*frames_ctx).format != ffi::AVPixelFormat::AV_PIX_FMT_D3D11 {
            (*frames_ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
        }
        if (*frames_ctx).sw_format != ffi::AVPixelFormat::AV_PIX_FMT_NV12 {
            (*frames_ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
        }
        if (*frames_ctx).width <= 0 {
            (*frames_ctx).width = width as c_int;
        }
        if (*frames_ctx).height <= 0 {
            (*frames_ctx).height = height as c_int;
        }
        log::debug!(
            "D3D11 frames ctx before init: format {:?}, sw_format {:?}, {}x{}, pool {}",
            (*frames_ctx).format,
            (*frames_ctx).sw_format,
            (*frames_ctx).width,
            (*frames_ctx).height,
            (*frames_ctx).initial_pool_size
        );
        let d3d11_frames = (*frames_ctx).hwctx.cast::<ffi::AVD3D11VAFramesContext>();
        (*d3d11_frames).BindFlags = bind_flags as u32;
        (*d3d11_frames).MiscFlags = 0;
        ffi::av_hwframe_ctx_init(frames_ref)
    };
    if code < 0 {
        unref_buffer(&mut frames_ref);
        return Err(ff_error(code));
    }
    Ok(frames_ref)
}

/// Allocates one AVFrame and takes one frame out of the pool.
#[allow(unsafe_code)]
fn get_frame(pool: &D3d11FramePool) -> Result<HwFrame, MediaError> {
    // SAFETY: av_frame_alloc returns a new frame or null.
    let frame = unsafe { ffi::av_frame_alloc() };
    if frame.is_null() {
        return Err(MediaError::Null("av_frame_alloc"));
    }
    // SAFETY: the frame is live and unshared; av_buffer_ref adds the pool
    // reference the frame then owns.
    let hw_frames_ctx = unsafe {
        (*frame).format = ffi::AVPixelFormat::AV_PIX_FMT_D3D11 as c_int;
        (*frame).hw_frames_ctx = ffi::av_buffer_ref(pool.frames_ref);
        (*frame).hw_frames_ctx
    };
    if hw_frames_ctx.is_null() {
        free_frame(frame);
        return Err(MediaError::Null("av_buffer_ref"));
    }
    // SAFETY: the frame and the pool reference are live.
    let code = unsafe { ffi::av_hwframe_get_buffer(pool.frames_ref, frame, 0) };
    if code < 0 {
        free_frame(frame);
        return Err(ff_error(code));
    }
    Ok(HwFrame { frame })
}

/// Copies a full-size NV12 texture into the frame's array texture slice.
#[allow(unsafe_code)]
fn copy_into(frame: &HwFrame, context: &ID3D11DeviceContext, nv12: &ID3D11Texture2D) -> Result<(), MediaError> {
    // SAFETY: data[0] stays valid while the AVFrame is alive; we only borrow it.
    let raw = unsafe { (*frame.frame).data[0] }.cast::<c_void>();
    let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }.ok_or(MediaError::Null("AVFrame data[0]"))?;
    let index = frame_index(frame);
    // SAFETY: both textures are live on the shared device and match the pool
    // size; an NV12 array slice is one subresource, and FFmpeg's own d3d11va
    // transfer code copies with the plain array index.
    unsafe {
        context.CopySubresourceRegion(texture, index, 0, 0, 0, nv12, 0, None);
    }
    Ok(())
}

/// Sets the presentation timestamp of one AVFrame.
#[allow(unsafe_code)]
fn set_frame_pts(frame: *mut ffi::AVFrame, pts: i64) {
    // SAFETY: the frame is live and owned by the HwFrame.
    unsafe {
        (*frame).pts = pts;
    }
}

/// Reads the texture array index of one AVFrame.
#[allow(unsafe_code)]
fn frame_index(frame: &HwFrame) -> u32 {
    // SAFETY: the frame is live; data[1] holds the array index as an integer
    // per the AV_PIX_FMT_D3D11 frame layout.
    unsafe { (*frame.frame).data[1] as usize as u32 }
}

/// Reads the D3D11 descriptor of the frame's array texture.
#[allow(unsafe_code)]
fn texture_desc(frame: &HwFrame) -> Option<D3D11_TEXTURE2D_DESC> {
    // SAFETY: data[0] stays valid while the AVFrame is alive; we only borrow it.
    let raw = unsafe { (*frame.frame).data[0] }.cast::<c_void>();
    let texture = unsafe { ID3D11Texture2D::from_raw_borrowed(&raw) }?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: texture is a live D3D11 texture; GetDesc only writes the descriptor.
    unsafe {
        texture.GetDesc(&mut desc);
    }
    Some(desc)
}
