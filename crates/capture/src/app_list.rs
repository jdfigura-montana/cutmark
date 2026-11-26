//! The running processes, and the list of apps for the per-app audio picker.
//!
//! See the per-app audio spec: Capture.

use std::collections::{BTreeMap, BTreeSet};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, LPARAM, S_OK, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDevice};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::CLSCTX_ALL;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    QueryFullProcessImageNameW, WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GW_OWNER, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible};
use windows::core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR};

use crate::app_audio::{AudioApp, ProcessEntry, normalize_exe_name};
use crate::audio::{ComGuard, collection_count, collection_item, device_enumerator, enum_render_endpoints};
use crate::{CaptureError, utf16_to_string, with_context};

/// The longest executable path Windows returns, in UTF-16 units.
const MAX_IMAGE_PATH: usize = 32_768;

/// An open process handle, closed on drop.
pub(crate) struct ProcessHandle(HANDLE);

impl ProcessHandle {
    /// Opens a process with the given rights; None when Windows refuses or it is gone.
    #[allow(unsafe_code)]
    fn open(pid: u32, rights: PROCESS_ACCESS_RIGHTS) -> Option<Self> {
        // SAFETY: plain call with value arguments; the handle is owned by the returned value.
        unsafe { OpenProcess(rights, false, pid) }.ok().map(Self)
    }

    /// Opens a process to wait for its exit.
    pub(crate) fn open_for_exit(pid: u32) -> Option<Self> {
        Self::open(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE)
    }

    /// True once the process has exited. Needs a handle from `open_for_exit`.
    #[allow(unsafe_code)]
    pub(crate) fn has_exited(&self) -> bool {
        // SAFETY: the handle is open until drop; a zero timeout only polls.
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }

    /// When the process was created, in FILETIME units.
    #[allow(unsafe_code)]
    fn created(&self) -> Option<u64> {
        let mut created = FILETIME::default();
        let (mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        // SAFETY: the handle is open and every out pointer points at a local.
        unsafe { GetProcessTimes(self.0, &mut created, &mut exited, &mut kernel, &mut user) }.ok()?;
        Some(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
    }

    /// The full path of the process's executable.
    #[allow(unsafe_code)]
    fn image_path(&self) -> Option<String> {
        let mut path = vec![0u16; MAX_IMAGE_PATH];
        let mut len = path.len() as u32;
        // SAFETY: len is the buffer's capacity going in and the written length coming out.
        unsafe { QueryFullProcessImageNameW(self.0, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len) }.ok()?;
        Some(String::from_utf16_lossy(path.get(..len as usize)?))
    }
}

impl Drop for ProcessHandle {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the handle is ours and closed exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The running processes. Creation times are not filled in; see `fill_creation_times`.
#[allow(unsafe_code)]
pub(crate) fn process_table() -> Result<Vec<ProcessEntry>, CaptureError> {
    // SAFETY: a process snapshot takes no pointers; we own the returned handle.
    let snapshot = ProcessHandle(with_context("failed to snapshot the running processes", unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })?);
    let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut table = Vec::new();
    // SAFETY: the snapshot is open and entry has its size set.
    let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    while next.is_ok() {
        table.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            exe: normalize_exe_name(&utf16_to_string(&entry.szExeFile)),
            created: None,
        });
        // SAFETY: as above.
        next = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
    Ok(table)
}

/// Fills in the creation time of the processes of `exe` and of their ancestors.
pub(crate) fn fill_creation_times(table: &mut [ProcessEntry], exe: &str) {
    let parent_of: BTreeMap<u32, u32> = table.iter().map(|process| (process.pid, process.parent_pid)).collect();
    let mut wanted = BTreeSet::new();
    for process in table.iter().filter(|process| process.exe == exe) {
        let mut pid = process.pid;
        // Stops at an ID that is already listed, which also ends a cycle.
        while wanted.insert(pid) {
            let Some(&parent) = parent_of.get(&pid) else {
                break;
            };
            pid = parent;
        }
    }
    for process in table.iter_mut().filter(|process| wanted.contains(&process.pid)) {
        process.created = ProcessHandle::open(process.pid, PROCESS_QUERY_LIMITED_INFORMATION).and_then(|handle| handle.created());
    }
}

/// Lists the running apps that own an audio session on an active render device;
/// with `include_all`, also those that only have a visible top-level window.
/// Sorted by display name. This process is left out.
pub fn list_audio_apps(include_all: bool) -> Result<Vec<AudioApp>, CaptureError> {
    let _com = ComGuard::new()?;
    let table = process_table()?;
    let own_pid = std::process::id();
    let exe_of: BTreeMap<u32, &str> =
        table.iter().filter(|process| process.pid != 0 && process.pid != own_pid).map(|process| (process.pid, process.exe.as_str())).collect();

    // Per executable: one process to read the description from, and whether it is making sound.
    let mut found: BTreeMap<&str, (u32, bool)> = BTreeMap::new();
    for (pid, active) in audio_sessions()? {
        if let Some(&exe) = exe_of.get(&pid) {
            found.entry(exe).or_insert((pid, false)).1 |= active;
        }
    }
    if include_all {
        for pid in window_processes() {
            if let Some(&exe) = exe_of.get(&pid) {
                found.entry(exe).or_insert((pid, false));
            }
        }
    }

    let mut apps: Vec<AudioApp> = found
        .into_iter()
        .map(|(exe, (pid, active))| {
            let mut pids: Vec<u32> = exe_of.iter().filter(|(_, other)| **other == exe).map(|(&pid, _)| pid).collect();
            pids.sort_unstable();
            let description =
                ProcessHandle::open(pid, PROCESS_QUERY_LIMITED_INFORMATION).and_then(|handle| handle.image_path()).and_then(|path| file_description(&path));
            AudioApp { exe: exe.to_owned(), display_name: description.unwrap_or_else(|| exe.to_owned()), pids, active }
        })
        .collect();
    apps.sort_by_cached_key(|app| (app.display_name.to_lowercase(), app.exe.clone()));
    Ok(apps)
}

/// The owner process and active state of every audio session on every active
/// render device. A device that cannot be read is logged and skipped.
fn audio_sessions() -> Result<Vec<(u32, bool)>, CaptureError> {
    let enumerator = device_enumerator()?;
    let devices = with_context("failed to enumerate audio endpoints", enum_render_endpoints(&enumerator))?;
    let count = with_context("failed to count audio endpoints", collection_count(&devices))?;
    let mut sessions = Vec::new();
    for index in 0..count {
        let read = collection_item(&devices, index).and_then(|device| device_sessions(&device, &mut sessions));
        if let Err(err) = read {
            log::warn!("skipping the audio sessions of a render device: {err}");
        }
    }
    Ok(sessions)
}

/// Adds one device's sessions to `sessions`, without the system sounds.
#[allow(unsafe_code)]
fn device_sessions(device: &IMMDevice, sessions: &mut Vec<(u32, bool)>) -> windows::core::Result<()> {
    // SAFETY: no activation parameters; the result is a new owned interface reference.
    let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }?;
    // SAFETY: plain COM call with an interface out parameter.
    let list = unsafe { manager.GetSessionEnumerator() }?;
    // SAFETY: only writes the out parameter.
    let count = unsafe { list.GetCount() }?;
    for index in 0..count {
        // SAFETY: index is below GetCount; a session that just ended returns an error.
        let Ok(control) = (unsafe { list.GetSession(index) }).and_then(|session| session.cast::<IAudioSessionControl2>()) else {
            continue;
        };
        // SAFETY: control is a live session; S_OK means it is the system sounds session.
        if unsafe { control.IsSystemSoundsSession() } == S_OK {
            continue;
        }
        // SAFETY: both calls only write their out parameters.
        let (pid, state) = unsafe { (control.GetProcessId(), control.GetState()) };
        if let (Ok(pid), Ok(state)) = (pid, state) {
            sessions.push((pid, state == AudioSessionStateActive));
        }
    }
    Ok(())
}

/// The processes that own a visible, titled window which no other window owns.
#[allow(unsafe_code)]
fn window_processes() -> BTreeSet<u32> {
    let mut pids = BTreeSet::new();
    // SAFETY: the callback only runs during this call, while pids is alive and not otherwise used.
    let listed = unsafe { EnumWindows(Some(collect_window_process), LPARAM(&raw mut pids as isize)) };
    if let Err(err) = listed {
        log::warn!("failed to enumerate the top-level windows: {err}");
    }
    pids
}

/// The EnumWindows callback of `window_processes`.
#[allow(unsafe_code)]
unsafe extern "system" fn collect_window_process(window: HWND, pids: LPARAM) -> BOOL {
    // SAFETY: window is the handle EnumWindows passes in; these calls only query it.
    let listed = unsafe { IsWindowVisible(window).as_bool() && GetWindow(window, GW_OWNER).is_err() && GetWindowTextLengthW(window) > 0 };
    if listed {
        let mut pid = 0u32;
        // SAFETY: pid is a local out parameter.
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
        // SAFETY: pids is the set window_processes passed in, borrowed by nothing else during the call.
        unsafe { &mut *(pids.0 as *mut BTreeSet<u32>) }.insert(pid);
    }
    BOOL::from(true)
}

/// The FileDescription of an executable's version resource; None when it has none.
fn file_description(path: &str) -> Option<String> {
    let block = version_block(&HSTRING::from(path))?;
    // The translation table lists the (language, code page) pairs of the string tables.
    let translation = version_value(&block, r"\VarFileInfo\Translation", 1).and_then(|bytes| bytes.get(..4));
    let listed = translation.map(|pair| (u16::from_le_bytes([pair[0], pair[1]]), u16::from_le_bytes([pair[2], pair[3]])));
    // US English in Unicode and in Windows-1252 cover resources without a usable table.
    listed.into_iter().chain([(0x0409, 0x04B0), (0x0409, 0x04E4)]).find_map(|(language, code_page)| {
        let name = format!(r"\StringFileInfo\{language:04x}{code_page:04x}\FileDescription");
        // A string value's length counts UTF-16 units.
        let bytes = version_value(&block, &name, 2)?;
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|unit| u16::from_le_bytes(*unit)).collect();
        Some(utf16_to_string(&units).trim().to_owned()).filter(|text| !text.is_empty())
    })
}

/// The version resource of a file.
#[allow(unsafe_code)]
fn version_block(path: &HSTRING) -> Option<Vec<u8>> {
    // SAFETY: path is a valid wide string; no handle out parameter is asked for.
    let size = unsafe { GetFileVersionInfoSizeW(path, None) };
    if size == 0 {
        return None;
    }
    let mut block = vec![0u8; size as usize];
    // SAFETY: block holds exactly size bytes.
    unsafe { GetFileVersionInfoW(path, None, size, block.as_mut_ptr().cast()) }.ok()?;
    Some(block)
}

/// The bytes of one value inside a version resource; `unit` is the size of what its length counts.
#[allow(unsafe_code)]
fn version_value<'a>(block: &'a [u8], name: &str, unit: usize) -> Option<&'a [u8]> {
    let name = HSTRING::from(name);
    let mut data: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: block is a version resource from GetFileVersionInfoW; the out pointers are locals.
    let found = unsafe { VerQueryValueW(block.as_ptr().cast(), PCWSTR(name.as_ptr()), &mut data, &mut len) };
    if !found.as_bool() || data.is_null() {
        return None;
    }
    // Translation lengths count bytes (unit 1), string lengths UTF-16 units (unit 2).
    let bytes = len as usize * unit;
    let offset = (data as usize).checked_sub(block.as_ptr() as usize)?;
    block.get(offset..offset.checked_add(bytes)?)
}
