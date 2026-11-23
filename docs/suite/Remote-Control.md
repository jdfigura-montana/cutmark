# Cutmark Remote Control v1 Spec

Suite-level spec. Defines how outside controllers (Stream Deck first; OBS hook, Bitfocus Companion, Loupedeck later) drive Cutmark apps. First consumer: Audio Editor 1.3 (see docs/audio-editor/1.3-StreamDeck.md).

## Decisions

- One Stream Deck plugin for the whole suite: "Cutmark", UUID `com.cutmark.suite`. The UUID can never change after the first public release, so confirm it before release.
- v1 ships Audio actions only. Other apps add action groups later in the same plugin.
- The plugin is a standalone download (`com.cutmark.suite.streamDeckPlugin`) published next to the app installers. The installers do not install, bundle or launch it. The user double-clicks it to install into Stream Deck.
- Not on the Elgato Marketplace for v1.
- Windows 11 only for v1. macOS is added to the manifest when the suite ships on Mac.
- Stream Deck+ dials are in v1.

## Architecture

```
Stream Deck app  <->  Cutmark plugin (Node, TypeScript)  <->  local WebSocket  <->  Cutmark app (Rust, cutmark-remote crate)
```

- The plugin is thin. It maps key/dial events to protocol commands and draws state it receives. No editing logic in the plugin.
- Each app runs its own control server from the shared crate `crates/cutmark-remote`.

## crates/cutmark-remote (Rust)

- Sync WebSocket server on `tungstenite`, one background thread, no async runtime.
- Binds `127.0.0.1` only, on an OS-assigned port.
- Commands go to the UI thread over a `crossbeam-channel` plus `ctx.request_repaint()`. The app handles them on its next frame through the same action path as keyboard shortcuts, so undo, guards ("not while recording"; text field focus does not apply to remote) and menus stay identical.
- The audio thread is never touched by this crate.
- State goes out from the UI thread: the app calls `remote.publish(state)` each frame. The crate diffs against the last sent state and sends only changes, max 20 messages per second.
- One client at a time in v1. A second connection gets `busy` and is closed.

### Security

- Random 32-byte token per app launch (hex).
- First message must be `hello` with the token, or the socket closes.
- Any connection carrying an `Origin` header is rejected (blocks web pages).
- Enabled by default; Preferences > Remote control has an on/off toggle and a status line ("Stream Deck connected" / "Waiting for Stream Deck" / "Off").

### Discovery files

Folder: `%APPDATA%\Cutmark\remote\` (same folder rules as `appearance.json`).

- `<app>.json` while the app runs: `{ "app": "audio", "pid": 1234, "port": 51234, "token": "...", "protocol": 1, "version": "1.3.0" }`. Written after the server binds, deleted on clean exit. The plugin ignores it if the pid is not running.
- `apps.json`, persistent: `{ "audio": { "exe": "C:\\...\\cutmark-audio.exe", "version": "1.3.0" } }`. Updated on every launch. Used by the plugin to launch an app that is not running.
- Single instance: only the first running instance of an app starts the server. Later instances log and skip it.

## Protocol v1 (JSON text frames)

Client to app:

| Message | Fields |
| --- | --- |
| `hello` | `token`, `client` ("streamdeck"), `protocol` (1) |
| `command` | `id` (string), `args` (object, optional) |
| `get_state` | none |

App to client:

| Message | Fields |
| --- | --- |
| `welcome` | `app`, `version`, `protocol`, `commands` (list of ids and labels the app accepts) |
| `state` | full snapshot (sent after welcome and on `get_state`) |
| `patch` | changed fields only |
| `error` | `code` (`bad_token`, `busy`, `unknown_command`, `blocked`, `bad_args`), `message`, `id` (the command id, when about a command) |

- Command ids are the app's shortcut table ids (S1 phase 14c), plus remote-only ids defined per app (dial and per-track commands).
- `blocked` is returned when a guard stops the command (for example recording in progress). The plugin shows the key's alert.
- Protocol major mismatch: the plugin shows "Update" on its keys and sends nothing.

## Plugin (plugins/suite/)

- Location in repo: `plugins/suite/` with the `com.cutmark.suite.sdPlugin` folder inside, built with the official `@elgato/streamdeck` SDK and the Stream Deck CLI.
- Manifest: `SDKVersion` 2, `Software.MinimumVersion` "7.1", `Nodejs.Version` "24", `OS` windows only, `ApplicationsToMonitor` windows: the app exe names (audio: `cutmark-audio.exe`, confirm against the build).
- WebSocket client: the `ws` npm package (sends no `Origin` header).
- Connection: on `applicationDidLaunch` (or plugin start with the app already running) read `<app>.json`, connect, `hello`. Retry every 2 seconds for 30 seconds, then wait for the next launch event. On `applicationDidTerminate` drop the socket.
- App not running: keys show the dimmed state with title "Open". Pressing launches the exe from `apps.json`. Dials show "Not running".
- Action categories in Stream Deck's list: "Cutmark Audio" in v1.
- Images: SVG, per Cutmark Design Language (monochrome glass, the blue accent for active/on states, red for record active). Two states per toggle key.
- Property inspectors use the SDK's built-in components (sdpi-components) only, no custom styling work.
- Build: local only (`streamdeck pack`). No CI job for the plugin (cost).
- Versioning: manifest `Version` is four-part (`1.0.0.0`) and must increase for every release or Stream Deck refuses the update.

## Testing

- Rust: unit tests for message parsing and state diffing; integration tests with a `tungstenite` client: bad token closes, `Origin` header rejected, second client gets `busy`, unknown command returns `unknown_command`, discovery file written and removed.
- Plugin: manual checklist on real hardware (MK.2 and Stream Deck+) and the Virtual Stream Deck.

## Out of scope for v1

Marketplace listing, macOS, multiple clients, remote control over the network, Companion/Loupedeck adapters, actions for Recorder, Prompter and Video Editor.
