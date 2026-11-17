//! Monitor enumeration via EnumDisplayMonitors.

use crate::{CaptureError, utf16_to_string};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::core::BOOL;

/// Value of MONITORINFOF_PRIMARY from winuser.h; kept local to avoid a whole feature.
const MONITORINFOF_PRIMARY: u32 = 0x0000_0001;

/// Describes one display monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Zero based position in the list, primary monitor first.
    pub index: usize,
    /// Display device name, for example \\.\DISPLAY1.
    pub name: String,
    /// Left edge of the monitor in virtual desktop coordinates.
    pub x: i32,
    /// Top edge of the monitor in virtual desktop coordinates.
    pub y: i32,
    /// Monitor width in pixels.
    pub width: i32,
    /// Monitor height in pixels.
    pub height: i32,
    /// True for the primary monitor.
    pub primary: bool,
    /// The HMONITOR handle as an isize.
    pub handle: isize,
}

/// Lists the monitors in the system, primary monitor first.
pub fn list_monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    let mut monitors = Vec::new();
    enum_monitors(&mut monitors)?;
    monitors.sort_by_key(|monitor| !monitor.primary);
    for (index, monitor) in monitors.iter_mut().enumerate() {
        monitor.index = index;
    }
    Ok(monitors)
}

/// Runs the monitor enumeration, filling `monitors`.
#[allow(unsafe_code)]
fn enum_monitors(monitors: &mut Vec<MonitorInfo>) -> Result<(), CaptureError> {
    // SAFETY: the callback only uses lparam to write into `monitors`, which
    // outlives the call.
    let ok = unsafe { EnumDisplayMonitors(None, None, Some(enum_proc), LPARAM(monitors as *mut Vec<MonitorInfo> as isize)) };
    if ok.as_bool() { Ok(()) } else { Err(CaptureError::Missing("monitor enumeration failed")) }
}

/// EnumDisplayMonitors callback: reads one monitor and appends it.
///
/// SAFETY: `lparam` must be a valid pointer to a `Vec<MonitorInfo>` that stays
/// alive for the whole enumeration.
#[allow(unsafe_code)]
unsafe extern "system" fn enum_proc(hmonitor: HMONITOR, _hdc: HDC, _rect: *mut RECT, lparam: LPARAM) -> BOOL {
    let monitors = unsafe { &mut *(lparam.0 as *mut Vec<MonitorInfo>) };
    match monitor_info(hmonitor) {
        Ok(info) => {
            monitors.push(info);
            BOOL(1)
        }
        Err(_) => BOOL(0),
    }
}

/// Builds a MonitorInfo for one monitor handle.
#[allow(unsafe_code)]
fn monitor_info(hmonitor: HMONITOR) -> Result<MonitorInfo, CaptureError> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = core::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: hmonitor is a live monitor handle and info is a valid
    // MONITORINFOEXW with its size set.
    let ok = unsafe { GetMonitorInfoW(hmonitor, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
    if !ok.as_bool() {
        return Err(CaptureError::Missing("GetMonitorInfoW failed"));
    }
    let rect = info.monitorInfo.rcMonitor;
    Ok(MonitorInfo {
        index: 0,
        name: utf16_to_string(&info.szDevice),
        x: rect.left,
        y: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        handle: hmonitor.0 as isize,
    })
}
