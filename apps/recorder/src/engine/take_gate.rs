//! Which of a running capture's output belongs to the take, and what time stamps it gets there. Pure logic.
//!
//! The capture never stops its clock: frame 0 is its start. The take is a list of spans of that clock, one from the
//! take's start and one from each resume, and the time between them is removed. A span that begins on encoders that
//! are already running begins on a forced keyframe: its video is ignored until the keyframe with the forced frame's
//! own timestamp arrives, so a regular keyframe that comes first is skipped.
//!
//! A main video frame belongs to the take by its pts, never by when its packet arrives, because an encoder hands a
//! packet out some time after it took the frame.

use std::ops::Range;

use super::audio::OUT_RATE;

/// The frame length assumed for an audio stream until two of its packets show the real one: AAC's.
const DEFAULT_AUDIO_FRAME: i64 = 1024;

/// What an encoder thread does with one frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct FramePlan {
    /// The frame must be encoded as a keyframe.
    pub(super) force_keyframe: bool,
    /// The frame belongs to the take, so it counts in the take's frame totals.
    pub(super) in_take: bool,
}

/// Where one video stream of a span stands with the keyframe it begins on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keyframe {
    /// No frame has been picked to be forced yet.
    Wanted,
    /// The frame with this pts is being forced, and the take waits for its packet.
    Forced(i64),
    /// Packets of the span are taken.
    Open,
}

impl Keyframe {
    /// Decides one frame of the span and picks it as the forced one if the span still wants one.
    fn force(&mut self, pts: i64) -> bool {
        let wanted = *self == Keyframe::Wanted;
        if wanted {
            *self = Keyframe::Forced(pts);
        }
        wanted
    }

    /// Asks for another keyframe when the forced frame never reached the encoder.
    fn failed(&mut self, pts: i64) {
        if *self == Keyframe::Forced(pts) {
            *self = Keyframe::Wanted;
        }
    }

    /// Whether the take accepts this packet; the forced keyframe opens the span.
    fn accept(&mut self, pts: i64, key: bool) -> bool {
        if key && *self == Keyframe::Forced(pts) {
            *self = Keyframe::Open;
        }
        *self == Keyframe::Open
    }
}

/// One unbroken stretch of the take, in main video frames of the capture clock.
#[derive(Debug, Clone, Copy)]
struct Span {
    start: u64,
    /// The first frame no longer in the span; None while it runs.
    end: Option<u64>,
    /// The take's length in frames before this span.
    before: u64,
    video: Keyframe,
    camera: Keyframe,
    /// Camera ticks this span is moved later by, so its stamps stay after the ones already written.
    camera_shift: i64,
}

impl Span {
    fn contains(&self, frame: u64) -> bool {
        frame >= self.start && self.end.is_none_or(|end| frame < end)
    }
}

/// One audio stream's place on its own packet grid.
#[derive(Debug, Clone, Copy, Default)]
struct AudioSlot {
    /// The capture pts of the last packet seen, taken or not.
    last_pts: Option<i64>,
    /// The stream's frame length in samples, once two packets have shown it.
    frame: Option<i64>,
    /// The take pts the next taken packet follows on at.
    next_out: Option<i64>,
}

/// The take's spans on the capture clock.
#[derive(Debug)]
pub(super) struct TakeGate {
    /// The QPC value of the capture's frame 0.
    start_qpc: i64,
    qpc_freq: i64,
    /// The main video's frames per second.
    fps: u32,
    /// The camera's frame rate as (numerator, denominator); None without a camera.
    camera_rate: Option<(u32, u32)>,
    spans: Vec<Span>,
    /// The first main video frame the video thread has not decided yet.
    next_video: u64,
    audio: Vec<AudioSlot>,
    /// The take pts of the last camera packet taken.
    last_camera: Option<i64>,
    /// How far each stream's output has got on the capture clock, taken or not: the first video frame, camera tick
    /// and, per track, PCM sample that has not arrived yet.
    video_reach: u64,
    camera_reach: u64,
    pcm_reach: Vec<u64>,
}

impl TakeGate {
    /// A gate with no take on a capture whose frame 0 is at `start_qpc`.
    pub(super) fn new(start_qpc: i64, qpc_freq: i64, fps: u32, camera_rate: Option<(u32, u32)>) -> Self {
        Self {
            start_qpc,
            qpc_freq: qpc_freq.max(1),
            fps: fps.max(1),
            camera_rate,
            spans: Vec::new(),
            next_video: 0,
            audio: Vec::new(),
            last_camera: None,
            video_reach: 0,
            camera_reach: 0,
            pcm_reach: Vec::new(),
        }
    }

    /// Forgets the take, so the next one starts fresh. What is known of the capture's streams stays.
    pub(super) fn end_take(&mut self) {
        self.spans.clear();
        self.last_camera = None;
        for slot in &mut self.audio {
            slot.next_out = None;
        }
    }

    /// Whether every stream's output has reached the end of the paused take, so nothing the take wants is still in an
    /// encoder: the main video, the camera while `camera` says it is running, `audio` streams and the PCM of `tracks`.
    pub(super) fn drained(&self, camera: bool, audio: usize, tracks: usize) -> bool {
        // A take with no span holds nothing to wait for.
        let Some(span) = self.spans.last() else { return true };
        let Some(end) = span.end else { return false };
        let sample = self.sample(end);
        let audio_reach = |slot: usize| {
            let state = self.audio.get(slot)?;
            state.last_pts.map(|pts| pts + state.frame.unwrap_or(DEFAULT_AUDIO_FRAME))
        };
        self.video_reach >= end
            && (!camera || self.camera_reach >= self.camera_tick(end))
            && (0..audio).all(|slot| audio_reach(slot).is_some_and(|reach| reach >= sample))
            && (0..tracks).all(|track| self.pcm_reach.get(track).is_some_and(|reach| *reach as i64 >= sample))
    }

    /// Notes that the PCM of `track` has arrived up to the capture sample `end`.
    pub(super) fn pcm_seen(&mut self, track: usize, end: u64) {
        if self.pcm_reach.len() <= track {
            self.pcm_reach.resize(track + 1, 0);
        }
        self.pcm_reach[track] = self.pcm_reach[track].max(end);
    }

    /// Starts the take together with the capture: it holds everything from frame 0, and every stream's first packet
    /// is a keyframe anyway.
    pub(super) fn begin_with_capture(&mut self) {
        self.spans = vec![Span { start: 0, end: None, before: 0, video: Keyframe::Open, camera: Keyframe::Open, camera_shift: 0 }];
    }

    /// Starts or resumes the take on running encoders, pressed at `press_qpc`. Returns the frame it begins on: the
    /// first one at or after the press, or the next one if the video thread already has that one. Does nothing but
    /// return the running span's start when the take already runs.
    pub(super) fn open(&mut self, press_qpc: i64) -> u64 {
        if let Some(span) = self.spans.last().filter(|span| span.end.is_none()) {
            return span.start;
        }
        let start = self.frame_at(press_qpc).max(self.next_video);
        let before = self.length();
        self.spans.push(Span { start, end: None, before, video: Keyframe::Wanted, camera: Keyframe::Wanted, camera_shift: 0 });
        start
    }

    /// Pauses the take, pressed at `press_qpc`; frames the video thread already has stay in it.
    pub(super) fn close(&mut self, press_qpc: i64) {
        let end = self.frame_at(press_qpc).max(self.next_video);
        let Some(span) = self.spans.last_mut().filter(|span| span.end.is_none()) else {
            return;
        };
        if end > span.start {
            span.end = Some(end);
        } else {
            // Paused again before the span held a single frame.
            self.spans.pop();
        }
    }

    /// The QPC time at which a frame of the capture clock is due.
    pub(super) fn frame_qpc(&self, frame: u64) -> i64 {
        let offset = i128::from(frame) * i128::from(self.qpc_freq) / i128::from(self.fps);
        (i128::from(self.start_qpc) + offset) as i64
    }

    /// The first frame due at or after a QPC time.
    fn frame_at(&self, qpc: i64) -> u64 {
        let elapsed = i128::from(qpc.saturating_sub(self.start_qpc).max(0));
        let freq = i128::from(self.qpc_freq);
        ((elapsed * i128::from(self.fps) + freq - 1) / freq) as u64
    }

    /// The length of the closed spans in frames.
    fn length(&self) -> u64 {
        self.spans.iter().filter_map(|span| span.end.map(|end| end - span.start)).sum()
    }

    /// Where a moment of the capture clock, in 100 ns units since its start, falls on the take clock in ms. A moment
    /// outside the take counts as the last moment of the take before it.
    pub(super) fn take_ms(&self, capture_100ns: i64) -> u64 {
        let now = capture_100ns.max(0) as u64 / 10_000;
        let ms = |frame: u64| frame * 1000 / u64::from(self.fps);
        self.spans
            .iter()
            .map(|span| {
                let length = span.end.map_or(u64::MAX, |end| ms(end) - ms(span.start));
                now.saturating_sub(ms(span.start)).min(length)
            })
            .sum()
    }

    /// Decides the main video frame `frame`, just before it is encoded.
    pub(super) fn video_frame(&mut self, frame: u64) -> FramePlan {
        self.next_video = self.next_video.max(frame + 1);
        match self.spans.iter_mut().find(|span| span.contains(frame)) {
            Some(span) => FramePlan { force_keyframe: span.video.force(frame as i64), in_take: true },
            None => FramePlan::default(),
        }
    }

    /// Says that a frame was not encoded, so a keyframe forced on it must go on a later one.
    pub(super) fn video_failed(&mut self, frame: u64) {
        for span in &mut self.spans {
            span.video.failed(frame as i64);
        }
    }

    /// How far a main video packet's stamps move back in the take; None when the take does not want the packet.
    pub(super) fn video_packet(&mut self, pts: i64, key: bool) -> Option<i64> {
        let frame = u64::try_from(pts).ok()?;
        self.video_reach = self.video_reach.max(frame + 1);
        let span = self.spans.iter_mut().find(|span| span.contains(frame))?;
        span.video.accept(pts, key).then_some(span.start as i64 - span.before as i64)
    }

    /// The camera tick nearest to a main video frame.
    fn camera_tick(&self, frame: u64) -> u64 {
        let (num, den) = self.camera_rate.unwrap_or((self.fps, 1));
        let ticks = u128::from(frame) * u128::from(num);
        let per = u128::from(self.fps) * u128::from(den.max(1));
        ((ticks + per / 2) / per) as u64
    }

    /// The span holding a camera tick, with how far the tick's stamps move back in the take, before any shift.
    fn camera_span(&self, tick: u64) -> Option<(usize, i64)> {
        let mut before = 0;
        for (index, span) in self.spans.iter().enumerate() {
            let start = self.camera_tick(span.start);
            let end = span.end.map(|end| self.camera_tick(end));
            if tick >= start && end.is_none_or(|end| tick < end) {
                return Some((index, start as i64 - before as i64));
            }
            before += end.map_or(0, |end| end - start);
        }
        None
    }

    /// Decides the camera tick `tick`, just before it is encoded.
    pub(super) fn camera_frame(&mut self, tick: u64) -> FramePlan {
        match self.camera_span(tick) {
            Some((index, _)) => FramePlan { force_keyframe: self.spans[index].camera.force(tick as i64), in_take: true },
            None => FramePlan::default(),
        }
    }

    /// Says that a camera tick was not encoded, so a keyframe forced on it must go on a later one.
    pub(super) fn camera_failed(&mut self, tick: u64) {
        for span in &mut self.spans {
            span.camera.failed(tick as i64);
        }
    }

    /// How far a camera packet's stamps move back in the take; None when the take does not want the packet.
    pub(super) fn camera_packet(&mut self, pts: i64, key: bool) -> Option<i64> {
        let tick = u64::try_from(pts).ok()?;
        self.camera_reach = self.camera_reach.max(tick + 1);
        let (index, offset) = self.camera_span(tick)?;
        let last = self.last_camera;
        let span = &mut self.spans[index];
        let opening = span.camera != Keyframe::Open;
        if !span.camera.accept(pts, key) {
            return None;
        }
        // A camera whose sync offset puts it ahead of the clock may have written ticks past the pause.
        if opening && let Some(last) = last {
            span.camera_shift = (last + 1 - (pts - offset)).max(0);
        }
        let offset = offset - span.camera_shift;
        self.last_camera = Some(pts - offset);
        Some(offset)
    }

    /// The 48 kHz sample on the capture clock where a main video frame begins.
    fn sample(&self, frame: u64) -> i64 {
        (u128::from(frame) * u128::from(OUT_RATE) / u128::from(self.fps)) as i64
    }

    /// The take pts and frame length of the packet of audio stream `slot` that has this capture pts; None when the
    /// take does not want it.
    ///
    /// The packets of a stream cannot be cut, and a span rarely begins on a packet border. So the taken packets stay
    /// on one unbroken grid, never more than half a frame from their true time: after a resume, a packet that would
    /// overlap the one before by more than that is left out.
    pub(super) fn audio_packet(&mut self, slot: usize, pts: i64) -> Option<(i64, i64)> {
        if self.audio.len() <= slot {
            self.audio.resize(slot + 1, AudioSlot::default());
        }
        let mut state = self.audio[slot];
        if let Some(last) = state.last_pts.filter(|last| pts > *last) {
            // The shortest step seen is the frame length.
            state.frame = Some(state.frame.map_or(pts - last, |frame| frame.min(pts - last)));
        }
        state.last_pts = Some(pts);
        let frame = state.frame.unwrap_or(DEFAULT_AUDIO_FRAME);
        let half = frame / 2;
        let taken = self.spans.iter().enumerate().find_map(|(index, span)| {
            // The take's first span also takes the packet before its start, which primes the decoder.
            let lead = if index == 0 { frame + half } else { half };
            let start = self.sample(span.start);
            let inside = pts >= start - lead && span.end.is_none_or(|end| pts < self.sample(end));
            inside.then(|| pts - (start - self.sample(span.before)))
        });
        let out = taken.and_then(|ideal| match state.next_out {
            None => Some(ideal),
            Some(next) if ideal - next < -half => None,
            Some(next) => Some(if ideal - next <= half { next } else { ideal }),
        });
        if let Some(out) = out {
            state.next_out = Some(out + frame);
        }
        self.audio[slot] = state;
        out.map(|out| (out, frame))
    }

    /// The parts of a PCM block that belong to the take. The block holds `frames` samples per channel from the
    /// capture sample `start`; each part is its range in the block and the take sample it begins at.
    pub(super) fn pcm(&self, start: u64, frames: usize) -> Vec<(Range<usize>, u64)> {
        let block_start = start as i64;
        let block_end = block_start + frames as i64;
        self.spans
            .iter()
            .filter_map(|span| {
                let from = self.sample(span.start);
                let low = block_start.max(from);
                let high = span.end.map_or(block_end, |end| block_end.min(self.sample(end)));
                let take_start = low - from + self.sample(span.before);
                (low < high).then(|| ((low - block_start) as usize..(high - block_start) as usize, take_start as u64))
            })
            .collect()
    }
}
