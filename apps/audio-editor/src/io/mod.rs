//! io: file decoding, encoding and project file handling; never plays audio.

pub mod autosave;
pub mod config;
pub mod decode;
pub mod export;
pub mod import;
pub mod limits;
pub mod mp3;
pub mod paths;
pub mod peaks;
pub mod project_file;
pub mod recent;
pub mod record_writer;
pub mod resample;
pub mod wav;

use std::path::Path;

use thiserror::Error;

/// Errors from project folder layout and project.json handling.
#[derive(Debug, Error)]
pub enum IoError {
    /// Invalid project name.
    #[error("invalid project name: {0}")]
    InvalidProjectName(String),
    /// The target project folder already exists.
    #[error("project folder already exists: {0}")]
    FolderExists(String),
    /// The path is not a project folder or project.json file.
    #[error("not a project: {0}")]
    NotAProject(String),
    /// project.json or an autosave failed validation, so the project does not
    /// open. The text is the reason, e.g. "the sample rate 0 Hz is outside ...".
    #[error("This project file is invalid: {0}")]
    InvalidProject(String),
    /// A path was refused before it reached a file or process call.
    #[error("unsafe path: {0}")]
    UnsafePath(String),
    /// Unsupported project.json format version.
    #[error("unsupported project file format version: found {found}, maximum supported {max}")]
    UnsupportedVersion {
        /// Version found in the file.
        found: u32,
        /// Highest version this build supports.
        max: u32,
    },
    /// Wrapped std::io error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Wrapped serde_json error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A waveform peaks cache file is corrupt (bad magic, bad version or truncated).
    #[error("corrupt peaks file: {0}")]
    CorruptPeaks(String),
    /// Audio data could not be decoded.
    #[error("could not decode audio: {0}")]
    Decode(String),
    /// Unsupported audio file or malformed.
    #[error("unsupported audio file: {0}")]
    UnsupportedAudio(String),
    /// A media file referenced by the bin is missing from the project's media/ folder.
    #[error("media file is missing: {0}")]
    MediaOffline(uuid::Uuid),
    /// An export sink could not be opened or written (bad format settings, encoder failure).
    #[error("export failed: {0}")]
    Export(String),
}

/// Returns true if the path is an existing file named project.json.
pub(crate) fn is_project_json(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .map(|n| n == "project.json")
            .unwrap_or(false)
}
