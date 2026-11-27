//! The Cutmark input track: the `{stem}.events` file beside a take. Only this crate reads or
//! writes it.
//!
//! See the input track spec, "The events file". The record types mirror the capture crate's input
//! events without depending on it; the Recorder converts.
//!
//! # Layout
//!
//! The file is a run of chunks. Each is a 4-byte type, the payload length (u32), a CRC32 (u32) over
//! the type, the length and the payload, then the payload. All integers are little endian. The first
//! chunk is `HEAD`, which carries the magic and the schema version, and the last is `TAIL`.
//! [`EventsWriter`] appends to the file in place instead of renaming a temp file over it, so that a
//! take that crashes keeps everything up to the last flush.

mod format;
mod reader;
mod types;
mod writer;

#[cfg(test)]
mod tests;

pub use reader::{CursorState, EventsTrack, Timed};

/// The reading side of an events file, under the name its users know it by.
pub type EventsReader = EventsTrack;
pub use types::{
    AppSwitch, CaptureSource, Chord, Counts, CursorSample, CursorShape, DisplayInfo, GeometryChange, GeometryEvent, Header, InputEvent, Modifiers, MouseAction,
    MouseButton, MouseEvent, ScreenRect, ShapeFrame, Streams, Tail,
};
pub use writer::EventsWriter;

/// The schema version this crate writes, and the only one it reads.
pub const SCHEMA_VERSION: u16 = 1;

/// Why an events file could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    /// The file system or the sink failed.
    #[error("events file I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// An earlier write failed, so anything appended now would never be read.
    #[error("an earlier write to the events file failed")]
    Broken,
    /// A record does not fit the file format.
    #[error("invalid record: {0}")]
    InvalidRecord(&'static str),
    /// The data does not start with a whole, valid `HEAD` chunk.
    #[error("not an events file, or its header is damaged")]
    BadHeader,
    /// The file was written with another schema version.
    #[error("events schema version {0} is not supported")]
    UnsupportedVersion(u16),
}
