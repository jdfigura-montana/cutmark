//! Integration tests: a real tungstenite client drives a server bound to a temp
//! folder (spec: Remote Control "Testing").

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::ErrorKind;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cutmark_remote::{CommandInfo, RemoteConfig, RemoteError, RemoteServer};
use serde_json::{Value, json};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Error as WsError, Message, WebSocket};

/// A temp discovery folder that cleans itself up.
struct TempDir(PathBuf);

impl TempDir {
    /// A fresh empty folder.
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("cutmark-remote-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp folder");
        Self(path)
    }

    /// The `<app>.json` discovery file inside the folder.
    fn app_file(&self) -> PathBuf {
        self.0.join("testapp.json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The config every test uses: one app, one command, the temp folder.
fn config(dir: &TempDir) -> RemoteConfig {
    RemoteConfig {
        app: "testapp",
        version: "1.3.0".to_string(),
        exe: PathBuf::from("C:\\apps\\cutmark-testapp.exe"),
        commands: vec![CommandInfo { id: "play_pause".to_string(), label: "Play or pause".to_string() }],
        dir: Some(dir.0.clone()),
    }
}

/// A server with its temp folder.
fn start(name: &str) -> (TempDir, RemoteServer) {
    let dir = TempDir::new(name);
    let server = RemoteServer::start(config(&dir)).expect("the server starts");
    (dir, server)
}

/// A connected client.
type Client = WebSocket<MaybeTlsStream<TcpStream>>;

/// Connects a plain client with generous timeouts.
fn connect(port: u16) -> Client {
    let (client, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/")).expect("connect");
    set_timeout(&client, Duration::from_secs(5));
    client
}

/// Sets the client's read timeout.
fn set_timeout(client: &Client, timeout: Duration) {
    if let MaybeTlsStream::Plain(stream) = client.get_ref() {
        stream.set_read_timeout(Some(timeout)).expect("read timeout");
    }
}

/// Sends a hello with `token` and consumes the welcome and the state that follow it.
fn hello(client: &mut Client, token: &str) -> Value {
    let hello = json!({ "type": "hello", "token": token, "client": "streamdeck", "protocol": 1 });
    client.send(Message::text(hello.to_string())).expect("hello is sent");
    let welcome = read_json(client);
    assert_eq!(welcome["type"], "welcome");
    let state = read_json(client);
    assert_eq!(state["type"], "state");
    welcome
}

/// Reads the next text message as JSON.
fn read_json(client: &mut Client) -> Value {
    match client.read().expect("a text message") {
        Message::Text(text) => serde_json::from_str(&text).expect("valid json"),
        other => panic!("expected a text message, got {other:?}"),
    }
}

/// Reads the next message; `Err` on a close or a socket error, the JSON of a text message otherwise.
fn read_close_or_json(client: &mut Client) -> Result<Value, WsError> {
    match client.read() {
        Ok(Message::Text(text)) => Ok(serde_json::from_str(&text).expect("valid json")),
        Ok(Message::Close(_)) => Err(WsError::ConnectionClosed),
        Ok(other) => panic!("expected text or close, got {other:?}"),
        Err(err) => Err(err),
    }
}

/// The token from the discovery file, which also checks the file's shape.
fn token_from(dir: &TempDir, server: &RemoteServer) -> String {
    let file: Value = serde_json::from_str(&std::fs::read_to_string(dir.app_file()).expect("discovery file")).expect("json");
    assert_eq!(file["app"], "testapp");
    assert_eq!(file["pid"], json!(std::process::id()));
    assert_eq!(file["port"], json!(server.port()));
    assert_eq!(file["protocol"], 1);
    assert_eq!(file["version"], "1.3.0");
    file["token"].as_str().expect("token").to_string()
}

#[test]
fn a_wrong_token_closes_the_connection() {
    let (_dir, server) = start("wrong-token");
    let mut client = connect(server.port());
    let hello = json!({ "type": "hello", "token": "wrong", "client": "streamdeck", "protocol": 1 });
    client.send(Message::text(hello.to_string())).expect("hello is sent");
    assert!(read_close_or_json(&mut client).is_err());
    assert!(!server.is_connected());
}

#[test]
fn a_connection_with_an_origin_header_is_rejected_with_403() {
    let (_, server) = start("origin");
    let mut request = format!("ws://127.0.0.1:{}/", server.port()).into_client_request().expect("request");
    request.headers_mut().insert(http::header::ORIGIN, http::HeaderValue::from_static("http://evil.example"));
    match tungstenite::connect(request) {
        Err(WsError::Http(response)) => assert_eq!(response.status(), http::StatusCode::FORBIDDEN),
        other => panic!("the handshake should be rejected, got {other:?}"),
    }
}

#[test]
fn a_second_client_gets_busy_and_is_closed() {
    let (dir, server) = start("busy");
    let token = token_from(&dir, &server);
    let mut first = connect(server.port());
    hello(&mut first, &token);
    assert!(server.is_connected());
    let mut second = connect(server.port());
    let busy = read_json(&mut second);
    assert_eq!(busy["type"], "error");
    assert_eq!(busy["code"], "busy");
    assert!(read_close_or_json(&mut second).is_err());
}

#[test]
fn an_unknown_command_gets_unknown_command() {
    let (dir, server) = start("unknown");
    let token = token_from(&dir, &server);
    let mut client = connect(server.port());
    hello(&mut client, &token);
    let command = json!({ "type": "command", "id": "nope" });
    client.send(Message::text(command.to_string())).expect("command is sent");
    let reply = read_json(&mut client);
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["code"], "unknown_command");
    assert_eq!(reply["id"], "nope");
}

#[test]
fn the_discovery_file_is_written_and_removed() {
    let (dir, server) = start("discovery");
    assert!(dir.app_file().exists());
    let _token = token_from(&dir, &server);
    let apps: Value = serde_json::from_str(&std::fs::read_to_string(dir.0.join("apps.json")).expect("apps.json")).expect("json");
    assert_eq!(apps["testapp"]["exe"], "C:\\apps\\cutmark-testapp.exe");
    assert_eq!(apps["testapp"]["version"], "1.3.0");
    drop(server);
    assert!(!dir.app_file().exists());
    // The plugin's launch table persists after the app is gone.
    assert!(dir.0.join("apps.json").exists());
}

#[test]
fn apps_json_keeps_the_other_apps_entries() {
    let dir = TempDir::new("merge");
    let seed = json!({ "gallery": { "exe": "C:\\apps\\gallery.exe", "version": "1.0.0" } });
    std::fs::write(dir.0.join("apps.json"), seed.to_string()).expect("seed apps.json");
    let server = RemoteServer::start(config(&dir)).expect("the server starts");
    drop(server);
    let apps: Value = serde_json::from_str(&std::fs::read_to_string(dir.0.join("apps.json")).expect("apps.json")).expect("json");
    assert_eq!(apps["gallery"]["version"], "1.0.0");
    assert_eq!(apps["testapp"]["version"], "1.3.0");
}

#[test]
fn a_stale_discovery_file_does_not_block_the_start() {
    let dir = TempDir::new("stale");
    let stale = json!({ "pid": 0 });
    std::fs::write(dir.app_file(), stale.to_string()).expect("write the discovery file");
    let server = RemoteServer::start(config(&dir)).expect("the server starts");
    drop(server);
}

#[test]
#[cfg(windows)]
fn a_live_pid_in_the_discovery_file_blocks_a_second_start() {
    let dir = TempDir::new("already-running");
    let live = json!({ "pid": std::process::id() });
    std::fs::write(dir.app_file(), live.to_string()).expect("write the discovery file");
    assert!(matches!(RemoteServer::start(config(&dir)), Err(RemoteError::AlreadyRunning)));
}

#[test]
fn a_command_arrives_on_the_channel_and_wakes_the_app() {
    let (dir, server) = start("command");
    let token = token_from(&dir, &server);
    let woke = Arc::new(AtomicBool::new(false));
    let woke_flag = Arc::clone(&woke);
    server.set_wake(Box::new(move || woke_flag.store(true, Ordering::SeqCst)));
    let mut client = connect(server.port());
    hello(&mut client, &token);
    let command = json!({ "type": "command", "id": "play_pause", "args": { "track": 2 } });
    client.send(Message::text(command.to_string())).expect("command is sent");
    let mut received = None;
    for _ in 0..500 {
        if let Some(arrived) = server.try_recv() {
            received = Some(arrived);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let command = received.expect("the command arrives");
    assert_eq!(command.id, "play_pause");
    assert_eq!(command.args, json!({ "track": 2 }));
    assert!(woke.load(Ordering::SeqCst));
}

#[test]
fn get_state_and_publish_send_a_full_state_then_patches() {
    let (dir, server) = start("state");
    let token = token_from(&dir, &server);
    let mut client = connect(server.port());
    let welcome = hello(&mut client, &token);
    assert_eq!(welcome["app"], "testapp");
    assert_eq!(welcome["protocol"], 1);
    assert_eq!(welcome["commands"][0]["id"], "play_pause");
    assert_eq!(welcome["commands"][0]["label"], "Play or pause");
    // Nothing published yet: an empty state.
    let request = json!({ "type": "get_state" });
    client.send(Message::text(request.to_string())).expect("get_state is sent");
    assert_eq!(read_json(&mut client)["state"], json!({}));
    server.publish(json!({ "playing": true, "position": 5 }));
    let first = read_json(&mut client);
    assert_eq!(first["type"], "patch");
    assert_eq!(first["patch"], json!({ "playing": true, "position": 5 }));
    assert!(server.is_connected());
    server.publish(json!({ "playing": false, "position": 5 }));
    let patch = read_json(&mut client);
    assert_eq!(patch["type"], "patch");
    assert_eq!(patch["patch"], json!({ "playing": false }));
    // A full snapshot on request again.
    client.send(Message::text(request.to_string())).expect("get_state is sent");
    let state = read_json(&mut client);
    assert_eq!(state["type"], "state");
    assert_eq!(state["state"], json!({ "playing": false, "position": 5 }));
}

#[test]
fn reply_error_reaches_the_client_with_the_command_id() {
    let (dir, server) = start("reply");
    let token = token_from(&dir, &server);
    let mut client = connect(server.port());
    hello(&mut client, &token);
    server.reply_error("record", "blocked", "recording is in progress");
    let reply = read_json(&mut client);
    assert_eq!(reply["type"], "error");
    assert_eq!(reply["code"], "blocked");
    assert_eq!(reply["message"], "recording is in progress");
    assert_eq!(reply["id"], "record");
}

#[test]
fn publish_is_limited_to_20_patches_a_second_and_ends_on_the_latest() {
    let (dir, server) = start("rate");
    let token = token_from(&dir, &server);
    let mut client = connect(server.port());
    hello(&mut client, &token);
    for tick in 0..30 {
        server.publish(json!({ "tick": tick }));
    }
    // Collect every patch for 700 ms: coalesced states, never more than 20 a second.
    let mut patches = Vec::new();
    let deadline = Instant::now() + Duration::from_millis(700);
    while Instant::now() < deadline {
        set_timeout(&client, Duration::from_millis(50));
        match client.read() {
            Ok(Message::Text(text)) => patches.push(serde_json::from_str::<Value>(&text).expect("valid json")),
            Ok(Message::Close(_)) => panic!("the connection closed"),
            Ok(_) => {}
            Err(WsError::Io(err)) if err.kind() == ErrorKind::WouldBlock || err.kind() == ErrorKind::TimedOut => continue,
            Err(err) => panic!("read failed: {err}"),
        }
    }
    assert!(patches.len() <= 20, "too many patches: {}", patches.len());
    assert_eq!(patches.last().expect("at least one patch")["patch"]["tick"], 29);
}
