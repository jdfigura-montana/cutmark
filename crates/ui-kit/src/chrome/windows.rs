//! The Windows half of the chrome: caption removal, hit testing, the maximize
//! button and the DWM corner and animation settings (spec: Design Language
//! "Window chrome", S1 row "Window chrome").
//!
//! Only this file may hold `unsafe`: the crate lints deny `unsafe_code` and
//! this module lifts the deny with `#![allow(unsafe_code)]`. Every unsafe
//! block carries a SAFETY comment (S1 Amendments, item 3). The subclass holds
//! shared state behind an `Arc` that lives in [`SHARED`]; the suite keeps one
//! chrome window per process, which is what the apps have.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;
use std::sync::MutexGuard;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use egui::{Pos2, Vec2};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT, HTMAXBUTTON, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed,
    NCCALCSIZE_PARAMS, PostMessageW, SC_MAXIMIZE, SC_RESTORE, SM_CXFRAME, SM_CXPADDEDBORDER, SPI_GETCLIENTAREAANIMATION, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SystemParametersInfoW, WM_NCCALCSIZE, WM_NCDESTROY, WM_NCHITTEST, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SETTINGCHANGE,
    WM_SYSCOMMAND,
};
use windows::core::BOOL;

use super::{HitRegions, Regions, ResizeEdge, Zone, hit_test};

/// The subclass id for this module's window procedure.
const SUBCLASS_ID: usize = 0x4355_544D; // 'CUTM'

/// The one chrome window's shared state; `None` before install and after
/// `WM_NCDESTROY`.
static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);

/// Set once the subclass is in place: `is_custom` reads it.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// The state shared between the egui frame (publish) and the subclass.
struct Shared {
    /// The egui context, for repaints and the runtime theme.
    ctx: egui::Context,
    /// Everything the window procedure reads and writes.
    state: Mutex<State>,
}

/// What the window procedure reads and writes.
#[derive(Default)]
struct State {
    /// The published regions in physical pixels.
    regions: HitRegions,
    /// The pointer is over the maximize button (non-client only, so egui
    /// cannot track it).
    maximize_hover: bool,
    /// The left button went down on the maximize button and has not come up.
    max_button_down: bool,
    /// A TME_LEAVE|TME_NONCLIENT track is active for the maximize button.
    tracking: bool,
}

/// Locks a mutex, recovering from poison: the guarded chrome state is plain
/// data, so a poisoned lock says nothing worth losing the window over.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Clones the shared state out of the global, if the chrome is installed.
fn shared() -> Option<Arc<Shared>> {
    lock(&SHARED).clone()
}

/// Hooks the eframe window at startup: subclass, round corners and the
/// animation setting. Failures log and leave the native title bar in place
/// (the recorded fallback).
pub(super) fn install(cc: &eframe::CreationContext) {
    let handle = match cc.window_handle() {
        Ok(handle) => handle,
        Err(error) => {
            log::warn!("chrome: no window handle: {error}");
            return;
        }
    };
    let hwnd = match handle.as_raw() {
        RawWindowHandle::Win32(handle) => HWND(handle.hwnd.get() as *mut c_void as _),
        other => {
            log::warn!("chrome: unsupported window handle {other:?}");
            return;
        }
    };
    hook(hwnd, &cc.egui_ctx);
}

/// Subclasses `hwnd` with the chrome procedure, sets the round corners and
/// reads the animation setting.
///
/// The first window wins: when a window is already hooked, `hwnd` keeps its
/// native title bar and the hooked window's state is never overwritten.
fn hook(hwnd: HWND, ctx: &egui::Context) {
    if shared().is_some() {
        log::warn!("chrome: a window is already hooked; the new window keeps the native title bar");
        return;
    }
    let shared = Arc::new(Shared { ctx: ctx.clone(), state: Mutex::new(State::default()) });

    // SAFETY: hwnd is the live top-level window eframe just created, on this
    // thread. The subclass id is ours alone, and the reference data (the Arc's
    // raw pointer) stays valid because SHARED holds a clone of the Arc until
    // WM_NCDESTROY drops it.
    let ok = unsafe { SetWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID, Arc::as_ptr(&shared) as usize) };
    if !ok.as_bool() {
        log::warn!("chrome: SetWindowSubclass failed; keeping the native title bar");
        return;
    }
    *lock(&SHARED) = Some(shared);
    INSTALLED.store(true, Ordering::Release);

    // Round window corners from the DWM (spec: Window chrome, Windows 11).
    // SAFETY: hwnd is a live window and the attribute reads one 4-byte value
    // behind a pointer we pass with its size.
    let preference = DWM_WINDOW_CORNER_PREFERENCE(DWMWCP_ROUND.0);
    unsafe {
        let _ =
            DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, ptr::from_ref(&preference).cast(), size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32);
    }
    apply_client_area_animation(ctx);
}

/// Converts the frame's regions to physical pixels and hands them to the
/// subclass's next hit test.
pub(super) fn publish(ctx: &egui::Context, regions: &Regions) {
    let Some(shared) = shared() else { return };
    let pixels_per_point = ctx.pixels_per_point();
    let scale = |rect: egui::Rect| super::to_physical(rect, pixels_per_point);
    let mut state = lock(&shared.state);
    state.regions = HitRegions { caption: regions.caption().iter().copied().map(scale).collect(), maximize: regions.maximize_button().map(scale) };
}

/// True once the subclass is in place.
pub(super) fn is_custom() -> bool {
    INSTALLED.load(Ordering::Acquire)
}

/// True while the pointer is over the maximize button, per the subclass.
pub(super) fn is_maximized_button_hovered() -> bool {
    match shared() {
        Some(shared) => lock(&shared.state).maximize_hover,
        None => false,
    }
}

/// The chrome's subclass procedure: caption removal, hit test, maximize
/// button tracking and the setting change hook.
///
/// SAFETY: called by the OS with an `hwnd` that belongs to this window; the
/// reference data is the `Arc<Shared>` pointer `install` passed.
unsafe extern "system" fn chrome_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, ref_data: usize) -> LRESULT {
    // SAFETY: ref_data is the Arc::as_ptr value install passed to
    // SetWindowSubclass; it stays valid until WM_NCDESTROY drops the Arc.
    let Some(shared) = (unsafe { (ref_data as *const Shared).as_ref() }) else {
        // SAFETY: no reference data: forward to the subclass chain untouched.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    };
    let handled: Option<LRESULT> = match msg {
        // Returning 0 with wParam TRUE turns the whole window into client
        // area: the caption disappears while frame, shadow and snap stay.
        // Maximized windows extend past the work area by the frame thickness,
        // so the content is inset to keep it on screen.
        WM_NCCALCSIZE if wparam.0 != 0 => {
            if is_zoomed(hwnd) {
                let frame = frame_thickness(hwnd);
                // SAFETY: lparam points at the NCCALCSIZE_PARAMS the OS filled
                // in for this call; only the outer rectangle is adjusted.
                let params = unsafe { &mut *(lparam.0 as *mut NCCALCSIZE_PARAMS) };
                let rect = &mut params.rgrc[0];
                rect.left += frame;
                rect.top += frame;
                rect.right -= frame;
                rect.bottom -= frame;
            }
            Some(LRESULT(0))
        }
        WM_NCHITTEST => {
            let mut rect = RECT::default();
            // SAFETY: hwnd is a live window; rect is a local the API fills in.
            unsafe {
                let _ = GetWindowRect(hwnd, &mut rect);
            }
            let point = Pos2::new((signed_loword(lparam) - rect.left) as f32, (signed_hiword(lparam) - rect.top) as f32);
            let size = Vec2::new((rect.right - rect.left) as f32, (rect.bottom - rect.top) as f32);
            let zone = {
                let state = lock(&shared.state);
                hit_test(point, size, is_zoomed(hwnd), &state.regions, dpi(hwnd))
            };
            Some(LRESULT(ht(zone) as isize))
        }
        WM_NCMOUSEMOVE if wparam.0 as u32 == HTMAXBUTTON => {
            let mut changed = false;
            {
                let mut state = lock(&shared.state);
                if !state.maximize_hover {
                    state.maximize_hover = true;
                    changed = true;
                }
                if !state.tracking {
                    // SAFETY: a local TRACKMOUSEEVENT pointing at our live
                    // hwnd, so WM_NCMOUSELEAVE comes back to us.
                    let mut track =
                        TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE | TME_NONCLIENT, hwndTrack: hwnd, dwHoverTime: 0 };
                    if unsafe { TrackMouseEvent(&mut track) }.is_ok() {
                        state.tracking = true;
                    }
                }
            }
            if changed {
                shared.ctx.request_repaint();
            }
            None
        }
        WM_NCMOUSELEAVE => {
            let mut changed = false;
            {
                let mut state = lock(&shared.state);
                state.tracking = false;
                if state.maximize_hover {
                    state.maximize_hover = false;
                    changed = true;
                }
            }
            if changed {
                shared.ctx.request_repaint();
            }
            None
        }
        // egui never sees non-client clicks, so the maximize and restore
        // happen here.
        WM_NCLBUTTONDOWN if wparam.0 as u32 == HTMAXBUTTON => {
            lock(&shared.state).max_button_down = true;
            Some(LRESULT(0))
        }
        WM_NCLBUTTONUP => {
            let fire = {
                let mut state = lock(&shared.state);
                let was_down = state.max_button_down;
                state.max_button_down = false;
                was_down && wparam.0 as u32 == HTMAXBUTTON
            };
            if fire {
                let command = if is_zoomed(hwnd) { SC_RESTORE } else { SC_MAXIMIZE };
                // SAFETY: posts to our own live window; the OS then runs the
                // standard maximize or restore path.
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_SYSCOMMAND, WPARAM(command as usize), LPARAM(0));
                }
            }
            None
        }
        // The user changed the animation setting (or similar); re-read it.
        WM_SETTINGCHANGE => {
            apply_client_area_animation(&shared.ctx);
            None
        }
        WM_NCDESTROY => {
            // SAFETY: the window is being destroyed and the subclass removes
            // itself; the shared Arc is dropped with it.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(chrome_proc), SUBCLASS_ID);
            }
            *lock(&SHARED) = None;
            INSTALLED.store(false, Ordering::Release);
            None
        }
        _ => None,
    };
    // SAFETY: forwards to the next procedure in the subclass chain, which is
    // the protocol for every message we do not fully handle ourselves.
    match handled {
        Some(result) => result,
        None => unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) },
    }
}

/// Maps a hit-test zone to its `WM_NCHITTEST` answer.
fn ht(zone: Zone) -> u32 {
    match zone {
        Zone::Client => HTCLIENT,
        Zone::Caption => HTCAPTION,
        Zone::MaximizeButton => HTMAXBUTTON,
        Zone::Resize(ResizeEdge::Left) => HTLEFT,
        Zone::Resize(ResizeEdge::Right) => HTRIGHT,
        Zone::Resize(ResizeEdge::Top) => HTTOP,
        Zone::Resize(ResizeEdge::Bottom) => HTBOTTOM,
        Zone::Resize(ResizeEdge::TopLeft) => HTTOPLEFT,
        Zone::Resize(ResizeEdge::TopRight) => HTTOPRIGHT,
        Zone::Resize(ResizeEdge::BottomLeft) => HTBOTTOMLEFT,
        Zone::Resize(ResizeEdge::BottomRight) => HTBOTTOMRIGHT,
    }
}

/// The screen x coordinate of a mouse message's lParam.
fn signed_loword(lparam: LPARAM) -> i32 {
    (lparam.0 as u16) as i16 as i32
}

/// The screen y coordinate of a mouse message's lParam.
fn signed_hiword(lparam: LPARAM) -> i32 {
    ((lparam.0 >> 16) as u16) as i16 as i32
}

/// True when the window is maximized.
fn is_zoomed(hwnd: HWND) -> bool {
    // SAFETY: hwnd is a live window; the call only reads its state.
    unsafe { IsZoomed(hwnd) }.as_bool()
}

/// The window's pixels per point, from its DPI.
fn dpi(hwnd: HWND) -> f32 {
    // SAFETY: hwnd is a live window; the call only reads its DPI.
    (unsafe { GetDpiForWindow(hwnd) }) as f32 / 96.0
}

/// The resize frame thickness at the window's DPI, in physical pixels.
fn frame_thickness(hwnd: HWND) -> i32 {
    // SAFETY: hwnd is a live window; both calls only read system metrics.
    unsafe {
        let dpi_value = GetDpiForWindow(hwnd);
        GetSystemMetricsForDpi(SM_CXFRAME, dpi_value) + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi_value)
    }
}

/// Reads `SPI_GETCLIENTAREAANIMATION` and moves it into the runtime theme:
/// animations off means `Theme::reduce_motion` on (and back).
fn apply_client_area_animation(ctx: &egui::Context) {
    let mut animated = BOOL::default();
    // SAFETY: pvparam points at the BOOL the action writes, and the action id
    // matches that parameter type.
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, Some(ptr::from_mut(&mut animated).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    let reduce_motion = !animated.as_bool();
    let mut theme = crate::Theme::get(ctx);
    if theme.reduce_motion != reduce_motion {
        theme.reduce_motion = reduce_motion;
        crate::Theme::set(ctx, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes the tests that touch the process-wide SHARED global.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// A fresh shared state for one test.
    fn test_shared() -> Arc<Shared> {
        Arc::new(Shared { ctx: egui::Context::default(), state: Mutex::new(State::default()) })
    }

    /// The install hook never overwrites an already-hooked window: the second
    /// call returns before touching the Win32 side, and the first window's
    /// state stays in the global.
    #[test]
    fn hook_refuses_to_overwrite_an_already_hooked_window() {
        let _order = lock(&TEST_LOCK);
        let existing = test_shared();
        let existing_ptr = Arc::as_ptr(&existing) as usize;
        *lock(&SHARED) = Some(existing);
        INSTALLED.store(true, Ordering::Release);

        // A null HWND would fail any Win32 call; the guard must return before
        // the subclass step, so nothing here touches the OS.
        hook(HWND(ptr::null_mut()), &egui::Context::default());

        let guard = lock(&SHARED);
        let current = guard.as_ref().map(|shared| Arc::as_ptr(shared) as usize);
        assert_eq!(current, Some(existing_ptr), "the first window's shared state must survive");
        assert!(INSTALLED.load(Ordering::Acquire), "install state must stay true");
        drop(guard);

        // Leave the globals clean for any other test.
        *lock(&SHARED) = None;
        INSTALLED.store(false, Ordering::Release);
    }

    /// After the guard path the hook proceeds normally once the global is
    /// empty again (the null HWND fails the subclass step, which logs and
    /// leaves the globals untouched instead of panicking).
    #[test]
    fn hook_with_a_free_slot_and_a_bad_hwnd_fails_without_panic_or_overwrite() {
        let _order = lock(&TEST_LOCK);
        *lock(&SHARED) = None;
        INSTALLED.store(false, Ordering::Release);

        hook(HWND(ptr::null_mut()), &egui::Context::default());

        assert!(lock(&SHARED).is_none(), "a failed subclass must not leave state behind");
        assert!(!INSTALLED.load(Ordering::Acquire), "a failed subclass must not report a custom chrome");
    }
}
