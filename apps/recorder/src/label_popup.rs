//! The Shift+F9 label popup: a small native Win32 window on its own thread, with no egui.
//!
//! It works while the main window is hidden and sits on top of borderless games. It is hidden from
//! screen capture. A save sends a `LabeledMark` command timed at the Shift+F9 press, never at the save.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use ::marks::MarkType;

use crate::engine::bus::BusSender;

/// The six categories the popup offers, in key order, with the mark type each one adds.
pub const CATEGORIES: [(&str, MarkType); 6] = [
    ("Highlight", MarkType::Highlight),
    ("Cut", MarkType::Cut),
    ("Retake", MarkType::Retake),
    ("Chapter", MarkType::Chapter),
    ("Note", MarkType::Note),
    ("B-roll", MarkType::BRoll),
];

/// The category selected when the popup opens: Note.
const DEFAULT_CATEGORY: usize = 4;

/// How long the popup waits for a key before it saves what it has, in milliseconds.
const IDLE_SAVE_MS: u32 = 15_000;

/// How long `commit_now` waits for the popup thread, in milliseconds.
const COMMIT_WAIT_MS: u32 = 500;

/// The category after `current` when cycling with Tab (or before it with Shift+Tab).
fn next_category(current: usize, backwards: bool) -> usize {
    let count = CATEGORIES.len();
    if backwards { (current + count - 1) % count } else { (current + 1) % count }
}

/// The category index a typed character picks, only while the label is still empty.
fn digit_category(ch: u32, label_empty: bool) -> Option<usize> {
    if !label_empty {
        return None;
    }
    let digit = char::from_u32(ch)?.to_digit(10)? as usize;
    (1..=CATEGORIES.len()).contains(&digit).then(|| digit - 1)
}

/// The label to save: the trimmed text, or none when it is empty.
fn clean_label(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// A cloneable handle to the popup thread; opens the popup and commits it from any thread.
#[derive(Clone)]
pub struct PopupHandle {
    hwnd: Arc<AtomicIsize>,
    sender: Arc<Mutex<Option<BusSender>>>,
}

impl PopupHandle {
    /// Starts the "label-popup" thread with its hidden window; the popup stays unavailable if that fails.
    pub fn start() -> Self {
        let handle = Self { hwnd: Arc::default(), sender: Arc::default() };
        let (hwnd, sender) = (Arc::clone(&handle.hwnd), Arc::clone(&handle.sender));
        if let Err(err) = std::thread::Builder::new().name("label-popup".to_owned()).spawn(move || win::run(&hwnd, sender)) {
            log::error!("the label popup thread could not be started: {err}");
        }
        handle
    }

    /// Gives the popup the command bus its saves are sent to.
    pub fn connect(&self, sender: BusSender) {
        *self.sender.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    }

    /// Opens the popup for a Shift+F9 pressed at `press_qpc`, or refocuses it when it is already open.
    pub fn open(&self, press_qpc: i64) {
        match self.hwnd.load(Ordering::Acquire) {
            0 => log::warn!("the label popup is not available"),
            hwnd => win::post_open(hwnd, press_qpc),
        }
    }

    /// Saves the open popup's current input at once; returns once the command is sent, within 500 ms.
    pub fn commit_now(&self) {
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            win::commit(hwnd);
        }
    }
}

#[allow(unsafe_code, reason = "the popup is a plain Win32 window: every call is a documented Win32 API on handles this module owns")]
mod win {
    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use ::marks::MarkType;
    use ui_kit::palette;
    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DT_CENTER, DT_LEFT, DT_SINGLELINE,
        DT_VCENTER, DeleteObject, DrawTextW, EndPaint, GetMonitorInfoW, HBRUSH, HDC, HFONT, InvalidateRect, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
        MonitorFromPoint, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT, VK_TAB};
    use windows::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, CreateWindowExW, DefWindowProcW, DispatchMessageW, GWLP_WNDPROC, GetMessageW, GetWindowTextLengthW, GetWindowTextW, HWND_TOPMOST,
        IDC_ARROW, KillTimer, LoadCursorW, MSG, MoveWindow, PostMessageW, RegisterClassW, SMTO_ABORTIFHUNG, SW_HIDE, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_SHOWWINDOW, SendMessageTimeoutW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos,
        SetWindowTextW, ShowWindow, TranslateMessage, WDA_EXCLUDEFROMCAPTURE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CHAR, WM_CLOSE, WM_CTLCOLOREDIT,
        WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_PAINT, WM_SETFONT, WM_TIMER, WNDCLASSW, WNDPROC, WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
    };
    use windows::core::w;

    use super::{CATEGORIES, COMMIT_WAIT_MS, DEFAULT_CATEGORY, IDLE_SAVE_MS, clean_label, digit_category, next_category};
    use crate::engine::bus::{BusSender, Command, CommandKind};

    /// Posted to open the popup; `lparam` is the press QPC time.
    const WM_APP_OPEN: u32 = WM_APP + 1;

    /// Sent to save the popup's current input at once; the window answers 1 when it is done.
    const WM_APP_COMMIT: u32 = WM_APP + 2;

    /// The id of the idle timer.
    const IDLE_TIMER: usize = 1;

    /// The `ES_AUTOHSCROLL` edit style: the label scrolls instead of stopping at the edge.
    const ES_AUTOHSCROLL: u32 = 0x0080;

    /// The `EM_LIMITTEXT` edit message, defined here because it lives in a Windows feature the Recorder does not otherwise need.
    const EM_LIMITTEXT: u32 = 0x00C5;

    /// The longest label the edit control accepts, in UTF-16 units.
    const LABEL_LIMIT: usize = 200;

    /// Where everything sits inside the popup, in pixels at the popup's DPI.
    #[derive(Clone, Copy)]
    struct Layout {
        width: i32,
        height: i32,
        radius: i32,
        pills: [RECT; 6],
        edit_frame: RECT,
        edit: RECT,
        hint: RECT,
    }

    /// Everything the popup thread keeps between messages.
    struct State {
        hwnd: isize,
        edit: isize,
        font: isize,
        edit_brush: isize,
        open: bool,
        press_qpc: i64,
        category: usize,
        layout: Layout,
        sender: Arc<Mutex<Option<BusSender>>>,
    }

    thread_local! {
        /// The popup thread's state; only that thread's window procedures touch it.
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
        /// The edit control's original window procedure.
        static EDIT_PROC: Cell<isize> = const { Cell::new(0) };
    }

    /// Runs `change` on the popup's state; `None` when there is no state or it is already borrowed.
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

    /// A palette color as a GDI color.
    fn rgb(color: egui::Color32) -> COLORREF {
        COLORREF(u32::from(color.r()) | (u32::from(color.g()) << 8) | (u32::from(color.b()) << 16))
    }

    /// Scales a pixel size from 96 DPI to `dpi`.
    fn scale(value: i32, dpi: u32) -> i32 {
        value * i32::try_from(dpi).unwrap_or(96) / 96
    }

    /// Works out the popup's layout at `dpi`.
    fn layout(dpi: u32) -> Layout {
        let s = |value: i32| scale(value, dpi);
        let (width, height, pad, gap) = (s(720), s(134), s(16), s(8));
        let pill_width = (width - 2 * pad - 5 * gap) / 6;
        let mut pills = [RECT::default(); 6];
        for (index, pill) in pills.iter_mut().enumerate() {
            let left = pad + i32::try_from(index).unwrap_or(0) * (pill_width + gap);
            *pill = RECT { left, top: pad, right: left + pill_width, bottom: pad + s(34) };
        }
        let edit_frame = RECT { left: pad, top: s(62), right: width - pad, bottom: s(100) };
        let edit = RECT { left: pad + s(10), top: s(70), right: width - pad - s(10), bottom: s(92) };
        let hint = RECT { left: pad, top: s(106), right: width - pad, bottom: s(128) };
        Layout { width, height, radius: s(6), pills, edit_frame, edit, hint }
    }

    /// Runs the popup thread: creates the hidden window, publishes its handle and pumps messages forever.
    pub(super) fn run(hwnd_slot: &AtomicIsize, sender: Arc<Mutex<Option<BusSender>>>) {
        // SAFETY: the call only changes this thread's DPI awareness and takes a constant.
        unsafe {
            SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let hwnd = match create(sender) {
            Ok(hwnd) => hwnd,
            Err(err) => {
                log::error!("the label popup window could not be created: {err}");
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

    /// Creates the hidden popup window with its edit control and stores the thread's state.
    fn create(sender: Arc<Mutex<Option<BusSender>>>) -> Result<HWND, String> {
        // SAFETY: every call gets valid arguments built here; the class and window belong to this thread.
        unsafe {
            let instance = GetModuleHandleW(None).map_err(|err| err.to_string())?;
            let class = w!("CutmarkLabelPopup");
            let cursor = LoadCursorW(None, IDC_ARROW).map_err(|err| err.to_string())?;
            let wc = WNDCLASSW { lpfnWndProc: Some(wnd_proc), hInstance: instance.into(), lpszClassName: class, hCursor: cursor, ..Default::default() };
            if RegisterClassW(&wc) == 0 {
                return Err("RegisterClassW failed".to_owned());
            }
            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                class,
                w!("Label this mark"),
                WS_POPUP | WS_CLIPCHILDREN,
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
            if let Err(err) = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) {
                log::warn!("the label popup could not be hidden from capture: {err}");
            }
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL),
                0,
                0,
                10,
                10,
                Some(hwnd),
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|err| err.to_string())?;
            SendMessageW(edit, EM_LIMITTEXT, Some(WPARAM(LABEL_LIMIT)), Some(LPARAM(0)));
            let old = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
            EDIT_PROC.with(|cell| cell.set(old));
            let edit_brush = CreateSolidBrush(rgb(palette::RECESSED));
            let state = State {
                hwnd: hwnd.0 as isize,
                edit: edit.0 as isize,
                font: 0,
                edit_brush: edit_brush.0 as isize,
                open: false,
                press_qpc: 0,
                category: DEFAULT_CATEGORY,
                layout: layout(96),
                sender,
            };
            STATE.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut() {
                    *guard = Some(state);
                }
            });
            Ok(hwnd)
        }
    }

    /// Asks the popup thread to open for a press at `press_qpc`.
    pub(super) fn post_open(hwnd: isize, press_qpc: i64) {
        // SAFETY: posting a message with plain integer values to a window handle is always sound; a stale handle only fails.
        let posted = unsafe { PostMessageW(Some(hwnd_of(hwnd)), WM_APP_OPEN, WPARAM(0), LPARAM(press_qpc as isize)) };
        if let Err(err) = posted {
            log::warn!("the label popup could not be opened: {err}");
        }
    }

    /// Makes the popup thread save its input, and waits for it for up to 500 ms.
    pub(super) fn commit(hwnd: isize) {
        // SAFETY: sending a message with plain integer values to a window handle is sound; the timeout bounds the wait.
        let answer = unsafe { SendMessageTimeoutW(hwnd_of(hwnd), WM_APP_COMMIT, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, COMMIT_WAIT_MS, None) };
        if answer.0 == 0 {
            log::warn!("the label popup did not answer the commit in time");
        }
    }

    /// The popup's window procedure.
    extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_APP_OPEN => {
                open(hwnd, lparam.0 as i64);
                LRESULT(0)
            }
            WM_APP_COMMIT => {
                save(hwnd, true);
                LRESULT(1)
            }
            WM_TIMER if wparam.0 == IDLE_TIMER => {
                save(hwnd, true);
                LRESULT(0)
            }
            WM_CLOSE => {
                save(hwnd, true);
                LRESULT(0)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                click(hwnd, lparam);
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => edit_colors(wparam),
            _ => default_proc(hwnd, msg, wparam, lparam),
        }
    }

    /// The edit control's window procedure: the popup's keys, then the normal edit behavior.
    extern "system" fn edit_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let parent = with_state(|state| state.hwnd).map(hwnd_of);
        match (msg, parent) {
            (WM_KEYDOWN, Some(parent)) => {
                rearm_timer(parent);
                let key = wparam.0 as u16;
                if key == VK_TAB.0 {
                    cycle_category(parent, shift_down());
                    return LRESULT(0);
                }
                if key == VK_RETURN.0 {
                    save(parent, true);
                    return LRESULT(0);
                }
                if key == VK_ESCAPE.0 {
                    save(parent, false);
                    return LRESULT(0);
                }
            }
            (WM_CHAR, Some(parent)) => {
                rearm_timer(parent);
                let ch = wparam.0 as u32;
                // Tab, Enter and Esc are handled on key down; the edit would only beep at them.
                if matches!(ch, 0x09 | 0x0D | 0x1B) {
                    return LRESULT(0);
                }
                if let Some(index) = digit_category(ch, edit_is_empty(hwnd)) {
                    set_category(parent, index);
                    return LRESULT(0);
                }
            }
            _ => {}
        }
        call_edit_original(hwnd, msg, wparam, lparam)
    }

    /// Calls the system's default window procedure.
    fn default_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // SAFETY: `hwnd` is the handle the system passed to this window's procedure.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Calls the edit control's original window procedure.
    fn call_edit_original(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let old = EDIT_PROC.with(Cell::get);
        // SAFETY: `old` is the procedure pointer SetWindowLongPtrW returned for this control (or 0, which becomes None),
        // and a function pointer and `isize` have the same size.
        unsafe {
            let original = std::mem::transmute::<isize, WNDPROC>(old);
            CallWindowProcW(original, hwnd, msg, wparam, lparam)
        }
    }

    /// Whether Shift is held down right now.
    fn shift_down() -> bool {
        // SAFETY: the call only reads the keyboard state.
        unsafe { GetKeyState(i32::from(VK_SHIFT.0)) < 0 }
    }

    /// Whether the edit control has no text.
    fn edit_is_empty(edit: HWND) -> bool {
        // SAFETY: `edit` is the handle the system passed to the edit control's procedure.
        unsafe { GetWindowTextLengthW(edit) == 0 }
    }

    /// Reads the text of the edit control.
    fn read_text(edit: HWND) -> String {
        // SAFETY: the buffer is owned here and sized from the text length plus the terminator.
        unsafe {
            let length = usize::try_from(GetWindowTextLengthW(edit)).unwrap_or(0);
            if length == 0 {
                return String::new();
            }
            let mut buffer = vec![0u16; length + 1];
            let copied = usize::try_from(GetWindowTextW(edit, &mut buffer)).unwrap_or(0);
            String::from_utf16_lossy(buffer.get(..copied).unwrap_or_default())
        }
    }

    /// Starts the idle timer again, so the popup saves 15 s after the last key.
    fn rearm_timer(hwnd: HWND) {
        // SAFETY: the timer belongs to a window of this thread; setting an existing timer id restarts it.
        unsafe {
            SetTimer(Some(hwnd), IDLE_TIMER, IDLE_SAVE_MS, None);
        }
    }

    /// Picks a category and redraws.
    fn set_category(hwnd: HWND, index: usize) {
        with_state(|state| state.category = index);
        redraw(hwnd);
    }

    /// Moves the selected category one step and redraws.
    fn cycle_category(hwnd: HWND, backwards: bool) {
        with_state(|state| state.category = next_category(state.category, backwards));
        redraw(hwnd);
    }

    /// Asks for a repaint of the whole popup.
    fn redraw(hwnd: HWND) {
        // SAFETY: `hwnd` is this thread's popup window; a null rectangle means the whole client area.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// Opens the popup for a press, or only refocuses it when it is already open.
    fn open(hwnd: HWND, press_qpc: i64) {
        let Some(was_open) = with_state(|state| {
            let was_open = state.open;
            if !was_open {
                state.open = true;
                state.press_qpc = press_qpc;
                state.category = DEFAULT_CATEGORY;
            }
            was_open
        }) else {
            return;
        };
        if !was_open {
            place_and_show(hwnd);
        }
        focus(hwnd);
    }

    /// The work area of the primary monitor, which is the one that is recorded.
    fn work_area() -> RECT {
        // SAFETY: `info` is a valid MONITORINFO with its size set, and the point is a constant.
        unsafe {
            let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(monitor, &mut info).as_bool() { info.rcWork } else { RECT { left: 0, top: 0, right: 1920, bottom: 1080 } }
        }
    }

    /// Builds a Segoe UI font of 11 points at `dpi`.
    fn make_font(dpi: u32) -> HFONT {
        let height = -((11 * i32::try_from(dpi).unwrap_or(96) + 36) / 72);
        // SAFETY: all arguments are plain values and a constant string; the font is owned by this module.
        unsafe { CreateFontW(height, 0, 0, 0, 400, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Segoe UI")) }
    }

    /// Sizes the popup for the monitor's DPI, centers it in the upper third and shows it on top.
    fn place_and_show(hwnd: HWND) {
        let area = work_area();
        // SAFETY: the calls only move, size and show this thread's popup and its edit control, and replace the font it owns.
        unsafe {
            // Move onto the monitor first, so the window reports that monitor's DPI.
            let _ = SetWindowPos(hwnd, None, area.left, area.top, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
            let dpi = GetDpiForWindow(hwnd);
            let lay = layout(if dpi == 0 { 96 } else { dpi });
            let font = make_font(dpi);
            let Some((edit, old_font)) = with_state(|state| {
                state.layout = lay;
                (state.edit, std::mem::replace(&mut state.font, font.0 as isize))
            }) else {
                return;
            };
            let edit = hwnd_of(edit);
            let _ = SetWindowTextW(edit, w!(""));
            SendMessageW(edit, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            if old_font != 0 {
                let _ = DeleteObject(HFONT(old_font as *mut c_void).into());
            }
            let _ = MoveWindow(edit, lay.edit.left, lay.edit.top, lay.edit.right - lay.edit.left, lay.edit.bottom - lay.edit.top, true);
            let x = area.left + ((area.right - area.left) - lay.width) / 2;
            let y = (area.top + (area.bottom - area.top) / 3 - lay.height / 2).max(area.top);
            if let Err(err) = SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, lay.width, lay.height, SWP_SHOWWINDOW) {
                log::warn!("the label popup could not be shown: {err}");
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// Brings the popup to the front and focuses the edit control; a click focuses it if the system refuses.
    fn focus(hwnd: HWND) {
        let Some(edit) = with_state(|state| state.edit) else { return };
        rearm_timer(hwnd);
        // SAFETY: both handles are windows of this process, and the calls take no pointers.
        unsafe {
            if !SetForegroundWindow(hwnd).as_bool() {
                log::debug!("the label popup was not allowed to take the foreground; a click will focus it");
            }
            if let Err(err) = SetFocus(Some(hwnd_of(edit))) {
                log::debug!("the label popup edit could not take the focus: {err}");
            }
        }
    }

    /// Saves the popup's input as a labeled mark at the press time, then hides it; does nothing when it is closed.
    fn save(hwnd: HWND, with_label: bool) {
        let taken = with_state(|state| {
            if !state.open {
                return None;
            }
            state.open = false;
            Some((state.press_qpc, state.category, state.edit, Arc::clone(&state.sender)))
        })
        .flatten();
        let Some((press_qpc, category, edit, sender)) = taken else { return };
        let text = if with_label { read_text(hwnd_of(edit)) } else { String::new() };
        // SAFETY: both calls take this thread's popup window and plain values.
        unsafe {
            let _ = KillTimer(Some(hwnd), IDLE_TIMER);
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        let mark_type = CATEGORIES.get(category).map_or(MarkType::Note, |entry| entry.1);
        let kind = CommandKind::LabeledMark { category: mark_type, label: clean_label(&text) };
        let guard = sender.lock().unwrap_or_else(PoisonError::into_inner);
        match guard.as_ref() {
            Some(bus) => {
                if bus.send(Command::new(kind, press_qpc)).is_err() {
                    log::warn!("the labeled mark was lost because the command bus has ended");
                }
            }
            None => log::warn!("the labeled mark was lost because the popup is not connected to the command bus"),
        }
    }

    /// Picks the category under a click, and focuses the edit control.
    fn click(hwnd: HWND, lparam: LPARAM) {
        let x = i32::from((lparam.0 & 0xFFFF) as i16);
        let y = i32::from(((lparam.0 >> 16) & 0xFFFF) as i16);
        let hit = with_state(|state| {
            let index = state.layout.pills.iter().position(|pill| x >= pill.left && x < pill.right && y >= pill.top && y < pill.bottom);
            (index, state.edit)
        });
        let Some((index, edit)) = hit else { return };
        if let Some(index) = index {
            set_category(hwnd, index);
        }
        // SAFETY: `edit` is the popup's own edit control.
        unsafe {
            let _ = SetFocus(Some(hwnd_of(edit)));
        }
    }

    /// Answers WM_CTLCOLOREDIT: light text on the recessed color.
    fn edit_colors(wparam: WPARAM) -> LRESULT {
        let hdc = HDC(wparam.0 as *mut c_void);
        // SAFETY: `hdc` is the device context the system passed with the message.
        unsafe {
            SetTextColor(hdc, rgb(palette::TEXT_PRIMARY));
            SetBkColor(hdc, rgb(palette::RECESSED));
        }
        LRESULT(with_state(|state| state.edit_brush).unwrap_or(0))
    }

    /// Paints the category row, the edit frame and the hint.
    fn paint(hwnd: HWND) {
        let snapshot = with_state(|state| (state.layout, state.category, state.font));
        let mut ps = PAINTSTRUCT::default();
        // SAFETY: `ps` is a valid PAINTSTRUCT, and BeginPaint and EndPaint are paired on this window.
        unsafe {
            let hdc = BeginPaint(hwnd, &mut ps);
            if let Some((lay, category, font)) = snapshot {
                draw(hdc, &lay, category, font);
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    /// Draws the popup's contents on `hdc`.
    fn draw(hdc: HDC, lay: &Layout, category: usize, font: isize) {
        round_box(hdc, RECT { left: 0, top: 0, right: lay.width, bottom: lay.height }, palette::PANEL_BG, palette::CONTROL_BORDER, 0);
        // SAFETY: `hdc` is the live paint context, and the font (or 0, which selects nothing useful but is harmless) is owned by this module.
        unsafe {
            if font != 0 {
                SelectObject(hdc, HFONT(font as *mut c_void).into());
            }
            SetBkMode(hdc, TRANSPARENT);
        }
        for (index, (name, _)) in CATEGORIES.iter().enumerate() {
            let Some(rect) = lay.pills.get(index) else { continue };
            let selected = index == category;
            let (fill, border, text) = if selected {
                (palette::ACCENT, palette::ACCENT, palette::APP_BG)
            } else {
                (palette::CONTROL_FILL, palette::CONTROL_BORDER, palette::TEXT_NORMAL)
            };
            round_box(hdc, *rect, fill, border, lay.radius);
            draw_text(hdc, *rect, &format!("{} {name}", index + 1), text, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
        round_box(hdc, lay.edit_frame, palette::RECESSED, palette::CONTROL_BORDER, lay.radius);
        draw_text(hdc, lay.hint, "Enter saves. Esc saves with no label. Tab switches category.", palette::TEXT_MUTED, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
    }

    /// Draws a filled box with a one pixel border and rounded corners.
    fn round_box(hdc: HDC, rect: RECT, fill: egui::Color32, border: egui::Color32, radius: i32) {
        // SAFETY: `hdc` is the live paint context; the pen and brush are created, selected, then restored and deleted here.
        unsafe {
            let pen = CreatePen(PS_SOLID, 1, rgb(border));
            let brush: HBRUSH = CreateSolidBrush(rgb(fill));
            let old_pen = SelectObject(hdc, pen.into());
            let old_brush = SelectObject(hdc, brush.into());
            let _ = RoundRect(hdc, rect.left, rect.top, rect.right - 1, rect.bottom - 1, radius * 2, radius * 2);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(pen.into());
            let _ = DeleteObject(brush.into());
        }
    }

    /// Draws one line of text in `rect`.
    fn draw_text(hdc: HDC, rect: RECT, text: &str, color: egui::Color32, format: windows::Win32::Graphics::Gdi::DRAW_TEXT_FORMAT) {
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rect = rect;
        // SAFETY: `hdc` is the live paint context; the text buffer and rectangle are owned here and outlive the call.
        unsafe {
            SetTextColor(hdc, rgb(color));
            DrawTextW(hdc, &mut wide, &mut rect, format);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_cycles_forward_and_shift_tab_back_with_wrapping() {
        assert_eq!(next_category(0, false), 1);
        assert_eq!(next_category(5, false), 0);
        assert_eq!(next_category(0, true), 5);
        assert_eq!(next_category(3, true), 2);
    }

    #[test]
    fn digits_pick_a_category_only_while_the_label_is_empty() {
        assert_eq!(digit_category(u32::from('1'), true), Some(0));
        assert_eq!(digit_category(u32::from('6'), true), Some(5));
        assert_eq!(digit_category(u32::from('7'), true), None);
        assert_eq!(digit_category(u32::from('0'), true), None);
        assert_eq!(digit_category(u32::from('a'), true), None);
        assert_eq!(digit_category(u32::from('3'), false), None);
    }

    #[test]
    fn an_empty_or_blank_label_is_no_label() {
        assert_eq!(clean_label(""), None);
        assert_eq!(clean_label("   "), None);
        assert_eq!(clean_label("  intro take "), Some("intro take".to_owned()));
    }

    #[test]
    fn the_default_category_is_note() {
        assert_eq!(CATEGORIES.get(DEFAULT_CATEGORY).map(|entry| entry.1), Some(MarkType::Note));
    }
}
