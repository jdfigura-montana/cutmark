//! The configuration an app hands to [`RemoteServer::start`](crate::RemoteServer::start).

use std::path::PathBuf;

/// One command an app accepts over the remote protocol.
#[derive(Clone, Debug)]
pub struct CommandInfo {
    /// The command id: an app shortcut table id or a remote-only id.
    pub id: String,
    /// The human-readable label the plugin can show.
    pub label: String,
}

/// The settings one app's control server runs with.
#[derive(Clone, Debug)]
pub struct RemoteConfig {
    /// The app's short name, used in the protocol and as the discovery file name (`audio`).
    pub app: &'static str,
    /// The app version reported in `welcome` and the discovery files.
    pub version: String,
    /// The app's own exe, recorded in `apps.json` so the plugin can launch the app.
    pub exe: PathBuf,
    /// The commands the app accepts; any other id gets `unknown_command`.
    pub commands: Vec<CommandInfo>,
    /// Overrides the discovery folder (tests use this); `None` means `%APPDATA%\Cutmark\remote\`.
    pub dir: Option<PathBuf>,
}
