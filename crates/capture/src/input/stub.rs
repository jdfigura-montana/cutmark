//! Stub so the crate compiles on other targets: the logger can never be created.

use std::sync::mpsc::Receiver;

use super::{InputEvent, InputOptions, InputStartInfo};
use crate::CaptureError;

/// The input logger needs Windows.
pub enum InputLogger {}

impl InputLogger {
    /// Always fails off Windows.
    pub fn start(_options: InputOptions) -> Result<Self, CaptureError> {
        Err(CaptureError::Unsupported("input logging"))
    }

    /// What the logger read when it started.
    pub fn start_info(&self) -> &InputStartInfo {
        match *self {}
    }

    /// The event queue.
    pub fn receiver(&self) -> &Receiver<InputEvent> {
        match *self {}
    }

    /// False once the thread ended.
    pub fn is_running(&self) -> bool {
        match *self {}
    }

    /// How many sends the full queue refused.
    pub fn dropped(&self) -> u64 {
        match *self {}
    }

    /// Stops the logger.
    pub fn stop(&mut self) {
        match *self {}
    }
}
