//! The Recorder engine. Nothing in this module tree imports egui.

/// The idle watch of app and desktop-except tracks: their state and meters between takes.
pub mod app_monitor;

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

/// The capture part of a session: the capture, the compositor and the encoders, with or without a take.
mod capture_part;

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
mod input_track;

pub mod marks;

/// The constant frame rate pacer: QPC time to exact frame indexes, pure logic.
pub mod pacer;

/// The pre-flight verdict: pass, warn or fail with one line per check.
pub mod preflight;

/// The replay buffer: rings of encoded packets, PCM and marks, and the writer that saves a snapshot as a take.
pub mod replay;

/// Saving the replay buffer: the writer thread's queue, the 2 s rule and the RAM estimate.
mod replay_save;

/// The Prompter link's files for one take: the script snapshot and the script position log.
pub mod script;

/// One capture shared by the replay buffer and a take: who listens to it, and when it starts and stops.
mod share;

/// Where a take's encoded output goes: the sink trait of the stream threads and the writer behind it.
mod sink;

/// Which of a capture's output belongs to the take, and on what time stamps. Pure logic.
mod take_gate;

/// The take part of a session: the files one take writes.
mod take_part;

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
