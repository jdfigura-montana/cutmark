//! The countdown overlay: a native Win32 window on its own thread, with no egui.
//!
//! It is a big number on a translucent dark circle in the middle of the recorded monitor. It never takes focus or
//! clicks, it sits on top of borderless games, and it is hidden from screen capture. While a countdown runs the
//! thread also owns a global Esc hotkey, which calls back so the controller can cancel the countdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

/// What the overlay calls when Esc is pressed while it is armed; it must be quick and must not block.
pub type EscapeCallback = Box<dyn Fn() + Send + Sync>;

/// A cloneable handle to the overlay thread; shows, updates and hides the overlay from any thread.
#[derive(Clone)]
pub struct CountdownHandle {
    hwnd: Arc<AtomicIsize>,
}

impl CountdownHandle {
    /// Starts the "countdown" thread with its hidden window; the overlay stays unavailable if that fails.
    ///
    /// `on_escape` is called on the overlay thread whenever Esc is pressed while the handle is armed.
    pub fn start(on_escape: EscapeCallback) -> Self {
        let handle = Self { hwnd: Arc::default() };
        let hwnd = Arc::clone(&handle.hwnd);
        if let Err(err) = std::thread::Builder::new().name("countdown".to_owned()).spawn(move || win::run(&hwnd, on_escape)) {
            log::error!("the countdown thread could not be started: {err}");
        }
        handle
    }

    /// Registers Esc as a global hotkey, for as long as the countdown runs; [`CountdownHandle::hide`] releases it.
    pub fn arm(&self) {
        self.post(win::Message::Arm, 0);
    }

    /// Shows the overlay with the number `n`, or changes the number when it is already shown.
    pub fn show(&self, n: u8) {
        self.post(win::Message::Show, usize::from(n));
    }

    /// Hides the overlay and releases the Esc hotkey; does nothing when neither is active.
    pub fn hide(&self) {
        self.post(win::Message::Hide, 0);
    }

    /// Sends a message to the overlay thread, or logs that the overlay is not available.
    fn post(&self, message: win::Message, value: usize) {
        match self.hwnd.load(Ordering::Acquire) {
            0 => log::warn!("the countdown overlay is not available"),
            hwnd => win::post(hwnd, message, value),
        }
    }
}

#[allow(unsafe_code, reason = "the overlay is a plain Win32 window: every call is a documented Win32 API on handles this module owns")]
mod win {
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use ui_kit::tokens;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        ANTIALIASED_QUALITY, BeginPaint, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
        DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect, GetMonitorInfoW, HBRUSH, HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
        MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey, VK_ESCAPE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, LWA_COLORKEY, LoadCursorW, MSG,
        PostMessageW, RegisterClassW, SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos,
        ShowWindow, TranslateMessage, WDA_EXCLUDEFROMCAPTURE, WM_APP, WM_ERASEBKGND, WM_HOTKEY, WM_PAINT, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    use super::EscapeCallback;

    /// The messages the handle posts to the overlay thread.
    #[derive(Clone, Copy)]
    pub(super) enum Message {
        /// Register the Esc hotkey.
        Arm,
        /// Show the overlay; `wparam` is the number.
        Show,
        /// Hide the overlay and release the Esc hotkey.
        Hide,
    }

    impl Message {
        /// The window message number.
        fn id(self) -> u32 {
            match self {
                Self::Arm => WM_APP_ARM,
                Self::Show => WM_APP_SHOW,
                Self::Hide => WM_APP_HIDE,
            }
        }
    }

    /// Posted to register the Esc hotkey.
    const WM_APP_ARM: u32 = WM_APP + 1;

    /// Posted to show the overlay; `wparam` is the number to draw.
    const WM_APP_SHOW: u32 = WM_APP + 2;

    /// Posted to hide the overlay and release the Esc hotkey.
    const WM_APP_HIDE: u32 = WM_APP + 3;

    /// The id of the Esc hotkey on this window.
    const ESC_HOTKEY_ID: i32 = 1;

    /// The size of the circle at 96 DPI, in pixels.
    const CIRCLE_PX: i32 = 260;

    /// How opaque the circle and number are, out of 255.
    const OVERLAY_ALPHA: u8 = 215;

    /// The color that is see-through around the circle; no palette color is this one.
    const KEY_COLOR: COLORREF = COLORREF(0x00FF_00FF);

    /// Everything the overlay thread keeps between messages.
    struct State {
        number: u8,
        visible: bool,
        hotkey: bool,
        font: isize,
        on_escape: EscapeCallback,
    }

    thread_local! {
        /// The overlay thread's state; only that thread's window procedure touches it.
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    /// Runs `change` on the overlay's state; `None` when there is no state or it is already borrowed.
    fn with_state<R>(change: impl FnOnce(&mut State) -> R) -> Option<R> {
        STATE.with(|cell| {
            let mut guard = cell.try_borrow_mut().ok()?;
            guard.as_mut().map(change)
        })
    }

    /// The window handle for a raw value.
    fn hwnd_of(raw: isize) -> HWND {
        HWND(raw as *mut c_void)
    }

    /// A token color as a GDI color.
    fn rgb(color: egui::Color32) -> COLORREF {
        COLORREF(u32::from(color.r()) | (u32::from(color.g()) << 8) | (u32::from(color.b()) << 16))
    }

    /// Scales a pixel size from 96 DPI to `dpi`.
    fn scale(value: i32, dpi: u32) -> i32 {
        value * i32::try_from(dpi).unwrap_or(96) / 96
    }

    /// Runs the overlay thread: creates the hidden window, publishes its handle and pumps messages forever.
    pub(super) fn run(hwnd_slot: &AtomicIsize, on_escape: EscapeCallback) {
        // SAFETY: the call only changes this thread's DPI awareness and takes a constant.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let hwnd = match create(on_escape) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                log::error!("the countdown window could not be created: {err}");
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

    /// Creates the hidden overlay window and stores the thread's state.
    fn create(on_escape: EscapeCallback) -> Result<HWND, String> {
        // SAFETY: every call gets valid arguments built here; the class and window belong to this thread.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|err| err.to_string())?;
            let class = w!("CutmarkCountdown");
            let cursor = LoadCursorW(None, IDC_ARROW).map_err(|err| err.to_string())?;
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance.into(), lpszClassName: class, hCursor: cursor, ..Default::default() };
            if RegisterClassW(&wc) == 0 {
                return Err("RegisterClassW failed".to_owned());
            }
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                w!("Countdown"),
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
            .map_err(|err| err.to_string())?;
            if let Err(err) = SetLayeredWindowAttributes(hwnd, KEY_COLOR, OVERLAY_ALPHA, LWA_COLORKEY | LWA_ALPHA) {
                log::warn!("the countdown overlay could not be made translucent: {err}");
            }
            if let Err(err) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                log::warn!("the countdown overlay could not be hidden from capture: {err}");
            }
            let state = State { number: 0, visible: false, hotkey: false, font: 0, on_escape };
            STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    *guard = Some(state);
                }
            });
            Ok(hwnd)
        }
    }

    /// Posts `message` with `value` to the overlay window.
    pub(super) fn post(hwnd: isize, message: Message, value: usize) {
        // SAFETY: posting a message with plain integer values to a window handle is always sound; a stale handle only fails.
        let posted = unsafe { PostMessageW(Some(hwnd_of(hwnd)), message.id(), WPARAM(value), LPARAM(0)) };
        if let Err(err) = posted {
            log::warn!("the countdown overlay could not be reached: {err}");
        }
    }

    /// The overlay's window procedure.
    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_APP_ARM => {
                arm(hwnd);
                LRESULT(0)
            }
            WM_APP_SHOW => {
                show(hwnd, u8::try_from(wparam.0).unwrap_or(u8::MAX));
                LRESULT(0)
            }
            WM_APP_HIDE => {
                hide(hwnd);
                LRESULT(0)
            }
            WM_HOTKEY if i32::try_from(wparam.0) == Ok(ESC_HOTKEY_ID) => {
                escape();
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

    /// Registers Esc as a global hotkey; a key that another program owns is logged and the countdown runs on.
    fn arm(hwnd: HWND) {
        if with_state(|state| state.hotkey) != Some(false) {
            return;
        }
        // SAFETY: the hotkey belongs to this thread's window; the key is a constant.
        let registered = unsafe { RegisterHotKey(Some(hwnd), ESC_HOTKEY_ID, HOT_KEY_MODIFIERS(0), u32::from(VK_ESCAPE.0)) };
        match registered {
            Ok(()) => {
                with_state(|state| state.hotkey = true);
            }
            Err(err) => log::warn!("Esc could not be registered to cancel the countdown: {err}"),
        }
    }

    /// Calls the Esc callback, but only while the hotkey is registered.
    fn escape() {
        let armed = with_state(|state| state.hotkey).unwrap_or(false);
        if !armed {
            return;
        }
        // The callback only queues a command, so holding the state borrow for it is brief; it never touches this window.
        with_state(|state| (state.on_escape)());
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

    /// Builds a bold Segoe UI font that is `height` pixels tall.
    fn make_font(height: i32) -> HFONT {
        // SAFETY: all arguments are plain values and a constant string; the font is owned by this module.
        unsafe { CreateFontW(-height, 0, 0, 0, 700, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Segoe UI")) }
    }

    /// Shows the overlay with `number`; when it is already up only the number changes.
    fn show(hwnd: HWND, number: u8) {
        let Some(was_visible) = with_state(|state| {
            state.number = number;
            std::mem::replace(&mut state.visible, true)
        }) else {
            return;
        };
        if !was_visible {
            place_and_show(hwnd);
        }
        // SAFETY: `hwnd` is this thread's overlay window; a null rectangle means the whole client area.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// Sizes the overlay for the monitor's DPI, centers it on the monitor and shows it on top without activating it.
    fn place_and_show(hwnd: HWND) {
        let area = monitor_rect();
        // SAFETY: the calls only move, size and show this thread's overlay, and replace the font it owns.
        unsafe {
            // Move onto the monitor first, so the window reports that monitor's DPI.
            let _ = SetWindowPos(hwnd, None, area.left, area.top, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            let dpi = GetDpiForWindow(hwnd);
            let size = scale(CIRCLE_PX, if dpi == 0 { 96 } else { dpi });
            let font = make_font(size * 11 / 20);
            let old_font = with_state(|state| std::mem::replace(&mut state.font, font.0 as isize)).unwrap_or(0);
            if old_font != 0 {
                let _ = DeleteObject(HFONT(old_font as *mut c_void).into());
            }
            let x = area.left + ((area.right - area.left) - size) / 2;
            let y = area.top + ((area.bottom - area.top) - size) / 2;
            if let Err(err) = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, size, size, SWP_SHOWWINDOW | SWP_NOACTIVATE) {
                log::warn!("the countdown overlay could not be shown: {err}");
            }
        }
    }

    /// Hides the overlay and releases the Esc hotkey.
    fn hide(hwnd: HWND) {
        let was_armed = with_state(|state| {
            state.visible = false;
            std::mem::replace(&mut state.hotkey, false)
        })
        .unwrap_or(false);
        // SAFETY: both calls take this thread's overlay window and plain values.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            if was_armed && let Err(err) = UnregisterHotKey(Some(hwnd), ESC_HOTKEY_ID) {
                log::warn!("the Esc hotkey of the countdown could not be released: {err}");
            }
        }
    }

    /// Paints the see-through background, the dark circle and the number.
    fn paint(hwnd: HWND) {
        let snapshot = with_state(|state| (state.number, state.font));
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: `ps` is a valid PAINTSTRUCT, and BeginPaint and EndPaint are paired on this window.
        unsafe {
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some((number, font)) = snapshot {
                let mut client = RECT::default();
                if GetClientRect(hwnd, &mut client).is_ok() {
                    draw(hdc, client, number, font);
                }
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    /// Draws the countdown on `hdc`: the key color everywhere, then the circle, then the number.
    fn draw(hdc: HDC, client: RECT, number: u8, font: isize) {
        // SAFETY: `hdc` is the live paint context; every pen and brush is created, selected, then restored and deleted here,
        // and the font (or 0, which selects nothing) is owned by this module.
        unsafe {
            let key: HBRUSH = CreateSolidBrush(KEY_COLOR);
            FillRect(hdc, &client, key);
            let _ = DeleteObject(key.into());

            let pen = CreatePen(PS_SOLID, 2, rgb(tokens::color::BG_HOVER));
            let brush: HBRUSH = CreateSolidBrush(rgb(tokens::color::BG_PANEL));
            let old_pen = SelectObject(hdc, pen.into());
            let old_brush = SelectObject(hdc, brush.into());
            let _ = Ellipse(hdc, client.left + 1, client.top + 1, client.right - 1, client.bottom - 1);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(pen.into());
            let _ = DeleteObject(brush.into());

            if font != 0 {
                SelectObject(hdc, HFONT(font as *mut c_void).into());
            }
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, rgb(tokens::color::TEXT_PRIMARY));
            let mut wide: Vec<u16> = number.to_string().encode_utf16().collect();
            let mut rect = client;
            DrawTextW(hdc, &mut wide, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }
}
