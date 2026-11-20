//! The local control API: a WebSocket server on 127.0.0.1 that turns client messages into bus commands and
//! controller state changes into events.

pub mod protocol;
pub mod server;
pub mod token;

pub use server::ApiServer;
