//! Background images: PNG and JPG files decoded through WIC onto immutable BGRA textures.
//!
//! The decoder converts every file to 32-bit premultiplied BGRA, scales anything past the size cap
//! down with WIC, and the texture then stays on the GPU for the compositor to sample.

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_SHADER_RESOURCE, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE, ID3D11Device, ID3D11ShaderResourceView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory, IWICPalette, WICBitmapDitherTypeNone,
    WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::core::{HRESULT, HSTRING, Interface};

use crate::{CaptureError, with_context};

/// The largest side of a background image, in pixels; anything bigger is scaled down by WIC.
const MAX_SIDE: u32 = 4096;

/// A background image decoded through WIC onto an immutable BGRA texture.
pub struct BackgroundImage {
    path: String,
    view: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
}

impl BackgroundImage {
    /// The path the image was loaded from.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The shader view of the image texture.
    pub fn view(&self) -> &ID3D11ShaderResourceView {
        &self.view
    }

    /// The width and height of the decoded image, in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Decodes the PNG or JPG at `path` onto an immutable BGRA texture on `device`.
///
/// The long side is capped at [`MAX_SIDE`]; bigger images are scaled down by WIC before they reach the GPU.
pub fn load(device: &ID3D11Device, path: &str) -> Result<BackgroundImage, CaptureError> {
    let (width, height, pixels) = decode(path)?;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr().cast(), SysMemPitch: width * 4, SysMemSlicePitch: 0 };
    let mut texture: Option<ID3D11Texture2D> = None;
    // SAFETY: desc and data describe the decoded pixel buffer and texture is a valid out pointer.
    with_context("failed to create the background image texture", unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) })?;
    let texture = texture.ok_or(CaptureError::Missing("CreateTexture2D returned no background texture"))?;
    let mut view: Option<ID3D11ShaderResourceView> = None;
    // SAFETY: the texture is live on the same device and view is a valid out pointer.
    with_context("failed to create the background image view", unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view)) })?;
    let view = view.ok_or(CaptureError::Missing("CreateShaderResourceView returned no background view"))?;
    Ok(BackgroundImage { path: path.to_owned(), view, width, height })
}

/// Initializes COM for the decode, balances it again, and reports a clear error for an unreadable file.
fn decode(path: &str) -> Result<(u32, u32, Vec<u8>), CaptureError> {
    if std::fs::metadata(path).is_err() {
        return Err(CaptureError::Compose(format!("the background image {path} does not exist or cannot be read")));
    }
    // SAFETY: CoInitializeEx only takes an apartment flag; a returned S_OK is balanced by CoUninitialize below.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let result = decode_with_wic(path);
    if hr == HRESULT(0) {
        // SAFETY: this thread was the one that got S_OK above, and every COM pointer is released by now.
        unsafe { CoUninitialize() };
    }
    result
}

/// Does the WIC work of [`decode`]: decoder, format converter, optional scaler, pixel copy.
fn decode_with_wic(path: &str) -> Result<(u32, u32, Vec<u8>), CaptureError> {
    let factory: IWICImagingFactory = with_context(
        "could not create the WIC imaging factory",
        // SAFETY: CLSID_WICImagingFactory is the WIC factory class id and the interface is a valid out parameter.
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) },
    )?;
    let wide = HSTRING::from(path);
    // SAFETY: wide is a live NUL terminated string for the duration of the call.
    let decoder = with_context("could not open the background image (is it a PNG or JPG?)", unsafe {
        factory.CreateDecoderFromFilename(&wide, None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
    })?;
    let frame = with_context("could not read the first frame of the background image", unsafe { decoder.GetFrame(0) })?;
    let converter = with_context("could not create the WIC format converter", unsafe { factory.CreateFormatConverter() })?;
    // SAFETY: the frame is a live bitmap source, no palette is wanted, and the call only reads the picture.
    with_context("could not convert the background image to 32-bit BGRA", unsafe {
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None::<&IWICPalette>, 0.0, WICBitmapPaletteTypeCustom)
    })?;
    let mut width = 0u32;
    let mut height = 0u32;
    // SAFETY: both out pointers are valid locals.
    with_context("could not read the background image size", unsafe { frame.GetSize(&mut width, &mut height) })?;
    if width == 0 || height == 0 {
        return Err(CaptureError::Compose(format!("the background image {path} has an empty frame")));
    }
    let source: IWICBitmapSource = if width.max(height) > MAX_SIDE {
        let scale = f64::from(MAX_SIDE) / f64::from(width.max(height));
        let scaled_w = ((f64::from(width) * scale) as u32).max(1);
        let scaled_h = ((f64::from(height) * scale) as u32).max(1);
        let scaler = with_context("could not create the WIC bitmap scaler", unsafe { factory.CreateBitmapScaler() })?;
        // SAFETY: the converter is a live bitmap source on the same factory.
        with_context("could not scale the background image down", unsafe {
            scaler.Initialize(&converter, scaled_w, scaled_h, WICBitmapInterpolationModeFant)
        })?;
        with_context("the WIC bitmap scaler is not a bitmap source", scaler.cast::<IWICBitmapSource>())?
    } else {
        with_context("the background image frame is not a bitmap source", frame.cast::<IWICBitmapSource>())?
    };
    let (width, height) = source_size(&source)?;
    let stride = width * 4;
    let mut pixels = vec![0u8; stride as usize * height as usize];
    // SAFETY: pixels is a live buffer of exactly stride times height bytes and a null rect means the whole image.
    with_context("could not decode the background image pixels", unsafe { source.CopyPixels(std::ptr::null(), stride, &mut pixels) })?;
    Ok((width, height, pixels))
}

/// Reads the size of a bitmap source after any scaling.
fn source_size(source: &IWICBitmapSource) -> Result<(u32, u32), CaptureError> {
    let mut width = 0u32;
    let mut height = 0u32;
    // SAFETY: both out pointers are valid locals.
    with_context("could not read the background image size", unsafe { source.GetSize(&mut width, &mut height) })?;
    Ok((width, height))
}
