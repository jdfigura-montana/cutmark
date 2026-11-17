//! GPU conversion from captured BGRA frames to NV12 at the output size,
//! with scaling and letterboxing on the D3D11 video processor.
//!
//! See the recorder spec: Screen capture, Output scaling. The input is
//! full-range sRGB (BGRA), the output is BT.709 limited-range NV12.

use crate::{CaptureError, Gpu, with_context};
use std::mem::ManuallyDrop;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0,
    D3D11_VIDEO_COLOR_RGBA, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255,
    D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_PLAYBACK_NORMAL, D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11Texture2D, ID3D11VideoContext,
    ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator, ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows::core::{BOOL, Interface};

/// Converts captured BGRA textures to NV12 at the output size on the D3D11
/// video processor, scaling and letterboxing as needed.
pub struct Nv12Converter {
    /// The shared D3D11 device, kept for output texture creation.
    device: ID3D11Device,
    /// The video side of the device, used to create views and the processor.
    video_device: ID3D11VideoDevice,
    /// The video side of the immediate context, used to run the conversion.
    video_context: ID3D11VideoContext,
    /// The enumerator the processor and views were created from.
    enumerator: ID3D11VideoProcessorEnumerator,
    /// The configured video processor doing the convert and scale.
    processor: ID3D11VideoProcessor,
    /// Expected input width in pixels.
    in_w: u32,
    /// Expected input height in pixels.
    in_h: u32,
    /// Output width in pixels.
    out_w: u32,
    /// Output height in pixels.
    out_h: u32,
    /// Cached input view with the texture it was created from.
    input_view: Option<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
    /// Cached output view with the texture it was created from.
    output_view: Option<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
}

impl Nv12Converter {
    /// Creates the converter for one input size, one output size and one
    /// output frame rate. The caller recreates it when the input size changes.
    pub fn new(gpu: &Gpu, in_w: u32, in_h: u32, out_w: u32, out_h: u32, fps: u32) -> Result<Self, CaptureError> {
        if in_w == 0 || in_h == 0 || out_w == 0 || out_h == 0 || fps == 0 {
            return Err(CaptureError::Missing("Nv12Converter sizes and frame rate must be greater than zero"));
        }
        let device = gpu.device().clone();
        let video_device: ID3D11VideoDevice = with_context("failed to query ID3D11VideoDevice", device.cast())?;
        let video_context: ID3D11VideoContext = with_context("failed to query ID3D11VideoContext", gpu.context().cast())?;
        let (enumerator, processor) = create_processor(&video_device, in_w, in_h, out_w, out_h, fps)?;
        let converter = Self { device, video_device, video_context, enumerator, processor, in_w, in_h, out_w, out_h, input_view: None, output_view: None };
        configure(&converter);
        Ok(converter)
    }

    /// Creates the NV12 output texture of the output size, usable later as the
    /// encoder's input.
    pub fn create_output(&self) -> Result<ID3D11Texture2D, CaptureError> {
        create_texture(&self.device, self.out_w, self.out_h)
    }

    /// Converts one BGRA input frame into one NV12 output frame with one
    /// VideoProcessorBlt. View creation is cached per texture.
    pub fn convert(&mut self, input: &ID3D11Texture2D, output: &ID3D11Texture2D) -> Result<(), CaptureError> {
        let (actual_w, actual_h) = texture_size(input)?;
        if (actual_w, actual_h) != (self.in_w, self.in_h) {
            return Err(CaptureError::InputSizeChanged { expected_w: self.in_w, expected_h: self.in_h, actual_w, actual_h });
        }
        let input_current = self.input_view.as_ref().is_some_and(|(texture, _)| com_same(texture, input));
        if !input_current {
            let view = create_input_view(&self.video_device, &self.enumerator, input)?;
            self.input_view = Some((input.clone(), view));
        }
        let output_current = self.output_view.as_ref().is_some_and(|(texture, _)| com_same(texture, output));
        if !output_current {
            let view = create_output_view(&self.video_device, &self.enumerator, output)?;
            self.output_view = Some((output.clone(), view));
        }
        let (_, input_view) = self.input_view.as_ref().ok_or(CaptureError::Missing("the cached input view is gone"))?;
        let (_, output_view) = self.output_view.as_ref().ok_or(CaptureError::Missing("the cached output view is gone"))?;
        blt(&self.video_context, &self.processor, input_view, output_view)
    }
}
/// True when the input and output aspect ratios differ.
fn aspects_differ(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> bool {
    in_w * out_h != out_w * in_h
}

/// The largest centred rectangle with the input aspect that fits the output.
fn fitted_rect(in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> RECT {
    // Compare with u64 math so wide monitors cannot overflow.
    let wide = u64::from(in_w) * u64::from(out_h) > u64::from(out_w) * u64::from(in_h);
    let (fit_w, fit_h) = if wide {
        (out_w, ((u64::from(out_w) * u64::from(in_h)) / u64::from(in_w)) as u32)
    } else {
        (((u64::from(out_h) * u64::from(in_w)) / u64::from(in_h)) as u32, out_h)
    };
    let x = (out_w.saturating_sub(fit_w) / 2) as i32;
    let y = (out_h.saturating_sub(fit_h) / 2) as i32;
    RECT { left: x, top: y, right: x + fit_w as i32, bottom: y + fit_h as i32 }
}

/// Sets the colour spaces, turns off auto processing, and letterboxes the
/// picture when the input and output aspects differ.
#[allow(unsafe_code)]
fn configure(converter: &Nv12Converter) {
    // Colour space bitfield: bit0 Usage (0 = playback), bit1 RGB_Range
    // (0 = full, 1 = limited), bit2 YCbCr_Matrix (1 = BT.709), bit3
    // YCbCr_XvYCC, bits 4 to 5 Nominal_Range.
    let input_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        // Full range RGB in (Usage 0 = playback, RGB_Range 0 = full,
        // YCbCr_Matrix 1 = BT.709, Nominal_Range 0 to 255).
        _bitfield: (1 << 2) | ((D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255.0 as u32) << 4),
    };
    let output_space = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        // BT.709 YCbCr out, limited 16 to 235 (Usage 0 = playback,
        // RGB_Range 1 = limited, YCbCr_Matrix 1 = BT.709,
        // Nominal_Range 16 to 235).
        _bitfield: (1 << 1) | (1 << 2) | ((D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235.0 as u32) << 4),
    };
    // SAFETY: the processor is a live COM object; the descriptors are plain values.
    unsafe {
        converter.video_context.VideoProcessorSetStreamColorSpace(&converter.processor, 0, &input_space);
        converter.video_context.VideoProcessorSetOutputColorSpace(&converter.processor, &output_space);
        // No denoise, no edge enhancement, no driver-side extras.
        converter.video_context.VideoProcessorSetStreamAutoProcessingMode(&converter.processor, 0, false);
    }
    if aspects_differ(converter.in_w, converter.in_h, converter.out_w, converter.out_h) {
        let rect = fitted_rect(converter.in_w, converter.in_h, converter.out_w, converter.out_h);
        let background = D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } } };
        // SAFETY: the processor is a live COM object; the rect and colour are plain values.
        unsafe {
            converter.video_context.VideoProcessorSetOutputTargetRect(&converter.processor, true, Some(&rect));
            converter.video_context.VideoProcessorSetStreamDestRect(&converter.processor, 0, true, Some(&rect));
            converter.video_context.VideoProcessorSetOutputBackgroundColor(&converter.processor, false, &background);
        }
    }
}

/// Creates the video processor enumerator and processor for the given sizes.
#[allow(unsafe_code)]
fn create_processor(
    video_device: &ID3D11VideoDevice,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    fps: u32,
) -> Result<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor), CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: DXGI_RATIONAL { Numerator: 30, Denominator: 1 },
        InputWidth: in_w,
        InputHeight: in_h,
        OutputFrameRate: DXGI_RATIONAL { Numerator: fps, Denominator: 1 },
        OutputWidth: out_w,
        OutputHeight: out_h,
        Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
    };
    // SAFETY: desc is a plain value; the call returns an owned enumerator reference.
    let enumerator = with_context("failed to create the video processor enumerator", unsafe { video_device.CreateVideoProcessorEnumerator(&desc) })?;
    // SAFETY: enumerator is a live COM object; rate conversion 0 is the plain path.
    let processor = with_context("failed to create the video processor", unsafe { video_device.CreateVideoProcessor(&enumerator, 0) })?;
    Ok((enumerator, processor))
}

/// Creates the NV12 output texture with the bind flags the video processor
/// output view needs.
#[allow(unsafe_code)]
fn create_texture(device: &ID3D11Device, width: u32, height: u32) -> Result<ID3D11Texture2D, CaptureError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut created: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is a plain value and created is a valid out pointer.
    with_context("failed to create the NV12 output texture", unsafe { device.CreateTexture2D(&desc, None, Some(&mut created)) })?;
    created.ok_or(CaptureError::Missing("CreateTexture2D returned no NV12 texture"))
}

/// Reads the size of a texture.
#[allow(unsafe_code)]
fn texture_size(texture: &ID3D11Texture2D) -> Result<(u32, u32), CaptureError> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: texture is a live D3D11 texture; GetDesc only writes the descriptor.
    unsafe { texture.GetDesc(&mut desc) };
    Ok((desc.Width, desc.Height))
}

/// Creates the video processor input view for one BGRA input texture.
#[allow(unsafe_code)]
fn create_input_view(
    video_device: &ID3D11VideoDevice,
    enumerator: &ID3D11VideoProcessorEnumerator,
    input: &ID3D11Texture2D,
) -> Result<ID3D11VideoProcessorInputView, CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
        FourCC: 0,
        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
    };
    let mut view: Option<ID3D11VideoProcessorInputView> = None;
    // SAFETY: all arguments are live COM objects and view is a valid out pointer.
    with_context("failed to create the video processor input view", unsafe {
        video_device.CreateVideoProcessorInputView(input, enumerator, &desc, Some(&mut view))
    })?;
    view.ok_or(CaptureError::Missing("CreateVideoProcessorInputView returned no view"))
}

/// Creates the video processor output view for one NV12 output texture.
#[allow(unsafe_code)]
fn create_output_view(
    video_device: &ID3D11VideoDevice,
    enumerator: &ID3D11VideoProcessorEnumerator,
    output: &ID3D11Texture2D,
) -> Result<ID3D11VideoProcessorOutputView, CaptureError> {
    let desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
    };
    let mut view: Option<ID3D11VideoProcessorOutputView> = None;
    // SAFETY: all arguments are live COM objects and view is a valid out pointer.
    with_context("failed to create the video processor output view", unsafe {
        video_device.CreateVideoProcessorOutputView(output, enumerator, &desc, Some(&mut view))
    })?;
    view.ok_or(CaptureError::Missing("CreateVideoProcessorOutputView returned no view"))
}

/// Runs one VideoProcessorBlt from the input view into the output view.
#[allow(unsafe_code)]
fn blt(
    video_context: &ID3D11VideoContext,
    processor: &ID3D11VideoProcessor,
    input_view: &ID3D11VideoProcessorInputView,
    output_view: &ID3D11VideoProcessorOutputView,
) -> Result<(), CaptureError> {
    let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
        Enable: BOOL(1),
        OutputIndex: 0,
        InputFrameOrField: 0,
        PastFrames: 0,
        FutureFrames: 0,
        ppPastSurfaces: std::ptr::null_mut(),
        pInputSurface: ManuallyDrop::new(Some(input_view.clone())),
        ppFutureSurfaces: std::ptr::null_mut(),
        ppPastSurfacesRight: std::ptr::null_mut(),
        pInputSurfaceRight: ManuallyDrop::new(None),
        ppFutureSurfacesRight: std::ptr::null_mut(),
    };
    // SAFETY: processor and views are live COM objects on the same device.
    let result = unsafe { video_context.VideoProcessorBlt(processor, output_view, 0, std::slice::from_ref(&stream)) };
    // SAFETY: we drop the one reference we placed in the stream, exactly once.
    unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
    with_context("VideoProcessorBlt failed", result)
}

/// True when both references point at the same COM object.
fn com_same(a: &ID3D11Texture2D, b: &ID3D11Texture2D) -> bool {
    std::ptr::eq(a.as_raw(), b.as_raw())
}
