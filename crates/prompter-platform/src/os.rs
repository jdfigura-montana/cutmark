//! OS version check: the Prompter supports Windows 11 only.

/// The first Windows 11 build. Windows 11 still reports major version 10, so the build number is
/// what tells it apart from Windows 10.
#[cfg(any(windows, test))]
const FIRST_WINDOWS_11_BUILD: u32 = 22_000;

/// Why [`check_windows_11`] failed.
#[derive(Debug, thiserror::Error)]
pub enum OsError {
    /// Windows is older than Windows 11.
    #[error("Windows build {build} is older than Windows 11")]
    TooOld {
        /// The build number Windows reported.
        build: u32,
    },
    /// Windows did not report its version.
    #[error("the Windows version query failed with status {status:#010x}")]
    Query {
        /// The NTSTATUS the query returned.
        status: i32,
    },
}

/// The build-number rule, kept pure so every platform can test it.
#[cfg(any(windows, test))]
fn is_windows_11_build(build: u32) -> bool {
    build >= FIRST_WINDOWS_11_BUILD
}

/// Succeeds on Windows 11 or later.
#[cfg(windows)]
pub fn check_windows_11() -> Result<(), OsError> {
    let build = windows_build()?;
    if is_windows_11_build(build) { Ok(()) } else { Err(OsError::TooOld { build }) }
}

/// There is no version rule outside Windows.
#[cfg(not(windows))]
pub fn check_windows_11() -> Result<(), OsError> {
    Ok(())
}

/// The running Windows build number. `RtlGetVersion` reports the real version whatever the
/// executable's manifest says, which `GetVersionExW` does not.
#[cfg(windows)]
fn windows_build() -> Result<u32, OsError> {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;

    let mut info = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
    // SAFETY: `info` is a live, writable OSVERSIONINFOW with its size field set, and the call
    // only writes into it before returning.
    #[allow(unsafe_code)]
    let status = unsafe { RtlGetVersion(&mut info) };
    if status.is_ok() { Ok(info.dwBuildNumber) } else { Err(OsError::Query { status: status.0 }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_11_starts_at_build_22000() {
        assert!(!is_windows_11_build(19_045));
        assert!(!is_windows_11_build(21_999));
        assert!(is_windows_11_build(22_000));
        assert!(is_windows_11_build(26_200));
    }
}
