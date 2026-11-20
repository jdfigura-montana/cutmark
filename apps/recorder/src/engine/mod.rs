//! The Recorder engine. Nothing in this module tree imports egui.

/// The command bus that hotkeys, buttons and the control API all send into.
pub mod bus;

/// The per-track audio pipeline: any device's packets in, 48 kHz stereo locked
/// to the recording timeline out. Pure logic, no WASAPI, no files, no egui.
#[allow(dead_code)]
pub mod audio_track;

/// The audio thread: every track's capture and pipeline, WAV sidecars, the
/// mixdown and per-track AAC streams, and their packets into the muxer.
#[allow(dead_code)]
pub mod audio;

/// The media clock: QPC times to recording time with paused time removed.
pub mod clock;

/// The controller thread: handles every command and owns the recording session,
/// so hotkeys work while the window is hidden.
pub mod controller;

/// The marks of one take: the sidecar writer, the Markdown and CSV logs and the MKV chapters.
pub mod marks;

/// The constant frame rate pacer: QPC time to exact frame indexes, pure logic.
pub mod pacer;

/// The Prompter link's files for one take: the script snapshot and the script position log.
pub mod script;

/// The recording session: session config, live stats, and start/pause/resume/stop.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod session;

/// The video thread: capture, NV12 conversion, AMF encode and MKV mux on one thread.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod video;
