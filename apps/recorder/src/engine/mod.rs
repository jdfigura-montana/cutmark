//! The Recorder engine. Nothing in this module tree imports egui.

/// The command bus that hotkeys, buttons and the control API all send into.
pub mod bus;

/// The constant frame rate pacer: QPC time to exact frame indexes, pure logic.
pub mod pacer;

/// The recording session: session config, live stats, and start/pause/resume/stop.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod session;

/// The video thread: capture, NV12 conversion, AMF encode and MKV mux on one thread.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod video;
