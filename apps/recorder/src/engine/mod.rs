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

/// The camera ISO file: its own pacer, encoder and mux thread, fed by the shared camera.
pub mod cam;

/// The media clock: QPC times to recording time with paused time removed.
pub mod clock;

/// The composite block of session.json: the canvas, the camera layer as the compositor places it, and the background.
pub mod composite;

/// The controller thread: handles every command and owns the recording session,
/// so hotkeys work while the window is hidden.
pub mod controller;

/// The marks of one take: the sidecar writer, the Markdown and CSV logs and the MKV chapters.
pub mod marks;

/// The constant frame rate pacer: QPC time to exact frame indexes, pure logic.
pub mod pacer;

/// The pre-flight verdict: pass, warn or fail with one line per check.
pub mod preflight;

/// The Prompter link's files for one take: the script snapshot and the script position log.
pub mod script;

/// Live numbers of a running take: bytes, bitrate, free disk space and time left.
pub mod stats;

/// The recording session: session config, live stats, and start/pause/resume/stop.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod session;

/// The video thread: capture, NV12 conversion, AMF encode and MKV mux on one thread.
/// The UI wires it up in a later phase 2 prompt.
#[allow(dead_code)]
pub mod video;
