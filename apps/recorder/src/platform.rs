//! Windows-specific window helpers.

use std::path::PathBuf;

use raw_window_handle::HasWindowHandle;

/// Opens Explorer with `path` selected; works for files and folders, and a failure is logged, not shown.
pub fn show_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    if let Err(err) = std::process::Command::new("explorer").arg(format!("/select,{}", path.display())).spawn() {
        log::warn!("could not open Explorer for {}: {err}", path.display());
    }
    #[cfg(not(windows))]
    let _ = path;
}

/// Opens a folder in Explorer; a failure is logged, not shown.
pub fn open_folder(folder: &std::path::Path) {
    if let Err(err) = std::process::Command::new("explorer").arg(folder).spawn() {
        log::warn!("could not open {}: {err}", folder.display());
    }
}

/// Opens the Windows notification settings page; the Recorder never changes the setting itself.
/// Opens the Windows notification settings page; the Recorder never changes the setting itself.
pub fn open_notification_settings() {
    #[cfg(windows)]
    if let Err(err) = std::process::Command::new("explorer").arg("ms-settings:notifications").spawn() {
        log::warn!("could not open the notification settings: {err}");
    }
}

/// Opens a file picker filtered to PNG and JPG images; None when it fails or the user cancels.
pub fn pick_image_file() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        pick_file("Images", "*.png;*.jpg;*.jpeg")
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Opens a file picker filtered to MP4 and MKV video files; None when it fails or the user cancels.
pub fn pick_video_file() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        pick_file("Videos", "*.mp4;*.mkv")
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Runs the image or video file picker on this thread's COM apartment.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the shell file picker: COM calls that run a modal open dialog")]
fn pick_file(filter_name: &'static str, filter_spec: &'static str) -> Option<PathBuf> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HRESULT;

    // SAFETY: CoInitializeEx only takes an apartment flag; a returned S_OK is balanced by CoUninitialize below.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let file = pick_with_dialog(filter_name, filter_spec);
    if let Err(error) = &file {
        log::warn!("the file picker failed: {error}");
    }
    if hr == HRESULT(0) {
        // SAFETY: this thread was the one that got S_OK above, and every COM pointer is released by now.
        unsafe { CoUninitialize() };
    }
    file.ok().flatten()
}

/// Shows the dialog with the given filter and turns the picked shell item into a path; None when the user cancels.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the shell file picker: COM calls that run a modal open dialog")]
fn pick_with_dialog(filter_name: &'static str, filter_spec: &'static str) -> Result<Option<PathBuf>, windows::core::Error> {
    use std::path::PathBuf;

    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{
        FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FileOpenDialog, IFileDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::core::{HRESULT, HSTRING, Interface, PCWSTR};

    // SAFETY: FileOpenDialog is the standard dialog class id and the interface is a valid out parameter.
    let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
    let file_dialog: IFileDialog = dialog.cast()?;
    let name = HSTRING::from(filter_name);
    let spec = HSTRING::from(filter_spec);
    let all_name = HSTRING::from("All files");
    let all_spec = HSTRING::from("*.*");
    // SAFETY: the dialog is live; the calls only set picker options and the filter list, whose strings
    // are live HSTRINGs for the duration of the call.
    unsafe {
        file_dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST)?;
        file_dialog.SetFileTypes(&[
            COMDLG_FILTERSPEC { pszName: PCWSTR(name.as_ptr()), pszSpec: PCWSTR(spec.as_ptr()) },
            COMDLG_FILTERSPEC { pszName: PCWSTR(all_name.as_ptr()), pszSpec: PCWSTR(all_spec.as_ptr()) },
        ])?;
    }
    // SAFETY: the dialog is live and a null owner means the dialog has no parent window.
    if let Err(error) = unsafe { file_dialog.Show(None) } {
        // Cancelling the dialog comes back as HRESULT 0x800704C7, which is not a failure.
        return if error.code() == HRESULT(0x8007_04C7u32 as i32) { Ok(None) } else { Err(error) };
    }
    let item = unsafe { dialog.GetResult() }?;
    // SAFETY: the item is live and SIGDN_FILESYSPATH asks for a file system path.
    let display = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
    // SAFETY: display is a live NUL terminated string handed out by GetDisplayName.
    let path = unsafe { display.to_string() }.ok().map(PathBuf::from);
    // SAFETY: balances the allocation behind GetDisplayName, which is done with once the path is taken.
    unsafe { CoTaskMemFree(Some(display.as_ptr().cast())) };
    Ok(path)
}

/// Hides a window from screen capture, or shows it to capture again, on a raw window handle value.
pub fn set_capture_exclusion(raw_hwnd: isize, exclude: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        set_display_affinity(raw_hwnd, exclude)
    }
    #[cfg(not(windows))]
    {
        let _ = (raw_hwnd, exclude);
        Ok(())
    }
}

/// Opens a folder picker titled `title`; None when it fails or the user cancels.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        pick_folder_dialog(title)
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        None
    }
}

/// Runs the folder picker on this thread's COM apartment.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the shell folder picker: COM calls that run a modal open dialog")]
fn pick_folder_dialog(title: &str) -> Option<PathBuf> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
    use windows::core::HRESULT;

    // SAFETY: CoInitializeEx only takes an apartment flag; a returned S_OK is balanced by CoUninitialize below.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let folder = pick_with_folder_dialog(title);
    if let Err(error) = &folder {
        log::warn!("the folder picker failed: {error}");
    }
    if hr == HRESULT(0) {
        // SAFETY: this thread was the one that got S_OK above, and every COM pointer is released by now.
        unsafe { CoUninitialize() };
    }
    folder.ok().flatten()
}

/// Shows the folder dialog with the given title and turns the picked shell item into a path; None when the user cancels.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the shell folder picker: COM calls that run a modal open dialog")]
fn pick_with_folder_dialog(title: &str) -> Result<Option<PathBuf>, windows::core::Error> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
    use windows::Win32::UI::Shell::{FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, FileOpenDialog, IFileDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    use windows::core::{HRESULT, HSTRING, Interface, PCWSTR};

    // SAFETY: FileOpenDialog is the standard dialog class id and the interface is a valid out parameter.
    let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? };
    let file_dialog: IFileDialog = dialog.cast()?;
    let title_h = HSTRING::from(title);
    // SAFETY: the dialog is live; the calls only set picker options and the title, whose string is live.
    unsafe {
        file_dialog.SetOptions(FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST | FOS_PICKFOLDERS)?;
        file_dialog.SetTitle(PCWSTR(title_h.as_ptr()))?;
    }
    // SAFETY: the dialog is live and a null owner means the dialog has no parent window.
    if let Err(error) = unsafe { file_dialog.Show(None) } {
        // Cancelling the dialog comes back as HRESULT 0x800704C7, which is not a failure.
        return if error.code() == HRESULT(0x8007_04C7u32 as i32) { Ok(None) } else { Err(error) };
    }
    let item = unsafe { dialog.GetResult() }?;
    // SAFETY: the item is live and SIGDN_FILESYSPATH asks for a file system path.
    let display = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH)? };
    // SAFETY: display is a live NUL terminated string handed out by GetDisplayName.
    let path = unsafe { display.to_string() }.ok().map(PathBuf::from);
    // SAFETY: balances the allocation behind GetDisplayName, which is done with once the path is taken.
    unsafe { CoTaskMemFree(Some(display.as_ptr().cast())) };
    Ok(path)
}

/// Gets the raw Win32 window handle of a freshly created window as an `isize`.
pub fn raw_hwnd(handle: &impl HasWindowHandle) -> Result<isize, String> {
    #[cfg(windows)]
    {
        use raw_window_handle::RawWindowHandle;

        let window = handle.window_handle().map_err(|err| format!("no window handle: {err}"))?;
        match window.as_raw() {
            RawWindowHandle::Win32(win32) => Ok(win32.hwnd.get()),
            other => Err(format!("not a Win32 window handle: {other:?}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = handle;
        Ok(0)
    }
}

/// Un-minimizes, shows and focuses the window so eframe starts running frames again.
pub fn show_and_focus(raw_hwnd: isize) {
    #[cfg(windows)]
    show_and_focus_win32(raw_hwnd);
    #[cfg(not(windows))]
    let _ = raw_hwnd;
}

/// Calls ShowWindow(SW_SHOW), ShowWindow(SW_RESTORE) and SetForegroundWindow on a raw window handle value.
#[cfg(windows)]
#[allow(unsafe_code, reason = "Win32 calls that un-minimize, show and focus the window hidden to the tray")]
fn show_and_focus_win32(raw_hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SW_RESTORE, SW_SHOW, SetForegroundWindow, ShowWindow};

    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    // SAFETY: `raw_hwnd` was taken from the live creation context of this window, so the window exists.
    // Each call only reads the handle and a plain flag value, and it hands back no pointers.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Calls `SetWindowDisplayAffinity` with `WDA_EXCLUDEFROMCAPTURE` or `WDA_NONE` on a raw window handle value.
#[cfg(windows)]
#[allow(unsafe_code, reason = "the only FFI call in the Recorder besides the tray helpers: a Win32 flag that excludes a window from capture")]
fn set_display_affinity(raw_hwnd: isize, exclude: bool) -> Result<(), String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE, WDA_NONE};

    let hwnd = HWND(raw_hwnd as *mut core::ffi::c_void);
    let affinity = if exclude { WDA_EXCLUDEFROMCAPTURE } else { WDA_NONE };
    // SAFETY: `raw_hwnd` was taken from the live creation context of this window, so the window exists.
    // The call only reads the handle and a plain flag value, and it hands back no pointers.
    unsafe { SetWindowDisplayAffinity(hwnd, affinity) }.map_err(|err| format!("SetWindowDisplayAffinity failed: {err}"))
}
