//! The foreground window hook. Out of context WinEvents arrive on the thread that set the hook,
//! while it reads its message queue; the callback only notes them, and the logger takes them after.

use std::cell::RefCell;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT};

use crate::{CaptureError, qpc_now};

/// A window that came to the foreground.
pub(crate) struct ForegroundChange {
    /// QPC stamp taken when the event arrived.
    pub(crate) qpc: i64,
    /// The HWND as an isize.
    pub(crate) window: isize,
}

thread_local! {
    /// Changes this thread's hook received and the logger has not taken yet.
    static PENDING: RefCell<Vec<ForegroundChange>> = const { RefCell::new(Vec::new()) };
}

/// The hook, removed on drop by the thread that set it.
pub(crate) struct ForegroundHook(HWINEVENTHOOK);

impl ForegroundHook {
    /// Hooks foreground changes of every process, delivered to the calling thread.
    #[allow(unsafe_code)]
    pub(crate) fn set() -> Result<Self, CaptureError> {
        // SAFETY: an out of context hook needs no module, and the callback is a plain function.
        let hook = unsafe { SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT) };
        if hook.is_invalid() {
            return Err(CaptureError::Missing("failed to hook foreground app changes"));
        }
        Ok(Self(hook))
    }

    /// The changes received since the last call, oldest first.
    pub(crate) fn take(&self) -> Vec<ForegroundChange> {
        PENDING.with_borrow_mut(std::mem::take)
    }
}

impl Drop for ForegroundHook {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the hook is ours, removed once, on its own thread.
        if !unsafe { UnhookWinEvent(self.0) }.as_bool() {
            log::warn!("could not remove the foreground app hook");
        }
    }
}

/// The WinEvent callback. It must not panic, so it skips a change it cannot store.
#[allow(unsafe_code)]
unsafe extern "system" fn on_foreground(_hook: HWINEVENTHOOK, _event: u32, window: HWND, _object: i32, _child: i32, _thread: u32, _time: u32) {
    if window.is_invalid() {
        return;
    }
    let change = ForegroundChange { qpc: qpc_now(), window: window.0 as isize };
    let _ = PENDING.try_with(|pending| {
        if let Ok(mut pending) = pending.try_borrow_mut() {
            pending.push(change);
        }
    });
}
