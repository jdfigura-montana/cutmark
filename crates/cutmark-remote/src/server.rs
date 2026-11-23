//! The control server: one listener thread on 127.0.0.1 that serves one
//! authenticated client (spec: Remote Control "crates/cutmark-remote (Rust)").

use std::io;
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use serde_json::Value;

use crate::config::RemoteConfig;
use crate::discovery;
use crate::error::RemoteError;
use crate::session::Session;
use crate::token;

/// How often the idle thread looks at the stop flag and takes new connections.
pub(crate) const POLL: Duration = Duration::from_millis(25);
/// The shortest gap between two patches: at most 20 a second.
pub(crate) const PATCH_SPACING: Duration = Duration::from_millis(50);

/// An error the app queued for the client about one command.
pub(crate) struct ErrorReply {
    /// The command id the error is about.
    pub(crate) id: String,
    /// The error code (`blocked`, `bad_args`, ...).
    pub(crate) code: String,
    /// The human-readable message.
    pub(crate) message: String,
}

/// The published state and whether the client has missed any of it.
#[derive(Default)]
struct Latest {
    state: Option<Value>,
    unsent: bool,
}

/// The state the server thread and the app share.
pub(crate) struct Shared {
    /// The app's short name, sent in `welcome`.
    pub(crate) app: &'static str,
    /// The app version, sent in `welcome`.
    pub(crate) version: String,
    /// The commands the app accepts.
    commands: Vec<crate::CommandInfo>,
    /// The per-launch token a client must name in `hello`.
    pub(crate) token: String,
    /// Set when the server should stop.
    stop: AtomicBool,
    /// Whether an authenticated client is connected.
    pub(crate) connected: AtomicBool,
    latest: Mutex<Latest>,
    commands_tx: Sender<RemoteCommand>,
    errors_tx: Sender<ErrorReply>,
    wake: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl Shared {
    /// The latest published state, or an empty object when nothing was published yet.
    pub(crate) fn latest_state(&self) -> Value {
        let latest = self.lock_latest();
        match &latest.state {
            Some(state) => state.clone(),
            None => serde_json::json!({}),
        }
    }

    /// The latest published state, when the client has not seen it yet.
    pub(crate) fn unsent_snapshot(&self) -> Option<Value> {
        let latest = self.lock_latest();
        if latest.unsent { latest.state.clone() } else { None }
    }

    /// Clears the unsent flag when `state` is still the latest one; a state published
    /// while a patch was on the wire keeps the flag up.
    pub(crate) fn mark_sent_if_current(&self, state: &Value) {
        let mut latest = self.lock_latest();
        if latest.state.as_ref() == Some(state) {
            latest.unsent = false;
        }
    }

    /// Locks the published state, carrying on if a thread panicked while holding it.
    fn lock_latest(&self) -> MutexGuard<'_, Latest> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the client may send `id`.
    pub(crate) fn knows_command(&self, id: &str) -> bool {
        self.commands.iter().any(|command| command.id == id)
    }

    /// The app's command list as id-and-label pairs.
    pub(crate) fn command_list(&self) -> Vec<(String, String)> {
        self.commands.iter().map(|command| (command.id.clone(), command.label.clone())).collect()
    }

    /// Hands a command to the UI thread and asks it to wake up.
    pub(crate) fn deliver(&self, command: RemoteCommand) {
        // A full queue only slows the plugin down; the UI side drains what it needs.
        let _ = self.commands_tx.send(command);
        self.call_wake();
    }

    /// Calls the app's wake callback, if one is set.
    fn call_wake(&self) {
        let wake = self.wake.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(wake) = wake.as_ref() {
            wake();
        }
    }
}

/// One command that arrived from the remote client.
#[derive(Debug, Clone)]
pub struct RemoteCommand {
    /// The command id from the app's command list.
    pub id: String,
    /// The command arguments, an empty object when the client sent none.
    pub args: Value,
}

/// One app's control server: a listener thread, the command queue and the discovery files.
pub struct RemoteServer {
    shared: Arc<Shared>,
    commands_rx: Receiver<RemoteCommand>,
    port: u16,
    app: &'static str,
    dir: PathBuf,
    thread: Option<JoinHandle<()>>,
}

impl RemoteServer {
    /// Starts the server: binds 127.0.0.1 on an OS-assigned port, writes the discovery
    /// files and spawns the listener thread. Fails with [`RemoteError::AlreadyRunning`]
    /// when another live instance of the app owns the discovery file.
    pub fn start(config: RemoteConfig) -> Result<RemoteServer, RemoteError> {
        let Some(dir) = discovery::folder(config.dir.as_deref()) else {
            return Err(RemoteError::Discovery(io::Error::new(io::ErrorKind::NotFound, "no Cutmark config folder")));
        };
        std::fs::create_dir_all(&dir).map_err(RemoteError::Discovery)?;
        let app_file = dir.join(format!("{}.json", config.app));
        if let Some(pid) = discovery::existing_pid(&app_file)
            && crate::pid::alive(pid)
        {
            return Err(RemoteError::AlreadyRunning);
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(RemoteError::Bind)?;
        let port = listener.local_addr().map_err(RemoteError::Bind)?.port();
        let token = token::generate();
        discovery::write_app_file(&dir, config.app, std::process::id(), port, &token, &config.version).map_err(RemoteError::Discovery)?;
        discovery::update_apps_file(&dir, config.app, &config.exe, &config.version).map_err(RemoteError::Discovery)?;
        let (commands_tx, commands_rx) = unbounded();
        let (errors_tx, errors_rx) = unbounded();
        let shared = Arc::new(Shared {
            app: config.app,
            version: config.version,
            commands: config.commands,
            token,
            stop: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            latest: Mutex::new(Latest::default()),
            commands_tx,
            errors_tx,
            wake: Mutex::new(None),
        });
        let thread_shared = Arc::clone(&shared);
        let name = format!("cutmark-remote-{}", config.app);
        let thread = std::thread::Builder::new().name(name).spawn(move || run(listener, thread_shared, errors_rx)).map_err(RemoteError::Thread)?;
        Ok(RemoteServer { shared, commands_rx, port, app: config.app, dir, thread: Some(thread) })
    }

    /// The port the server bound; the discovery file carries it too.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Takes the next command, on the UI thread; `None` when none arrived since the last call.
    pub fn try_recv(&self) -> Option<RemoteCommand> {
        self.commands_rx.try_recv().ok()
    }

    /// Replaces the callback the server calls when a command arrives; the app passes `request_repaint`.
    pub fn set_wake(&self, wake: Box<dyn Fn() + Send>) {
        *self.shared.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(wake);
    }

    /// Replaces the app's published state; the server sends the changed fields, at most 20 patches a second.
    pub fn publish(&self, state: Value) {
        let mut latest = self.shared.lock_latest();
        latest.state = Some(state);
        latest.unsent = true;
    }

    /// Queues an error about one command for the client (`blocked`, `bad_args`, ...).
    pub fn reply_error(&self, command_id: &str, code: &str, message: &str) {
        let reply = ErrorReply { id: command_id.to_string(), code: code.to_string(), message: message.to_string() };
        let _ = self.shared.errors_tx.send(reply);
    }

    /// Whether an authenticated client is connected right now.
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed)
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        discovery::remove_app_file(&self.dir, self.app);
    }
}

/// The listener thread: serves one client at a time and rejects the rest with `busy`.
fn run(listener: TcpListener, shared: Arc<Shared>, errors: Receiver<ErrorReply>) {
    let _ = listener.set_nonblocking(true);
    let mut session: Option<Session> = None;
    let mut sent: Option<Value> = None;
    // A patch may go out at once: start as if the last one was a full spacing ago.
    let mut last_patch = Instant::now().checked_sub(PATCH_SPACING).unwrap_or_else(Instant::now);
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if session.is_some() {
                    Session::reject_busy(stream);
                } else if let Some(new) = Session::accept(stream) {
                    session = Some(new);
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => std::thread::sleep(POLL),
        }
        if let Some(active) = session.as_mut() {
            active.service(&shared, &errors, &mut sent, &mut last_patch);
            if active.dead() {
                shared.connected.store(false, Ordering::Relaxed);
                // The next client gets a full state again.
                sent = None;
                session = None;
            }
        } else {
            std::thread::sleep(POLL);
        }
    }
    if let Some(mut active) = session {
        active.close();
    }
}
