//! Why [`RemoteServer::start`](crate::RemoteServer::start) can fail.

use std::io;

use thiserror::Error;

/// Why a control server could not start.
#[derive(Debug, Error)]
pub enum RemoteError {
    /// Another live instance of the app is already serving remote control.
    #[error("remote control is already running for this app")]
    AlreadyRunning,
    /// The listener socket could not be opened.
    #[error("the remote control listener could not be opened ({0})")]
    Bind(#[source] io::Error),
    /// The Cutmark config folder is missing or a discovery file could not be written.
    #[error("the remote control discovery files could not be written ({0})")]
    Discovery(#[source] io::Error),
    /// The server thread could not be started.
    #[error("the remote control thread could not be started ({0})")]
    Thread(#[source] io::Error),
}
