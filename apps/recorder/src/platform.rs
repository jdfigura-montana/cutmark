//! Windows-specific window helpers.

use raw_window_handle::HasWindowHandle;

/// Hides a window from screen capture, or shows it to capture again, on a raw window handle value.
pub fn set_capture_exclusion(raw_hwnd: isize, exclude: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        set_display_affinity(raw_hwnd, exclude)
    }
    #[cfg(not(windows))]
    {
        let _ = (raw_hwnd, exclude);
        Ok(())
    }
}

/// Gets the raw Win32 window handle of a freshly created window as an `isize`.
pub fn raw_hwnd(handle: &impl HasWindowHandle) -> Result<isize, String> {
    #[cfg(windows)]
    {
        use raw_window_handle::RawWindowHandle;

        let window = handle.window_handle().map_err(|err| format!("no window handle: {err}"))?;
        match window.as_raw() {
            RawWindowHandle::Win32(win32) => Ok(win32.hwnd.get()),
            other => Err(format!("not a Win32 window handle: {other:?}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Ok(0)
    }
}

/// Un-minimizes, shows and focuses the window so eframe starts running frames again.
pub fn show_and_focus(raw_hwnd: isize) {
    #[cfg(windows)]
    show_and_focus_win32(raw_hwnd);
    #[cfg(not(windows))]
    let _ = raw_hwnd;
}

/// Calls ShowWindow(SW_SHOW), ShowWindow(SW_RESTORE) and SetForegroundWindow on a raw window handle value.
#[cfg(windows)]
#[allow(unsafe_code, reason = "Win32 calls that un-minimize, show and focus the window hidden to the tray")]
fn show_and_focus_win32(raw_hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow};

    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    // SAFETY: `raw_hwnd` was taken from the live creation context of this window, so the window exists.
    // Each call only reads the handle and a plain flag value, and it hands back no pointers.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Calls `SetWindowDisplayAffinity` with `WDA_EXCLUDEFROMCAPTURE` or `WDA_NONE` on a raw window handle value.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the only FFI call in the Recorder besides the tray helpers: a Win32 flag that excludes a window from capture")]
fn set_display_affinity(raw_hwnd: isize, exclude: bool) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    // SAFETY: `raw_hwnd` was taken from the live creation context of this window, so the window exists.
    // The call only reads the handle and a plain flag value, and it hands back no pointers.
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }.map_err(|err| format!("SetWindowDisplayAffinity failed: {err}"))
}
