//! One client connection: handshake, authentication and the per-iteration service.

use std::io;
use std::net::TcpStream;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use serde_json::Value;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error as WsError, Message, WebSocket};

use crate::protocol::{self, ClientMsg, ServerMsg};
use crate::server::{ErrorReply, PATCH_SPACING, POLL, RemoteCommand, Shared};

/// How long a new connection may take to finish its handshake before it is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long any write to the client may block before the connection is dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// The largest message a client may send, in bytes.
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
/// The most messages one service pass reads, so one client cannot starve the thread.
const MAX_READS_PER_SERVICE: usize = 64;

/// A connection from its WebSocket handshake to its close.
pub(crate) struct Session {
    socket: WebSocket<TcpStream>,
    authed: bool,
    dead: bool,
}

impl Session {
    /// Handshakes a new connection; `None` when the handshake fails or the connection
    /// carries an Origin header.
    pub(crate) fn accept(stream: TcpStream) -> Option<Session> {
        let stream = stream;
        if stream.set_nonblocking(false).is_err()
            || stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).is_err()
            || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
        {
            return None;
        }
        let _ = stream.set_nodelay(true);
        let socket = tungstenite::accept_hdr_with_config(stream, origin_guard, Some(socket_config())).ok()?;
        if socket.get_ref().set_read_timeout(Some(POLL)).is_err() {
            return None;
        }
        Some(Session { socket, authed: false, dead: false })
    }

    /// Handshakes a connection that arrived while a client is served, tells it `busy` and closes it.
    pub(crate) fn reject_busy(stream: TcpStream) {
        let Some(mut session) = Session::accept(stream) else { return };
        let busy = ServerMsg::Error { code: "busy".to_string(), message: "another client is connected".to_string(), id: None };
        let _ = session.socket.send(Message::text(protocol::to_text(&busy)));
        let _ = session.socket.close(None);
    }

    /// Whether the connection is over and the client slot may be freed.
    pub(crate) fn dead(&self) -> bool {
        self.dead
    }

    /// Reads what the client sent, answers it, and sends due error replies and patches.
    pub(crate) fn service(&mut self, shared: &Shared, errors: &Receiver<ErrorReply>, sent: &mut Option<Value>, last_patch: &mut Instant) {
        self.read_all(shared, sent);
        if self.dead || !self.authed {
            self.flush();
            return;
        }
        while let Ok(reply) = errors.try_recv() {
            self.send(ServerMsg::Error { code: reply.code, message: reply.message, id: Some(reply.id) });
            if self.dead {
                break;
            }
        }
        if !self.dead {
            self.send_due_patch(shared, sent, last_patch);
        }
        self.flush();
    }

    /// Closes the connection politely; used when the server shuts down.
    pub(crate) fn close(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }

    /// Pushes queued writes onto the wire; a failing socket ends the session.
    fn flush(&mut self) {
        if self.socket.flush().is_err() {
            self.dead = true;
        }
    }

    /// Sends one message; a failing socket ends the session.
    fn send(&mut self, message: ServerMsg) {
        if self.socket.send(Message::text(protocol::to_text(&message))).is_err() {
            self.dead = true;
        }
    }

    /// Sends one patch, at most every [`PATCH_SPACING`], always ending on the latest state.
    fn send_due_patch(&mut self, shared: &Shared, sent: &mut Option<Value>, last_patch: &mut Instant) {
        if last_patch.elapsed() < PATCH_SPACING {
            return;
        }
        let Some(state) = shared.unsent_snapshot() else { return };
        let message = match sent {
            // The client has no snapshot yet: the first state always goes out whole.
            None => ServerMsg::State(state.clone()),
            Some(previous) => match protocol::diff(previous, &state) {
                Some(patch) => ServerMsg::Patch(patch),
                // The client already holds this state; only the unsent flag was stale.
                None => {
                    shared.mark_sent_if_current(&state);
                    return;
                }
            },
        };
        self.send(message);
        if self.dead {
            return;
        }
        *sent = Some(state.clone());
        shared.mark_sent_if_current(&state);
        *last_patch = Instant::now();
    }

    /// Acts on one client message.
    fn handle_text(&mut self, text: &str, shared: &Shared, sent: &mut Option<Value>) {
        let Some(message) = protocol::parse(text) else {
            // Before hello the client is a stranger: anything malformed closes it.
            if !self.authed {
                self.dead = true;
            }
            return;
        };
        if !self.authed {
            self.authenticate(message, shared, sent);
            return;
        }
        match message {
            ClientMsg::Command { id, args } => {
                if shared.knows_command(&id) {
                    shared.deliver(RemoteCommand { id, args });
                } else {
                    let error =
                        ServerMsg::Error { code: "unknown_command".to_string(), message: format!("the app does not accept the command {id}"), id: Some(id) };
                    self.send(error);
                }
            }
            ClientMsg::GetState => {
                let latest = shared.latest_state();
                self.send(ServerMsg::State(latest.clone()));
                *sent = Some(latest);
            }
            // A second hello changes nothing.
            ClientMsg::Hello { .. } => {}
        }
    }

    /// Reads every message that is already available; a timeout just means none.
    fn read_all(&mut self, shared: &Shared, sent: &mut Option<Value>) {
        for _ in 0..MAX_READS_PER_SERVICE {
            match self.socket.read() {
                Ok(Message::Text(text)) => self.handle_text(&text, shared, sent),
                Ok(Message::Close(_)) => {
                    self.dead = true;
                    return;
                }
                Ok(_) => {}
                Err(err) if is_timeout(&err) => return,
                Err(_) => {
                    self.dead = true;
                    return;
                }
            }
            if self.dead {
                return;
            }
        }
    }

    /// Checks the first message: it must be a hello with the right token and
    /// protocol, or the socket closes (spec: Remote Control "Security").
    fn authenticate(&mut self, message: ClientMsg, shared: &Shared, sent: &mut Option<Value>) {
        let ClientMsg::Hello { token, protocol: client_protocol } = message else {
            self.dead = true;
            return;
        };
        if token != shared.token || client_protocol != protocol::PROTOCOL {
            self.close();
            self.dead = true;
            return;
        }
        self.authed = true;
        shared.connected.store(true, Ordering::Relaxed);
        self.send(ServerMsg::Welcome { app: shared.app, version: shared.version.clone(), commands: shared.command_list() });
        if self.dead {
            return;
        }
        let latest = shared.latest_state();
        self.send(ServerMsg::State(latest.clone()));
        *sent = Some(latest);
    }
}

/// The handshake callback: any connection carrying an Origin header is rejected with
/// 403, which keeps web pages from reaching the server (spec: Remote Control "Security").
// The error response is big but the callback runs once per connection, on a cold path.
#[allow(clippy::result_large_err)]
fn origin_guard(request: &Request, response: Response) -> Result<Response, ErrorResponse> {
    if request.headers().contains_key(http::header::ORIGIN) {
        let rejected = match http::Response::builder().status(http::StatusCode::FORBIDDEN).body::<Option<String>>(None) {
            Ok(rejected) => rejected,
            // Unreachable for a fixed status; without it the handshake would just go on.
            Err(_) => return Ok(response),
        };
        return Err(rejected);
    }
    Ok(response)
}

/// The WebSocket limits: no message larger than [`MAX_MESSAGE_BYTES`].
fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES)).max_frame_size(Some(MAX_MESSAGE_BYTES))
}

/// Whether a read or write gave up waiting rather than failing.
fn is_timeout(err: &WsError) -> bool {
    matches!(err, WsError::Io(io_err) if matches!(io_err.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
}
