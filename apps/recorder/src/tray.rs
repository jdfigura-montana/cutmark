//! The tray icon: a grey or red dot with a menu to show the window, start or stop and quit.
//!
//! The icon lives on its own thread with a Win32 message loop, so it changes while the window is hidden
//! and no frames run.

use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use ui_kit::tokens::color;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetWindowThreadProcessId, MSG, PostThreadMessageW, TranslateMessage, WM_APP, WM_QUIT,
};

use crate::engine::bus::{BusSender, Command, CommandKind};
use crate::engine::controller::ControlSender;
use crate::platform;

/// Width and height of the tray icon in pixels.
const ICON_SIZE: u32 = 32;
/// Tray icon colour while idle (`text.tertiary`).
const IDLE_RGB: egui::Color32 = color::TEXT_TERTIARY;
/// Tray icon colour while recording (`sem.record`).
const RECORDING_RGB: egui::Color32 = color::SEM_RECORD;
/// Tooltip text while idle.
const TOOLTIP_IDLE: &str = "CutMark Recorder";
/// Tooltip text while recording.
const TOOLTIP_RECORDING: &str = "CutMark Recorder - recording";
/// Tooltip text while paused.
const TOOLTIP_PAUSED: &str = "Paused";

/// What the tray icon shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// No take is running: the grey dot.
    Idle,
    /// A take is running: the red dot.
    Recording,
    /// A take is paused: the red dot with a "Paused" tooltip.
    Paused,
}

/// A cloneable handle that changes the tray icon from any thread.
#[derive(Clone)]
pub struct TrayHandle {
    tx: Sender<TrayState>,
    thread_id: u32,
}

impl TrayHandle {
    /// Sends the state to the tray thread and wakes it; the tray thread applies the newest state it has received.
    pub fn set_state(&self, state: TrayState) {
        if self.tx.send(state).is_err() {
            log::debug!("the tray state was not sent because the tray thread has ended");
            return;
        }
        wake(self.thread_id, WM_APP);
    }
}

/// The tray icon and its thread: it handles its own menu clicks for the window and sends recording commands to the controller.
pub struct Tray {
    handle: TrayHandle,
    thread: Option<JoinHandle<()>>,
}

impl Tray {
    /// Starts the "tray" thread that builds the tray icon and menu and runs its message loop. Show and Quit run
    /// on that thread directly, so they work while the window is hidden; start or stop goes to the controller
    /// stamped with QPC at the click, and Quit tells the controller to stop before it closes the window.
    pub fn new(ctx: &egui::Context, hwnd: Option<isize>, bus: BusSender, control: ControlSender) -> Result<Self, String> {
        let (state_tx, state_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let ctx = ctx.clone();
        let thread = std::thread::Builder::new()
            .name("tray".to_owned())
            .spawn(move || run_thread(&ctx, hwnd, bus, control, &state_rx, &ready_tx))
            .map_err(|err| format!("could not start the tray thread: {err}"))?;
        match ready_rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self { handle: TrayHandle { tx: state_tx, thread_id }, thread: Some(thread) }),
            Ok(Err(err)) => {
                join(thread);
                Err(err)
            }
            Err(_) => {
                join(thread);
                Err("the tray thread ended before the icon was ready".to_owned())
            }
        }
    }

    /// A handle that changes the icon from any thread.
    pub fn handle(&self) -> TrayHandle {
        self.handle.clone()
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        wake(self.handle.thread_id, WM_QUIT);
        if let Some(thread) = self.thread.take() {
            join(thread);
        }
    }
}

/// Waits for the tray thread and logs if it panicked.
fn join(thread: JoinHandle<()>) {
    if thread.join().is_err() {
        log::error!("the tray thread panicked");
    }
}

/// Posts a message with no parameters to the tray thread's message queue.
#[allow(unsafe_code)]
fn wake(thread_id: u32, message: u32) {
    // SAFETY: PostThreadMessageW only takes plain integers; a thread id that is gone makes it fail, which is logged.
    if let Err(err) = unsafe { PostThreadMessageW(thread_id, message, WPARAM(0), LPARAM(0)) } {
        log::debug!("could not wake the tray thread: {err}");
    }
}

/// The tray icon with the two pictures it switches between.
struct TrayIcons {
    icon: TrayIcon,
    idle_icon: Icon,
    recording_icon: Icon,
}

impl TrayIcons {
    /// Swaps the icon colour and tooltip to match the state.
    fn apply(&self, state: TrayState) {
        let (icon, tooltip) = match state {
            TrayState::Idle => (&self.idle_icon, TOOLTIP_IDLE),
            TrayState::Recording => (&self.recording_icon, TOOLTIP_RECORDING),
            TrayState::Paused => (&self.recording_icon, TOOLTIP_PAUSED),
        };
        if let Err(err) = self.icon.set_icon(Some(icon.clone())) {
            log::warn!("could not change the tray icon: {err}");
        }
        if let Err(err) = self.icon.set_tooltip(Some(tooltip)) {
            log::warn!("could not change the tray tooltip: {err}");
        }
    }
}

/// The body of the tray thread: builds the icon, reports its thread id, then runs the message loop until WM_QUIT.
fn run_thread(
    ctx: &egui::Context,
    hwnd: Option<isize>,
    bus: BusSender,
    control: ControlSender,
    state_rx: &Receiver<TrayState>,
    ready: &Sender<Result<u32, String>>,
) {
    let icons = match build_icons(ctx, hwnd, bus, control) {
        Ok(icons) => icons,
        Err(err) => {
            // The starting thread is waiting for this answer; if it has gone there is nobody to tell.
            let _ = ready.send(Err(err));
            return;
        }
    };
    let thread_id = window_thread_id(icons.icon.window_handle());
    if thread_id == 0 {
        let _ = ready.send(Err("could not find the thread of the tray window".to_owned()));
        return;
    }
    if ready.send(Ok(thread_id)).is_err() {
        return;
    }
    message_loop(&icons, state_rx);
}

/// The id of the thread that created the window, or 0 if it cannot be found.
#[allow(unsafe_code)]
fn window_thread_id(window: *mut c_void) -> u32 {
    let hwnd = HWND(window);
    // SAFETY: the window belongs to the tray icon, which the caller keeps alive; a null process id pointer is allowed.
    unsafe { GetWindowThreadProcessId(hwnd, None) }
}

/// Runs the Win32 message loop on the tray thread, applying the newest state after every message, until WM_QUIT.
#[allow(unsafe_code)]
fn message_loop(icons: &TrayIcons, state_rx: &Receiver<TrayState>) {
    let mut shown = TrayState::Idle;
    loop {
        if let Some(state) = state_rx.try_iter().last()
            && state != shown
        {
            icons.apply(state);
            shown = state;
        }
        let mut msg = MSG::default();
        // SAFETY: `msg` is a valid MSG that lives for the call; no window filter reads every message of this thread.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        // 0 is WM_QUIT and -1 is an error; both end the loop.
        if got.0 <= 0 {
            break;
        }
        if msg.message == WM_APP {
            continue;
        }
        // SAFETY: `msg` was filled in by GetMessageW just above and is passed back unchanged.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Builds the menu and the tray icon; call it on the thread that will run the message loop.
fn build_icons(ctx: &egui::Context, hwnd: Option<isize>, bus: BusSender, control: ControlSender) -> Result<TrayIcons, String> {
    let idle_icon = circle_icon(rgb_bytes(IDLE_RGB))?;
    let recording_icon = circle_icon(rgb_bytes(RECORDING_RGB))?;

    let show = MenuItem::new("Show window", true, None);
    let start_stop = MenuItem::new("Start or stop recording", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append_items(&[&show, &start_stop, &quit]).map_err(|err| format!("could not build the tray menu: {err}"))?;

    let (show_id, start_stop_id, quit_id) = (show.id().clone(), start_stop.id().clone(), quit.id().clone());
    let ctx = ctx.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            if let Some(hwnd) = hwnd {
                platform::show_and_focus(hwnd);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.request_repaint();
        } else if event.id == start_stop_id {
            let press_qpc = capture::qpc_now();
            if bus.send(Command::new(CommandKind::StartStop, press_qpc)).is_err() {
                log::warn!("a tray menu click was dropped because the app is gone");
            }
        } else if event.id == quit_id {
            control.stop();
            if let Some(hwnd) = hwnd {
                platform::show_and_focus(hwnd);
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ctx.request_repaint();
        }
    }));

    let icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_icon(idle_icon.clone())
        .with_tooltip(TOOLTIP_IDLE)
        .build()
        .map_err(|err| format!("could not create the tray icon: {err}"))?;
    Ok(TrayIcons { icon, idle_icon, recording_icon })
}

/// A token color as the RGB bytes a tray icon pixel row needs.
fn rgb_bytes(color: egui::Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

/// Draws a filled circle with a soft edge as a 32x32 RGBA icon.
fn circle_icon(rgb: [u8; 3]) -> Result<Icon, String> {
    Icon::from_rgba(circle_rgba(rgb), ICON_SIZE, ICON_SIZE).map_err(|err| format!("could not build the tray icon image: {err}"))
}

/// The RGBA pixels of a filled circle that touches a one pixel margin around the icon.
fn circle_rgba(rgb: [u8; 3]) -> Vec<u8> {
    let size = ICON_SIZE as f32;
    let centre = (size - 1.0) / 2.0;
    let radius = size / 2.0 - 1.0;
    let mut pixels = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let distance = ((x as f32 - centre).powi(2) + (y as f32 - centre).powi(2)).sqrt();
            let coverage = (radius + 0.5 - distance).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (coverage * 255.0).round() as u8]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_is_opaque_in_the_middle_and_clear_in_the_corner() {
        let pixels = circle_rgba(rgb_bytes(RECORDING_RGB));
        assert_eq!(pixels.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        let alpha_at = |x: u32, y: u32| pixels[((y * ICON_SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha_at(16, 16), 255);
        assert_eq!(alpha_at(0, 0), 0);
    }
}
