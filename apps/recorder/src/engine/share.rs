//! One capture shared by the replay buffer and a take. The capture runs while either of them wants it: the buffer is
//! one listener, the take is another, and each comes and goes without touching the other.
//!
//! Generic over the capture, so the sharing is tested with a fake one.

use std::time::Duration;

use super::session::EngineError;

/// What the sharing needs from a capture.
pub(super) trait SharedCapture: Sized {
    /// What a capture or a take is made from.
    type Config;
    /// A take that is recording.
    type Take;
    /// What a finished take leaves behind.
    type Stopped;

    /// Starts a capture together with a take, which holds everything from the start.
    fn open_for_take(config: Self::Config) -> Result<(Self, Self::Take), EngineError>;
    /// Starts a capture whose only listener is a new replay buffer of `length`.
    fn open_for_replay(config: Self::Config, length: Duration) -> Result<Self, EngineError>;
    /// Starts a take on the running capture, pressed at `press_qpc`. The config names the take; its streams are the
    /// capture's.
    fn join_take(&mut self, config: Self::Config, press_qpc: i64) -> Result<Self::Take, EngineError>;
    /// Finishes a take, pressed at `press_qpc`, while the capture goes on.
    fn leave_take(&mut self, take: Self::Take, press_qpc: i64) -> Self::Stopped;
    /// Stops the capture and finishes the take that is still on it.
    fn close(self, take: Option<Self::Take>) -> Option<Self::Stopped>;
    /// Gives the running capture a new replay buffer of `length`.
    fn add_rings(&mut self, length: Duration) -> Result<(), EngineError>;
    /// Drops the replay buffer; the capture goes on.
    fn drop_rings(&mut self);
    /// Whether the capture ended on its own, because one of its threads failed.
    fn is_dead(&self) -> bool;
}

/// The one capture and who listens to it. There is a capture exactly while the buffer is on or a take runs.
pub(super) struct Sharing<C: SharedCapture> {
    capture: Option<C>,
    take: Option<C::Take>,
    replay: bool,
}

impl<C: SharedCapture> Default for Sharing<C> {
    fn default() -> Self {
        Self { capture: None, take: None, replay: false }
    }
}

impl<C: SharedCapture> Sharing<C> {
    /// Whether the replay buffer is on.
    pub(super) fn replay_on(&self) -> bool {
        self.replay
    }

    /// The running capture.
    pub(super) fn capture(&self) -> Option<&C> {
        self.capture.as_ref()
    }

    /// The running take.
    pub(super) fn take(&self) -> Option<&C::Take> {
        self.take.as_ref()
    }

    /// The running take, to change it.
    pub(super) fn take_mut(&mut self) -> Option<&mut C::Take> {
        self.take.as_mut()
    }

    /// Turns the buffer on: on the take's capture while one runs, else on a new capture made from `config`. Nothing
    /// happens when it is already on.
    pub(super) fn replay_start(&mut self, config: impl FnOnce() -> C::Config, length: Duration) -> Result<(), EngineError> {
        if self.replay {
            return Ok(());
        }
        match &mut self.capture {
            Some(capture) => capture.add_rings(length)?,
            None => self.capture = Some(C::open_for_replay(config(), length)?),
        }
        self.replay = true;
        Ok(())
    }

    /// Turns the buffer off and drops what it holds. The capture stops unless a take still records it.
    pub(super) fn replay_stop(&mut self) {
        if !std::mem::take(&mut self.replay) {
            return;
        }
        if self.take.is_some() {
            if let Some(capture) = &mut self.capture {
                capture.drop_rings();
            }
        } else if let Some(capture) = self.capture.take() {
            // With no take there is nothing the end of the capture could leave behind.
            let _ = capture.close(None);
        }
    }

    /// Starts a take: on the buffer's capture while it is on, else on a new capture.
    pub(super) fn start_take(&mut self, config: C::Config, press_qpc: i64) -> Result<(), EngineError> {
        if self.take.is_some() {
            return Err(EngineError::Failed("a take is already running".into()));
        }
        let take = match &mut self.capture {
            Some(capture) => capture.join_take(config, press_qpc)?,
            None => {
                let (capture, take) = C::open_for_take(config)?;
                self.capture = Some(capture);
                take
            }
        };
        self.take = Some(take);
        Ok(())
    }

    /// Finishes the take; None when none runs. The capture goes on while the buffer is on. A capture that died is
    /// closed, which reports why, and takes the buffer with it.
    pub(super) fn stop_take(&mut self, press_qpc: i64) -> Option<C::Stopped> {
        let take = self.take.take()?;
        if self.replay
            && let Some(capture) = &mut self.capture
            && !capture.is_dead()
        {
            return Some(capture.leave_take(take, press_qpc));
        }
        self.replay = false;
        self.capture.take()?.close(Some(take))
    }

    /// Whether the capture behind the buffer died with no take on it, so nothing else would notice.
    pub(super) fn buffer_died(&self) -> bool {
        self.replay && self.take.is_none() && self.capture.as_ref().is_some_and(C::is_dead)
    }
}

#[cfg(test)]
#[path = "share_tests.rs"]
mod tests;
