//! Windows-specific window helpers.

use raw_window_handle::HasWindowHandle;

/// Hides a window from screen capture; every Recorder window or popup must call this when it is created.
pub fn exclude_from_capture(handle: &impl HasWindowHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        use raw_window_handle::RawWindowHandle;

        let window = handle.window_handle().map_err(|err| format!("no window handle: {err}"))?;
        match window.as_raw() {
            RawWindowHandle::Win32(win32) => set_display_affinity(win32.hwnd.get()),
            other => Err(format!("not a Win32 window handle: {other:?}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Ok(())
    }
}

/// Calls `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)` on a raw window handle value.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the only FFI call in the Recorder: a Win32 flag that excludes a window from capture")]
fn set_display_affinity(raw_hwnd: isize) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE};

    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    // SAFETY: `raw_hwnd` comes from a live `HasWindowHandle` that the caller borrows for the whole call, so
    // the window exists. The call only reads the handle and a plain flag value, and it hands back no pointers.
    unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) }.map_err(|err| format!("SetWindowDisplayAffinity failed: {err}"))
}
