//! Whether a process id belongs to a live process, for the single-instance check
//! (spec: Remote Control "Discovery files").

/// Whether `pid` belongs to a running process. Non-Windows treats every pid as gone:
/// the suite ships on Windows only, and there the check just disables itself.
pub(crate) fn alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        windows_alive(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

/// The Win32 check: a process that can be opened and has not exited is alive.
#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject};

    // SAFETY: OpenProcess and WaitForSingleObject are plain Win32 calls on a pid we
    // read from a file; the handle is closed before returning.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            // A pid that cannot be opened at all is gone (or fully protected); treat it
            // as gone so a stale discovery file can never lock the app out permanently.
            return false;
        }
        let exited = WaitForSingleObject(handle, 0) == WAIT_OBJECT_0;
        CloseHandle(handle);
        !exited
    }
}
