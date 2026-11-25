//! Prompter view window setup (spec "View window setup"): hidden from capture, never takes focus,
//! stays on top. Windows backend only; elsewhere every call reports it is unsupported.

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

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{GetLastError, HWND, SetLastError, WIN32_ERROR};
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GetWindowDisplayAffinity, GetWindowLongPtrW, GetWindowThreadProcessId, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOMOVE, SWP_NOSIZE, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, WDA_EXCLUDEFROMCAPTURE,
    };
    use windows::core::PCWSTR;

    use super::{NativeWindow, ViewStatus, ViewWindow, has_no_activate, has_topmost, with_no_activate};
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

        fn status(&self) -> ViewStatus {
            let ex_style = ex_style(*self).unwrap_or(0);
            ViewStatus {
                excluded: display_affinity(*self).is_ok_and(|a| a == WDA_EXCLUDEFROMCAPTURE.0),
                no_activate: has_no_activate(ex_style),
                topmost: has_topmost(ex_style),
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
    fn hidden_window_takes_all_three_calls() -> std::result::Result<(), Box<dyn std::error::Error>> {
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP};
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
        assert_eq!(window.status(), ViewStatus { excluded: true, no_activate: true, topmost: true });

        // SAFETY: this thread created the window and nothing uses it after this call.
        #[allow(unsafe_code)]
        unsafe { DestroyWindow(win::hwnd(window)) }?;
        Ok(())
    }
}
