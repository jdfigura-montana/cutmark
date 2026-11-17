//! The Recorder engine. Nothing in this module tree imports egui.

/// The command bus that hotkeys, buttons and the control API all send into.
pub mod bus;

/// The constant frame rate pacer: QPC time to exact frame indexes, pure logic.
pub mod pacer;
