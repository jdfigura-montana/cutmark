//! The command bus: every hotkey, button and API call becomes a [`Command`] sent here.

use std::sync::mpsc::{self, Receiver, SendError, Sender};
use std::time::Duration;

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

/// What a user or a client asked the Recorder to do.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "the record buttons and the control API construct the other variants in later phases")]
pub enum CommandKind {
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
    /// Add a point mark of this category with an optional label, saved by the label popup.
    LabeledMark {
        /// The mark type the popup's category maps to.
        category: ::marks::MarkType,
        /// The label the user typed, or none.
        label: Option<String>,
    },
    /// Toggle mute on one audio track.
    Mute {
        /// Index of the track to toggle.
        track: usize,
    },
    /// Switch to the named profile, only while idle.
    SetProfile(String),
    /// Ask for the current status.
    GetStatus,
    /// Record 5 seconds with the active profile into a temp folder and judge the result, only while idle.
    Preflight {
        /// Whether the temp files are kept after the report is built.
        keep_files: bool,
    },
    /// Record 12 seconds with screen flashes into a temp folder and measure the camera's delay, only while idle.
    SyncTest,
    /// Turn the replay buffer on, whether or not a take is running.
    ReplayStart,
    /// Turn the replay buffer off and drop what it holds; a running take goes on.
    ReplayStop,
    /// Save what the replay buffer holds as a take.
    ReplaySave,
    /// Turn the replay buffer on when it is off, else off; what its hotkey asks for.
    ReplayToggle,
}

/// One command and the QPC time it was issued, taken when the hotkey, tray item or button fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// What was asked for.
    pub kind: CommandKind,
    /// The QPC time the command was issued, not the time it was processed.
    pub press_qpc: i64,
}

impl Command {
    /// A command of this kind issued at `press_qpc`.
    pub fn new(kind: CommandKind, press_qpc: i64) -> Self {
        Self { kind, press_qpc }
    }
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

/// The command queue. Handles send into it, and the controller thread receives from it.
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

    /// Waits up to `timeout` for the oldest queued command; `None` when nothing arrived.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Command> {
        self.rx.recv_timeout(timeout).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_come_out_in_order_with_their_press_times_then_nothing() -> Result<(), SendError<Command>> {
        let bus = Bus::default();
        let first = bus.sender();
        let second = first.clone();
        first.send(Command::new(CommandKind::StartStop, 10))?;
        second.send(Command::new(CommandKind::Mark(MarkKind::Silent), 20))?;
        let wait = Duration::from_millis(5);
        assert_eq!(bus.recv_timeout(wait), Some(Command::new(CommandKind::StartStop, 10)));
        assert_eq!(bus.recv_timeout(wait), Some(Command::new(CommandKind::Mark(MarkKind::Silent), 20)));
        assert_eq!(bus.recv_timeout(wait), None);
        Ok(())
    }
}
