//! audio::player: transport state machine with declick ramps; pure code, no cpal.

use crate::audio::mix_plan::MixPlan;
use crate::audio::mixer;
use crate::audio::scrub::Scrub;

/// Transport command sent from the editor to the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Start playback from `from`.
    Play { from: u64 },
    /// Play `start..end` once, then stop and return to `start`.
    PlayRange { start: u64, end: u64 },
    /// Freeze playback at the current position.
    Pause,
    /// Stop playback and return the playhead to where playback started.
    Stop,
    /// Move the playhead to `p` (playback continues from there when playing).
    Seek(u64),
    /// Start previewing the preview material from its beginning.
    Preview,
    /// End preview playback.
    StopPreview,
    /// Start scrubbing at ``pos``; stops any playback or preview immediately.
    ScrubStart(u64),
    /// Move the scrub target; the last target in one callback wins.
    ScrubTarget(u64),
    /// End scrubbing; the gain fades out and the state becomes Stopped.
    ScrubEnd,
}

/// Transport state of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not playing; output is silence.
    Stopped,
    /// Rendering the timeline.
    Playing,
    /// Frozen at a position; output is silence.
    Paused,
    /// Rendering the preview material instead of the timeline.
    Previewing,
    /// Rendering the scrub reader.
    Scrubbing,
}

/// Change applied once the declick ramp-down reaches zero gain.
#[derive(Debug, Clone, Copy)]
enum Pending {
    /// Rewind to `return_to` and stop.
    Stop,
    /// Freeze at the captured position.
    Pause { at: u64 },
    /// Jump to `to` and keep playing from there.
    Seek { to: u64 },
    /// Begin (or restart) timeline playback.
    StartPlay { from: u64, range_end: Option<u64> },
    /// Begin preview playback from its start.
    StartPreview,
    /// The preview material ended on its own.
    EndPreview,
    /// Preview was cancelled.
    StopPreview,
}

/// Direction of the active declick ramp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RampDir {
    /// Gain rising from 0 to 1.
    Up,
    /// Gain falling from `ramp_from` to 0.
    Down,
}

/// Transport state machine and block renderer with 5 ms declick ramps.
pub struct Player {
    /// Declick ramp length in frames (rate / 200).
    ramp_len: u32,
    /// Timeline playhead in samples.
    position: u64,
    /// Position playback returns to on stop.
    return_to: u64,
    /// End of a play range, if one is playing.
    range_end: Option<u64>,
    /// Preview playhead in samples.
    preview_pos: u64,
    /// Current transport state.
    state: State,
    /// Frames already rendered of the active ramp.
    ramp_pos: u32,
    /// Total frames of the active ramp (0 means steady full gain).
    ramp_total: u32,
    /// Gain at the start of a down ramp.
    ramp_from: f32,
    /// Direction of the active ramp.
    ramp_dir: RampDir,
    /// Change waiting for the ramp-down to finish.
    pending: Option<Pending>,
    /// The scrub reader, owned by the player.
    scrub: Scrub,
    /// Last scrub target; where the playhead lands after a scrub ends.
    last_target: u64,
    /// Frames left in the scrub fade-out countdown, if ending.
    scrub_stop: Option<u64>,
}

impl Player {
    /// Creates a stopped player with a ramp of `rate / 200` frames (5 ms).
    pub fn new(rate: u32) -> Player {
        Player {
            ramp_len: (rate / 200).max(1),
            position: 0,
            return_to: 0,
            range_end: None,
            preview_pos: 0,
            state: State::Stopped,
            ramp_pos: 0,
            ramp_total: 0,
            ramp_from: 0.0,
            ramp_dir: RampDir::Up,
            pending: None,
            scrub: Scrub::new(rate),
            last_target: 0,
            scrub_stop: None,
        }
    }

    /// Applies a transport command.
    pub fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Play { from } => self.start_play(from, None),
            Command::PlayRange { start, end } => self.start_play(start, Some(end.max(start))),
            Command::Pause => {
                if self.state == State::Playing {
                    let at = self.position;
                    self.ramp_to(Pending::Pause { at });
                }
            }
            Command::Stop => match self.state {
                State::Stopped => {}
                State::Paused => {
                    self.state = State::Stopped;
                    self.position = self.return_to;
                }
                State::Playing => self.ramp_to(Pending::Stop),
                State::Previewing => self.ramp_to(Pending::StopPreview),
                State::Scrubbing => {
                    self.state = State::Stopped;
                    self.position = self.last_target;
                    self.scrub_stop = None;
                }
            },
            Command::Seek(p) => match self.state {
                State::Playing => self.ramp_to(Pending::Seek { to: p }),
                State::Stopped | State::Paused => self.position = p,
                State::Previewing => {}
                State::Scrubbing => {}
            },
            Command::Preview => match self.state {
                State::Stopped | State::Paused | State::Scrubbing => {
                    self.state = State::Previewing;
                    self.preview_pos = 0;
                    self.start_ramp_up();
                }
                State::Playing | State::Previewing => self.ramp_to(Pending::StartPreview),
            },
            Command::StopPreview => {
                if self.state == State::Previewing {
                    self.ramp_to(Pending::StopPreview);
                }
            }
            Command::ScrubStart(pos) => {
                self.pending = None;
                self.ramp_total = 0;
                self.scrub_stop = None;
                self.last_target = pos;
                self.scrub.start(pos);
                self.state = State::Scrubbing;
            }
            Command::ScrubTarget(pos) => {
                self.last_target = pos;
                if self.state == State::Scrubbing {
                    self.scrub.set_target(pos);
                    self.scrub_stop = None;
                }
            }
            Command::ScrubEnd => {
                if self.state == State::Scrubbing {
                    let at = self.scrub.position();
                    self.scrub.set_target(at.round() as u64);
                    self.scrub_stop = Some(u64::from(self.ramp_len) * 2);
                }
            }
        }
    }

    /// Renders one block of stereo interleaved output at the project rate.
    /// No allocation; safe for the audio thread.
    pub fn process(&mut self, timeline: &MixPlan, preview: &MixPlan, out: &mut [f32]) {
        let mut cursor = 0usize;
        while cursor < out.len() {
            let frames = (out.len() - cursor) / 2;
            if frames == 0 {
                break;
            }
            if self.state == State::Scrubbing {
                // The scrub reader owns the whole block: variable speed, its
                // own declick gain, meters included. The mouse drives the
                // position; `scrub_stop` ends the scrub after its countdown.
                self.scrub.process(timeline, out);
                self.position = self.scrub.position().round().max(0.0) as u64;
                let frames64 = (out.len() / 2) as u64;
                if let Some(left) = self.scrub_stop {
                    if left <= frames64 {
                        self.scrub_stop = None;
                        self.state = State::Stopped;
                        self.position = self.last_target;
                    } else {
                        self.scrub_stop = Some(left - frames64);
                    }
                }
                return;
            }
            if self.pending.is_none() && !self.active() {
                for s in &mut out[cursor..] {
                    *s = 0.0;
                }
                return;
            }
            if self.pending.is_none() {
                let remaining = self
                    .play_end(timeline, preview)
                    .saturating_sub(self.render_pos());
                if remaining == 0 {
                    self.finish();
                    continue;
                }
                if remaining <= self.ramp_len as u64 {
                    let gain = self.current_gain();
                    if gain <= 0.0 {
                        self.finish();
                        continue;
                    }
                    // Declick into the end so gain reaches 0 exactly at the end.
                    self.ramp_dir = RampDir::Down;
                    self.ramp_pos = 0;
                    self.ramp_total = remaining as u32;
                    self.ramp_from = gain;
                    self.pending = Some(if self.state == State::Previewing {
                        Pending::EndPreview
                    } else {
                        Pending::Stop
                    });
                }
            }
            let piece = self.piece_len(frames, timeline, preview);
            let (plan, pos) = if self.state == State::Previewing {
                (preview, self.preview_pos)
            } else {
                (timeline, self.position)
            };
            let slice = &mut out[cursor..cursor + piece * 2];
            mixer::render(plan, pos, slice, true);
            self.apply_gain(slice, piece);
            if self.state == State::Previewing {
                self.preview_pos += piece as u64;
            } else {
                self.position += piece as u64;
            }
            if self.ramp_total > 0 {
                self.ramp_pos += piece as u32;
                if self.ramp_pos >= self.ramp_total {
                    self.ramp_total = 0;
                    if let Some(p) = self.pending.take() {
                        self.apply(p);
                    }
                }
            }
            cursor += piece * 2;
        }
    }

    /// Current timeline playhead in samples.
    pub fn position(&self) -> u64 {
        self.position
    }

    /// Current preview playhead in samples.
    pub fn preview_position(&self) -> u64 {
        self.preview_pos
    }

    /// Current transport state.
    pub fn state(&self) -> State {
        self.state
    }

    /// Whether the player renders audio (timeline or preview).
    fn active(&self) -> bool {
        matches!(self.state, State::Playing | State::Previewing)
    }

    /// Where the current source ends: preview length, or timeline/range end.
    fn play_end(&self, timeline: &MixPlan, preview: &MixPlan) -> u64 {
        if self.state == State::Previewing {
            preview.length
        } else {
            self.range_end
                .map_or(timeline.length, |e| e.min(timeline.length))
        }
    }

    /// Position the active source renders from.
    fn render_pos(&self) -> u64 {
        if self.state == State::Previewing {
            self.preview_pos
        } else {
            self.position
        }
    }

    /// Current output gain (1.0 when no ramp is active).
    fn current_gain(&self) -> f32 {
        if self.ramp_total == 0 {
            return 1.0;
        }
        let total = self.ramp_total as f32;
        match self.ramp_dir {
            RampDir::Up => (self.ramp_pos as f32 / total).min(1.0),
            RampDir::Down => self.ramp_from * (self.ramp_total - self.ramp_pos) as f32 / total,
        }
    }

    /// Starts a ramp from zero to full gain over the declick length.
    fn start_ramp_up(&mut self) {
        self.ramp_dir = RampDir::Up;
        self.ramp_pos = 0;
        self.ramp_total = self.ramp_len;
        self.ramp_from = 0.0;
    }

    /// Starts a ramp from `from` down to zero over the declick length.
    fn start_ramp_down(&mut self, from: f32) {
        self.ramp_dir = RampDir::Down;
        self.ramp_pos = 0;
        self.ramp_total = self.ramp_len;
        self.ramp_from = from;
    }

    /// Begins timeline playback, declicking out of any current sound first.
    fn start_play(&mut self, from: u64, range_end: Option<u64>) {
        match self.state {
            State::Stopped | State::Paused | State::Scrubbing => {
                self.scrub_stop = None;
                self.state = State::Playing;
                self.position = from;
                self.return_to = from;
                self.range_end = range_end;
                self.start_ramp_up();
            }
            State::Playing | State::Previewing => {
                self.ramp_to(Pending::StartPlay { from, range_end });
            }
        }
    }

    /// Schedules `pending` behind a ramp-down, or applies it at once when silent.
    fn ramp_to(&mut self, pending: Pending) {
        if self.pending.is_some() {
            self.pending = Some(pending);
            return;
        }
        let gain = self.current_gain();
        if gain <= 0.0 {
            self.apply(pending);
        } else {
            self.start_ramp_down(gain);
            self.pending = Some(pending);
        }
    }

    /// Applies a deferred change now that the gain has reached zero.
    fn apply(&mut self, p: Pending) {
        self.ramp_total = 0;
        match p {
            Pending::Stop => {
                self.state = State::Stopped;
                self.position = self.return_to;
            }
            Pending::Pause { at } => {
                self.state = State::Paused;
                self.position = at;
            }
            Pending::Seek { to } => {
                self.position = to;
                self.return_to = to;
                self.start_ramp_up();
            }
            Pending::StartPlay { from, range_end } => {
                self.state = State::Playing;
                self.position = from;
                self.return_to = from;
                self.range_end = range_end;
                self.start_ramp_up();
            }
            Pending::StartPreview => {
                self.state = State::Previewing;
                self.preview_pos = 0;
                self.start_ramp_up();
            }
            Pending::EndPreview | Pending::StopPreview => {
                self.state = State::Stopped;
                self.preview_pos = 0;
            }
        }
    }

    /// Natural end of the current source: stop and return home.
    fn finish(&mut self) {
        self.ramp_total = 0;
        self.pending = None;
        if self.state == State::Previewing {
            self.state = State::Stopped;
            self.preview_pos = 0;
        } else {
            self.state = State::Stopped;
            self.position = self.return_to;
        }
    }

    /// Frames to render before the next ramp or end boundary.
    fn piece_len(&self, frames: usize, timeline: &MixPlan, preview: &MixPlan) -> usize {
        if self.pending.is_some() {
            return (self.ramp_total - self.ramp_pos).min(frames as u32) as usize;
        }
        let mut piece = frames as u32;
        if self.ramp_total > 0 && self.ramp_dir == RampDir::Up {
            piece = piece.min(self.ramp_total - self.ramp_pos);
        }
        // The declick into the end must start exactly `ramp_len` frames early.
        let remaining = self
            .play_end(timeline, preview)
            .saturating_sub(self.render_pos());
        let bound = remaining.saturating_sub(self.ramp_len as u64);
        piece = piece.min(bound.min(u32::MAX as u64) as u32);
        piece as usize
    }

    /// Scales each rendered frame of `slice` by the ramp gain.
    fn apply_gain(&self, slice: &mut [f32], piece: usize) {
        if self.ramp_total == 0 {
            return;
        }
        let total = self.ramp_total as f32;
        for i in 0..piece {
            let g = match self.ramp_dir {
                RampDir::Up => ((self.ramp_pos + i as u32) as f32 / total).min(1.0),
                RampDir::Down => {
                    let rem = (self.ramp_total - self.ramp_pos - i as u32) as f32;
                    self.ramp_from * (rem / total)
                }
            };
            let o = i * 2;
            slice[o] *= g;
            slice[o + 1] *= g;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::audio::meters::{MeterBank, StereoMeter};
    use crate::audio::mix_plan::{ClipPlan, MixPlan, TrackPlan};
    use crate::audio::player::{Command, Player, State};

    const RATE: u32 = 48_000;
    const RAMP: u32 = RATE / 200;

    /// A one-track plan with one mono constant clip covering the whole length.
    fn const_plan(len: u64, value: f32) -> MixPlan {
        let samples: Arc<[f32]> = Arc::from(vec![value; len as usize].into_boxed_slice());
        MixPlan {
            rate: RATE,
            length: len,
            master_gain: 1.0,
            tracks: vec![TrackPlan {
                audible: true,
                gain_l: 1.0,
                gain_r: 1.0,
                clips: vec![ClipPlan {
                    start: 0,
                    len,
                    src_in: 0,
                    channels: 1,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                    samples: Some(samples),
                }],
            }],
            meters: Arc::new(MeterBank {
                track_ids: Vec::new(),
                tracks: Vec::new(),
            }),
            master: Arc::new(StereoMeter::new()),
        }
    }

    /// Runs `blocks` blocks of `size` frames, appending all output to `sink`.
    fn run(
        player: &mut Player,
        timeline: &MixPlan,
        preview: &MixPlan,
        blocks: usize,
        size: usize,
        sink: &mut Vec<f32>,
    ) {
        for _ in 0..blocks {
            let start = sink.len();
            sink.resize(start + size * 2, f32::NAN);
            player.process(timeline, preview, &mut sink[start..]);
        }
    }

    #[test]
    fn play_advances_position_over_blocks() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 0 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 10, 512, &mut sink);
        assert_eq!(p.position(), 5_120);
        assert_eq!(p.state(), State::Playing);
    }

    #[test]
    fn stop_returns_to_play_start_after_ramp() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 1_000 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 2, 512, &mut sink);
        assert_eq!(p.position(), 2_024);
        p.handle(Command::Stop);
        sink.clear();
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert_eq!(p.position(), 1_000);
        assert_eq!(p.state(), State::Stopped);
        // After the 240-frame ramp the rest of the block is silent.
        for frame in 300..512 {
            assert_eq!(sink[frame * 2], 0.0, "frame {frame} not silent");
            assert_eq!(sink[frame * 2 + 1], 0.0);
        }
    }

    #[test]
    fn pause_keeps_position_and_play_resumes() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 0 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 4, 512, &mut sink);
        p.handle(Command::Pause);
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert_eq!(p.position(), 2_048);
        assert_eq!(p.state(), State::Paused);
        p.handle(Command::Play { from: p.position() });
        assert_eq!(p.state(), State::Playing);
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert_eq!(p.position(), 2_560);
        assert!(sink[400 * 2].abs() > 0.1, "audio continues after resume");
    }

    #[test]
    fn reaching_timeline_length_stops_and_returns() {
        let timeline = const_plan(1_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 0 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 10, 512, &mut sink);
        assert_eq!(p.state(), State::Stopped);
        assert_eq!(p.position(), 0);
        // The block that contains the end is silent after the material ends.
        assert_eq!(sink[9 * 1024 + 2 * 500], 0.0);
    }

    #[test]
    fn play_range_stops_at_end_and_returns_to_start() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::PlayRange {
            start: 1_000,
            end: 5_000,
        });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 8, 512, &mut sink);
        assert_eq!(p.state(), State::Stopped);
        assert_eq!(p.position(), 1_000);
        // Frame 3999 is the last rendered frame (position 4999): declicked near 0.
        let last = sink[3_999 * 2];
        assert!(last.abs() < 0.01, "last frame before range end was {last}");
        // Everything after the range is silence.
        assert_eq!(sink[4_000 * 2], 0.0);
    }

    #[test]
    fn first_frames_ramp_up_from_zero() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 0 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert!(sink[0].abs() < 1e-6, "frame 0 should be silent");
        let before = sink[(RAMP as usize - 1) * 2];
        assert!(before < 0.5, "frame before ramp end not yet full: {before}");
        let full = sink[RAMP as usize * 2];
        assert!(
            (full - 0.5).abs() < 1e-6,
            "frame {RAMP} should be full level: {full}"
        );
        // Monotonic ramp.
        for i in 1..RAMP as usize {
            assert!(sink[i * 2] >= sink[(i - 1) * 2]);
        }
    }

    #[test]
    fn seek_during_play_jumps_and_continues() {
        let timeline = const_plan(100_000, 0.5);
        let preview = MixPlan::empty(RATE);
        let mut p = Player::new(RATE);
        p.handle(Command::Play { from: 0 });
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 2, 512, &mut sink);
        p.handle(Command::Seek(10_000));
        sink.clear();
        run(&mut p, &timeline, &preview, 3, 512, &mut sink);
        assert_eq!(p.state(), State::Playing);
        let pos = p.position();
        assert!(
            pos >= 10_000,
            "position {pos} did not reach the seek target"
        );
        assert!(pos < 10_000 + 3 * 512 + RAMP as u64 + 1);
        assert!(sink[1_400 * 2].abs() > 0.1, "audio continues after seek");
    }

    #[test]
    fn preview_plays_preview_plan_and_stops() {
        let timeline = const_plan(100_000, 0.5);
        let preview_samples: Arc<[f32]> = Arc::from(vec![0.25f32; 2_000].into_boxed_slice());
        let preview = MixPlan::preview(
            1_000,
            2,
            preview_samples,
            RATE,
            0.0,
            Arc::new(StereoMeter::new()),
        );
        let mut p = Player::new(RATE);
        p.handle(Command::Seek(1_234));
        assert_eq!(p.position(), 1_234);
        p.handle(Command::Preview);
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert_eq!(p.state(), State::Previewing);
        assert_eq!(
            p.position(),
            1_234,
            "timeline position untouched by preview"
        );
        assert_eq!(p.preview_position(), 512);
        assert!(
            (sink[400 * 2] - 0.25).abs() < 1e-6,
            "preview material rendered"
        );
        run(&mut p, &timeline, &preview, 9, 512, &mut sink);
        assert_eq!(p.state(), State::Stopped);
        assert_eq!(p.preview_position(), 0);
        assert_eq!(p.position(), 1_234);
    }

    #[test]
    fn play_while_previewing_ends_the_preview_first() {
        let timeline = const_plan(100_000, 0.5);
        let preview_samples: Arc<[f32]> = Arc::from(vec![0.25f32; 2_000].into_boxed_slice());
        let preview = MixPlan::preview(
            1_000,
            2,
            preview_samples,
            RATE,
            0.0,
            Arc::new(StereoMeter::new()),
        );
        let mut p = Player::new(RATE);
        p.handle(Command::Preview);
        let mut sink = Vec::new();
        run(&mut p, &timeline, &preview, 1, 512, &mut sink);
        assert_eq!(p.state(), State::Previewing);
        p.handle(Command::Play { from: 5_000 });
        run(&mut p, &timeline, &preview, 3, 512, &mut sink);
        assert_eq!(p.state(), State::Playing);
        assert!(
            p.position() >= 5_000,
            "timeline playback started after preview"
        );
        // Early frames were still preview (0.25), later frames are timeline (0.5).
        assert!((sink[400 * 2] - 0.25).abs() < 1e-6);
        assert!((sink[1_400 * 2] - 0.5).abs() < 0.01);
    }
}
