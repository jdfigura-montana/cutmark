//! editor::transport: forwards play/pause/stop/seek commands to the audio engine.

use std::sync::atomic::Ordering;

use crate::audio::player::Command;
use crate::editor::editor::Editor;

/// Transport state as the UI sees it (mirrors the audio player's state).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportState {
    /// Not playing.
    Stopped,
    /// Rendering the timeline.
    Playing,
    /// Frozen at a position.
    Paused,
    /// Rendering bin preview material.
    Previewing,
    /// Rendering the scrub reader (audible scrubbing).
    Scrubbing,
}

/// Maps a `Shared::state` code to a [`TransportState`].
fn code_to_state(code: u8) -> TransportState {
    match code {
        1 => TransportState::Playing,
        2 => TransportState::Paused,
        3 => TransportState::Previewing,
        4 => TransportState::Scrubbing,
        _ => TransportState::Stopped,
    }
}

impl Editor {
    /// Transport commands are refused while an export is running or a take is
    /// being recorded. (Stop is the exception while recording: it ends the take.)
    fn transport_locked(&self) -> bool {
        self.is_locked() || self.is_recording()
    }

    /// Current transport state; `Stopped` when audio did not start.
    pub fn transport_state(&self) -> TransportState {
        match &self.audio {
            Some(engine) => code_to_state(engine.shared().state.load(Ordering::Relaxed)),
            None => TransportState::Stopped,
        }
    }

    /// Space: play from the playhead, or stop while playing (or previewing).
    /// While recording it ends the take and keeps it.
    pub fn toggle_play(&mut self) {
        if self.is_recording() {
            self.stop_recording();
            return;
        }
        if self.transport_locked() {
            return;
        }
        let Some(engine) = &self.audio else {
            return;
        };
        match self.transport_state() {
            TransportState::Stopped | TransportState::Paused => {
                let from = self.view.playhead;
                engine.send(Command::Play { from });
            }
            TransportState::Playing => engine.send(Command::Stop),
            TransportState::Previewing => engine.send(Command::StopPreview),
            // A scrub owns the mouse; Space does not fight it.
            TransportState::Scrubbing => {}
        }
    }

    /// P: pause in place while playing, or resume from the paused position.
    pub fn toggle_pause(&mut self) {
        if self.transport_locked() {
            return;
        }
        let Some(engine) = &self.audio else {
            return;
        };
        match self.transport_state() {
            TransportState::Playing => engine.send(Command::Pause),
            TransportState::Paused => {
                let from = engine.shared().playhead.load(Ordering::Relaxed);
                engine.send(Command::Play { from });
            }
            TransportState::Stopped | TransportState::Previewing | TransportState::Scrubbing => {}
        }
    }

    /// Stop button: the same as Space while playing; nothing otherwise. While
    /// recording it ends the take and keeps it.
    pub fn stop(&mut self) {
        if self.is_recording() {
            self.stop_recording();
            return;
        }
        if self.transport_locked() {
            return;
        }
        if self.transport_state() == TransportState::Playing
            && let Some(engine) = &self.audio
        {
            engine.send(Command::Stop);
        }
    }

    /// Plays `start..end` once, then stops and returns to `start`.
    ///
    /// Used by Shift+Space (phase 6).
    pub fn play_range(&mut self, start: u64, end: u64) {
        if self.transport_locked() {
            return;
        }
        if let Some(engine) = &self.audio {
            engine.send(Command::PlayRange { start, end });
        }
    }

    /// Moves the playhead to `pos`; while playing, playback continues there.
    pub fn seek(&mut self, pos: u64) {
        if self.transport_locked() {
            return;
        }
        self.view.playhead = pos;
        if let Some(engine) = &self.audio {
            engine.send(Command::Seek(pos));
        }
    }

    /// Starts a scrub at `pos`: playback stops and the scrub reader takes
    /// over. The playhead jumps to `pos`.
    pub fn scrub_start(&mut self, pos: u64) {
        if self.transport_locked() {
            return;
        }
        self.view.playhead = pos;
        if let Some(engine) = &self.audio {
            engine.send(Command::ScrubStart(pos));
        }
    }

    /// Moves the scrub target to `pos` and the playhead with it. The audio
    /// speed follows the mouse (clamped to 4x); with no audio only the
    /// playhead moves.
    pub fn scrub_to(&mut self, pos: u64) {
        if self.transport_locked() {
            return;
        }
        self.view.playhead = pos;
        if let Some(engine) = &self.audio {
            engine.send(Command::ScrubTarget(pos));
        }
    }

    /// Ends the scrub: the gain fades out and the playhead stays at the last
    /// scrub position.
    pub fn scrub_end(&mut self) {
        if self.transport_locked() {
            return;
        }
        if let Some(engine) = &self.audio {
            engine.send(Command::ScrubEnd);
        }
    }

    /// Home: playhead to 0.
    pub fn jump_start(&mut self) {
        self.seek(0);
    }

    /// End: playhead to the project end.
    pub fn jump_end(&mut self) {
        self.seek(self.project_length());
    }

    /// Reads the audio `Shared` each frame: while playing, the audio playhead
    /// is copied into the view; when playback just went from playing to
    /// stopped, the final position (the return point) is copied instead.
    pub fn poll_transport(&mut self) {
        // While a take exists the playhead is where the recording is (the end
        // of what has been written), not where the audio player is: the
        // player runs on past the project end and carries the output latency.
        if let Some(playhead) = self.record_playhead() {
            self.view.playhead = playhead;
            if let Some(engine) = &self.audio {
                let state = code_to_state(engine.shared().state.load(Ordering::Relaxed));
                self.transport_prev = Some(state);
            }
            return;
        }
        let Some(engine) = &self.audio else {
            return;
        };
        let shared = engine.shared();
        let state = code_to_state(shared.state.load(Ordering::Relaxed));
        // While scrubbing the mouse drives the playhead; the audio position
        // must not overwrite it.
        if state != TransportState::Scrubbing
            && (state == TransportState::Playing || (self.transport_prev == Some(TransportState::Playing) && state == TransportState::Stopped))
        {
            self.view.playhead = shared.playhead.load(Ordering::Relaxed);
        }
        self.transport_prev = Some(state);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::fixtures;
    use crate::model::project::ViewState;

    /// An in-memory editor with no audio engine.
    fn editor() -> Editor {
        Editor::with_project(fixtures::project_with_clips(&[&[(0, 100)]]), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") })
    }

    #[test]
    fn bin_preview_methods_are_noops_without_audio() {
        let mut editor = editor();
        editor.preview_media(uuid::Uuid::nil());
        assert_eq!(editor.previewing(), None);
        assert_eq!(editor.preview_progress(), 0.0);
        editor.stop_preview();
        assert_eq!(editor.previewing(), None);
    }

    #[test]
    fn without_audio_the_state_is_stopped_and_commands_do_nothing() {
        let mut editor = editor();
        assert_eq!(editor.transport_state(), TransportState::Stopped);
        editor.view.playhead = 5_000;
        editor.toggle_play();
        editor.toggle_pause();
        editor.stop();
        editor.play_range(0, 1_000);
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 5_000);
    }

    #[test]
    fn scrub_methods_move_the_playhead_without_audio() {
        let mut editor = editor();
        editor.scrub_start(1_000);
        assert_eq!(editor.view.playhead, 1_000);
        editor.scrub_to(2_000);
        assert_eq!(editor.view.playhead, 2_000);
        editor.scrub_end();
        assert_eq!(editor.view.playhead, 2_000);
        // Polling never moves the playhead without audio.
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 2_000);
    }

    #[test]
    fn scrub_end_keeps_the_last_scrub_position() {
        let mut editor = editor();
        editor.scrub_start(500);
        editor.scrub_to(900);
        editor.scrub_to(700);
        editor.scrub_end();
        assert_eq!(editor.view.playhead, 700);
    }

    #[test]
    fn seek_moves_the_playhead() {
        let mut editor = editor();
        editor.seek(123_456);
        assert_eq!(editor.view.playhead, 123_456);
    }

    #[test]
    fn jump_start_and_jump_end_move_the_playhead() {
        let mut editor = editor();
        editor.seek(50_000);
        editor.jump_end();
        assert_eq!(editor.view.playhead, editor.project_length());
        editor.jump_start();
        assert_eq!(editor.view.playhead, 0);
    }

    #[test]
    fn poll_without_audio_keeps_the_view_playhead() {
        let mut editor = editor();
        editor.seek(9_000);
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 9_000);
    }
}
