//! The logger thread: a 4 ms cursor sampler and a message-only window that receives Raw Input, on
//! one high priority thread.

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use windows::Win32::Foundation::{HANDLE, HWND, RECT, WAIT_OBJECT_0};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateEventW, CreateWaitableTimerExW, GetCurrentThread, INFINITE, SetEvent, SetThreadPriority, SetWaitableTimer,
    THREAD_PRIORITY_HIGHEST, TIMER_ALL_ACCESS,
};
use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RIDEV_REMOVE,
    RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, CreateWindowExW, DestroyWindow, DispatchMessageW, GetCursorInfo, GetWindowRect, HCURSOR, HWND_MESSAGE, IsIconic, IsWindow, MSG,
    MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, QS_ALLINPUT, RI_MOUSE_HWHEEL, RI_MOUSE_WHEEL, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_INPUT,
};
use windows::core::{PCWSTR, w};

use super::filter::SampleFilter;
use super::render::CursorReader;
use super::shape::{Resolved, ShapeCache};
use super::{
    CaptureSource, CursorSample, DisplayInfo, GeometryChange, GeometryEvent, InputEvent, InputOptions, InputStartInfo, MouseAction, MouseButton, MouseEvent,
    ScreenRect,
};
use crate::monitor::monitor_dpi;
use crate::{CaptureError, list_monitors, qpc_frequency, qpc_now, qpc_now_100ns, with_context};

/// The sampling period in 100 nanosecond units: 4 ms.
const PERIOD_100NS: i64 = 40_000;
/// Ticks between window rectangle checks: about 16 ms, one frame at 60 fps.
const WINDOW_CHECK_TICKS: u64 = 4;
/// Ticks between display layout checks: about 250 ms. A message-only window receives no broadcast
/// messages such as WM_DISPLAYCHANGE, so the layout is polled.
const DISPLAY_CHECK_TICKS: u64 = 62;
/// Event queue length; several seconds of constant motion.
const CHANNEL_CAPACITY: usize = 4096;
/// Messages handled before the sampling timer gets another turn.
const MESSAGES_PER_PASS: usize = 64;
/// The cursor base size when the setting was never changed.
const DEFAULT_CURSOR_SIZE: u32 = 32;
/// HID usage page and usage of a mouse.
const USAGE_PAGE_GENERIC: u16 = 0x01;
const USAGE_MOUSE: u16 = 0x02;

/// Raw Input allows one mouse registration per process, so only one logger may run.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Holds the one logger slot; dropping it frees the slot.
struct ActiveSlot;

impl ActiveSlot {
    fn claim() -> Option<Self> {
        (!ACTIVE.swap(true, Ordering::SeqCst)).then_some(Self)
    }
}

impl Drop for ActiveSlot {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// A running input logger. Only one can run at a time in a process.
///
/// The process should be Per-Monitor DPI aware, or Windows may report every monitor as 96 DPI.
pub struct InputLogger {
    receiver: Receiver<InputEvent>,
    start_info: InputStartInfo,
    stop_event: Arc<OwnedHandle>,
    thread: Option<JoinHandle<()>>,
    dropped: Arc<AtomicU64>,
}

impl InputLogger {
    /// Starts the logger thread and waits until it is sampling.
    pub fn start(options: InputOptions) -> Result<Self, CaptureError> {
        let slot = ActiveSlot::claim().ok_or(CaptureError::Missing("an input logger is already running"))?;
        let stop_event = Arc::new(create_stop_event()?);
        let dropped = Arc::new(AtomicU64::new(0));
        let (sender, receiver) = mpsc::sync_channel(CHANNEL_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let outbox = Outbox { sender, dropped: Arc::clone(&dropped) };
        let thread_stop = Arc::clone(&stop_event);
        let thread = thread::Builder::new()
            .name("input-logger".to_owned())
            .spawn(move || {
                let _slot = slot;
                match Worker::new(options, outbox) {
                    Ok((mut worker, start_info)) => {
                        if ready_sender.send(Ok(start_info)).is_ok() {
                            worker.run(handle_of(&thread_stop));
                        }
                    }
                    Err(err) => {
                        let _ = ready_sender.send(Err(err));
                    }
                }
            })
            .map_err(|_| CaptureError::Missing("failed to spawn the input logger thread"))?;
        let setup = ready_receiver.recv().map_err(|_| CaptureError::Missing("the input logger thread exited before setup finished")).and_then(|result| result);
        match setup {
            Ok(start_info) => Ok(Self { receiver, start_info, stop_event, thread: Some(thread), dropped }),
            Err(err) => {
                let _ = thread.join();
                Err(err)
            }
        }
    }

    /// What the logger read when it started.
    pub fn start_info(&self) -> &InputStartInfo {
        &self.start_info
    }

    /// The event queue. Drain it often: the thread drops events while it is full. It stays readable
    /// after [`InputLogger::stop`], and disconnects once the thread has ended and it is empty.
    pub fn receiver(&self) -> &Receiver<InputEvent> {
        &self.receiver
    }

    /// False once the thread ended, whether stopped or failed.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|thread| !thread.is_finished())
    }

    /// How many sends the full queue refused. Samples, shapes and geometry are offered again;
    /// mouse events are lost.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Stops the logger and joins its thread; Drop does the same.
    #[allow(unsafe_code)]
    pub fn stop(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        // SAFETY: the event stays open while self holds it.
        if let Err(err) = unsafe { SetEvent(handle_of(&self.stop_event)) } {
            // Joining would hang; the thread is left to end with the process.
            log::error!("could not signal the input logger to stop: {err}");
            return;
        }
        if thread.join().is_err() {
            log::error!("the input logger thread panicked");
        }
    }
}

impl Drop for InputLogger {
    fn drop(&mut self) {
        self.stop();
    }
}

fn handle_of(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// A manual reset event that tells the thread to stop.
#[allow(unsafe_code)]
fn create_stop_event() -> Result<OwnedHandle, CaptureError> {
    // SAFETY: plain call with value arguments.
    let event = with_context("failed to create the input logger stop event", unsafe { CreateEventW(None, true, false, PCWSTR::null()) })?;
    // SAFETY: the handle is new, valid and owned by nothing else.
    Ok(unsafe { OwnedHandle::from_raw_handle(event.0) })
}

/// The sending side of the event queue; it never blocks.
struct Outbox {
    sender: SyncSender<InputEvent>,
    dropped: Arc<AtomicU64>,
}

impl Outbox {
    /// False when the event was not queued.
    fn send(&self, event: InputEvent) -> bool {
        match self.sender.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(TrySendError::Disconnected(_)) => false,
        }
    }
}

/// Everything the logger thread owns. Field order matters: the Raw Input registration is undone
/// before its target window is destroyed.
struct Worker {
    _raw_mouse: Option<RawMouse>,
    _window: MessageWindow,
    timer: OwnedHandle,
    outbox: Outbox,
    reader: CursorReader,
    shapes: ShapeCache,
    filter: SampleFilter,
    /// The latest cursor position.
    position: (i32, i32),
    /// The captured window, for window capture.
    target: Option<isize>,
    window_rect: Option<ScreenRect>,
    displays: Vec<DisplayInfo>,
}

impl Worker {
    /// Sets up the calling thread: DPI awareness, priority, timer, window and Raw Input.
    #[allow(unsafe_code)]
    fn new(options: InputOptions, outbox: Outbox) -> Result<(Self, InputStartInfo), CaptureError> {
        // SAFETY: changes only this thread's DPI awareness, so every coordinate is physical.
        if unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.0.is_null() {
            log::warn!("could not make the input logger thread Per-Monitor DPI aware");
        }
        // SAFETY: the pseudo handle always names the calling thread.
        if let Err(err) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) } {
            log::warn!("could not raise the input logger thread priority: {err}");
        }
        let frequency = qpc_frequency();
        if frequency <= 0 {
            return Err(CaptureError::Missing("the QPC frequency is not available"));
        }
        let timer = create_timer()?;
        let window = MessageWindow::new()?;
        let raw_mouse = if options.clicks { Some(RawMouse::register(window.0)?) } else { None };
        let displays = read_displays()?;
        let start_info = InputStartInfo { qpc_frequency: frequency, cursor_size: cursor_size(), displays: displays.clone() };
        let target = match options.source {
            CaptureSource::Window { handle } => Some(handle),
            CaptureSource::Monitor(_) | CaptureSource::Region(_) => None,
        };
        let worker = Self {
            _raw_mouse: raw_mouse,
            _window: window,
            timer,
            outbox,
            reader: CursorReader::new(),
            shapes: ShapeCache::default(),
            filter: SampleFilter::new(frequency),
            position: (0, 0),
            target,
            window_rect: None,
            displays,
        };
        Ok((worker, start_info))
    }

    /// Samples and pumps messages until the stop event is set or Windows fails.
    #[allow(unsafe_code)]
    fn run(&mut self, stop: HANDLE) {
        let handles = [stop, handle_of(&self.timer)];
        let mut due = qpc_now_100ns();
        let mut ticks = 0u64;
        if !self.arm_timer(due) {
            return;
        }
        loop {
            // SAFETY: both handles stay open for the whole loop.
            let woke = unsafe { MsgWaitForMultipleObjectsEx(Some(&handles), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) };
            match woke.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => break,
                1 => {
                    self.tick(ticks);
                    ticks += 1;
                    // Keep a steady 4 ms grid; after a stall, start over instead of catching up.
                    let now = qpc_now_100ns();
                    due += PERIOD_100NS;
                    if due <= now {
                        due = now + PERIOD_100NS;
                    }
                    if !self.arm_timer(due) {
                        return;
                    }
                }
                2 => self.pump_messages(),
                _ => {
                    log::error!("the input logger wait failed: {}", windows::core::Error::from_thread());
                    return;
                }
            }
        }
        // Keep clicks that were already queued when the stop arrived.
        self.pump_messages();
    }

    /// Arms the timer for the given QPC time in 100 ns units.
    #[allow(unsafe_code)]
    fn arm_timer(&self, due: i64) -> bool {
        // A negative due time is relative; zero would mean an absolute time in the past.
        let relative = -(due - qpc_now_100ns()).max(1);
        // SAFETY: the timer is open and the due time is read during the call.
        let armed = unsafe { SetWaitableTimer(handle_of(&self.timer), &relative, 0, None, None, false) };
        if let Err(err) = &armed {
            log::error!("could not arm the input logger timer: {err}");
        }
        armed.is_ok()
    }

    fn tick(&mut self, ticks: u64) {
        if ticks.is_multiple_of(WINDOW_CHECK_TICKS) {
            self.check_window();
        }
        if ticks % DISPLAY_CHECK_TICKS == DISPLAY_CHECK_TICKS - 1 {
            self.check_displays();
        }
        self.sample();
    }

    /// Reads the cursor and sends a sample if it changed or the keep-alive is due.
    #[allow(unsafe_code)]
    fn sample(&mut self) {
        let qpc = qpc_now();
        let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        // SAFETY: info is a valid CURSORINFO with its size set. The call fails on the secure
        // desktop (UAC prompt, lock screen); the cursor then counts as hidden where it was.
        let read = unsafe { GetCursorInfo(&mut info) }.is_ok();
        if read {
            self.position = (info.ptScreenPos.x, info.ptScreenPos.y);
        }
        let visible = read && info.flags.0 & CURSOR_SHOWING.0 != 0 && !info.hCursor.is_invalid();
        let shape = if visible { self.shape_id(info.hCursor) } else { None };
        let sample = CursorSample { qpc, x: self.position.0, y: self.position.1, shape, visible };
        if self.filter.wants(&sample) && self.outbox.send(InputEvent::Sample(sample)) {
            self.filter.sent(sample);
        }
    }

    /// The shape id for a cursor handle, reading and sending the shape the first time.
    fn shape_id(&mut self, cursor: HCURSOR) -> Option<u32> {
        let handle = cursor.0 as isize;
        if let Some(known) = self.shapes.lookup(handle) {
            return known;
        }
        let Some(image) = self.reader.read(cursor) else {
            log::debug!("could not read a cursor shape");
            self.shapes.failed(handle);
            return None;
        };
        match self.shapes.resolve(handle, image) {
            Resolved::Known(id) => Some(id),
            Resolved::New { shape, hash } => {
                let id = shape.id;
                if !self.outbox.send(InputEvent::Shape(shape)) {
                    return None;
                }
                self.shapes.commit(handle, hash, id);
                Some(id)
            }
        }
    }

    #[allow(unsafe_code)]
    fn pump_messages(&mut self) {
        let mut message = MSG::default();
        for _ in 0..MESSAGES_PER_PASS {
            // SAFETY: message is a valid out pointer; only this thread's queue is read.
            if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                break;
            }
            if message.message == WM_INPUT {
                self.raw_input(HRAWINPUT(message.lParam.0 as *mut core::ffi::c_void));
            }
            // SAFETY: message came from PeekMessageW. For WM_INPUT this also lets
            // DefWindowProc release the raw input data.
            unsafe { DispatchMessageW(&message) };
        }
    }

    /// Handles one Raw Input message: sends its button and wheel events with the latest position.
    #[allow(unsafe_code)]
    fn raw_input(&mut self, input: HRAWINPUT) {
        let qpc = qpc_now();
        let mut raw = RAWINPUT::default();
        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
        let header = std::mem::size_of::<RAWINPUTHEADER>() as u32;
        // SAFETY: raw is a writable buffer of `size` bytes.
        let written = unsafe { GetRawInputData(input, RID_INPUT, Some((&raw mut raw).cast()), &mut size, header) };
        if written == u32::MAX || raw.header.dwType != RIM_TYPEMOUSE.0 {
            return;
        }
        // SAFETY: the type says the mouse member is the one filled in.
        let buttons = unsafe { raw.data.mouse.Anonymous.Anonymous };
        let mut actions = mouse_actions(buttons.usButtonFlags, buttons.usButtonData).peekable();
        // Plain movement carries no buttons; the sampler covers it.
        if actions.peek().is_none() {
            return;
        }
        // Read the cursor now so the event is paired with the newest position.
        self.sample();
        let (x, y) = self.position;
        for action in actions {
            self.outbox.send(InputEvent::Mouse(MouseEvent { qpc, x, y, action }));
        }
    }

    /// Sends the captured window's rectangle when it moved or resized. A minimized or closed
    /// window keeps its last rectangle.
    fn check_window(&mut self) {
        let Some(rect) = self.target.and_then(visible_window_rect) else { return };
        if self.window_rect != Some(rect) && self.send_geometry(GeometryChange::WindowRect(rect)) {
            self.window_rect = Some(rect);
        }
    }

    /// Sends the display layout when a resolution, DPI or monitor changed.
    fn check_displays(&mut self) {
        // The enumeration can fail while displays are changing; the next check tries again.
        let Ok(displays) = read_displays() else { return };
        if !displays.is_empty() && displays != self.displays && self.send_geometry(GeometryChange::Displays(displays.clone())) {
            self.displays = displays;
        }
    }

    fn send_geometry(&self, change: GeometryChange) -> bool {
        self.outbox.send(InputEvent::Geometry(GeometryEvent { qpc: qpc_now(), change }))
    }
}

/// The button and wheel events in one Raw Input mouse packet.
fn mouse_actions(flags: u16, data: u16) -> impl Iterator<Item = MouseAction> {
    // Down is bit 2n and up is bit 2n + 1, in this button order.
    const BUTTONS: [MouseButton; 5] = [MouseButton::Left, MouseButton::Right, MouseButton::Middle, MouseButton::X1, MouseButton::X2];
    let flags = u32::from(flags);
    let buttons = BUTTONS.into_iter().enumerate().flat_map(move |(index, button)| {
        let down = (flags & (1 << (index * 2)) != 0).then_some(MouseAction::Down(button));
        let up = (flags & (2 << (index * 2)) != 0).then_some(MouseAction::Up(button));
        down.into_iter().chain(up)
    });
    // The wheel delta is a signed value in an unsigned field.
    let delta = data.cast_signed();
    let wheel = (flags & RI_MOUSE_WHEEL != 0).then_some(MouseAction::Wheel(delta));
    let horizontal = (flags & RI_MOUSE_HWHEEL != 0).then_some(MouseAction::HorizontalWheel(delta));
    buttons.chain(wheel).chain(horizontal)
}

/// A 4 ms capable timer; the high resolution kind does not change the system timer rate.
#[allow(unsafe_code)]
fn create_timer() -> Result<OwnedHandle, CaptureError> {
    // SAFETY: plain call with value arguments.
    let timer = with_context("failed to create the input logger timer", unsafe {
        CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
    })?;
    // SAFETY: the handle is new, valid and owned by nothing else.
    Ok(unsafe { OwnedHandle::from_raw_handle(timer.0) })
}

/// A message-only window, destroyed on drop by the thread that made it.
struct MessageWindow(HWND);

impl MessageWindow {
    #[allow(unsafe_code)]
    fn new() -> Result<Self, CaptureError> {
        // SAFETY: the built in STATIC class needs no registration, and its window procedure
        // passes WM_INPUT on to DefWindowProc.
        let window = with_context("failed to create the input logger window", unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("cutmark-input"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        })?;
        Ok(Self(window))
    }
}

impl Drop for MessageWindow {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the window is ours, destroyed once, on its own thread.
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

/// The Raw Input mouse registration. A process has only one per device kind, and a UI framework may
/// already hold it, so drop puts the earlier registration back.
struct RawMouse {
    previous: Option<RAWINPUTDEVICE>,
}

impl RawMouse {
    /// Delivers mouse input to `window`, also while other apps have the focus.
    fn register(window: HWND) -> Result<Self, CaptureError> {
        let previous = registered_mouse();
        let device = RAWINPUTDEVICE { usUsagePage: USAGE_PAGE_GENERIC, usUsage: USAGE_MOUSE, dwFlags: RIDEV_INPUTSINK, hwndTarget: window };
        with_context("failed to register for raw mouse input", register_device(device))?;
        Ok(Self { previous })
    }
}

impl Drop for RawMouse {
    fn drop(&mut self) {
        if let Some(previous) = self.previous
            && register_device(previous).is_ok()
        {
            return;
        }
        let remove = RAWINPUTDEVICE { usUsagePage: USAGE_PAGE_GENERIC, usUsage: USAGE_MOUSE, dwFlags: RIDEV_REMOVE, hwndTarget: HWND::default() };
        if let Err(err) = register_device(remove) {
            log::warn!("could not remove the raw mouse input registration: {err}");
        }
    }
}

#[allow(unsafe_code)]
fn register_device(device: RAWINPUTDEVICE) -> windows::core::Result<()> {
    // SAFETY: one valid device entry, with the size of the entry type.
    unsafe { RegisterRawInputDevices(&[device], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }
}

/// The process's current Raw Input mouse registration, if any.
#[allow(unsafe_code)]
fn registered_mouse() -> Option<RAWINPUTDEVICE> {
    let size = std::mem::size_of::<RAWINPUTDEVICE>() as u32;
    let mut count = 0u32;
    // SAFETY: with no buffer the call only writes the number of registrations.
    unsafe { GetRegisteredRawInputDevices(None, &mut count, size) };
    if count == 0 {
        return None;
    }
    let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
    // SAFETY: the buffer holds `count` entries.
    let written = unsafe { GetRegisteredRawInputDevices(Some(devices.as_mut_ptr()), &mut count, size) };
    if written == u32::MAX {
        return None;
    }
    devices.into_iter().take(written as usize).find(|device| device.usUsagePage == USAGE_PAGE_GENERIC && device.usUsage == USAGE_MOUSE)
}

/// The visible frame of a window, without the invisible resize border; None while it is minimized
/// or gone.
#[allow(unsafe_code)]
fn visible_window_rect(handle: isize) -> Option<ScreenRect> {
    let window = HWND(handle as *mut core::ffi::c_void);
    // SAFETY: both calls accept any value and answer false for a window that is gone.
    if !unsafe { IsWindow(Some(window)) }.as_bool() || unsafe { IsIconic(window) }.as_bool() {
        return None;
    }
    let mut rect = RECT::default();
    // SAFETY: the out pointer is a RECT and the size matches it.
    let framed = unsafe { DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, (&raw mut rect).cast(), std::mem::size_of::<RECT>() as u32) }.is_ok();
    if !framed {
        // SAFETY: rect is a valid out pointer.
        unsafe { GetWindowRect(window, &mut rect) }.ok()?;
    }
    Some(ScreenRect { x: rect.left, y: rect.top, width: rect.right - rect.left, height: rect.bottom - rect.top })
}

/// The display layout with each monitor's DPI.
fn read_displays() -> Result<Vec<DisplayInfo>, CaptureError> {
    Ok(list_monitors()?
        .into_iter()
        .map(|monitor| DisplayInfo {
            dpi: monitor_dpi(monitor.handle),
            rect: ScreenRect { x: monitor.x, y: monitor.y, width: monitor.width, height: monitor.height },
            primary: monitor.primary,
            name: monitor.name,
        })
        .collect())
}

/// The cursor size setting (Settings, Accessibility, Mouse pointer), as a base size in pixels.
#[allow(unsafe_code)]
fn cursor_size() -> u32 {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: value is a writable DWORD and size is its size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Cursors"),
            w!("CursorBaseSize"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&mut size),
        )
    };
    if status.is_ok() && value > 0 { value } else { DEFAULT_CURSOR_SIZE }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{
        RI_MOUSE_BUTTON_5_UP, RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_WHEEL,
    };

    use super::{MouseAction, MouseButton, mouse_actions};

    fn actions(flags: u32, data: i16) -> Vec<MouseAction> {
        mouse_actions(flags as u16, data.cast_unsigned()).collect()
    }

    #[test]
    fn raw_mouse_flags_decode() {
        assert_eq!(actions(0, 0), []);
        assert_eq!(actions(RI_MOUSE_LEFT_BUTTON_DOWN | RI_MOUSE_LEFT_BUTTON_UP, 0), [MouseAction::Down(MouseButton::Left), MouseAction::Up(MouseButton::Left)]);
        assert_eq!(actions(RI_MOUSE_MIDDLE_BUTTON_DOWN, 0), [MouseAction::Down(MouseButton::Middle)]);
        assert_eq!(actions(RI_MOUSE_BUTTON_5_UP, 0), [MouseAction::Up(MouseButton::X2)]);
        assert_eq!(actions(RI_MOUSE_WHEEL, -120), [MouseAction::Wheel(-120)]);
        assert_eq!(actions(RI_MOUSE_HWHEEL, 120), [MouseAction::HorizontalWheel(120)]);
    }
}
