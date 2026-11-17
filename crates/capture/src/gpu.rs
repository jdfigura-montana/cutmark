//! DXGI adapter enumeration and the shared D3D11 device.

use crate::{CaptureError, utf16_to_string, with_context};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_12_0, D3D_FEATURE_LEVEL_12_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_DESC1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIFactory1};
use windows::core::Interface;

/// Describes one DXGI adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    /// Zero based position among the listed (non software) adapters.
    pub index: usize,
    /// Adapter description, for example the GPU name.
    pub name: String,
    /// PCI vendor id, for example 0x10DE for NVIDIA.
    pub vendor_id: u32,
    /// Low part of the adapter LUID, used to match this adapter later.
    pub luid_low: u32,
    /// High part of the adapter LUID, used to match this adapter later.
    pub luid_high: i32,
    /// Dedicated video memory in MiB.
    pub dedicated_vram_mb: u64,
}

/// Lists the DXGI adapters in the system, without the Microsoft Basic Render Driver.
pub fn list_adapters() -> Result<Vec<AdapterInfo>, CaptureError> {
    let factory: IDXGIFactory1 = open_factory()?;
    let mut adapters = Vec::new();
    let mut index = 0u32;
    loop {
        let desc = match adapter_desc(&factory, index)? {
            Some(desc) => desc,
            None => break,
        };
        let name = utf16_to_string(&desc.Description);
        let is_basic = name.eq_ignore_ascii_case("Microsoft Basic Render Driver") || desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
        if !is_basic {
            let luid = desc.AdapterLuid;
            adapters.push(AdapterInfo {
                index: adapters.len(),
                name,
                vendor_id: desc.VendorId,
                luid_low: luid.LowPart,
                luid_high: luid.HighPart,
                dedicated_vram_mb: (desc.DedicatedVideoMemory / (1024 * 1024)) as u64,
            });
        }
        index += 1;
    }
    Ok(adapters)
}

/// The shared D3D11 device and its immediate context.
///
/// Clone is a COM refcount bump, so clones share one device. The windows crate
/// marks D3D11 interfaces Send and Sync, and the adapter info is plain data.
#[derive(Clone)]
pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    adapter: AdapterInfo,
}

impl Gpu {
    /// Creates the shared device on the given adapter, or on adapter 0 when None.
    pub fn create(adapter_index: Option<usize>) -> Result<Self, CaptureError> {
        let index = adapter_index.unwrap_or(0);
        let adapters = list_adapters()?;
        let adapter = adapters.get(index).ok_or(CaptureError::AdapterIndex(index))?;
        let (device, context) = create_device(index)?;
        Ok(Self { device, context, adapter: adapter.clone() })
    }

    /// The shared D3D11 device.
    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    /// The immediate context of the shared device.
    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    /// The adapter the device was created on.
    pub fn adapter(&self) -> &AdapterInfo {
        &self.adapter
    }
}

/// Opens a DXGI factory 1.
#[allow(unsafe_code)]
fn open_factory() -> Result<IDXGIFactory1, CaptureError> {
    // SAFETY: no input pointers; the call returns an owned factory reference.
    unsafe { CreateDXGIFactory1() }.map_err(|source| CaptureError::Windows { context: "failed to create a DXGI factory", source })
}

/// Reads one adapter description, or None past the end of the list.
#[allow(unsafe_code)]
fn adapter_desc(factory: &IDXGIFactory1, index: u32) -> Result<Option<DXGI_ADAPTER_DESC1>, CaptureError> {
    // SAFETY: factory is a live COM object and index is within the loop's range.
    let adapter = match unsafe { factory.EnumAdapters1(index) } {
        Ok(adapter) => adapter,
        Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => return Ok(None),
        Err(source) => return Err(CaptureError::Windows { context: "failed to enumerate a DXGI adapter", source }),
    };
    // SAFETY: adapter is a live COM object.
    let desc = unsafe { adapter.GetDesc1() }.map_err(|source| CaptureError::Windows { context: "failed to read the adapter description", source })?;
    Ok(Some(desc))
}

/// Creates the D3D11 device on the given adapter and enables multithread protection.
#[allow(unsafe_code)]
fn create_device(adapter_index: usize) -> Result<(ID3D11Device, ID3D11DeviceContext), CaptureError> {
    let factory: IDXGIFactory1 = open_factory()?;
    // SAFETY: factory is a live COM object and adapter_index was range checked.
    let adapter = unsafe { factory.EnumAdapters1(adapter_index as u32) }
        .map_err(|source| CaptureError::Windows { context: "failed to get the DXGI adapter for the device", source })?;
    let feature_levels = [D3D_FEATURE_LEVEL_12_1, D3D_FEATURE_LEVEL_12_0, D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    let mut context = None;
    let mut level = D3D_FEATURE_LEVEL(0);
    let flags = D3D11_CREATE_DEVICE_FLAG(D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0);
    // SAFETY: all out pointers are valid locals and the adapter is a live COM object.
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            flags,
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut level),
            Some(&mut context),
        )
    }
    .map_err(|source| CaptureError::Windows { context: "failed to create the D3D11 device", source })?;
    if level.0 < D3D_FEATURE_LEVEL_11_0.0 {
        return Err(CaptureError::Missing("adapter does not support feature level 11_0"));
    }
    let device = device.ok_or(CaptureError::Missing("D3D11CreateDevice returned no device"))?;
    let context = context.ok_or(CaptureError::Missing("D3D11CreateDevice returned no context"))?;
    enable_multithread(&device)?;
    Ok((device, context))
}

/// Turns on multithread protection so FFmpeg and WGC can use the device from other threads.
#[allow(unsafe_code)]
fn enable_multithread(device: &ID3D11Device) -> Result<(), CaptureError> {
    let multithread: ID3D11Multithread = with_context("failed to query ID3D11Multithread", device.cast())?;
    // SAFETY: device is a live COM object.
    let enabled = unsafe { multithread.SetMultithreadProtected(true) };
    if enabled.as_bool() { Ok(()) } else { Err(CaptureError::Missing("SetMultithreadProtected returned false")) }
}
