//! The sync flash: a native Win32 window on its own thread that covers the recorded monitor in pure white for a moment.
//!
//! The camera sync test records the flash on the screen capture and, through the room, on the camera. So unlike the
//! countdown overlay this window is NOT hidden from capture. It stays on top, never takes focus and lets clicks through.

use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

/// A cloneable handle to the flash thread; flashes the screen from any thread.
#[derive(Clone)]
pub struct SyncFlashHandle {
    hwnd: Arc<AtomicIsize>,
}

impl SyncFlashHandle {
    /// Starts the "sync-flash" thread with its hidden window; the flash stays unavailable if that fails.
    pub fn start() -> Self {
        let handle = Self { hwnd: Arc::default() };
        let hwnd = Arc::clone(&handle.hwnd);
        if let Err(err) = std::thread::Builder::new().name("sync-flash".to_owned()).spawn(move || win::run(&hwnd)) {
            log::error!("the sync flash thread could not be started: {err}");
        }
        handle
    }

    /// Covers the recorded monitor in white for `ms` milliseconds, then hides the window again.
    pub fn flash(&self, ms: u32) {
        match self.hwnd.load(Ordering::Acquire) {
            0 => log::warn!("the sync flash is not available"),
            hwnd => win::post(hwnd, ms),
        }
    }
}

#[allow(unsafe_code, reason = "the flash is a plain Win32 window: every call is a documented Win32 API on handles this module owns")]
mod win {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetMonitorInfoW, HBRUSH, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint,
        PAINTSTRUCT, UpdateWindow,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, HWND_TOPMOST, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostMessageW,
        RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos, ShowWindow, TranslateMessage, WM_APP, WM_ERASEBKGND, WM_PAINT,
        WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    /// Posted to flash the screen; `wparam` is how long, in milliseconds.
    const WM_APP_FLASH: u32 = WM_APP + 1;

    /// The id of the timer that ends a flash.
    const END_TIMER_ID: usize = 1;

    /// The longest a flash may last, in milliseconds, so a bad value cannot leave the screen white.
    const MAX_FLASH_MS: u32 = 2000;

    /// White, as a GDI color.
    const WHITE: COLORREF = COLORREF(0x00FF_FFFF);

    /// The window handle for a raw value.
    fn hwnd_of(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// Runs the flash thread: creates the hidden window, publishes its handle and pumps messages forever.
    pub(super) fn run(hwnd_slot: &AtomicIsize) {
        // SAFETY: the call only changes this thread's DPI awareness and takes a constant.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let hwnd = match create() {
            Ok(hwnd) => hwnd,
            Err(err) => {
                log::error!("the sync flash window could not be created: {err}");
                return;
            }
        };
        hwnd_slot.store(hwnd.0 as isize, Ordering::Release);
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid MSG owned by this thread, and the loop only passes it back to the system.
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }

    /// Creates the hidden, topmost, non-activating, click-through window. It is not excluded from capture.
    fn create() -> Result<HWND, String> {
        // SAFETY: every call gets valid arguments built here; the class and window belong to this thread.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|err| err.to_string())?;
            let class = w!("CutmarkSyncFlash");
            let cursor = LoadCursorW(None, IDC_ARROW).map_err(|err| err.to_string())?;
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance.into(), lpszClassName: class, hCursor: cursor, ..Default::default() };
            if RegisterClassW(&wc) == 0 {
                return Err("RegisterClassW failed".to_owned());
            }
            CreateWindowExW(
                WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("Sync flash"),
                WS_POPUP,
                0,
                0,
                100,
                100,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|err| err.to_string())
        }
    }

    /// Posts a flash of `ms` milliseconds to the flash window.
    pub(super) fn post(hwnd: isize, ms: u32) {
        // SAFETY: posting a message with plain integer values to a window handle is always sound; a stale handle only fails.
        let posted = unsafe { PostMessageW(Some(hwnd_of(hwnd)), WM_APP_FLASH, WPARAM(ms as usize), LPARAM(0)) };
        if let Err(err) = posted {
            log::warn!("the sync flash could not be reached: {err}");
        }
    }

    /// The flash window's window procedure.
    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_APP_FLASH => {
                show(hwnd, u32::try_from(wparam.0).unwrap_or(MAX_FLASH_MS).min(MAX_FLASH_MS));
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == END_TIMER_ID => {
                hide(hwnd);
                LRESULT(0)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => default_proc(hwnd, msg, wparam, lparam),
        }
    }

    /// Calls the system's default window procedure.
    fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: `hwnd` is the handle the system passed to this window's procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// The monitor that is recorded: the primary one.
    fn monitor_rect() -> RECT {
        // SAFETY: `info` is a valid MONITORINFO with its size set, and the point is a constant.
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() { info.rcMonitor } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } }
        }
    }

    /// Covers the monitor on top without activating the window, paints it at once and starts the timer that ends the flash.
    fn show(hwnd: HWND, ms: u32) {
        let area = monitor_rect();
        // SAFETY: the calls only move, show, repaint and time this thread's own window.
        unsafe {
            let placed =
                SetWindowPos(hwnd, Some(HWND_TOPMOST), area.left, area.top, area.right - area.left, area.bottom - area.top, SWP_SHOWWINDOW | SWP_NOACTIVATE);
            if let Err(err) = placed {
                log::warn!("the sync flash could not be shown: {err}");
                return;
            }
            let _ = UpdateWindow(hwnd);
            if SetTimer(Some(hwnd), END_TIMER_ID, ms.max(1), None) == 0 {
                log::warn!("the sync flash could not be timed, so it is hidden at once");
                hide(hwnd);
            }
        }
    }

    /// Hides the window and stops its timer.
    fn hide(hwnd: HWND) {
        // SAFETY: both calls take this thread's own window and plain values.
        unsafe {
            let _ = KillTimer(Some(hwnd), END_TIMER_ID);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    /// Paints the whole window white.
    fn paint(hwnd: HWND) {
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: `ps` is a valid PAINTSTRUCT, BeginPaint and EndPaint are paired on this window, and the brush is
        // created, used and deleted here.
        unsafe {
            let hdc = BeginPaint(hwnd, &mut ps);
            let mut client = RECT::default();
            if GetClientRect(hwnd, &mut client).is_ok() {
                let brush: HBRUSH = CreateSolidBrush(WHITE);
                FillRect(hdc, &client, brush);
                let _ = DeleteObject(brush.into());
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }
}
