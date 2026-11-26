//! Prompter view window setup (spec "View window setup"): hidden from capture, never takes focus,
//! stays on top, see-through where nothing is drawn. Windows backend only; elsewhere every call
//! reports it is unsupported.

use crate::error::Result;

/// A top-level window handle, kept as an integer so callers need no OS types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeWindow(pub isize);

/// What a window currently has applied, read back from the window itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewStatus {
    /// The window is excluded from screen capture.
    pub excluded: bool,
    /// The window carries both the no-activate and the tool-window styles.
    pub no_activate: bool,
    /// The window is in the topmost band.
    pub topmost: bool,
    /// The window's own GDI surface, under what the renderer draws, reads as cleared at a few
    /// sample points. When it does not, a transparent view shows white behind it.
    pub backing_is_clear: bool,
}

/// The OS setup a prompter view window needs.
pub trait ViewWindow {
    /// This process's top-level window with this title. Windows compares titles without regard to
    /// case and returns one match, so a title must be unique: if the match belongs to another
    /// process, the answer is `None`.
    fn find_own_window(title: &str) -> Option<NativeWindow>;

    /// Removes the window from screen capture, then reads the setting back to confirm it.
    fn exclude_from_capture(&self) -> Result<()>;

    /// Stops the window from taking focus and marks it as a tool window.
    fn set_no_activate(&self) -> Result<()>;

    /// Puts the window back on top of the topmost band without activating it.
    fn reassert_topmost(&self) -> Result<()>;

    /// Clears the window's GDI surface so the desktop shows through the renderer's transparent
    /// pixels. Windows fills it white when the window is shown, so call this on a visible window,
    /// and again after anything that shows it. On a hidden window it clears nothing.
    fn clear_backing(&self) -> Result<()>;

    /// An opaque id for the monitor the window is mostly on, for noticing that it moved.
    fn monitor(&self) -> Option<isize>;

    /// Where the mouse pointer is, in physical pixels from the top left of the window's client
    /// area. A pointer outside the window gives a negative value or one past its size. It can be
    /// read during a drag from another program, when the window gets no mouse messages.
    fn cursor_position(&self) -> Option<(i32, i32)>;

    /// Reads the current state. A value that cannot be read counts as not applied.
    fn status(&self) -> ViewStatus;
}

// The extended-style bits, spelled out so the bit math can be tested on every platform.
#[cfg(any(windows, test))]
const EX_TOPMOST: u32 = 0x0000_0008;
#[cfg(any(windows, test))]
const EX_TOOL_WINDOW: u32 = 0x0000_0080;
#[cfg(any(windows, test))]
const EX_NO_ACTIVATE: u32 = 0x0800_0000;

/// `ex_style` with the never-focus bits added and every other bit kept.
#[cfg(any(windows, test))]
fn with_no_activate(ex_style: u32) -> u32 {
    ex_style | EX_NO_ACTIVATE | EX_TOOL_WINDOW
}

/// Whether both never-focus bits are set.
#[cfg(any(windows, test))]
fn has_no_activate(ex_style: u32) -> bool {
    with_no_activate(ex_style) == ex_style
}

/// Whether the topmost bit is set.
#[cfg(any(windows, test))]
fn has_topmost(ex_style: u32) -> bool {
    ex_style & EX_TOPMOST != 0
}

/// Where the backing is read back: the four corners, one pixel in, and the center.
#[cfg(any(windows, test))]
fn sample_points(width: i32, height: i32) -> [(i32, i32); 5] {
    let (right, bottom) = ((width - 2).max(0), (height - 2).max(0));
    let (left, top) = (1.min(right), 1.min(bottom));
    [(left, top), (right, top), (left, bottom), (right, bottom), (width / 2, height / 2)]
}

/// Whether every sample is a cleared pixel. A cleared pixel is black (with zero alpha); the value
/// for an unreadable pixel is not black, so it counts as not clear.
#[cfg(any(windows, test))]
fn all_clear(colors: impl IntoIterator<Item = u32>) -> bool {
    colors.into_iter().all(|color| color == 0)
}

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{GetLastError, HWND, SetLastError, WIN32_ERROR};
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::{BLACKNESS, GetDC, GetPixel, HDC, MONITOR_DEFAULTTONEAREST, MonitorFromWindow, PatBlt, ReleaseDC, ScreenToClient};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GCLP_HBRBACKGROUND, GWL_EXSTYLE, GetClientRect, GetCursorPos, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowThreadProcessId,
        HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetClassLongPtrW, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
        WDA_EXCLUDEFROMCAPTURE,
    };
    use windows::core::PCWSTR;

    use super::{NativeWindow, ViewStatus, ViewWindow, all_clear, has_no_activate, has_topmost, sample_points, with_no_activate};
    use crate::error::{PlatformError, Result};

    impl ViewWindow for NativeWindow {
        fn find_own_window(title: &str) -> Option<NativeWindow> {
            // An interior NUL would cut the title short and match a different window.
            if title.contains('\0') {
                return None;
            }
            let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is NUL-terminated and outlives the call; a null class matches any.
            #[allow(unsafe_code)]
            let hwnd = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(wide.as_ptr())) }.ok()?;
            let mut process_id = 0u32;
            // SAFETY: `process_id` is a live, writable u32; a stale handle only makes the call fail.
            #[allow(unsafe_code)]
            unsafe {
                GetWindowThreadProcessId(hwnd, Some(&mut process_id))
            };
            (process_id == std::process::id()).then_some(NativeWindow(hwnd.0 as isize))
        }

        fn exclude_from_capture(&self) -> Result<()> {
            // SAFETY: the call takes only values; a bad handle makes it return an error.
            #[allow(unsafe_code)]
            unsafe { SetWindowDisplayAffinity(hwnd(*self), WDA_EXCLUDEFROMCAPTURE) }.map_err(|e| win32_error("SetWindowDisplayAffinity", &e))?;
            let affinity = display_affinity(*self)?;
            if affinity == WDA_EXCLUDEFROMCAPTURE.0 { Ok(()) } else { Err(PlatformError::ExclusionNotApplied { affinity }) }
        }

        fn set_no_activate(&self) -> Result<()> {
            let ex_style = ex_style(*self)?;
            set_ex_style(*self, with_no_activate(ex_style))?;
            // SAFETY: the call takes only values; a bad handle makes it return an error.
            #[allow(unsafe_code)]
            unsafe { SetWindowPos(hwnd(*self), None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) }
                .map_err(|e| win32_error("SetWindowPos", &e))
        }

        fn reassert_topmost(&self) -> Result<()> {
            // SAFETY: the call takes only values; a bad handle makes it return an error.
            #[allow(unsafe_code)]
            unsafe { SetWindowPos(hwnd(*self), Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) }
                .map_err(|e| win32_error("SetWindowPos", &e))
        }

        fn clear_backing(&self) -> Result<()> {
            // The brush belongs to the window class, not the window, and winit uses one class for
            // all its windows, so the Library may share this change. winit 0.30 already registers
            // the class without a brush, and an empty brush does not stop the white fill when a
            // window is shown: the PatBlt below is what clears it.
            remove_class_background(*self)?;
            let (width, height) = client_size(*self)?;
            let dc = WindowDc::of(*self)?;
            // SAFETY: the DC stays valid while `dc` lives; the call takes only values besides it.
            #[allow(unsafe_code)]
            let painted = unsafe { PatBlt(dc.dc, 0, 0, width, height, BLACKNESS) };
            if painted.as_bool() { Ok(()) } else { Err(last_error("PatBlt")) }
        }

        fn monitor(&self) -> Option<isize> {
            // SAFETY: the call takes only values; a bad handle gets the nearest or no monitor.
            #[allow(unsafe_code)]
            let monitor = unsafe { MonitorFromWindow(hwnd(*self), MONITOR_DEFAULTTONEAREST) };
            (!monitor.is_invalid()).then_some(monitor.0 as isize)
        }

        fn cursor_position(&self) -> Option<(i32, i32)> {
            let mut point = POINT::default();
            // SAFETY: `point` is a live, writable POINT.
            #[allow(unsafe_code)]
            unsafe { GetCursorPos(&mut point) }.ok()?;
            // SAFETY: `point` is a live, writable POINT; a bad handle makes the call fail.
            #[allow(unsafe_code)]
            let converted = unsafe { ScreenToClient(hwnd(*self), &mut point) };
            converted.as_bool().then_some((point.x, point.y))
        }

        fn status(&self) -> ViewStatus {
            let ex_style = ex_style(*self).unwrap_or(0);
            ViewStatus {
                excluded: display_affinity(*self).is_ok_and(|a| a == WDA_EXCLUDEFROMCAPTURE.0),
                no_activate: has_no_activate(ex_style),
                topmost: has_topmost(ex_style),
                backing_is_clear: backing_is_clear(*self),
            }
        }
    }

    pub(super) fn hwnd(window: NativeWindow) -> HWND {
        HWND(window.0 as *mut core::ffi::c_void)
    }

    /// The Win32 code inside a `windows` error; any other HRESULT is passed through whole.
    fn win32_error(call: &'static str, error: &windows::core::Error) -> PlatformError {
        let code = WIN32_ERROR::from_error(error).map_or(error.code().0 as u32, |e| e.0);
        PlatformError::Win32 { call, code }
    }

    /// A window's DC, released when dropped.
    pub(super) struct WindowDc {
        window: HWND,
        pub(super) dc: HDC,
    }

    impl WindowDc {
        pub(super) fn of(window: NativeWindow) -> Result<Self> {
            let window = hwnd(window);
            // SAFETY: the call takes only a value; a bad handle makes it return a null DC.
            #[allow(unsafe_code)]
            let dc = unsafe { GetDC(Some(window)) };
            if dc.is_invalid() { Err(last_error("GetDC")) } else { Ok(Self { window, dc }) }
        }
    }

    impl Drop for WindowDc {
        fn drop(&mut self) {
            // SAFETY: `dc` came from GetDC for this window and is released only here.
            #[allow(unsafe_code)]
            unsafe {
                ReleaseDC(Some(self.window), self.dc)
            };
        }
    }

    fn last_error(call: &'static str) -> PlatformError {
        // SAFETY: the call takes nothing and reads this thread's last-error value.
        #[allow(unsafe_code)]
        let error = unsafe { GetLastError() };
        PlatformError::Win32 { call, code: error.0 }
    }

    fn remove_class_background(window: NativeWindow) -> Result<()> {
        // The call returns the previous brush, which may be 0, so the last-error rule applies.
        // SAFETY: both calls take only values; a null brush is a documented value for the class.
        #[allow(unsafe_code)]
        let (previous, error) = unsafe {
            SetLastError(WIN32_ERROR(0));
            let previous = SetClassLongPtrW(hwnd(window), GCLP_HBRBACKGROUND, 0);
            (previous, GetLastError())
        };
        if previous == 0 && error.0 != 0 {
            return Err(PlatformError::Win32 { call: "SetClassLongPtrW", code: error.0 });
        }
        Ok(())
    }

    fn client_size(window: NativeWindow) -> Result<(i32, i32)> {
        let mut rect = RECT::default();
        // SAFETY: `rect` is a live, writable RECT; a bad handle makes the call return an error.
        #[allow(unsafe_code)]
        unsafe { GetClientRect(hwnd(window), &mut rect) }.map_err(|e| win32_error("GetClientRect", &e))?;
        Ok((rect.right - rect.left, rect.bottom - rect.top))
    }

    /// A handful of pixel reads, cheap enough for the read-backs. It sees only the GDI surface,
    /// not what the renderer composes above it.
    fn backing_is_clear(window: NativeWindow) -> bool {
        let Ok((width, height)) = client_size(window) else {
            return false;
        };
        let Ok(dc) = WindowDc::of(window) else {
            return false;
        };
        width > 0
            && height > 0
            && all_clear(sample_points(width, height).into_iter().map(|(x, y)| {
                // SAFETY: the DC stays valid while `dc` lives; unreadable points return CLR_INVALID.
                #[allow(unsafe_code)]
                let color = unsafe { GetPixel(dc.dc, x, y) };
                color.0
            }))
    }

    fn display_affinity(window: NativeWindow) -> Result<u32> {
        let mut affinity = 0u32;
        // SAFETY: `affinity` is a live, writable u32; a bad handle makes the call return an error.
        #[allow(unsafe_code)]
        unsafe { GetWindowDisplayAffinity(hwnd(window), &mut affinity) }.map_err(|e| win32_error("GetWindowDisplayAffinity", &e))?;
        Ok(affinity)
    }

    fn ex_style(window: NativeWindow) -> Result<u32> {
        // A style of 0 is legal, so only the last-error value tells a failure apart.
        // SAFETY: both calls take only values.
        #[allow(unsafe_code)]
        let (style, error) = unsafe {
            SetLastError(WIN32_ERROR(0));
            let style = GetWindowLongPtrW(hwnd(window), GWL_EXSTYLE);
            (style, GetLastError())
        };
        if style == 0 && error.0 != 0 {
            return Err(PlatformError::Win32 { call: "GetWindowLongPtrW", code: error.0 });
        }
        Ok(style as u32)
    }

    fn set_ex_style(window: NativeWindow, style: u32) -> Result<()> {
        // The call returns the previous style, which may be 0, so the same last-error rule applies.
        // SAFETY: both calls take only values; GWL_EXSTYLE holds plain style bits, not a pointer.
        #[allow(unsafe_code)]
        let (previous, error) = unsafe {
            SetLastError(WIN32_ERROR(0));
            let previous = SetWindowLongPtrW(hwnd(window), GWL_EXSTYLE, style as isize);
            (previous, GetLastError())
        };
        if previous == 0 && error.0 != 0 {
            return Err(PlatformError::Win32 { call: "SetWindowLongPtrW", code: error.0 });
        }
        Ok(())
    }
}

#[cfg(not(windows))]
impl ViewWindow for NativeWindow {
    fn find_own_window(_title: &str) -> Option<NativeWindow> {
        None
    }

    fn exclude_from_capture(&self) -> Result<()> {
        Err(crate::error::PlatformError::Unsupported)
    }

    fn set_no_activate(&self) -> Result<()> {
        Err(crate::error::PlatformError::Unsupported)
    }

    fn reassert_topmost(&self) -> Result<()> {
        Err(crate::error::PlatformError::Unsupported)
    }

    fn clear_backing(&self) -> Result<()> {
        Err(crate::error::PlatformError::Unsupported)
    }

    fn monitor(&self) -> Option<isize> {
        None
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        None
    }

    fn status(&self) -> ViewStatus {
        ViewStatus::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_the_no_activate_styles_keeps_existing_bits() {
        let existing = 0x0004_0000 | 0x0008_0000 | EX_TOPMOST;
        let styled = with_no_activate(existing);
        assert_eq!(styled & existing, existing);
        assert_eq!(styled, existing | EX_NO_ACTIVATE | EX_TOOL_WINDOW);
        assert_eq!(with_no_activate(styled), styled);
    }

    #[test]
    fn no_activate_needs_both_bits() {
        assert!(!has_no_activate(0));
        assert!(!has_no_activate(EX_NO_ACTIVATE));
        assert!(!has_no_activate(EX_TOOL_WINDOW));
        assert!(has_no_activate(with_no_activate(0)));
    }

    #[test]
    fn topmost_is_read_from_its_own_bit() {
        assert!(!has_topmost(with_no_activate(0)));
        assert!(has_topmost(EX_TOPMOST));
    }

    #[test]
    fn backing_samples_stay_inside_the_window() {
        assert_eq!(sample_points(760, 420), [(1, 1), (758, 1), (1, 418), (758, 418), (380, 210)]);
        for size in [1, 2, 3] {
            assert!(sample_points(size, size).iter().all(|&(x, y)| (0..size).contains(&x) && (0..size).contains(&y)));
        }
    }

    #[test]
    fn the_backing_is_clear_only_if_every_sample_is_black() {
        assert!(all_clear([0, 0, 0]));
        assert!(!all_clear([0, 0x00FF_FFFF, 0]));
        // CLR_INVALID, what an unreadable pixel returns.
        assert!(!all_clear([0, u32::MAX]));
    }

    #[cfg(windows)]
    #[test]
    fn style_bits_match_the_windows_constants() {
        use windows::Win32::UI::WindowsAndMessaging::{WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST};

        assert_eq!(EX_TOPMOST, WS_EX_TOPMOST.0);
        assert_eq!(EX_TOOL_WINDOW, WS_EX_TOOLWINDOW.0);
        assert_eq!(EX_NO_ACTIVATE, WS_EX_NOACTIVATE.0);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "creates a real window; run by hand on a Windows desktop"]
    fn real_window_takes_every_call() -> std::result::Result<(), Box<dyn std::error::Error>> {
        use windows::Win32::Graphics::Gdi::{PatBlt, WHITENESS};
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, SW_SHOWNOACTIVATE, ShowWindow, WINDOW_EX_STYLE, WS_POPUP};
        use windows::core::{HSTRING, w};

        let title = format!("Cutmark Prompter platform test {}", std::process::id());
        // SAFETY: both strings outlive the call, STATIC is a built-in class, and the optional
        // handles and pointer are all absent. Without WS_VISIBLE the window stays hidden.
        #[allow(unsafe_code)]
        let created =
            unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), &HSTRING::from(title.as_str()), WS_POPUP, 0, 0, 200, 100, None, None, None, None) }?;

        let window = NativeWindow::find_own_window(&title).ok_or("the test window was not found")?;
        assert_eq!(window, NativeWindow(created.0 as isize));
        assert_eq!(window.status(), ViewStatus::default());

        window.exclude_from_capture()?;
        window.set_no_activate()?;
        window.reassert_topmost()?;
        // A hidden window has no pixels to read, so its backing does not count as clear.
        assert_eq!(window.status(), ViewStatus { excluded: true, no_activate: true, topmost: true, backing_is_clear: false });
        assert!(window.monitor().is_some());

        // The backing needs a visible window. Paint it white, then clear it.
        // SAFETY: this thread created the window, which stays alive until the end of the test.
        #[allow(unsafe_code)]
        let _ = unsafe { ShowWindow(created, SW_SHOWNOACTIVATE) };
        {
            let dc = win::WindowDc::of(window)?;
            // SAFETY: the DC stays valid while `dc` lives.
            #[allow(unsafe_code)]
            let painted = unsafe { PatBlt(dc.dc, 0, 0, 200, 100, WHITENESS) };
            assert!(painted.as_bool());
        }
        assert!(!window.status().backing_is_clear);
        window.clear_backing()?;
        assert!(window.status().backing_is_clear);

        // SAFETY: this thread created the window and nothing uses it after this call.
        #[allow(unsafe_code)]
        unsafe { DestroyWindow(win::hwnd(window)) }?;
        Ok(())
    }
}
