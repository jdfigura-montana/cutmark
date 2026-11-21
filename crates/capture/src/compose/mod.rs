//! The D3D11 compositor: draws the screen and the NV12 camera into one BGRA frame on the GPU.
//!
//! It runs on the shared device, so no picture ever reaches the CPU. The shaders in `shaders.hlsl` are
//! compiled when the compositor is created. The layout math lives in [`layout`]. See the recorder
//! spec: Layouts and Compositor.
#![allow(unsafe_code)]

mod layout;

pub use layout::{CamPlacement, Corner, Layout, PipSettings, Placement, RectF, cam_crop, fit_rect, pip_placement, place};

use crate::{CamFrame, CaptureError, Gpu, with_context};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_ENABLE_STRICTNESS, D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
use windows::Win32::Graphics::Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, D3D_SRV_DIMENSION_TEXTURE2D, ID3DBlob, ID3DInclude};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BLEND_DESC, D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_ONE,
    D3D11_BLEND_OP_ADD, D3D11_BUFFER_DESC, D3D11_COLOR_WRITE_ENABLE_ALL, D3D11_COMPARISON_NEVER, D3D11_CPU_ACCESS_READ, D3D11_CULL_NONE, D3D11_FILL_SOLID,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC, D3D11_SAMPLER_DESC,
    D3D11_SHADER_RESOURCE_VIEW_DESC, D3D11_SHADER_RESOURCE_VIEW_DESC_0, D3D11_TEX2D_SRV, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIEWPORT, ID3D11BlendState, ID3D11Buffer, ID3D11ClassLinkage, ID3D11DepthStencilView, ID3D11Device,
    ID3D11DeviceContext, ID3D11InputLayout, ID3D11Multithread, ID3D11PixelShader, ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState,
    ID3D11ShaderResourceView, ID3D11Texture2D, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM, DXGI_SAMPLE_DESC};
use windows::core::{BOOL, Interface, PCSTR, s};

/// The HLSL source of every compositor shader.
const SHADER_SOURCE: &str = include_str!("shaders.hlsl");

/// How many output textures the compositor rotates through.
const RING_LEN: usize = 3;

/// How many shader views are kept per kind of input texture, enough for a camera's whole texture ring.
const VIEW_CACHE: usize = 8;

/// The matrix that turns a camera's YCbCr into RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YuvMatrix {
    /// BT.601, the standard definition matrix; the default when the camera does not say.
    #[default]
    Bt601,
    /// BT.709, the HD matrix.
    Bt709,
    /// BT.2020.
    Bt2020,
}

/// Whether a camera's luma runs from 16 to 235 or over the whole 0 to 255 range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YuvRange {
    /// 16 to 235 for luma and 16 to 240 for chroma; the default when the camera does not say.
    #[default]
    Limited,
    /// 0 to 255.
    Full,
}

/// The colour matrix and range of a camera's NV12 frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CamColor {
    /// The YCbCr to RGB matrix.
    pub matrix: YuvMatrix,
    /// The range of the YCbCr values.
    pub range: YuvRange,
}

impl CamColor {
    /// Reads the Media Foundation values MF_MT_YUV_MATRIX and MF_MT_VIDEO_NOMINAL_RANGE; 0 or an unknown value keeps the default.
    pub fn from_media_type(matrix: u32, range: u32) -> Self {
        let matrix = match matrix {
            1 | 3 => YuvMatrix::Bt709,
            4 | 5 => YuvMatrix::Bt2020,
            _ => YuvMatrix::Bt601,
        };
        let range = if range == 1 { YuvRange::Full } else { YuvRange::Limited };
        Self { matrix, range }
    }

    /// The constants the NV12 shader needs: red from Cr, green from Cb, green from Cr and blue from Cb, then
    /// the luma offset and scale and the chroma offset and scale.
    fn shader_constants(&self) -> ([f32; 4], [f32; 4]) {
        let (kr, kb) = match self.matrix {
            YuvMatrix::Bt601 => (0.299_f32, 0.114_f32),
            YuvMatrix::Bt709 => (0.2126, 0.0722),
            YuvMatrix::Bt2020 => (0.2627, 0.0593),
        };
        let kg = 1.0 - kr - kb;
        let red_from_cr = 2.0 * (1.0 - kr);
        let blue_from_cb = 2.0 * (1.0 - kb);
        let coef = [red_from_cr, kb / kg * blue_from_cb, kr / kg * red_from_cr, blue_from_cb];
        let range = match self.range {
            YuvRange::Limited => [16.0 / 255.0, 255.0 / 219.0, 128.0 / 255.0, 255.0 / 224.0],
            YuvRange::Full => [0.0, 1.0, 128.0 / 255.0, 1.0],
        };
        (coef, range)
    }
}

/// One finished composite: a BGRA texture of the output size and the number of the frame.
#[derive(Clone)]
pub struct ComposedFrame {
    /// The output texture, one of a small ring that is reused as newer frames are composed.
    pub texture: ID3D11Texture2D,
    /// The frame number, counting from 1.
    pub seq: u64,
}

/// The shader constants of one draw; the layout matches the cbuffer in `shaders.hlsl`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Params {
    dst_rect: [f32; 4],
    src_uv: [f32; 4],
    border_color: [f32; 4],
    shape: [f32; 4],
    yuv_coef: [f32; 4],
    yuv_range: [f32; 4],
}

/// One output texture with its render target view.
struct Target {
    texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
}

/// Holds the device's multithread lock, so one composite's state changes and draws stay together.
struct ContextLock<'a>(&'a ID3D11Multithread);

impl<'a> ContextLock<'a> {
    /// Takes the lock; the lock may be taken again by the same thread.
    fn enter(multithread: &'a ID3D11Multithread) -> Self {
        // SAFETY: the interface is live; Enter only takes the device's critical section.
        unsafe { multithread.Enter() };
        Self(multithread)
    }
}

impl Drop for ContextLock<'_> {
    fn drop(&mut self) {
        // SAFETY: balances the Enter in `enter`, on the same thread.
        unsafe { self.0.Leave() };
    }
}

/// Draws the screen and the camera into BGRA output textures on the shared D3D11 device.
pub struct Compositor {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    multithread: Option<ID3D11Multithread>,
    out_w: u32,
    out_h: u32,
    vs: ID3D11VertexShader,
    ps_bgra: ID3D11PixelShader,
    ps_nv12: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    params: ID3D11Buffer,
    targets: Vec<Target>,
    next: usize,
    seq: u64,
    screen_views: Vec<(ID3D11Texture2D, ID3D11ShaderResourceView)>,
    cam_views: Vec<(ID3D11Texture2D, [ID3D11ShaderResourceView; 2])>,
    cam_color: CamColor,
    cam_error_logged: bool,
}

impl Compositor {
    /// Compiles the shaders and creates the output ring for one output size; fails with a clear message if a shader does not compile.
    pub fn new(gpu: &Gpu, output_w: u32, output_h: u32) -> Result<Self, CaptureError> {
        if output_w == 0 || output_h == 0 {
            return Err(CaptureError::Missing("the compositor output size must be greater than zero"));
        }
        let device = gpu.device().clone();
        let vs_code = compile(s!("vs_main"), s!("vs_5_0"), "vs_main")?;
        let bgra_code = compile(s!("ps_bgra"), s!("ps_5_0"), "ps_bgra")?;
        let nv12_code = compile(s!("ps_nv12"), s!("ps_5_0"), "ps_nv12")?;
        let mut vs: Option<ID3D11VertexShader> = None;
        let mut ps_bgra: Option<ID3D11PixelShader> = None;
        let mut ps_nv12: Option<ID3D11PixelShader> = None;
        // SAFETY: the bytecode slices come from live blobs and every out pointer is a valid local.
        unsafe {
            with_context(
                "failed to create the compositor vertex shader",
                device.CreateVertexShader(blob_bytes(&vs_code), None::<&ID3D11ClassLinkage>, Some(&mut vs)),
            )?;
            with_context(
                "failed to create the BGRA pixel shader",
                device.CreatePixelShader(blob_bytes(&bgra_code), None::<&ID3D11ClassLinkage>, Some(&mut ps_bgra)),
            )?;
            with_context(
                "failed to create the NV12 pixel shader",
                device.CreatePixelShader(blob_bytes(&nv12_code), None::<&ID3D11ClassLinkage>, Some(&mut ps_nv12)),
            )?;
        }
        let multithread = match device.cast::<ID3D11Multithread>() {
            Ok(multithread) => Some(multithread),
            Err(error) => {
                log::warn!("compositor: could not query ID3D11Multithread: {error}");
                None
            }
        };
        let mut targets = Vec::with_capacity(RING_LEN);
        for _ in 0..RING_LEN {
            targets.push(create_target(&device, output_w, output_h)?);
        }
        Ok(Self {
            context: gpu.context().clone(),
            multithread,
            out_w: output_w,
            out_h: output_h,
            vs: vs.ok_or(CaptureError::Missing("CreateVertexShader returned no shader"))?,
            ps_bgra: ps_bgra.ok_or(CaptureError::Missing("CreatePixelShader returned no BGRA shader"))?,
            ps_nv12: ps_nv12.ok_or(CaptureError::Missing("CreatePixelShader returned no NV12 shader"))?,
            sampler: create_sampler(&device)?,
            blend: create_blend(&device)?,
            raster: create_raster(&device)?,
            params: create_params_buffer(&device)?,
            device,
            targets,
            next: 0,
            seq: 0,
            screen_views: Vec::new(),
            cam_views: Vec::new(),
            cam_color: CamColor::default(),
            cam_error_logged: false,
        })
    }

    /// The output size the compositor was created for.
    pub fn output_size(&self) -> (u32, u32) {
        (self.out_w, self.out_h)
    }

    /// Sets the matrix and range the camera frames are converted with.
    pub fn set_cam_color(&mut self, color: CamColor) {
        self.cam_color = color;
    }

    /// Composes one frame into the next output texture and returns it with its sequence number.
    ///
    /// Screen only draws the screen. Picture in picture draws the screen, then the camera. Cam only draws
    /// `bg_color`, then the camera fitted to the frame. A picture that is missing is left out.
    pub fn compose(
        &mut self,
        screen: Option<&ID3D11Texture2D>,
        cam: Option<&CamFrame>,
        layout: &Layout,
        pip: &PipSettings,
        bg_color: [u8; 3],
    ) -> Result<ComposedFrame, CaptureError> {
        let screen_size = screen.map(texture_size);
        let cam_size = cam.map(|frame| texture_size(&frame.texture));
        let placement = place(*layout, screen_size, cam_size, pip, self.out_w, self.out_h);
        let multithread = self.multithread.clone();
        let _lock = multithread.as_ref().map(ContextLock::enter);
        let screen_view = match (placement.screen, screen) {
            (Some(_), Some(texture)) => Some(self.screen_view(texture)?),
            _ => None,
        };
        let cam_views = match (placement.cam, cam) {
            (Some(_), Some(frame)) => match self.cam_views(&frame.texture) {
                Ok(views) => Some(views),
                Err(error) => {
                    if !self.cam_error_logged {
                        self.cam_error_logged = true;
                        log::warn!("compositor: the camera frames cannot be drawn: {error}");
                    }
                    None
                }
            },
            _ => None,
        };
        let index = self.next;
        self.next = (self.next + 1) % RING_LEN;
        self.seq += 1;
        let target = self.targets.get(index).ok_or(CaptureError::Missing("the compositor output ring is empty"))?;
        let (texture, rtv) = (target.texture.clone(), target.rtv.clone());
        let clear = match layout {
            Layout::CamOnly => [f32::from(bg_color[0]) / 255.0, f32::from(bg_color[1]) / 255.0, f32::from(bg_color[2]) / 255.0, 1.0],
            Layout::ScreenOnly | Layout::Pip => [0.0, 0.0, 0.0, 1.0],
        };
        self.begin(&rtv, clear);
        if let (Some(rect), Some(view)) = (placement.screen, &screen_view) {
            self.draw_bgra(rect, view);
        }
        if let (Some(cam), Some(views)) = (placement.cam, &cam_views) {
            self.draw_nv12(&cam, pip, views);
        }
        self.end();
        Ok(ComposedFrame { texture, seq: self.seq })
    }

    /// Binds the render target and every fixed state, and clears the target.
    fn begin(&self, rtv: &ID3D11RenderTargetView, clear: [f32; 4]) {
        let viewport = D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.out_w as f32, Height: self.out_h as f32, MinDepth: 0.0, MaxDepth: 1.0 };
        // SAFETY: the context and every object are live and on the same device; the arrays outlive the calls.
        unsafe {
            self.context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None::<&ID3D11DepthStencilView>);
            self.context.ClearRenderTargetView(rtv, &clear);
            self.context.OMSetBlendState(&self.blend, None, 0xFFFF_FFFF);
            self.context.RSSetState(&self.raster);
            self.context.RSSetViewports(Some(&[viewport]));
            self.context.IASetInputLayout(None::<&ID3D11InputLayout>);
            self.context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            self.context.VSSetShader(&self.vs, None);
            self.context.VSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            self.context.PSSetConstantBuffers(0, Some(&[Some(self.params.clone())]));
            self.context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
        }
    }

    /// Unbinds the target and the inputs and submits the work.
    fn end(&self) {
        // SAFETY: the context is live; the calls only clear bindings and submit queued work.
        unsafe {
            self.context.OMSetRenderTargets(None, None::<&ID3D11DepthStencilView>);
            self.context.PSSetShaderResources(0, Some(&[None, None, None]));
            self.context.Flush();
        }
    }

    /// Draws a BGRA texture into the rectangle.
    fn draw_bgra(&self, rect: RectF, view: &ID3D11ShaderResourceView) {
        let params = Params {
            dst_rect: [rect.left, rect.top, rect.right, rect.bottom],
            src_uv: [0.0, 0.0, 1.0, 1.0],
            shape: [0.0, 0.0, self.out_w as f32, self.out_h as f32],
            ..Params::default()
        };
        // SAFETY: the context and every object are live; params outlives the copy that UpdateSubresource makes.
        unsafe {
            self.context.UpdateSubresource(&self.params, 0, None, std::ptr::from_ref(&params).cast(), 0, 0);
            self.context.PSSetShader(&self.ps_bgra, None);
            self.context.PSSetShaderResources(0, Some(&[Some(view.clone())]));
            self.context.Draw(4, 0);
        }
    }

    /// Draws the camera into its rectangle with the crop, the rounded corners and the border.
    fn draw_nv12(&self, cam: &CamPlacement, pip: &PipSettings, views: &[ID3D11ShaderResourceView; 2]) {
        let (coef, range) = self.cam_color.shader_constants();
        let [red, green, blue] = pip.border_color;
        let params = Params {
            dst_rect: [cam.dst.left, cam.dst.top, cam.dst.right, cam.dst.bottom],
            src_uv: [cam.src.left, cam.src.top, cam.src.right, cam.src.bottom],
            border_color: [f32::from(red) / 255.0, f32::from(green) / 255.0, f32::from(blue) / 255.0, 1.0],
            shape: [pip.radius_px.max(0.0), pip.border_px.max(0.0), self.out_w as f32, self.out_h as f32],
            yuv_coef: coef,
            yuv_range: range,
        };
        let [luma, chroma] = views;
        // SAFETY: the context and every object are live; params outlives the copy that UpdateSubresource makes.
        unsafe {
            self.context.UpdateSubresource(&self.params, 0, None, std::ptr::from_ref(&params).cast(), 0, 0);
            self.context.PSSetShader(&self.ps_nv12, None);
            self.context.PSSetShaderResources(1, Some(&[Some(luma.clone()), Some(chroma.clone())]));
            self.context.Draw(4, 0);
        }
    }

    /// The shader view of a screen texture, created once per texture.
    fn screen_view(&mut self, texture: &ID3D11Texture2D) -> Result<ID3D11ShaderResourceView, CaptureError> {
        let device = &self.device;
        cached(&mut self.screen_views, texture, || {
            let mut view: Option<ID3D11ShaderResourceView> = None;
            // SAFETY: the texture is live on the same device and view is a valid out pointer.
            with_context("failed to create the screen shader view", unsafe { device.CreateShaderResourceView(texture, None, Some(&mut view)) })?;
            view.ok_or(CaptureError::Missing("CreateShaderResourceView returned no screen view"))
        })
    }

    /// The two shader views of a camera texture, the Y plane and the UV plane, created once per texture.
    fn cam_views(&mut self, texture: &ID3D11Texture2D) -> Result<[ID3D11ShaderResourceView; 2], CaptureError> {
        let device = &self.device;
        cached(&mut self.cam_views, texture, || {
            let luma = plane_view(device, texture, DXGI_FORMAT_R8_UNORM)?;
            let chroma = plane_view(device, texture, DXGI_FORMAT_R8G8_UNORM)?;
            Ok([luma, chroma])
        })
    }
}

/// Returns the cached value for a texture, or makes and caches it; the oldest entry goes when the cache is full.
fn cached<V: Clone>(
    cache: &mut Vec<(ID3D11Texture2D, V)>,
    texture: &ID3D11Texture2D,
    make: impl FnOnce() -> Result<V, CaptureError>,
) -> Result<V, CaptureError> {
    if let Some((_, value)) = cache.iter().find(|(known, _)| std::ptr::eq(known.as_raw(), texture.as_raw())) {
        return Ok(value.clone());
    }
    let value = make()?;
    if cache.len() >= VIEW_CACHE {
        cache.remove(0);
    }
    cache.push((texture.clone(), value.clone()));
    Ok(value)
}

/// Creates a shader view of one plane of an NV12 texture.
fn plane_view(device: &ID3D11Device, texture: &ID3D11Texture2D, format: DXGI_FORMAT) -> Result<ID3D11ShaderResourceView, CaptureError> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: D3D_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 } },
    };
    let mut view: Option<ID3D11ShaderResourceView> = None;
    // SAFETY: desc is a plain value, the texture is live on the same device and view is a valid out pointer.
    with_context("failed to create a camera plane view (the camera texture needs the shader resource bind flag)", unsafe {
        device.CreateShaderResourceView(texture, Some(&desc), Some(&mut view))
    })?;
    view.ok_or(CaptureError::Missing("CreateShaderResourceView returned no camera plane view"))
}

/// Compiles one entry point of the embedded shader source.
fn compile(entry: PCSTR, target: PCSTR, name: &str) -> Result<ID3DBlob, CaptureError> {
    let mut code: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    // SAFETY: the pointer and length describe the embedded source text, the names are NUL terminated
    // literals, and both out pointers are valid locals.
    let result = unsafe {
        D3DCompile(
            SHADER_SOURCE.as_ptr().cast(),
            SHADER_SOURCE.len(),
            s!("compose.hlsl"),
            None,
            None::<&ID3DInclude>,
            entry,
            target,
            D3DCOMPILE_ENABLE_STRICTNESS | D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    match result {
        Ok(()) => code.ok_or(CaptureError::Missing("D3DCompile returned no bytecode")),
        Err(error) => {
            let detail = errors.as_ref().map(|blob| String::from_utf8_lossy(blob_bytes(blob)).trim().to_owned()).unwrap_or_default();
            Err(CaptureError::Compose(format!("the compositor shader {name} did not compile: {error} {detail}")))
        }
    }
}

/// The bytes of a blob.
fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    // SAFETY: a blob owns GetBufferSize bytes at GetBufferPointer for as long as it lives.
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) }
}

/// Creates one output texture, a render target that shaders can also read, with its view.
fn create_target(device: &ID3D11Device, width: u32, height: u32) -> Result<Target, CaptureError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is a plain value and texture is a valid out pointer.
    with_context("failed to create a compositor output texture", unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) })?;
    let texture = texture.ok_or(CaptureError::Missing("CreateTexture2D returned no compositor texture"))?;
    let mut rtv: Option<ID3D11RenderTargetView> = None;
    // SAFETY: the texture is live on the same device and rtv is a valid out pointer.
    with_context("failed to create a compositor render target view", unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) })?;
    let rtv = rtv.ok_or(CaptureError::Missing("CreateRenderTargetView returned no view"))?;
    Ok(Target { texture, rtv })
}

/// Creates the linear, clamping sampler.
fn create_sampler(device: &ID3D11Device) -> Result<ID3D11SamplerState, CaptureError> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_NEVER,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
    };
    let mut sampler: Option<ID3D11SamplerState> = None;
    // SAFETY: desc is a plain value and sampler is a valid out pointer.
    with_context("failed to create the compositor sampler", unsafe { device.CreateSamplerState(&desc, Some(&mut sampler)) })?;
    sampler.ok_or(CaptureError::Missing("CreateSamplerState returned no sampler"))
}

/// Creates the blend state for premultiplied alpha.
fn create_blend(device: &ID3D11Device) -> Result<ID3D11BlendState, CaptureError> {
    let target = D3D11_RENDER_TARGET_BLEND_DESC {
        BlendEnable: BOOL(1),
        SrcBlend: D3D11_BLEND_ONE,
        DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D11_BLEND_OP_ADD,
        SrcBlendAlpha: D3D11_BLEND_ONE,
        DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D11_BLEND_OP_ADD,
        RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    let desc = D3D11_BLEND_DESC { AlphaToCoverageEnable: BOOL(0), IndependentBlendEnable: BOOL(0), RenderTarget: [target; 8] };
    let mut state: Option<ID3D11BlendState> = None;
    // SAFETY: desc is a plain value and state is a valid out pointer.
    with_context("failed to create the compositor blend state", unsafe { device.CreateBlendState(&desc, Some(&mut state)) })?;
    state.ok_or(CaptureError::Missing("CreateBlendState returned no state"))
}

/// Creates the rasterizer state: solid, no culling.
fn create_raster(device: &ID3D11Device) -> Result<ID3D11RasterizerState, CaptureError> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: BOOL(0),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: BOOL(1),
        ScissorEnable: BOOL(0),
        MultisampleEnable: BOOL(0),
        AntialiasedLineEnable: BOOL(0),
    };
    let mut state: Option<ID3D11RasterizerState> = None;
    // SAFETY: desc is a plain value and state is a valid out pointer.
    with_context("failed to create the compositor rasterizer state", unsafe { device.CreateRasterizerState(&desc, Some(&mut state)) })?;
    state.ok_or(CaptureError::Missing("CreateRasterizerState returned no state"))
}

/// Creates the constant buffer that holds one draw's parameters.
fn create_params_buffer(device: &ID3D11Device) -> Result<ID3D11Buffer, CaptureError> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: std::mem::size_of::<Params>() as u32,
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer: Option<ID3D11Buffer> = None;
    // SAFETY: desc is a plain value and buffer is a valid out pointer.
    with_context("failed to create the compositor constant buffer", unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer)) })?;
    buffer.ok_or(CaptureError::Missing("CreateBuffer returned no constant buffer"))
}

/// Reads the size of a texture.
fn texture_size(texture: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: texture is a live D3D11 texture; GetDesc only writes the descriptor.
    unsafe { texture.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}

/// Copies a BGRA texture to the CPU and returns its width, height and tightly packed BGRA bytes; it waits for the GPU, so it is for probes and tests.
pub fn read_bgra(gpu: &Gpu, texture: &ID3D11Texture2D) -> Result<(u32, u32, Vec<u8>), CaptureError> {
    let (width, height) = texture_size(texture);
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    // SAFETY: desc is a plain value and staging is a valid out pointer.
    with_context("failed to create the readback texture", unsafe { gpu.device().CreateTexture2D(&desc, None, Some(&mut staging)) })?;
    let staging = staging.ok_or(CaptureError::Missing("CreateTexture2D returned no readback texture"))?;
    let context = gpu.context();
    // SAFETY: both textures are live, on the same device, with the same size and format.
    unsafe { context.CopyResource(&staging, texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: staging is a live CPU readable texture and mapped is a valid out pointer.
    with_context("failed to map the readback texture", unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) })?;
    let (w, h, pitch) = (width as usize, height as usize, mapped.RowPitch as usize);
    let mut pixels = Vec::with_capacity(w * h * 4);
    if !mapped.pData.is_null() && h > 0 {
        // SAFETY: a successful Map gives RowPitch bytes for each of the h rows, and the last row is read only up to w * 4 bytes; the memory stays valid until Unmap.
        let bytes = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), pitch * (h - 1) + w * 4) };
        for row in 0..h {
            if let Some(line) = bytes.get(row * pitch..row * pitch + w * 4) {
                pixels.extend_from_slice(line);
            }
        }
    }
    // SAFETY: staging was mapped above and the slice is no longer used.
    unsafe { context.Unmap(&staging, 0) };
    if pixels.len() == w * h * 4 { Ok((width, height, pixels)) } else { Err(CaptureError::Missing("the readback returned a short frame")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_constants_are_a_whole_number_of_registers() {
        assert_eq!(std::mem::size_of::<Params>() % 16, 0);
    }

    #[test]
    fn media_foundation_values_map_to_a_matrix_and_range() {
        assert_eq!(CamColor::from_media_type(0, 0), CamColor { matrix: YuvMatrix::Bt601, range: YuvRange::Limited });
        assert_eq!(CamColor::from_media_type(1, 2), CamColor { matrix: YuvMatrix::Bt709, range: YuvRange::Limited });
        assert_eq!(CamColor::from_media_type(2, 1), CamColor { matrix: YuvMatrix::Bt601, range: YuvRange::Full });
        assert_eq!(CamColor::from_media_type(4, 3).matrix, YuvMatrix::Bt2020);
    }

    #[test]
    fn bt601_constants_match_the_textbook_values() {
        let (coef, range) = CamColor::default().shader_constants();
        for (actual, expected) in coef.iter().zip([1.402, 0.344_136, 0.714_136, 1.772]) {
            assert!((actual - expected).abs() < 1e-3, "{actual} vs {expected}");
        }
        assert!((range[1] - 255.0 / 219.0).abs() < 1e-6);
    }

    #[test]
    fn bt709_constants_match_the_textbook_values() {
        let (coef, _) = CamColor { matrix: YuvMatrix::Bt709, range: YuvRange::Limited }.shader_constants();
        for (actual, expected) in coef.iter().zip([1.5748, 0.187_324, 0.468_124, 1.8556]) {
            assert!((actual - expected).abs() < 1e-3, "{actual} vs {expected}");
        }
    }

    #[test]
    fn full_range_has_no_luma_scaling() {
        let (_, range) = CamColor { matrix: YuvMatrix::Bt601, range: YuvRange::Full }.shader_constants();
        assert_eq!((range[0], range[1], range[3]), (0.0, 1.0, 1.0));
    }
}
