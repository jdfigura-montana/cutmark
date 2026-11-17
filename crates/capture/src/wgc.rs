//! Windows Graphics Capture of a monitor, frames kept on the shared D3D11 device.

use std::sync::{Arc, Mutex};

use crate::{CaptureError, Gpu, qpc_now, with_context};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat};
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{D3D11_TEXTURE2D_DESC, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::core::{IInspectable, Interface, Ref, factory};

/// A snapshot of the newest captured frame: our own copy of the texture.
#[derive(Clone)]
pub struct LatestFrame {
    /// The latest BGRA texture on the shared device.
    pub texture: ID3D11Texture2D,
    /// QPC stamp taken when the frame arrived.
    pub qpc: i64,
    /// Frame width in pixels.
    pub width: i32,
    /// Frame height in pixels.
    pub height: i32,
    /// Sequence number, incremented once per new frame; the pacer uses it to spot repeats.
    pub seq: u64,
}

/// State shared between the capture session and its event handlers.
#[derive(Default)]
struct FrameState {
    /// Our own copy of the newest frame's texture, or None before the first frame.
    latest: Option<ID3D11Texture2D>,
    /// QPC stamp of the newest frame.
    qpc: i64,
    /// Width of the newest frame in pixels.
    width: i32,
    /// Height of the newest frame in pixels.
    height: i32,
    /// Sequence number of the newest frame.
    seq: u64,
    /// Total number of frames received, including frames after item close.
    received: u64,
    /// True once the capture item closed; then frames stop being delivered.
    closed: bool,
}

/// A running Windows Graphics Capture of one monitor.
pub struct MonitorCapture {
    item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    state: Arc<Mutex<FrameState>>,
    frame_token: i64,
    closed_token: i64,
    running: bool,
}

impl MonitorCapture {
    /// Starts capturing the given monitor on the shared device.
    pub fn start(gpu: &Gpu, monitor: &crate::MonitorInfo, cursor: bool) -> Result<Self, CaptureError> {
        let device = gpu.device().clone();
        let context = gpu.context().clone();
        // The FrameArrived callback copies frames on this device from a WGC
        // thread while the recording thread drives the same immediate context.
        // Without the runtime's multithread protection those concurrent calls
        // can wedge the capture after the first frame.
        let multithread: ID3D10Multithread = with_context("failed to query ID3D10Multithread from the shared device", device.cast())?;
        // SAFETY: multithread is a live interface on a live device; the call
        // only flips the context protection flag.
        #[allow(unsafe_code)]
        let protected = unsafe { multithread.SetMultithreadProtected(true) };
        with_context("failed to enable the D3D11 multithread protection", if protected.as_bool() { Ok(()) } else { Err(windows::core::Error::empty()) })?;
        let winrt_device = wrap_device(&device)?;
        let item = item_for_monitor(monitor.handle)?;
        let size = SizeInt32 { Width: monitor.width, Height: monitor.height };
        let frame_pool = with_context(
            "failed to create the free-threaded capture frame pool",
            Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size),
        )?;
        let session = with_context("failed to create the capture session", frame_pool.CreateCaptureSession(&item))?;
        with_context("failed to set the cursor capture flag", session.SetIsCursorCaptureEnabled(cursor))?;
        // The border setter needs newer builds; ignore failure on older ones.
        let _ = session.SetIsBorderRequired(false);
        let state = Arc::new(Mutex::new(FrameState::default()));
        let frame_state = state.clone();
        let frame_device = device.clone();
        let frame_context = context.clone();
        let frame_handler = TypedEventHandler::new(move |sender: Ref<'_, Direct3D11CaptureFramePool>, _args: Ref<'_, IInspectable>| {
            on_frame_arrived(&frame_state, &frame_device, &frame_context, sender)
        });
        let frame_token = with_context("failed to subscribe to FrameArrived", frame_pool.FrameArrived(&frame_handler))?;
        let closed_state = state.clone();
        let closed_handler = TypedEventHandler::new(move |_sender: Ref<'_, GraphicsCaptureItem>, _args: Ref<'_, IInspectable>| on_item_closed(&closed_state));
        let closed_token = with_context("failed to subscribe to Closed", item.Closed(&closed_handler))?;
        with_context("failed to start capture", session.StartCapture())?;
        Ok(Self { item, frame_pool, session, state, frame_token, closed_token, running: true })
    }

    /// A clone of the newest frame's texture with its QPC stamp, size and sequence number.
    pub fn latest(&self) -> Option<LatestFrame> {
        let state = self.state.lock().ok()?;
        if state.closed {
            return None;
        }
        let texture = state.latest.as_ref()?;
        Some(LatestFrame { texture: texture.clone(), qpc: state.qpc, width: state.width, height: state.height, seq: state.seq })
    }

    /// How many frames arrived so far, counting frames dropped after item close.
    pub fn frames_received(&self) -> u64 {
        self.state.lock().map(|state| state.received).unwrap_or(0)
    }

    /// Stops the capture session and releases the frame pool.
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Stops delivering frames and closes the session; safe to call from Drop.
    fn shutdown(&mut self) {
        if !self.running {
            return;
        }
        self.running = false;
        let _ = self.frame_pool.RemoveFrameArrived(self.frame_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
    }
}

impl Drop for MonitorCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Wraps the shared D3D11 device as a WinRT IDirect3DDevice.
#[allow(unsafe_code)]
fn wrap_device(device: &ID3D11Device) -> Result<IDirect3DDevice, CaptureError> {
    let dxgi_device: IDXGIDevice = with_context("failed to query IDXGIDevice from the shared device", device.cast())?;
    // SAFETY: dxgi_device is a live COM object; the call returns an owned IInspectable.
    let inspectable = with_context("failed to wrap the D3D11 device as an IDirect3DDevice", unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) })?;
    with_context("failed to cast the wrapped device to IDirect3DDevice", inspectable.cast())
}

/// Creates a capture item for a monitor handle.
#[allow(unsafe_code)]
fn item_for_monitor(handle: isize) -> Result<GraphicsCaptureItem, CaptureError> {
    let interop: IGraphicsCaptureItemInterop =
        with_context("failed to get the capture item interop factory", factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>())?;
    // SAFETY: interop is a live COM object and handle came from list_monitors.
    with_context("failed to create a capture item for the monitor", unsafe {
        interop.CreateForMonitor::<GraphicsCaptureItem>(HMONITOR(handle as *mut core::ffi::c_void))
    })
}

/// Runs on FrameArrived: stamps QPC, copies the frame into our own texture and records it.
#[allow(unsafe_code)]
fn on_frame_arrived(
    state: &Mutex<FrameState>,
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    sender: Ref<'_, Direct3D11CaptureFramePool>,
) -> windows::core::Result<()> {
    let qpc = qpc_now();
    let Ok(pool) = sender.ok() else {
        return Ok(());
    };
    let frame = match pool.TryGetNextFrame() {
        Ok(frame) => frame,
        Err(err) => {
            log::warn!("WGC TryGetNextFrame failed: {err}");
            return Ok(());
        }
    };
    let texture = match frame.Surface() {
        Ok(surface) => match surface_texture(&surface) {
            Ok(texture) => texture,
            Err(err) => {
                log::warn!("WGC frame surface conversion failed: {err}");
                return Ok(());
            }
        },
        Err(err) => {
            log::warn!("WGC frame Surface failed: {err}");
            return Ok(());
        }
    };
    // Release the WGC frame right away; we own a copy from here on.
    drop(frame);
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: texture is a live D3D11 texture; GetDesc only writes the descriptor.
    unsafe { texture.GetDesc(&mut desc) };
    let Ok(mut state) = state.lock() else {
        return Ok(());
    };
    state.received += 1;
    if state.closed {
        return Ok(());
    }
    let size_changed = match &state.latest {
        Some(current) => {
            let mut current_desc = D3D11_TEXTURE2D_DESC::default();
            // SAFETY: current is a live D3D11 texture; GetDesc only writes the descriptor.
            unsafe { current.GetDesc(&mut current_desc) };
            current_desc.Width != desc.Width || current_desc.Height != desc.Height
        }
        None => true,
    };
    if size_changed {
        let mut created: Option<ID3D11Texture2D> = None;
        // SAFETY: desc comes from the live source texture; created is a valid out pointer.
        if let Err(err) = unsafe { device.CreateTexture2D(&desc, None, Some(&mut created)) } {
            log::warn!("WGC latest frame texture creation failed: {err}");
            return Ok(());
        }
        state.latest = created;
        state.width = desc.Width as i32;
        state.height = desc.Height as i32;
    }
    if let Some(latest) = &state.latest {
        // SAFETY: both textures are live, on the same device, with matching size and format.
        unsafe { context.CopyResource(latest, &texture) };
        state.qpc = qpc;
        state.seq += 1;
    }
    Ok(())
}

/// Gets the D3D11 texture behind a captured WinRT surface.
#[allow(unsafe_code)]
fn surface_texture(surface: &windows::Graphics::DirectX::Direct3D11::IDirect3DSurface) -> Result<ID3D11Texture2D, CaptureError> {
    let access: IDirect3DDxgiInterfaceAccess = with_context("failed to query IDirect3DDxgiInterfaceAccess", surface.cast())?;
    // SAFETY: access is a live COM object; GetInterface returns an owned texture reference.
    with_context("failed to get the D3D11 texture from the capture surface", unsafe { access.GetInterface::<ID3D11Texture2D>() })
}

/// Runs on item Closed: the monitor is gone, log and stop delivering frames.
fn on_item_closed(state: &Mutex<FrameState>) -> windows::core::Result<()> {
    log::warn!("capture item closed: the monitor is unplugged or disabled");
    if let Ok(mut state) = state.lock() {
        state.closed = true;
    }
    Ok(())
}
