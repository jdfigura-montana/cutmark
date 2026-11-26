//! Single instance: the first copy of an app holds a named mutex and listens on a named pipe; a
//! later copy asks it to come forward and exits. Windows backend only; elsewhere every copy is the
//! first one and hears nothing.

use std::sync::mpsc::{self, Receiver};

use crate::error::{PlatformError, Result};

/// The message [`SingleInstance::signal_focus`] sends.
pub const FOCUS_MESSAGE: &str = "focus";

/// What [`SingleInstance::acquire`] found.
#[derive(Debug)]
pub enum Acquired {
    /// No other copy is running. Keep the value alive for as long as the app runs.
    Primary(SingleInstance),
    /// Another copy is running. Call [`SingleInstance::signal_focus`], then exit.
    Secondary,
}

/// The running app's claim to be the only copy. Dropping it gives the claim up.
#[derive(Debug)]
pub struct SingleInstance {
    messages: Receiver<String>,
    setup_error: Option<PlatformError>,
    #[cfg(windows)]
    _claim: win::Claim,
}

impl SingleInstance {
    /// Claims the single instance of `app_id` in this logon session. `app_id` is a short fixed
    /// name without backslashes. If Windows cannot say whether another copy is running, the
    /// answer is `Primary`, so a failure here never keeps the app from starting.
    #[cfg(windows)]
    pub fn acquire(app_id: &str) -> Acquired {
        win::acquire(app_id)
    }

    /// There is no single-instance rule outside Windows.
    #[cfg(not(windows))]
    pub fn acquire(_app_id: &str) -> Acquired {
        let (_, messages) = mpsc::channel();
        Acquired::Primary(SingleInstance { messages, setup_error: None })
    }

    /// Lets the running copy take the foreground and sends it [`FOCUS_MESSAGE`]. Waits a couple
    /// of seconds at most for its pipe, which may be starting up or serving another caller.
    #[cfg(windows)]
    pub fn signal_focus(app_id: &str) -> Result<()> {
        win::signal_focus(app_id)
    }

    /// There is no running copy to signal outside Windows.
    #[cfg(not(windows))]
    pub fn signal_focus(_app_id: &str) -> Result<()> {
        Err(PlatformError::Unsupported)
    }

    /// The next message another copy sent, if one is waiting. Never blocks.
    pub fn try_recv(&self) -> Option<String> {
        self.messages.try_recv().ok()
    }

    /// Why this copy cannot hear other copies, if the claim or the pipe server failed to start.
    pub fn setup_error(&self) -> Option<PlatformError> {
        self.setup_error
    }
}

#[cfg(windows)]
mod win {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED};
    use windows::Win32::Foundation::{GetLastError, HANDLE, WIN32_ERROR};
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, SECURITY_IDENTIFICATION};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::System::WindowsProgramming::GetUserNameW;
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::{HSTRING, PWSTR};

    use super::{Acquired, FOCUS_MESSAGE, SingleInstance, mpsc, mutex_name, pipe_name};
    use crate::error::{PlatformError, Result};

    /// The longest message the server reads; the rest is dropped.
    const MAX_MESSAGE: u32 = 1024;
    /// How long a sender waits for a pipe that is not there yet or is serving someone else.
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
    const CONNECT_RETRY: Duration = Duration::from_millis(20);

    /// The mutex and the server thread. Dropping it stops the server and closes the mutex.
    #[derive(Debug)]
    pub(super) struct Claim {
        server: Option<Server>,
        _mutex: Option<OwnedHandle>,
    }

    #[derive(Debug)]
    struct Server {
        name: String,
        stop: Arc<AtomicBool>,
    }

    impl Drop for Claim {
        fn drop(&mut self) {
            if let Some(server) = &self.server {
                server.stop.store(true, Ordering::SeqCst);
                // The thread waits for a client, so connect once to wake it. It is not joined:
                // a client that never hangs up must not stall the app's exit.
                let _ = open_client(&server.name);
            }
        }
    }

    pub(super) fn acquire(app_id: &str) -> Acquired {
        let mutex = match create_mutex(&mutex_name(app_id)) {
            Ok(Some(mutex)) => mutex,
            Ok(None) => return Acquired::Secondary,
            Err(error) => return primary(None, None, Some(error), mpsc::channel().1),
        };
        let (sender, messages) = mpsc::channel();
        match start_server(pipe_name(app_id, &user_name()), sender) {
            Ok(server) => primary(Some(mutex), Some(server), None, messages),
            Err(error) => primary(Some(mutex), None, Some(error), messages),
        }
    }

    fn primary(mutex: Option<OwnedHandle>, server: Option<Server>, setup_error: Option<PlatformError>, messages: mpsc::Receiver<String>) -> Acquired {
        Acquired::Primary(SingleInstance { messages, setup_error, _claim: Claim { server, _mutex: mutex } })
    }

    /// The new mutex, or `None` if another copy already has it.
    fn create_mutex(name: &str) -> Result<Option<OwnedHandle>> {
        // SAFETY: the name outlives the call, and no security attributes are passed.
        #[allow(unsafe_code)]
        let (created, last_error) = unsafe {
            let created = CreateMutexW(None, false, &HSTRING::from(name));
            (created, GetLastError())
        };
        match created {
            Ok(handle) => {
                let handle = owned(handle);
                Ok((last_error != ERROR_ALREADY_EXISTS).then_some(handle))
            }
            // The mutex exists, but its owner (a copy run as administrator) will not share it.
            Err(error) if WIN32_ERROR::from_error(&error) == Some(ERROR_ACCESS_DENIED) => Ok(None),
            Err(error) => Err(win32_error("CreateMutexW", &error)),
        }
    }

    /// Takes ownership of a handle a create call just returned.
    fn owned(handle: HANDLE) -> OwnedHandle {
        // SAFETY: every caller passes a valid handle it has just created and shares with no one.
        #[allow(unsafe_code)]
        unsafe {
            OwnedHandle::from_raw_handle(handle.0)
        }
    }

    /// Creates the pipe before returning, so it exists once `acquire` answers, then serves it on a
    /// background thread.
    fn start_server(name: String, sender: Sender<String>) -> Result<Server> {
        let pipe = create_pipe(&name)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("single-instance".to_owned())
            .spawn(move || serve(&pipe, &sender, &thread_stop))
            .map_err(|e| io_error("thread spawn", &e))?;
        Ok(Server { name, stop })
    }

    /// One inbound instance for this machine only. It fails if the name is already served.
    fn create_pipe(name: &str) -> Result<File> {
        // SAFETY: the name outlives the call. With no security attributes the pipe gets the
        // default ones, which let only this user, administrators and the system write to it.
        #[allow(unsafe_code)]
        let (handle, last_error) = unsafe {
            let handle = CreateNamedPipeW(
                &HSTRING::from(name),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                MAX_MESSAGE,
                0,
                None,
            );
            (handle, GetLastError())
        };
        if handle.is_invalid() {
            return Err(PlatformError::Win32 { call: "CreateNamedPipeW", code: last_error.0 });
        }
        Ok(File::from(owned(handle)))
    }

    /// One message per connection: everything the client writes before it hangs up.
    fn serve(pipe: &File, sender: &Sender<String>, stop: &AtomicBool) {
        // A pipe that cannot take a connection ends the server instead of spinning.
        while wait_for_client(pipe) {
            let mut bytes = Vec::new();
            let read = pipe.take(u64::from(MAX_MESSAGE)).read_to_end(&mut bytes);
            disconnect(pipe);
            if stop.load(Ordering::SeqCst) {
                break;
            }
            if read.is_ok()
                && let Ok(message) = String::from_utf8(bytes)
                && !message.is_empty()
                && sender.send(message).is_err()
            {
                break;
            }
        }
    }

    /// Blocks until a client has connected. False if the pipe is unusable.
    fn wait_for_client(pipe: &File) -> bool {
        // SAFETY: the handle is open while `pipe` lives, and it was not opened for overlapped use.
        #[allow(unsafe_code)]
        let connected = unsafe { ConnectNamedPipe(HANDLE(pipe.as_raw_handle()), None) };
        match connected {
            Ok(()) => true,
            // A client got in before this call. With ERROR_NO_DATA it has already hung up, and
            // what it wrote can still be read.
            Err(error) => matches!(WIN32_ERROR::from_error(&error), Some(ERROR_PIPE_CONNECTED | ERROR_NO_DATA)),
        }
    }

    fn disconnect(pipe: &File) {
        // A failure shows up as the next wait failing, so there is nothing to do with it here.
        // SAFETY: the handle is open while `pipe` lives.
        #[allow(unsafe_code)]
        let _ = unsafe { DisconnectNamedPipe(HANDLE(pipe.as_raw_handle())) };
    }

    pub(super) fn signal_focus(app_id: &str) -> Result<()> {
        // Only the process the user just started may hand over the foreground. It can be refused,
        // and then the running copy still gets the message, so the result does not matter.
        // SAFETY: the call takes only a value.
        #[allow(unsafe_code)]
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        send(&pipe_name(app_id, &user_name()), FOCUS_MESSAGE)
    }

    fn send(name: &str, message: &str) -> Result<()> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match open_client(name) {
                Ok(mut pipe) => return pipe.write_all(message.as_bytes()).map_err(|e| io_error("WriteFile", &e)),
                Err(error) if is_worth_retrying(&error) && Instant::now() < deadline => std::thread::sleep(CONNECT_RETRY),
                Err(error) => return Err(io_error("CreateFileW", &error)),
            }
        }
    }

    /// Opens the writing end. Whoever serves the name may identify this process but not act as it.
    fn open_client(name: &str) -> io::Result<File> {
        OpenOptions::new().write(true).security_qos_flags(SECURITY_IDENTIFICATION.0).open(name)
    }

    fn is_worth_retrying(error: &io::Error) -> bool {
        error.raw_os_error().is_some_and(|code| code == ERROR_FILE_NOT_FOUND.0 as i32 || code == ERROR_PIPE_BUSY.0 as i32)
    }

    /// The signed-in user's name, or an empty string if Windows will not say. Both copies get the
    /// same answer either way.
    fn user_name() -> String {
        // UNLEN (256) and the terminating NUL.
        let mut buffer = [0u16; 257];
        let mut length = buffer.len() as u32;
        // SAFETY: `buffer` is writable for `length` units, and both outlive the call.
        #[allow(unsafe_code)]
        let result = unsafe { GetUserNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut length) };
        if result.is_err() {
            return String::new();
        }
        let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    /// The Win32 code inside a `windows` error; any other HRESULT is passed through whole.
    fn win32_error(call: &'static str, error: &windows::core::Error) -> PlatformError {
        let code = WIN32_ERROR::from_error(error).map_or(error.code().0 as u32, |e| e.0);
        PlatformError::Win32 { call, code }
    }

    fn io_error(call: &'static str, error: &io::Error) -> PlatformError {
        PlatformError::Win32 { call, code: error.raw_os_error().unwrap_or(0) as u32 }
    }
}

/// `Local\` keeps the mutex to one logon session.
#[cfg(any(windows, test))]
fn mutex_name(app_id: &str) -> String {
    format!(r"Local\Cutmark.{app_id}")
}

/// Pipe names are shared by the whole machine, so the user's name keeps users apart.
#[cfg(any(windows, test))]
fn pipe_name(app_id: &str, user: &str) -> String {
    format!(r"\\.\pipe\cutmark-{app_id}-{user}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_app_and_user() {
        assert_eq!(mutex_name("prompter"), r"Local\Cutmark.prompter");
        assert_eq!(pipe_name("prompter", "Ana María"), r"\\.\pipe\cutmark-prompter-Ana María");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "uses a real mutex and named pipe; run by hand on a Windows desktop"]
    fn a_second_acquire_is_secondary_and_its_message_arrives() -> std::result::Result<(), Box<dyn std::error::Error>> {
        use std::time::{Duration, Instant};

        // Unique to this run, so a running app or another test run is left alone.
        let app_id = format!("platform-test-{}", std::process::id());
        let Acquired::Primary(first) = SingleInstance::acquire(&app_id) else {
            return Err("the first acquire was not the primary".into());
        };
        assert_eq!(first.setup_error(), None);
        assert_eq!(first.try_recv(), None);
        assert!(matches!(SingleInstance::acquire(&app_id), Acquired::Secondary));

        // More than once, to show the server takes the next caller.
        for _ in 0..3 {
            SingleInstance::signal_focus(&app_id)?;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        while received.len() < 3 && Instant::now() < deadline {
            match first.try_recv() {
                Some(message) => received.push(message),
                None => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        assert_eq!(received, [FOCUS_MESSAGE; 3]);

        // Dropping the primary frees the claim.
        drop(first);
        assert!(matches!(SingleInstance::acquire(&app_id), Acquired::Primary(_)));
        Ok(())
    }
}
