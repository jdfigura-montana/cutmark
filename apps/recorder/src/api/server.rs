//! The control API server: a listener thread on 127.0.0.1, one thread per client and the event feed.
//!
//! Commands go to the controller's bus stamped with the QPC time of their arrival. Events come from the controller's
//! state changes, plus a `status` event once a second while a take runs.

use std::io;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use thiserror::Error;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error as WsError, Message};

use super::protocol::{EventKind, Op, Request, StatusData, auth_ok, error_message, event, parse, result_error, result_ok};
use super::token;
use crate::engine::bus::CommandKind;
use crate::engine::controller::{ApiIntent, ApiLink, ControllerState, Phase};
use crate::profiles;

/// The most clients that can be connected at once.
const MAX_CLIENTS: usize = 8;

/// The largest message, and the largest frame, a client may send, in bytes.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// How long a new connection has to finish the handshake and send a good `auth`.
const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a write to a client may block before the client is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the threads look at the stop flag and the event queue.
const POLL: Duration = Duration::from_millis(50);

/// How often the `status` event is sent while a take runs.
const STATUS_INTERVAL: Duration = Duration::from_secs(1);

/// How many events may wait for one slow client before more are dropped for it.
const EVENT_QUEUE: usize = 256;

/// Asks the window to repaint, so it shows a changed client count.
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

/// Why the server could not start.
#[derive(Debug, Error)]
pub enum ApiError {
    /// The port could not be opened, most likely because another program uses it.
    #[error("port {port} could not be opened ({source}); another program may be using it")]
    Bind {
        /// The port that was tried.
        port: u16,
        /// The operating system's reason.
        source: io::Error,
    },
    /// The listening socket could not be set up.
    #[error("the listening socket could not be set up: {0}")]
    Socket(io::Error),
    /// A server thread could not be started.
    #[error("a control API thread could not be started: {0}")]
    Thread(io::Error),
}

/// One connected, authenticated client's queue of events.
struct Subscriber {
    id: u64,
    tx: SyncSender<String>,
}

/// The authenticated clients, who receive every event.
struct Hub {
    clients: Mutex<Vec<Subscriber>>,
    next_id: AtomicU64,
    repaint: Repaint,
}

impl Hub {
    /// An empty hub that calls `repaint` whenever the client count changes.
    fn new(repaint: Repaint) -> Self {
        Self { clients: Mutex::new(Vec::new()), next_id: AtomicU64::new(0), repaint }
    }

    /// Locks the client list, carrying on with the data if a thread panicked while holding it.
    fn clients(&self) -> std::sync::MutexGuard<'_, Vec<Subscriber>> {
        self.clients.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds a client; it gets every event broadcast from now on.
    fn subscribe(self: &Arc<Self>) -> Subscription {
        let (tx, rx) = sync_channel(EVENT_QUEUE);
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.clients().push(Subscriber { id, tx });
        (self.repaint)();
        Subscription { id, rx, hub: Arc::clone(self) }
    }

    /// Removes a client.
    fn unsubscribe(&self, id: u64) {
        self.clients().retain(|client| client.id != id);
        (self.repaint)();
    }

    /// Queues `text` for every client; a client whose queue is full misses it.
    fn broadcast(&self, text: &str) {
        for client in self.clients().iter() {
            if let Err(TrySendError::Full(_)) = client.tx.try_send(text.to_owned()) {
                log::warn!("a control API client is too slow, so an event was dropped for it");
            }
        }
    }

    /// How many clients are authenticated.
    fn len(&self) -> usize {
        self.clients().len()
    }
}

/// A client's place in the hub; leaving the hub when dropped.
struct Subscription {
    id: u64,
    rx: Receiver<String>,
    hub: Arc<Hub>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(self.id);
    }
}

/// Turns controller state changes into events by comparing each state with the one before.
#[derive(Default)]
struct EventTracker {
    phase: Phase,
    marks_count: usize,
    take: Option<String>,
    error: Option<String>,
    audio_notice: Option<String>,
    marks_warning: Option<String>,
}

impl EventTracker {
    /// A tracker that treats `state` as already announced, so a server started mid-take sends no old news.
    fn baseline(state: &ControllerState) -> Self {
        Self {
            phase: state.phase,
            marks_count: state.marks_count,
            take: StatusData::from_state(state).take,
            error: state.error.clone(),
            audio_notice: state.audio_notice.clone(),
            marks_warning: state.marks_warning.clone(),
        }
    }

    /// The events, as messages, that took the Recorder from the last state seen to `state`.
    fn observe(&mut self, state: &ControllerState) -> Vec<String> {
        let mut events = Vec::new();
        let mut status = StatusData::from_state(state);
        if status.take.is_some() {
            self.take.clone_from(&status.take);
        }
        let phase_event = match (self.phase, state.phase) {
            (Phase::Idle, Phase::Recording) => Some(EventKind::RecordingStarted),
            (Phase::Recording, Phase::Paused) => Some(EventKind::Paused),
            (Phase::Paused, Phase::Recording) => Some(EventKind::Resumed),
            (Phase::Recording | Phase::Paused, Phase::Idle) => Some(EventKind::RecordingStopped),
            _ => None,
        };
        self.phase = state.phase;
        if let Some(kind) = phase_event {
            if status.take.is_none() {
                status.take.clone_from(&self.take);
            }
            events.push(event(kind, status.to_value()));
        }
        self.observe_marks(state, &mut events);
        warn_on_change(&mut self.error, &state.error, "recording", &mut events);
        warn_on_change(&mut self.audio_notice, &state.audio_notice, "audio", &mut events);
        warn_on_change(&mut self.marks_warning, &state.marks_warning, "marks", &mut events);
        events
    }

    /// Adds a `markAdded` event for each mark the state has that the last one did not.
    fn observe_marks(&mut self, state: &ControllerState, events: &mut Vec<String>) {
        let count = state.marks_count;
        if count > self.marks_count {
            let listed = state.recent_marks.len();
            let shown = (count - self.marks_count).min(listed);
            let first_index = count - shown + 1;
            for (offset, (timecode, category, label)) in state.recent_marks.iter().skip(listed - shown).enumerate() {
                events.push(event(EventKind::MarkAdded, json!({ "index": first_index + offset, "timecode": timecode, "category": category, "label": label })));
            }
        }
        self.marks_count = count;
    }
}

/// Adds a `warning` event when `current` holds a text that `last` did not, and remembers `current`.
fn warn_on_change(last: &mut Option<String>, current: &Option<String>, source: &str, events: &mut Vec<String>) {
    if current != last {
        if let Some(message) = current {
            events.push(event(EventKind::Warning, json!({ "source": source, "message": message })));
        }
        last.clone_from(current);
    }
}

/// The running control API server. Dropping it stops it.
pub struct ApiServer {
    port: u16,
    stop: Arc<AtomicBool>,
    hub: Arc<Hub>,
    link: ApiLink,
    listener_id: u64,
    threads: Vec<JoinHandle<()>>,
}

impl ApiServer {
    /// Binds 127.0.0.1:`port` and starts the listener, the status ticker and the event feed.
    ///
    /// `token` is what clients must send first. Port 0 picks a free port; [`ApiServer::port`] says which.
    ///
    /// Never call this or [`ApiServer::stop`] while holding the controller's state lock: the threads need it to finish.
    pub fn start(port: u16, token: String, link: ApiLink, repaint: Repaint) -> Result<Self, ApiError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).map_err(|source| ApiError::Bind { port, source })?;
        listener.set_nonblocking(true).map_err(ApiError::Socket)?;
        let port = listener.local_addr().map_err(ApiError::Socket)?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let hub = Arc::new(Hub::new(repaint));
        let tracker = Mutex::new(link.read_state(EventTracker::baseline));
        let feed = Arc::clone(&hub);
        let listener_id = link.add_listener(Box::new(move |state| {
            let events = tracker.lock().unwrap_or_else(PoisonError::into_inner).observe(state);
            for message in &events {
                feed.broadcast(message);
            }
        }));
        let mut server = Self { port, stop: Arc::clone(&stop), hub: Arc::clone(&hub), link: link.clone(), listener_id, threads: Vec::new() };
        let ctx = ClientCtx { token: Arc::from(token), link: link.clone(), hub: Arc::clone(&hub), stop: Arc::clone(&stop) };
        server.spawn("api-listener", move || accept_loop(&listener, &ctx))?;
        server.spawn("api-status", move || status_loop(&link, &hub, &stop))?;
        log::info!("the control API listens on 127.0.0.1:{port}");
        Ok(server)
    }

    /// The port the server listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many authenticated clients are connected.
    pub fn client_count(&self) -> usize {
        self.hub.len()
    }

    /// Stops the threads, closes every connection and waits for them. Safe to call more than once.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.link.remove_listener(self.listener_id);
        for handle in self.threads.drain(..) {
            if handle.join().is_err() {
                log::error!("a control API thread panicked");
            }
        }
    }

    /// Starts a named thread that the server owns.
    fn spawn(&mut self, name: &str, work: impl FnOnce() + Send + 'static) -> Result<(), ApiError> {
        let handle = thread::Builder::new().name(name.to_owned()).spawn(work).map_err(ApiError::Thread)?;
        self.threads.push(handle);
        Ok(())
    }
}

impl Drop for ApiServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// What every client thread shares.
struct ClientCtx {
    token: Arc<str>,
    link: ApiLink,
    hub: Arc<Hub>,
    stop: Arc<AtomicBool>,
}

/// Accepts connections until told to stop, starting a thread for each, then waits for those threads.
fn accept_loop(listener: &TcpListener, ctx: &ClientCtx) {
    let mut clients: Vec<JoinHandle<()>> = Vec::new();
    while !ctx.stop.load(Ordering::Relaxed) {
        clients.retain(|client| !client.is_finished());
        match listener.accept() {
            Ok((stream, peer)) => {
                if clients.len() >= MAX_CLIENTS {
                    log::warn!("a control API connection from {peer} was refused: {MAX_CLIENTS} clients are already connected");
                    continue;
                }
                let client_ctx = ClientCtx { token: Arc::clone(&ctx.token), link: ctx.link.clone(), hub: Arc::clone(&ctx.hub), stop: Arc::clone(&ctx.stop) };
                match thread::Builder::new().name("api-client".to_owned()).spawn(move || serve_client(stream, &client_ctx)) {
                    Ok(handle) => clients.push(handle),
                    Err(err) => log::warn!("a control API client thread could not be started: {err}"),
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(err) => {
                log::warn!("accepting a control API connection failed: {err}");
                thread::sleep(POLL);
            }
        }
    }
    for client in clients {
        if client.join().is_err() {
            log::error!("a control API client thread panicked");
        }
    }
}

/// Sends the `status` event once a second while a take runs, until told to stop.
fn status_loop(link: &ApiLink, hub: &Hub, stop: &AtomicBool) {
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(POLL);
        if last.elapsed() < STATUS_INTERVAL {
            continue;
        }
        last = Instant::now();
        let status = link.read_state(|state| (state.phase != Phase::Idle).then(|| StatusData::from_state(state)));
        if let Some(status) = status {
            hub.broadcast(&event(EventKind::Status, status.to_value()));
        }
    }
}

/// The WebSocket limits: 64 KB per message and per frame.
fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES)).max_frame_size(Some(MAX_MESSAGE_BYTES))
}

/// Whether a read or write gave up waiting rather than failing.
fn is_timeout(err: &WsError) -> bool {
    matches!(err, WsError::Io(io_err) if matches!(io_err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
}

/// Runs one client from handshake to close.
fn serve_client(stream: TcpStream, ctx: &ClientCtx) {
    let connected = Instant::now();
    // The accepted socket may inherit the listener's non-blocking mode; the reads below use timeouts instead.
    if stream.set_nonblocking(false).is_err() || stream.set_read_timeout(Some(AUTH_TIMEOUT)).is_err() || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return;
    }
    // Small replies should not wait for more data; if this fails they are merely a little slower.
    let _ = stream.set_nodelay(true);
    let mut socket = match tungstenite::accept_with_config(stream, Some(socket_config())) {
        Ok(socket) => socket,
        Err(err) => {
            log::info!("a control API handshake failed: {err}");
            return;
        }
    };
    if socket.get_ref().set_read_timeout(Some(POLL)).is_err() {
        return;
    }
    let mut subscription: Option<Subscription> = None;
    'session: loop {
        if ctx.stop.load(Ordering::Relaxed) {
            break;
        }
        if subscription.is_none() && connected.elapsed() >= AUTH_TIMEOUT {
            // The client is being dropped anyway, so a failed send changes nothing.
            let _ = socket.send(Message::text(error_message("auth_timeout")));
            break;
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let press_qpc = capture::qpc_now();
                let (replies, close) = respond(text.as_str(), press_qpc, ctx, &mut subscription);
                for reply in replies {
                    if socket.send(Message::text(reply)).is_err() {
                        break 'session;
                    }
                }
                if close {
                    break;
                }
            }
            Ok(Message::Binary(_)) => {
                let (reply, close) = match subscription {
                    Some(_) => (result_error(None, "binary messages are not supported"), false),
                    None => (error_message("unauthorized"), true),
                };
                if socket.send(Message::text(reply)).is_err() || close {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(err) if is_timeout(&err) => {}
            Err(WsError::Capacity(_)) => {
                // The client is being dropped anyway, so a failed send changes nothing.
                let _ = socket.send(Message::text(error_message("message_too_large")));
                break;
            }
            Err(_) => break,
        }
        if let Some(subscription) = &subscription {
            while let Ok(text) = subscription.rx.try_recv() {
                if socket.send(Message::text(text)).is_err() {
                    break 'session;
                }
            }
        }
        match socket.flush() {
            Ok(()) => {}
            Err(err) if is_timeout(&err) => {}
            Err(_) => break,
        }
    }
    // Closing is courtesy: the client may be gone already.
    let _ = socket.close(None);
    let _ = socket.flush();
}

/// Answers one text message: the messages to send back, and whether to close the connection after them.
fn respond(text: &str, press_qpc: i64, ctx: &ClientCtx, subscription: &mut Option<Subscription>) -> (Vec<String>, bool) {
    let request = parse(text);
    if subscription.is_none() {
        return match request {
            Ok(Request { op: Op::Auth { token: given }, .. }) if token::matches(&ctx.token, &given) => {
                *subscription = Some(ctx.hub.subscribe());
                (vec![auth_ok()], false)
            }
            _ => (vec![error_message("unauthorized")], true),
        };
    }
    let reply = match request {
        Ok(request) => match execute(&ctx.link, request.op, press_qpc) {
            Ok(data) => result_ok(request.id.as_ref(), data),
            Err(message) => result_error(request.id.as_ref(), &message),
        },
        Err(rejected) => result_error(rejected.id.as_ref(), &rejected.error.to_string()),
    };
    (vec![reply], false)
}

/// Carries out one command from an authenticated client; the error text goes back in the reply.
fn execute(link: &ApiLink, op: Op, press_qpc: i64) -> Result<Value, String> {
    let phase = link.read_state(|state| state.phase);
    match op {
        Op::Auth { .. } => Err("already authenticated".to_owned()),
        Op::GetStatus => Ok(link.read_state(|state| StatusData::from_state(state).to_value())),
        Op::Start => {
            if phase != Phase::Idle {
                return Err("a take is already running".to_owned());
            }
            link.send(CommandKind::StartStop, Some(ApiIntent::Start), press_qpc)?;
            Ok(json!({}))
        }
        Op::Stop => {
            if phase == Phase::Idle {
                return Err("no take is running".to_owned());
            }
            link.send(CommandKind::StartStop, Some(ApiIntent::Stop), press_qpc)?;
            Ok(json!({}))
        }
        Op::Pause => {
            match phase {
                Phase::Idle => return Err("no take is running".to_owned()),
                Phase::Paused => return Err("the take is already paused".to_owned()),
                Phase::Recording => {}
            }
            link.send(CommandKind::Pause, None, press_qpc)?;
            Ok(json!({}))
        }
        Op::Resume => {
            match phase {
                Phase::Idle => return Err("no take is running".to_owned()),
                Phase::Recording => return Err("the take is not paused".to_owned()),
                Phase::Paused => {}
            }
            link.send(CommandKind::Resume, None, press_qpc)?;
            Ok(json!({}))
        }
        Op::Mark { category, label } => {
            if phase == Phase::Idle {
                return Err("no take is running".to_owned());
            }
            link.send(CommandKind::LabeledMark { category, label }, Some(ApiIntent::Mark), press_qpc)?;
            Ok(json!({}))
        }
        Op::Mute { track } => execute_mute(link, track, press_qpc),
        Op::SetProfile { name } => {
            if phase != Phase::Idle {
                return Err("the profile cannot change while a take is running".to_owned());
            }
            let Some(profile) = profiles::find(&name) else {
                return Err(format!("there is no profile named {name}"));
            };
            link.send(CommandKind::SetProfile(name), None, press_qpc)?;
            Ok(json!({ "profile": profile.name }))
        }
    }
}

/// Toggles mute on the named track, or on the mic track when none is named.
fn execute_mute(link: &ApiLink, track: Option<String>, press_qpc: i64) -> Result<Value, String> {
    let names = link.read_state(|state| state.track_names.clone());
    let Some(track) = track else {
        if names.is_empty() {
            return Err("there are no audio tracks".to_owned());
        }
        let index = names.iter().position(|name| name.eq_ignore_ascii_case("Mic")).unwrap_or(0);
        link.send(CommandKind::Mute { track: index }, Some(ApiIntent::Mute(None)), press_qpc)?;
        return Ok(json!({}));
    };
    let Some((index, name)) = names.iter().enumerate().find(|(_, name)| name.eq_ignore_ascii_case(&track)) else {
        return Err(format!("there is no track named {track}"));
    };
    link.send(CommandKind::Mute { track: index }, Some(ApiIntent::Mute(Some(name.clone()))), press_qpc)?;
    Ok(json!({ "track": name }))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::error::Error;

    use tungstenite::stream::MaybeTlsStream;
    use tungstenite::{Message, WebSocket};

    use super::*;

    type Client = WebSocket<MaybeTlsStream<TcpStream>>;

    const WAIT: Duration = Duration::from_secs(5);

    fn no_repaint() -> Repaint {
        Arc::new(|| {})
    }

    fn connect(port: u16) -> Result<Client, Box<dyn Error>> {
        let (mut client, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/"))?;
        if let MaybeTlsStream::Plain(stream) = client.get_mut() {
            stream.set_read_timeout(Some(WAIT))?;
        }
        Ok(client)
    }

    fn say(client: &mut Client, message: &Value) -> Result<(), Box<dyn Error>> {
        client.send(Message::text(message.to_string()))?;
        Ok(())
    }

    fn hear(client: &mut Client) -> Result<Value, Box<dyn Error>> {
        match client.read()? {
            Message::Text(text) => Ok(serde_json::from_str(text.as_str())?),
            other => Err(format!("expected a text message, got {other:?}").into()),
        }
    }

    fn signed_in(port: u16, token: &str) -> Result<Client, Box<dyn Error>> {
        let mut client = connect(port)?;
        say(&mut client, &json!({ "op": "auth", "token": token }))?;
        assert_eq!(hear(&mut client)?, json!({ "op": "auth", "ok": true }));
        Ok(client)
    }

    #[test]
    fn a_wrong_token_gets_unauthorized_and_is_closed() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let server = ApiServer::start(0, "right".to_owned(), link, no_repaint())?;
        let mut client = connect(server.port())?;
        say(&mut client, &json!({ "op": "auth", "token": "wrong" }))?;
        assert_eq!(hear(&mut client)?, json!({ "op": "error", "code": "unauthorized" }));
        assert!(matches!(client.read(), Ok(Message::Close(_)) | Err(_)));
        assert_eq!(server.client_count(), 0);
        Ok(())
    }

    #[test]
    fn a_first_message_that_is_not_auth_is_unauthorized() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let server = ApiServer::start(0, "right".to_owned(), link, no_repaint())?;
        let mut client = connect(server.port())?;
        say(&mut client, &json!({ "op": "start" }))?;
        assert_eq!(hear(&mut client)?, json!({ "op": "error", "code": "unauthorized" }));
        Ok(())
    }

    #[test]
    fn commands_reach_the_bus_with_their_arrival_time_and_replies_echo_the_id() -> Result<(), Box<dyn Error>> {
        let (link, bus) = ApiLink::detached(ControllerState { active_profile: "Tutorial AV1 1440p60".to_owned(), ..ControllerState::default() });
        let server = ApiServer::start(0, "right".to_owned(), link, no_repaint())?;
        let mut client = signed_in(server.port(), "right")?;
        assert_eq!(server.client_count(), 1);

        say(&mut client, &json!({ "op": "getStatus", "id": 1 }))?;
        let status = hear(&mut client)?;
        assert_eq!(status["id"], json!(1));
        assert_eq!(status["ok"], json!(true));
        assert_eq!(status["data"]["state"], json!("idle"));
        assert_eq!(status["data"]["profile"], json!("Tutorial AV1 1440p60"));

        let before = capture::qpc_now();
        say(&mut client, &json!({ "op": "start", "id": "a" }))?;
        let started = hear(&mut client)?;
        assert_eq!((started["ok"].clone(), started["id"].clone()), (json!(true), json!("a")));
        let command = bus.recv_timeout(WAIT).ok_or("the start command did not reach the bus")?;
        assert_eq!(command.kind, CommandKind::StartStop);
        assert!(command.press_qpc >= before && command.press_qpc <= capture::qpc_now());

        // Nothing runs, so these are refused and nothing is queued for them.
        for op in ["stop", "pause", "resume", "mark"] {
            say(&mut client, &json!({ "op": op, "id": op }))?;
            let reply = hear(&mut client)?;
            assert_eq!((reply["ok"].clone(), reply["id"].clone()), (json!(false), json!(op)), "{reply}");
        }
        assert!(bus.recv_timeout(Duration::from_millis(100)).is_none());
        Ok(())
    }

    #[test]
    fn bad_messages_get_an_error_result_and_the_connection_stays_open() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let server = ApiServer::start(0, "right".to_owned(), link, no_repaint())?;
        let mut client = signed_in(server.port(), "right")?;
        client.send(Message::text("{not json"))?;
        assert_eq!(hear(&mut client)?["ok"], json!(false));
        say(&mut client, &json!({ "op": "explode", "id": 5 }))?;
        let unknown = hear(&mut client)?;
        assert_eq!((unknown["ok"].clone(), unknown["id"].clone()), (json!(false), json!(5)));
        say(&mut client, &json!({ "op": "mark", "type": "Banana" }))?;
        assert_eq!(hear(&mut client)?["ok"], json!(false));
        say(&mut client, &json!({ "op": "setProfile", "name": "No such profile", "id": 6 }))?;
        assert_eq!(hear(&mut client)?["ok"], json!(false));
        say(&mut client, &json!({ "op": "getStatus" }))?;
        assert_eq!(hear(&mut client)?["ok"], json!(true));
        Ok(())
    }

    #[test]
    fn a_port_in_use_is_an_error_not_a_crash() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let first = ApiServer::start(0, "right".to_owned(), link.clone(), no_repaint())?;
        let second = ApiServer::start(first.port(), "right".to_owned(), link, no_repaint());
        assert!(matches!(second, Err(ApiError::Bind { .. })));
        Ok(())
    }

    #[test]
    fn stopping_closes_the_connections_and_frees_the_port() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let mut server = ApiServer::start(0, "right".to_owned(), link.clone(), no_repaint())?;
        let port = server.port();
        let mut client = signed_in(port, "right")?;
        server.stop();
        assert!(matches!(client.read(), Ok(Message::Close(_)) | Err(_)));
        let again = ApiServer::start(port, "right".to_owned(), link, no_repaint())?;
        assert_eq!(again.port(), port);
        Ok(())
    }

    #[test]
    fn a_ninth_client_is_not_served() -> Result<(), Box<dyn Error>> {
        let (link, _bus) = ApiLink::detached(ControllerState::default());
        let server = ApiServer::start(0, "right".to_owned(), link, no_repaint())?;
        let mut clients = Vec::new();
        for _ in 0..MAX_CLIENTS {
            clients.push(signed_in(server.port(), "right")?);
        }
        assert_eq!(server.client_count(), MAX_CLIENTS);
        let refused = tungstenite::connect(format!("ws://127.0.0.1:{}/", server.port()));
        assert!(refused.is_err());
        Ok(())
    }

    fn recording_state() -> ControllerState {
        ControllerState { phase: Phase::Recording, active_profile: "P".to_owned(), fps: 60, ..ControllerState::default() }
    }

    fn names(events: &[String]) -> Vec<String> {
        events
            .iter()
            .map(|text| serde_json::from_str::<Value>(text).ok().and_then(|value| value["event"].as_str().map(str::to_owned)).unwrap_or_default())
            .collect()
    }

    #[test]
    fn phase_changes_become_events_in_order() {
        let mut tracker = EventTracker::default();
        assert!(tracker.observe(&ControllerState::default()).is_empty());
        let mut state = recording_state();
        assert_eq!(names(&tracker.observe(&state)), ["recordingStarted"]);
        assert!(tracker.observe(&state).is_empty());
        state.phase = Phase::Paused;
        assert_eq!(names(&tracker.observe(&state)), ["paused"]);
        state.phase = Phase::Recording;
        assert_eq!(names(&tracker.observe(&state)), ["resumed"]);
        state.phase = Phase::Idle;
        assert_eq!(names(&tracker.observe(&state)), ["recordingStopped"]);
    }

    #[test]
    fn new_marks_become_mark_added_events_with_their_index() {
        let mut tracker = EventTracker::baseline(&recording_state());
        let mut state = recording_state();
        state.marks_count = 2;
        state.recent_marks =
            VecDeque::from([("00:00:01:00".to_owned(), "Note".to_owned(), None), ("00:00:02:00".to_owned(), "Retake".to_owned(), Some("redo".to_owned()))]);
        let events = tracker.observe(&state);
        assert_eq!(names(&events), ["markAdded", "markAdded"]);
        let second: Value = serde_json::from_str(&events[1]).unwrap_or(Value::Null);
        assert_eq!(second["data"], json!({ "index": 2, "timecode": "00:00:02:00", "category": "Retake", "label": "redo" }));
        assert!(tracker.observe(&state).is_empty());
        // A new take starts counting from 0 again.
        state.marks_count = 0;
        state.recent_marks.clear();
        assert!(tracker.observe(&state).is_empty());
        state.marks_count = 1;
        state.recent_marks = VecDeque::from([("00:00:03:00".to_owned(), "Cut".to_owned(), None)]);
        let events = tracker.observe(&state);
        let first: Value = serde_json::from_str(&events[0]).unwrap_or(Value::Null);
        assert_eq!(first["data"]["index"], json!(1));
    }

    #[test]
    fn a_new_warning_text_is_announced_once() {
        let mut tracker = EventTracker::default();
        let mut state = ControllerState { audio_notice: Some("Audio device not found".to_owned()), ..ControllerState::default() };
        let events = tracker.observe(&state);
        assert_eq!(names(&events), ["warning"]);
        let warning: Value = serde_json::from_str(&events[0]).unwrap_or(Value::Null);
        assert_eq!(warning["data"], json!({ "source": "audio", "message": "Audio device not found" }));
        assert!(tracker.observe(&state).is_empty());
        state.audio_notice = None;
        assert!(tracker.observe(&state).is_empty());
        state.marks_warning = Some("Marks problem".to_owned());
        assert_eq!(names(&tracker.observe(&state)), ["warning"]);
    }

    #[test]
    fn a_server_started_mid_take_does_not_replay_it() {
        let mut mid_take = recording_state();
        mid_take.marks_count = 3;
        mid_take.error = Some("old".to_owned());
        let mut tracker = EventTracker::baseline(&mid_take);
        assert!(tracker.observe(&mid_take).is_empty());
    }
}
