//! The command bus: every hotkey, button and API call becomes a [`Command`] sent here.

use std::sync::mpsc::{self, Receiver, SendError, Sender};

/// Which kind of mark a mark command adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    /// A mark with no label.
    Silent,
    /// A mark that asks for a label.
    WithLabel,
    /// A highlight of the last 30 seconds.
    HighlightLast,
}

/// One thing a user or a client asked the Recorder to do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "the record buttons and the control API construct the other variants in later phases")]
pub enum Command {
    /// Start recording, or stop if already recording.
    StartStop,
    /// Pause the current recording.
    Pause,
    /// Resume a paused recording.
    Resume,
    /// Pause if recording, resume if paused.
    PauseResume,
    /// Add a mark of the given kind.
    Mark(MarkKind),
    /// Toggle mute on one audio track.
    Mute {
        /// Index of the track to toggle.
        track: usize,
    },
    /// Switch to the named profile, only while idle.
    SetProfile(String),
    /// Ask for the current status.
    GetStatus,
}

/// A cloneable handle that sends commands to the bus from any thread.
#[derive(Debug, Clone)]
pub struct BusSender {
    tx: Sender<Command>,
}

impl BusSender {
    /// Queues a command. Fails only if the bus has been dropped.
    pub fn send(&self, command: Command) -> Result<(), SendError<Command>> {
        self.tx.send(command)
    }
}

/// The command queue. Handles send into it, and the app drains it once per frame.
#[derive(Debug)]
pub struct Bus {
    tx: Sender<Command>,
    rx: Receiver<Command>,
}

impl Default for Bus {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self { tx, rx }
    }
}

impl Bus {
    /// A new sending handle for this bus.
    pub fn sender(&self) -> BusSender {
        BusSender { tx: self.tx.clone() }
    }

    /// Takes every queued command, oldest first, without waiting.
    pub fn drain(&self) -> Vec<Command> {
        self.rx.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_returns_commands_in_order_then_nothing() -> Result<(), SendError<Command>> {
        let bus = Bus::default();
        let first = bus.sender();
        let second = first.clone();
        first.send(Command::StartStop)?;
        second.send(Command::Mark(MarkKind::Silent))?;
        assert_eq!(bus.drain(), vec![Command::StartStop, Command::Mark(MarkKind::Silent)]);
        assert!(bus.drain().is_empty());
        Ok(())
    }
}
