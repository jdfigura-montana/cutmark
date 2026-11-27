//! Cutmark's shared remote control server: one local WebSocket per app that a
//! Stream Deck plugin drives (spec: docs/suite/Remote-Control.md).
//!
//! An app builds a [`RemoteConfig`], calls [`RemoteServer::start`], polls
//! [`RemoteServer::try_recv`] every frame and calls [`RemoteServer::publish`] with
//! its state. The crate owns the socket, the token check, the discovery files and
//! the patch rate limit; the app owns the commands and their guards.

mod config;
mod discovery;
mod error;
mod pid;
mod protocol;
mod server;
mod session;
mod token;

pub use config::{CommandInfo, RemoteConfig};
pub use error::RemoteError;
pub use server::{RemoteCommand, RemoteServer};
