//! The per-track audio pipeline: any device's packets in, 48 kHz stereo locked
//! to the recording timeline out. Pure logic, no WASAPI, no files, no egui.
//!
//! See the recorder spec: Audio capture and tracks (Per-track controls, Format)
//! and Sync and reliability rules (Audio, Pause). [`AppLink`] is the attach and
//! detach state of an app track (per-app audio spec, Tracks).

use std::sync::mpsc::Receiver;

use capture::{AppTarget, AudioApp, AudioPacket, CaptureError, LoopbackMode};
use rubato::audioadapter_buffers::direct::{InterleavedSlice, SequentialSliceOfVecs};
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler, Resizable};

/// Proportional gain of the drift controller, in ratio per frame of error.
const DRIFT_KP: f64 = 2.0e-4;
/// Integral gain of the drift controller, in ratio per accumulated frame of error.
const DRIFT_KI: f64 = 2.0e-6;
/// Largest relative ratio correction the controller may apply, plus or minus 0.2 percent.
const ADJUST_LIMIT: f64 = 0.002;
/// Relative ratio bound passed to the resampler, leaving headroom over the controller.
const RATIO_HEADROOM: f64 = 1.01;
/// Errors larger than this fraction of a second force a resync instead of a trim.
const RESYNC_LIMIT_S: f64 = 0.05;
/// Interpolation quality of the resampler.
const POLY_DEGREE: PolynomialDegree = PolynomialDegree::Cubic;

/// The timeline index a packet sample stamped qpc_100ns lands on, in output frames.
fn expected_index(qpc_100ns: i64, offset_100ns: i64, start_100ns: i64, out_rate: u32) -> f64 {
    let raw = (i128::from(qpc_100ns) + i128::from(offset_100ns) - i128::from(start_100ns)) * i128::from(out_rate) / 10_000_000;
    raw.max(0) as f64
}

/// Converts, resamples and time-locks one audio device onto the recording timeline.
///
/// Input packets carry QPC stamps; their samples are placed at
/// `(qpc_100ns + offset_100ns - start_100ns) * out_rate / 10_000_000` on the
/// output timeline. A gentle proportional-integral controller trims the resampler
/// ratio so a device clock a fraction of a percent off still stays locked.
#[allow(dead_code)]
pub struct TrackPipeline {
    /// The output sample rate, always 48000 in the recorder.
    out_rate: u32,
    /// The QPC value that recording time 0 maps to, shifted forward by pauses.
    start_100ns: i64,
    /// The per-track sync offset, in 100 nanosecond units.
    offset_100ns: i64,
    /// Linear output gain applied when frames are taken.
    gain: f32,
    /// Whether the track output is muted.
    muted: bool,
    /// The QPC value when pause was called, if currently paused.
    paused_at: Option<i64>,
    /// The resampler, created on the first packet once the input rate is known.
    resampler: Option<Async<f32>>,
    /// The input rate the resampler was built for.
    in_rate: u32,
    /// The nominal output over input ratio for the current device.
    base_ratio: f64,
    /// Fixed input chunk size of the resampler, about 10 ms of input frames.
    chunk: usize,
    /// The resampler's delay in output frames, accounted for when placing audio.
    delay: usize,
    /// Interleaved stereo input frames staged until they fill one resampler chunk.
    staged: Vec<f32>,
    /// Interleaved stereo output frames ready to be taken.
    out: Vec<f32>,
    /// Total output frames produced so far, silence fill included.
    out_frames: u64,
    /// The current relative ratio correction of the drift controller.
    adj: f64,
    /// The accumulated error of the drift controller.
    integ: f64,
    /// Per channel scratch buffers the resampler writes each chunk into.
    out_data: Vec<Vec<f32>>,
    /// The output peak since the last read of the meter.
    peak: f32,
    /// Whether output has clipped since the last read of the meter.
    clip: bool,
    /// The highest output peak since the pipeline was created; reading the meter does not reset it.
    peak_hold: f32,
}

#[allow(dead_code)]
impl TrackPipeline {
    /// Creates a pipeline whose timeline starts at start_100ns and outputs at out_rate.
    pub fn new(start_100ns: i64, out_rate: u32) -> Self {
        Self {
            out_rate,
            start_100ns,
            offset_100ns: 0,
            gain: 1.0,
            muted: false,
            paused_at: None,
            resampler: None,
            in_rate: 0,
            base_ratio: 1.0,
            chunk: 480,
            delay: 0,
            staged: Vec::new(),
            out: Vec::new(),
            out_frames: 0,
            adj: 0.0,
            integ: 0.0,
            out_data: Vec::new(),
            peak: 0.0,
            clip: false,
            peak_hold: 0.0,
        }
    }

    /// Places one device packet on the timeline; packets pushed inside a pause are discarded.
    pub fn push(&mut self, p: &AudioPacket) {
        if self.paused_at.is_some() || p.sample_rate == 0 || p.frames == 0 {
            return;
        }
        // A track that switches sources may change rate, which needs a new resampler.
        let first = self.resampler.is_none() || p.sample_rate != self.in_rate;
        if first && !self.init_resampler(p.sample_rate) {
            return;
        }
        self.stage_as_stereo(p);
        let expected = expected_index(p.qpc_100ns, self.offset_100ns, self.start_100ns, self.out_rate);
        let limit = f64::from(self.out_rate) * RESYNC_LIMIT_S;
        // The packet's first sample comes out of the resampler delay frames from now.
        let err = expected - (self.out_frames + self.delay as u64) as f64;
        if first || p.discontinuity || err.abs() > limit {
            self.adj = 0.0;
            self.integ = 0.0;
            if err > 0.0 {
                // Ahead of us: pad silence up to the expected index.
                self.pad_silence(err.round() as u64);
            } else if err < 0.0 {
                // Behind us: drop input so the stream catches up.
                let ratio = self.base_ratio * (1.0 + self.adj);
                self.drop_staged(((-err / ratio).round()) as usize);
            }
        } else {
            let clamp = ADJUST_LIMIT / DRIFT_KI;
            self.integ = (self.integ + err).clamp(-clamp, clamp);
            self.adj = (DRIFT_KP * err + DRIFT_KI * self.integ).clamp(-ADJUST_LIMIT, ADJUST_LIMIT);
        }
        if let Some(res) = self.resampler.as_mut() {
            let _ = res.set_resample_ratio(self.base_ratio * (1.0 + self.adj), false);
        }
        self.process_staged();
    }

    /// Pads silence so the output reaches at least index frames on the timeline.
    pub fn fill_silence_to(&mut self, index: u64) {
        if self.out_frames < index {
            self.pad_silence(index - self.out_frames);
        }
    }

    /// Stops the timeline at now_100ns; packets pushed while paused are discarded.
    pub fn pause(&mut self, now_100ns: i64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now_100ns);
        }
    }

    /// Resumes after a pause, shifting the timeline by the paused time so there is no gap.
    pub fn resume(&mut self, now_100ns: i64) {
        if let Some(at) = self.paused_at.take() {
            self.start_100ns += now_100ns - at;
        }
    }

    /// Sets the per-track output gain in dB.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain = 10f32.powf(gain_db / 20.0);
    }

    /// Mutes or unmutes the track output.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Sets the sync offset in ms, clamped to -500..500; it shifts later packets.
    pub fn set_offset_ms(&mut self, offset_ms: f32) {
        self.offset_100ns = (offset_ms.clamp(-500.0, 500.0) * 10_000.0) as i64;
    }

    /// Drains up to max_frames of interleaved stereo output with gain and mute applied.
    pub fn take(&mut self, max_frames: usize) -> Vec<f32> {
        let count = max_frames.min(self.out.len() / 2) * 2;
        let mut taken = Vec::with_capacity(count);
        for value in self.out.drain(..count) {
            let sample = if self.muted { 0.0 } else { value * self.gain };
            let magnitude = sample.abs();
            if magnitude > self.peak {
                self.peak = magnitude;
            }
            if magnitude > self.peak_hold {
                self.peak_hold = magnitude;
            }
            if sample >= 1.0 || sample <= -1.0 {
                self.clip = true;
            }
            taken.push(sample);
        }
        taken
    }

    /// Total output frames produced so far, silence fill included.
    pub fn produced_frames(&self) -> u64 {
        self.out_frames
    }

    /// The linear output peak since the last call, which resets it.
    pub fn peak_since_last_read(&mut self) -> f32 {
        let peak = self.peak;
        self.peak = 0.0;
        peak
    }

    /// The highest output peak since the pipeline was created, in dBFS; -100.0 for silence.
    pub fn peak_hold_dbfs(&self) -> f32 {
        if self.peak_hold > 0.0 { 20.0 * self.peak_hold.log10() } else { -100.0 }
    }

    /// True if any output sample reached 1.0 or more since the last call, which resets it.
    pub fn clipped(&mut self) -> bool {
        let clip = self.clip;
        self.clip = false;
        clip
    }

    /// Builds the resampler for a device at in_rate; false if the rate is unusable.
    fn init_resampler(&mut self, in_rate: u32) -> bool {
        let base = f64::from(self.out_rate) / f64::from(in_rate);
        if !base.is_finite() || base <= 0.0 {
            return false;
        }
        // About 10 ms of input frames per resampler chunk.
        let chunk = (in_rate / 100).clamp(16, 8192) as usize;
        match Async::new_poly(base, RATIO_HEADROOM, POLY_DEGREE, chunk, 2, FixedAsync::Input) {
            Ok(res) => {
                self.delay = res.output_delay();
                let out_max = res.output_frames_max();
                self.out_data = vec![vec![0.0; out_max]; 2];
                self.resampler = Some(res);
                self.in_rate = in_rate;
                self.base_ratio = base;
                self.chunk = chunk;
                true
            }
            Err(_) => false,
        }
    }

    /// Converts a packet to stereo and stages it; mono doubles, more than 2 keeps the first two.
    fn stage_as_stereo(&mut self, p: &AudioPacket) {
        let channels = usize::from(p.channels.max(1));
        let frames = (p.samples.len() / channels).min(p.frames as usize);
        if channels == 1 {
            for value in &p.samples[..frames] {
                self.staged.push(*value);
                self.staged.push(*value);
            }
        } else {
            for frame in 0..frames {
                self.staged.push(p.samples[frame * channels]);
                self.staged.push(p.samples[frame * channels + 1]);
            }
        }
    }

    /// Appends n frames of silence to the output.
    fn pad_silence(&mut self, frames: u64) {
        for _ in 0..frames {
            self.out.push(0.0);
            self.out.push(0.0);
        }
        self.out_frames += frames;
    }

    /// Removes up to frames of staged input from the front.
    fn drop_staged(&mut self, frames: usize) {
        let count = frames.min(self.staged.len() / 2) * 2;
        self.staged.drain(..count);
    }

    /// Feeds all staged input through the resampler into the output.
    ///
    /// The chunk size shrinks to the staged amount, so nothing waits in staging
    /// and the output count stays locked to the timeline.
    fn process_staged(&mut self) {
        while self.staged.len() >= 2 {
            let frames = (self.staged.len() / 2).min(self.chunk);
            let mut produced = 0usize;
            {
                let res = match self.resampler.as_mut() {
                    Some(res) => res,
                    None => break,
                };
                let _ = res.set_chunk_size(frames);
                let out_len = self.out_data[0].len();
                let window = &self.staged[..frames * 2];
                let input = InterleavedSlice::new(window, 2, frames);
                let output = SequentialSliceOfVecs::new_mut(&mut self.out_data, 2, out_len);
                if let (Ok(input), Ok(mut output)) = (input, output)
                    && let Ok((_, count)) = res.process_into_buffer(&input, &mut output, None)
                {
                    produced = count;
                }
            }
            for (left, right) in self.out_data[0].iter().zip(&self.out_data[1]).take(produced) {
                self.out.push(*left);
                self.out.push(*right);
            }
            self.out_frames += produced as u64;
            // Always drain the chunk, even on error, so the stream keeps moving.
            self.staged.drain(..frames * 2);
        }
    }
}

/// A running capture as an app link uses it: `capture::AppLoopback`, or for a desktop-except track also the
/// default output's `capture::AudioCapture`.
pub(super) trait AppSource {
    /// The packet queue.
    fn receiver(&self) -> &Receiver<AudioPacket>;
    /// Whether the capture ended on its own.
    fn lost(&self) -> bool;
    /// The app's top-level processes the capture was started for; empty for the desktop capture.
    fn instance_pids(&self) -> &[u32];
    /// Packets produced so far.
    fn packets(&self) -> u64;
    /// Packets dropped because the queue was full.
    fn overflows(&self) -> u64;
}

/// What app links ask of the system.
pub(super) trait AppSystem {
    /// Starts a capture of the app with the given executable name, or of everything but it.
    fn start_app(&mut self, exe: &str, mode: LoopbackMode) -> Result<Box<dyn AppSource>, CaptureError>;
    /// Starts the loopback of the default output device.
    fn start_desktop(&mut self) -> Result<Box<dyn AppSource>, CaptureError>;
    /// The app's top-level processes now, found without starting a capture; empty when they cannot be read.
    fn top_level_pids(&mut self, exe: &str) -> Vec<u32>;
}

/// A change of an app track that is worth a mark; a restart for new processes is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AppChange {
    /// The track has a capture again.
    Attached,
    /// The track lost its capture and records silence.
    Detached,
}

/// Every process of `exe` in a listing, or None when the app is not running.
pub(super) fn app_pids<'a>(apps: &'a [AudioApp], exe: &str) -> Option<&'a [u32]> {
    apps.iter().find(|app| app.exe == exe).map(|app| app.pids.as_slice()).filter(|pids| !pids.is_empty())
}

/// One app track's capture: attached while the app runs, detached otherwise.
pub(super) struct AppLink {
    /// The executable's file name in lowercase.
    exe: String,
    /// The running capture; None while detached.
    source: Option<Box<dyn AppSource>>,
    /// Every process of the app when the capture's instances were last checked.
    seen_pids: Vec<u32>,
}

impl AppLink {
    /// A detached link to the app named by `exe`.
    pub(super) fn new(exe: &str) -> Self {
        Self { exe: AppTarget::new(exe).exe().to_owned(), source: None, seen_pids: Vec::new() }
    }

    /// The executable's file name in lowercase.
    pub(super) fn exe(&self) -> &str {
        &self.exe
    }

    /// The running capture, if attached.
    pub(super) fn source(&self) -> Option<&dyn AppSource> {
        self.source.as_deref()
    }

    /// One poll step; `running` holds every process of the app, or None when it is not running.
    ///
    /// A capture covers only the top-level processes it started with, so it is restarted when those change.
    pub(super) fn poll(&mut self, running: Option<&[u32]>, system: &mut dyn AppSystem) -> Option<AppChange> {
        let Some(pids) = running else {
            return self.source.take().map(|_| AppChange::Detached);
        };
        let Some(source) = &self.source else {
            return match system.start_app(&self.exe, LoopbackMode::Include) {
                Ok(source) => {
                    self.source = Some(source);
                    self.seen_pids = pids.to_vec();
                    Some(AppChange::Attached)
                }
                Err(err) => {
                    log::debug!("{} is running but cannot be captured yet: {err}", self.exe);
                    None
                }
            };
        };
        if source.lost() {
            self.source = None;
            return Some(AppChange::Detached);
        }
        if let Some(next) = restart_if_changed(&self.exe, LoopbackMode::Include, source.as_ref(), &mut self.seen_pids, pids, system) {
            self.source = Some(next);
        }
        None
    }
}

/// A new capture to replace `source`, when the app's top-level processes are no longer the ones it was started for.
///
/// A listing names every process, children included, so a change in `pids` only means the top-level
/// processes are worth reading again. On a failure `seen_pids` stays, and the next poll tries again.
fn restart_if_changed(
    exe: &str,
    mode: LoopbackMode,
    source: &dyn AppSource,
    seen_pids: &mut Vec<u32>,
    pids: &[u32],
    system: &mut dyn AppSystem,
) -> Option<Box<dyn AppSource>> {
    if pids == seen_pids.as_slice() {
        return None;
    }
    let now = system.top_level_pids(exe);
    if now.is_empty() {
        return None;
    }
    if sorted(&now) == sorted(source.instance_pids()) {
        *seen_pids = pids.to_vec();
        return None;
    }
    match system.start_app(exe, mode) {
        Ok(next) => {
            log::info!("{exe} has a different set of processes, restarting its capture");
            *seen_pids = pids.to_vec();
            Some(next)
        }
        // The running capture stays.
        Err(err) => {
            log::debug!("{exe} changed its processes but cannot be captured again yet: {err}");
            None
        }
    }
}

/// How long a switch waits for the new source's first packet; a silent source may send none. 200 ms.
const SWITCH_TIMEOUT_100NS: i64 = 2_000_000;

/// One of the two captures a desktop-except track switches between.
struct Feed {
    /// The running capture.
    source: Box<dyn AppSource>,
    /// True for everything but the app, false for the default output's loopback.
    excluding: bool,
}

/// Where the audio taken so far ends, so the next source joins it without a gap or an overlap.
#[derive(Default)]
struct Seam {
    /// The QPC time the last sample given to the track ends at, in 100 ns units.
    covered_100ns: i64,
    /// Set after a switch until the new source gives its first packet.
    joining: bool,
}

impl Seam {
    /// Gives a packet to the track. Right after a switch, packets the old source already covered are
    /// skipped, and the first one used is marked so the pipeline pads or trims it to its exact QPC place.
    fn give(&mut self, mut packet: AudioPacket, push: &mut dyn FnMut(&AudioPacket)) {
        let length = i64::from(packet.frames) * 10_000_000 / i64::from(packet.sample_rate.max(1));
        let end = packet.qpc_100ns + length;
        if self.joining {
            if end <= self.covered_100ns {
                return;
            }
            self.joining = false;
            packet.discontinuity = true;
        }
        self.covered_100ns = self.covered_100ns.max(end);
        push(&packet);
    }
}

/// A desktop-except track: the default output's loopback while the app is not running, and
/// everything but the app while it is. "Attached" means the app is being left out.
pub(super) struct ExceptLink {
    /// The executable's file name in lowercase.
    exe: String,
    /// The capture that feeds the track.
    current: Option<Feed>,
    /// The capture that takes over at its first packet.
    next: Option<Feed>,
    /// When the switch to `next` began being waited on.
    waiting_since: Option<i64>,
    /// Every process of the app when the capture's instances were last checked.
    seen_pids: Vec<u32>,
    /// Whether the last change reported was that the app is left out.
    excluding: bool,
    /// Where the audio taken so far ends.
    seam: Seam,
}

impl ExceptLink {
    /// A link with no capture yet; the first poll starts one.
    pub(super) fn new(exe: &str) -> Self {
        Self {
            exe: AppTarget::new(exe).exe().to_owned(),
            current: None,
            next: None,
            waiting_since: None,
            seen_pids: Vec::new(),
            excluding: false,
            seam: Seam::default(),
        }
    }

    /// One poll step, as [`AppLink::poll`].
    pub(super) fn poll(&mut self, running: Option<&[u32]>, system: &mut dyn AppSystem) -> Option<AppChange> {
        let Some(pids) = running else {
            self.want_desktop(system);
            return self.report(false);
        };
        if let Some(feed) = self.next.as_ref().or(self.current.as_ref()).filter(|feed| feed.excluding) {
            if let Some(source) = restart_if_changed(&self.exe, LoopbackMode::Exclude, feed.source.as_ref(), &mut self.seen_pids, pids, system) {
                self.switch_to(Feed { source, excluding: true });
            }
            return None;
        }
        match system.start_app(&self.exe, LoopbackMode::Exclude) {
            Ok(source) => {
                self.seen_pids = pids.to_vec();
                self.switch_to(Feed { source, excluding: true });
                self.report(true)
            }
            // The desktop capture, if there is one, stays until the app can be left out.
            Err(err) => {
                log::debug!("{} is running but cannot be excluded yet: {err}", self.exe);
                None
            }
        }
    }

    /// Gives every queued packet to the track, completing a switch at the new source's first packet,
    /// and replaces a capture that ended on its own.
    pub(super) fn drain(&mut self, now_100ns: i64, system: &mut dyn AppSystem, push: &mut dyn FnMut(&AudioPacket)) -> Option<AppChange> {
        self.hand_over(now_100ns, push);
        if let Some(feed) = &self.current {
            while let Ok(packet) = feed.source.receiver().try_recv() {
                self.seam.give(packet, push);
            }
        }
        self.recover(system)
    }

    /// Moves to the next source once it has a packet, or once waiting any longer is no use.
    fn hand_over(&mut self, now_100ns: i64, push: &mut dyn FnMut(&AudioPacket)) {
        let Some(next) = &self.next else {
            return;
        };
        let since = *self.waiting_since.get_or_insert(now_100ns);
        let first = next.source.receiver().try_recv().ok();
        let old_ended = self.current.as_ref().is_none_or(|feed| feed.source.lost());
        if first.is_none() && !old_ended && !next.source.lost() && now_100ns - since < SWITCH_TIMEOUT_100NS {
            return;
        }
        let boundary = first.as_ref().map_or(i64::MAX, |packet| packet.qpc_100ns);
        if let Some(old) = self.current.take() {
            while let Ok(packet) = old.source.receiver().try_recv() {
                if packet.qpc_100ns < boundary {
                    self.seam.give(packet, push);
                }
            }
            // Dropping the old feed here stops its capture.
        }
        self.current = self.next.take();
        self.waiting_since = None;
        self.seam.joining = true;
        if let Some(packet) = first {
            self.seam.give(packet, push);
        }
    }

    /// Drops a capture that ended. When the app has closed, the desktop capture starts at once;
    /// while it still runs, the next poll excludes it again, and the desktop is never captured with it.
    fn recover(&mut self, system: &mut dyn AppSystem) -> Option<AppChange> {
        if self.next.is_some() || !self.current.as_ref().is_some_and(|feed| feed.source.lost()) {
            return None;
        }
        let was_excluding = self.current.take().is_some_and(|feed| feed.excluding);
        if !was_excluding || !system.top_level_pids(&self.exe).is_empty() {
            return None;
        }
        self.want_desktop(system);
        self.report(false)
    }

    /// Makes the desktop capture the one that feeds the track, or is about to.
    fn want_desktop(&mut self, system: &mut dyn AppSystem) {
        if self.current.as_ref().is_some_and(|feed| !feed.excluding && !feed.source.lost()) {
            // A switch away that has not happened yet is called off.
            self.next = None;
            self.waiting_since = None;
            return;
        }
        if self.next.as_ref().is_some_and(|feed| !feed.excluding) {
            return;
        }
        match system.start_desktop() {
            Ok(source) => self.switch_to(Feed { source, excluding: false }),
            Err(err) => log::debug!("the default output cannot be captured yet: {err}"),
        }
    }

    /// Takes a new capture: at once when nothing feeds the track, else at its first packet.
    fn switch_to(&mut self, feed: Feed) {
        self.waiting_since = None;
        if self.current.is_some() {
            self.next = Some(feed);
        } else {
            self.current = Some(feed);
            self.seam.joining = true;
        }
    }

    /// The change to report, if leaving the app out has started or stopped.
    fn report(&mut self, excluding: bool) -> Option<AppChange> {
        if self.excluding == excluding {
            return None;
        }
        self.excluding = excluding;
        Some(if excluding { AppChange::Attached } else { AppChange::Detached })
    }
}

/// The link of an app track or of a desktop-except track.
pub(super) enum TrackLink {
    /// One app's sound.
    App(AppLink),
    /// Everything but one app.
    Except(ExceptLink),
}

impl TrackLink {
    /// A link with no capture yet.
    pub(super) fn new(exe: &str, except: bool) -> Self {
        if except { Self::Except(ExceptLink::new(exe)) } else { Self::App(AppLink::new(exe)) }
    }

    /// The executable's file name in lowercase.
    pub(super) fn exe(&self) -> &str {
        match self {
            Self::App(link) => link.exe(),
            Self::Except(link) => &link.exe,
        }
    }

    /// Whether the app is being captured, or for desktop-except being left out.
    pub(super) fn attached(&self) -> bool {
        match self {
            Self::App(link) => link.source().is_some(),
            Self::Except(link) => link.excluding,
        }
    }

    /// The capture that feeds the track, if any.
    pub(super) fn source(&self) -> Option<&dyn AppSource> {
        match self {
            Self::App(link) => link.source(),
            Self::Except(link) => link.current.as_ref().map(|feed| feed.source.as_ref()),
        }
    }

    /// True when a desktop-except track should have the desktop capture and has none.
    pub(super) fn desktop_missing(&self) -> bool {
        matches!(self, Self::Except(link) if !link.excluding && link.current.is_none())
    }

    /// One poll step; `running` holds every process of the app, or None when it is not running.
    pub(super) fn poll(&mut self, running: Option<&[u32]>, system: &mut dyn AppSystem) -> Option<AppChange> {
        match self {
            Self::App(link) => link.poll(running, system),
            Self::Except(link) => link.poll(running, system),
        }
    }

    /// Gives every queued packet to `push`; only a desktop-except link can change here.
    pub(super) fn drain(&mut self, now_100ns: i64, system: &mut dyn AppSystem, push: &mut dyn FnMut(&AudioPacket)) -> Option<AppChange> {
        match self {
            Self::App(link) => {
                if let Some(source) = link.source() {
                    while let Ok(packet) = source.receiver().try_recv() {
                        push(&packet);
                    }
                }
                None
            }
            Self::Except(link) => link.drain(now_100ns, system, push),
        }
    }
}

/// A sorted copy of a list of process IDs.
fn sorted(pids: &[u32]) -> Vec<u32> {
    let mut pids = pids.to_vec();
    pids.sort_unstable();
    pids
}

#[cfg(test)]
pub(super) mod fake {
    //! A stand-in for `capture::AppLoopback`.

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::mpsc::{self, SyncSender};

    use super::*;

    pub(in crate::engine) struct FakeSource {
        receiver: Receiver<AudioPacket>,
        lost: Rc<Cell<bool>>,
        pids: Vec<u32>,
    }

    impl AppSource for FakeSource {
        fn receiver(&self) -> &Receiver<AudioPacket> {
            &self.receiver
        }
        fn lost(&self) -> bool {
            self.lost.get()
        }
        fn instance_pids(&self) -> &[u32] {
            &self.pids
        }
        fn packets(&self) -> u64 {
            0
        }
        fn overflows(&self) -> u64 {
            0
        }
    }

    /// What the fake system reports, and what was started on it.
    #[derive(Clone, Default)]
    pub(in crate::engine) struct FakeSystem {
        /// The app's top-level processes now, which the next capture also reports.
        pub(in crate::engine) instances: Rc<RefCell<Vec<u32>>>,
        /// The kind of each capture started: an app mode, or None for the desktop.
        pub(in crate::engine) modes: Rc<RefCell<Vec<Option<LoopbackMode>>>>,
        /// How many captures were started.
        pub(in crate::engine) starts: Rc<Cell<usize>>,
        /// Makes every start fail.
        pub(in crate::engine) refuse: Rc<Cell<bool>>,
        /// The lost flag of the capture started last.
        pub(in crate::engine) lost: Rc<RefCell<Rc<Cell<bool>>>>,
        /// The packet sender of the capture started last.
        pub(in crate::engine) sender: Rc<RefCell<Option<SyncSender<AudioPacket>>>>,
    }

    impl AppSystem for FakeSystem {
        fn start_app(&mut self, _exe: &str, mode: LoopbackMode) -> Result<Box<dyn AppSource>, CaptureError> {
            self.capture(Some(mode))
        }
        fn start_desktop(&mut self) -> Result<Box<dyn AppSource>, CaptureError> {
            self.capture(None)
        }
        fn top_level_pids(&mut self, _exe: &str) -> Vec<u32> {
            self.instances.borrow().clone()
        }
    }

    impl FakeSystem {
        /// The packet sender of the capture started last.
        pub(in crate::engine) fn latest_sender(&self) -> Option<SyncSender<AudioPacket>> {
            self.sender.borrow().clone()
        }

        fn capture(&self, mode: Option<LoopbackMode>) -> Result<Box<dyn AppSource>, CaptureError> {
            if self.refuse.get() {
                return Err(CaptureError::Missing("the fake refuses to start"));
            }
            let (sender, receiver) = mpsc::sync_channel(256);
            let lost = Rc::new(Cell::new(false));
            *self.sender.borrow_mut() = Some(sender);
            *self.lost.borrow_mut() = Rc::clone(&lost);
            self.starts.set(self.starts.get() + 1);
            self.modes.borrow_mut().push(mode);
            let pids = if mode.is_some() { self.instances.borrow().clone() } else { Vec::new() };
            Ok(Box::new(FakeSource { receiver, lost, pids }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_link_attaches_detaches_and_attaches_again() {
        let system = fake::FakeSystem::default();
        *system.instances.borrow_mut() = vec![10];
        let mut apps = system.clone();
        let mut link = AppLink::new("C:\\Apps\\Discord.exe");
        assert_eq!(link.exe(), "discord.exe");
        assert_eq!(link.poll(None, &mut apps), None);
        system.refuse.set(true);
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);
        system.refuse.set(false);
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Attached));
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);
        assert_eq!(link.poll(None, &mut apps), Some(AppChange::Detached));
        assert!(link.source().is_none());
        assert_eq!(link.poll(None, &mut apps), None);
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Attached));
        // A capture that ends on its own detaches, and the next poll attaches again.
        system.lost.borrow().set(true);
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Detached));
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Attached));
        assert!(system.modes.borrow().iter().all(|mode| *mode == Some(LoopbackMode::Include)));
    }

    #[test]
    fn an_app_link_restarts_only_when_the_top_level_processes_change() {
        let system = fake::FakeSystem::default();
        *system.instances.borrow_mut() = vec![10];
        let mut apps = system.clone();
        let mut link = AppLink::new("game.exe");
        assert_eq!(link.poll(Some(&[10, 11]), &mut apps), Some(AppChange::Attached));
        // A new child process: the top-level processes are the same, so no capture is started.
        assert_eq!(link.poll(Some(&[10, 11, 12]), &mut apps), None);
        assert_eq!(link.poll(Some(&[10, 11, 12]), &mut apps), None);
        assert_eq!(system.starts.get(), 1);
        // A second instance: the new capture takes over with no attach or detach.
        *system.instances.borrow_mut() = vec![10, 20];
        assert_eq!(link.poll(Some(&[10, 11, 12, 20]), &mut apps), None);
        assert_eq!(system.starts.get(), 2);
        assert_eq!(link.source().map(|source| source.instance_pids().to_vec()), Some(vec![10, 20]));
        // Processes that cannot be read restart nothing, and are read again at the next poll.
        system.instances.borrow_mut().clear();
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);
        assert_eq!(system.starts.get(), 2);
        *system.instances.borrow_mut() = vec![10];
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);
        assert_eq!(system.starts.get(), 3);
    }

    /// A 10 ms stereo packet of a constant level, starting `at_100ns` on the timeline.
    fn level_packet(at_100ns: i64, rate: u32, level: f32) -> AudioPacket {
        let frames = rate / 100;
        let samples = vec![level; frames as usize * 2];
        AudioPacket { qpc_100ns: at_100ns, frames, channels: 2, sample_rate: rate, samples, discontinuity: false, silent: false }
    }

    /// Sends one 10 ms packet per index, at that index's place on the timeline.
    fn send(sender: &std::sync::mpsc::SyncSender<AudioPacket>, indexes: std::ops::Range<i64>, rate: u32, level: f32) -> bool {
        indexes.into_iter().all(|n| sender.send(level_packet(n * PACKET_100NS, rate, level)).is_ok())
    }

    /// Drains the link and returns the (start time, level) of each packet it gave.
    fn given(link: &mut ExceptLink, now_100ns: i64, system: &mut fake::FakeSystem) -> (Option<AppChange>, Vec<(i64, f32)>) {
        let mut packets = Vec::new();
        let change = link.drain(now_100ns, system, &mut |packet| packets.push((packet.qpc_100ns / PACKET_100NS, packet.samples[0])));
        (change, packets)
    }

    #[test]
    fn a_desktop_except_link_switches_at_the_new_sources_first_packet_and_back() -> Result<(), &'static str> {
        let system = fake::FakeSystem::default();
        let mut apps = system.clone();
        let mut link = ExceptLink::new("Spotify.exe");
        // Not running: the desktop, with nothing to report.
        assert_eq!(link.poll(None, &mut apps), None);
        let desktop = system.latest_sender().ok_or("no desktop capture")?;
        assert!(send(&desktop, 0..2, 48_000, 0.1));
        assert_eq!(given(&mut link, 0, &mut apps), (None, vec![(0, 0.1), (1, 0.1)]));

        // The app appears: reported at once, but the desktop feeds the track until the new capture has audio.
        *system.instances.borrow_mut() = vec![10];
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Attached));
        assert!(send(&desktop, 2..3, 48_000, 0.1));
        assert_eq!(given(&mut link, 100_000, &mut apps).1, vec![(2, 0.1)]);
        let except = system.latest_sender().ok_or("no exclude capture")?;
        assert!(send(&desktop, 3..6, 48_000, 0.1));
        assert!(send(&except, 4..6, 48_000, 0.2));
        // The desktop gives what came before the boundary, then it is stopped.
        assert_eq!(given(&mut link, 200_000, &mut apps).1, vec![(3, 0.1), (4, 0.2), (5, 0.2)]);
        assert!(!send(&desktop, 6..7, 48_000, 0.1), "the desktop capture still runs");
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);

        // The app exits and its capture ends: the desktop starts in the same drain.
        system.instances.borrow_mut().clear();
        system.lost.borrow().set(true);
        assert_eq!(given(&mut link, 300_000, &mut apps).0, Some(AppChange::Detached));
        assert_eq!(link.poll(None, &mut apps), None);
        let desktop = system.latest_sender().ok_or("no desktop capture")?;
        // A packet from before the seam is skipped.
        assert!(send(&desktop, 5..8, 48_000, 0.1));
        assert_eq!(given(&mut link, 400_000, &mut apps).1, vec![(6, 0.1), (7, 0.1)]);
        assert_eq!(*system.modes.borrow(), [None, Some(LoopbackMode::Exclude), None]);
        Ok(())
    }

    #[test]
    fn a_desktop_except_link_does_not_wait_for_a_silent_source_and_never_captures_a_running_app() -> Result<(), &'static str> {
        let system = fake::FakeSystem::default();
        let mut apps = system.clone();
        let mut link = ExceptLink::new("spotify.exe");
        link.poll(None, &mut apps);
        let desktop = system.latest_sender().ok_or("no desktop capture")?;

        // The app cannot be excluded yet: the desktop stays and nothing is reported.
        *system.instances.borrow_mut() = vec![10];
        system.refuse.set(true);
        assert_eq!(link.poll(Some(&[10]), &mut apps), None);
        system.refuse.set(false);
        assert_eq!(link.poll(Some(&[10]), &mut apps), Some(AppChange::Attached));
        // The new capture says nothing: the switch happens after the timeout.
        given(&mut link, 1_000_000, &mut apps);
        given(&mut link, 1_000_000 + SWITCH_TIMEOUT_100NS - 1, &mut apps);
        assert!(send(&desktop, 0..1, 48_000, 0.1));
        given(&mut link, 1_000_000 + SWITCH_TIMEOUT_100NS, &mut apps);
        assert!(!send(&desktop, 1..2, 48_000, 0.1), "the desktop capture still runs");

        // A child process restarts nothing; a new top-level process restarts the capture with no report.
        assert_eq!(link.poll(Some(&[10, 11]), &mut apps), None);
        assert_eq!(system.starts.get(), 2);
        *system.instances.borrow_mut() = vec![10, 20];
        assert_eq!(link.poll(Some(&[10, 11, 20]), &mut apps), None);
        assert_eq!(system.starts.get(), 3);
        given(&mut link, 5_000_000, &mut apps);
        given(&mut link, 5_000_000 + SWITCH_TIMEOUT_100NS, &mut apps);

        // The capture ends while the app still runs: silence until the next poll, never the desktop.
        system.lost.borrow().set(true);
        assert_eq!(given(&mut link, 8_000_000, &mut apps).0, None);
        assert!(link.current.is_none() && link.excluding);
        assert_eq!(link.poll(Some(&[10, 20]), &mut apps), None);
        assert_eq!(system.modes.borrow().last(), Some(&Some(LoopbackMode::Exclude)));

        // A listing without the app switches back even before the capture notices.
        assert_eq!(link.poll(None, &mut apps), Some(AppChange::Detached));
        assert_eq!(system.modes.borrow().last(), Some(&None));
        Ok(())
    }

    #[test]
    fn sample_positions_stay_continuous_across_switches_between_rates() -> Result<(), &'static str> {
        let system = fake::FakeSystem::default();
        let mut apps = system.clone();
        let mut link = ExceptLink::new("spotify.exe");
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let step = |link: &mut ExceptLink, pipe: &mut TrackPipeline, apps: &mut fake::FakeSystem, n: i64| {
            link.drain(n * PACKET_100NS, apps, &mut |packet| pipe.push(packet));
        };
        link.poll(None, &mut apps);
        // A 44.1 kHz desktop for a second.
        let desktop = system.latest_sender().ok_or("no desktop capture")?;
        assert!(send(&desktop, 0..100, 44_100, 0.5));
        step(&mut link, &mut pipe, &mut apps, 100);
        // The app appears; the 48 kHz exclude capture's first packet overlaps the desktop by two packets.
        *system.instances.borrow_mut() = vec![10];
        link.poll(Some(&[10]), &mut apps);
        let except = system.latest_sender().ok_or("no exclude capture")?;
        assert!(send(&desktop, 100..103, 44_100, 0.5));
        step(&mut link, &mut pipe, &mut apps, 103);
        assert!(send(&except, 101..200, 48_000, 0.5));
        step(&mut link, &mut pipe, &mut apps, 200);
        let at_first_switch = pipe.produced_frames();
        // The app closes; the desktop capture starts one packet late.
        system.instances.borrow_mut().clear();
        link.poll(None, &mut apps);
        let desktop = system.latest_sender().ok_or("no desktop capture")?;
        assert!(send(&desktop, 201..300, 44_100, 0.5));
        step(&mut link, &mut pipe, &mut apps, 300);

        assert!((at_first_switch as i64 - 200 * 480).abs() <= 480, "{at_first_switch} frames after the first switch");
        let produced = pipe.produced_frames() as i64;
        assert!((produced - 300 * 480).abs() <= 480, "{produced} frames after both switches");
        let out = drain_all(&mut pipe);
        let (mut quiet, mut longest) = (0, 0);
        for frame in out.chunks(2).skip(8) {
            quiet = if frame[0].abs() < 0.25 { quiet + 1 } else { 0 };
            longest = longest.max(quiet);
        }
        assert!(longest <= 480 + 16, "a gap of {longest} frames");
        Ok(())
    }

    const OUT_RATE: u32 = 48_000;
    const PACKET_100NS: i64 = 100_000;

    /// One 10 ms packet of a sine wave at the given device rate and channel count.
    fn sine_packet(qpc: i64, rate: u32, channels: u16, freq: f64, amp: f32, phase: f64) -> AudioPacket {
        let frames = rate / 100;
        let mut samples = Vec::with_capacity(frames as usize * usize::from(channels));
        for i in 0..frames {
            let t = f64::from(i) / f64::from(rate);
            let value = amp * ((2.0 * core::f64::consts::PI * freq * t + phase).sin() as f32);
            for _ in 0..channels {
                samples.push(value);
            }
        }
        AudioPacket { qpc_100ns: qpc, frames, channels, sample_rate: rate, samples, discontinuity: false, silent: false }
    }

    /// Drains every ready output frame from the pipeline.
    fn drain_all(pipe: &mut TrackPipeline) -> Vec<f32> {
        let mut all = Vec::new();
        loop {
            let chunk = pipe.take(OUT_RATE as usize);
            if chunk.is_empty() {
                break;
            }
            all.extend(chunk);
        }
        all
    }

    #[test]
    fn fast_48k_source_stays_locked_for_30_minutes() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let packets = 180_090; // 30 minutes of 10 ms packets at a clock 0.05 percent fast
        let mut worst: i128 = 0;
        for n in 0..packets {
            let qpc = (n as f64 * PACKET_100NS as f64 / 1.0005).round() as i64;
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            if n % 100 == 0 {
                let timeline_end = (i128::from(qpc) + i128::from(PACKET_100NS)) * i128::from(OUT_RATE) / 10_000_000;
                let diff = (timeline_end - i128::from(pipe.produced_frames())).abs();
                worst = worst.max(diff);
            }
        }
        assert!(worst <= 800, "worst timeline error was {worst} frames");
        let final_diff = (i128::from(pipe.produced_frames()) - 30 * 60 * i128::from(OUT_RATE)).abs();
        assert!(final_diff <= 48, "final length off by {final_diff} frames");
    }

    #[test]
    fn mono_44k1_source_gives_the_right_stereo_length() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..1000 {
            pipe.push(&sine_packet(qpc, 44_100, 1, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let diff = (i128::from(pipe.produced_frames()) - 10 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 48, "length off by {diff} frames");
        let out = drain_all(&mut pipe);
        assert_eq!(out.len() / 2, pipe.produced_frames() as usize);
    }

    #[test]
    fn late_first_packet_starts_with_two_seconds_of_silence() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        pipe.push(&sine_packet(2 * 10_000_000, 48_000, 2, 440.0, 0.9, 0.0));
        let out = drain_all(&mut pipe);
        let quiet = (96_000 - 8) * 2;
        assert!(out.len() >= quiet, "only {} output frames", out.len() / 2);
        assert!(out[..quiet].iter().all(|v| *v == 0.0), "expected pure silence first");
        let start = first_loud(&out, 0, 0.05);
        assert!((96_000 - 8..=96_000 + 8).contains(&start), "signal started at {start}");
    }

    #[test]
    fn ten_second_gap_with_silence_fill_stays_continuous() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        // A loopback device sends nothing from 1 s to 11 s; the caller fills silence.
        pipe.fill_silence_to(u64::from(5 * OUT_RATE));
        pipe.fill_silence_to(u64::from(11 * OUT_RATE));
        // A late packet from inside the gap overlaps filled output and is dropped.
        pipe.push(&sine_packet(10 * 10_000_000 + 5 * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        // Packets resume at 11 s with a loud marker whose first sample peaks.
        pipe.push(&sine_packet(11 * 10_000_000, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        qpc = 11 * 10_000_000 + PACKET_100NS;
        for _ in 1..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let out = drain_all(&mut pipe);
        assert!(out[..96_000].iter().any(|v| v.abs() > 0.1), "pre-gap signal missing");
        assert!(out[100_000..1_000_000].iter().all(|v| *v == 0.0), "gap is not continuous silence");
        let land = first_loud(&out, 500_000, 0.8);
        assert!((528_000 - 48..=528_000 + 48).contains(&land), "packet after the gap landed at {land}");
        let diff = (i128::from(pipe.produced_frames()) - 12 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 96, "total length off by {diff} frames");
    }

    #[test]
    fn offset_plus_200ms_shifts_placement_by_9600_frames() {
        let mut plain = TrackPipeline::new(0, OUT_RATE);
        plain.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        let plain_start = first_loud(&drain_all(&mut plain), 0, 0.5);
        let mut shifted = TrackPipeline::new(0, OUT_RATE);
        shifted.set_offset_ms(200.0);
        shifted.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        let shifted_start = first_loud(&drain_all(&mut shifted), 0, 0.5);
        let shift = shifted_start as i64 - plain_start as i64;
        assert!((shift - 9_600).abs() <= 4, "shift was {shift} frames");
    }

    #[test]
    fn gain_mute_and_clip_indicator_follow_the_output() {
        let mut plain = TrackPipeline::new(0, OUT_RATE);
        for n in 0..100 {
            plain.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        }
        drain_all(&mut plain);
        let peak_plain = plain.peak_since_last_read();
        assert!((0.8..=1.0).contains(&peak_plain), "peak was {peak_plain}");

        let mut halved = TrackPipeline::new(0, OUT_RATE);
        halved.set_gain_db(-6.0);
        for n in 0..100 {
            halved.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        }
        drain_all(&mut halved);
        let peak_half = halved.peak_since_last_read();
        assert!((peak_half - 0.5 * peak_plain).abs() <= 0.01 * peak_plain, "gain -6 dB peak {peak_half} vs {peak_plain}");

        let mut muted = TrackPipeline::new(0, OUT_RATE);
        muted.set_muted(true);
        muted.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, 0.0));
        let out = drain_all(&mut muted);
        assert!(out.iter().all(|v| *v == 0.0), "muted output is not silent");
        assert_eq!(muted.peak_since_last_read(), 0.0);
        assert!(!muted.clipped());

        let mut hot = TrackPipeline::new(0, OUT_RATE);
        hot.push(&sine_packet(0, 48_000, 2, 440.0, 1.2, 0.0));
        drain_all(&mut hot);
        assert!(hot.clipped(), "a 1.2 amplitude sine must report clipping");
    }

    #[test]
    fn the_peak_hold_keeps_the_loudest_output_across_meter_reads() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        assert_eq!(pipe.peak_hold_dbfs(), -100.0);
        for n in 0..50 {
            pipe.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.5, core::f64::consts::FRAC_PI_2));
        }
        drain_all(&mut pipe);
        assert!(pipe.peak_since_last_read() > 0.0);
        // A later quiet stretch and a meter read do not lower the hold.
        pipe.fill_silence_to(pipe.produced_frames() + 4_800);
        drain_all(&mut pipe);
        assert_eq!(pipe.peak_since_last_read(), 0.0);
        let hold = pipe.peak_hold_dbfs();
        // A 0.5 amplitude sine is -6 dBFS; the resampler's interpolation overshoots it a little.
        assert!((-7.0..=-4.5).contains(&hold), "peak hold was {hold} dBFS");
    }

    #[test]
    fn five_second_pause_adds_no_frames_and_no_gap() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let before = pipe.produced_frames();
        pipe.pause(qpc);
        // Five wall seconds pass; packets arriving during the pause are discarded.
        for _ in 0..500 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        assert_eq!(pipe.produced_frames(), before);
        pipe.resume(qpc);
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let diff = (i128::from(pipe.produced_frames()) - 2 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 48, "total length off by {diff} frames");
        let out = drain_all(&mut pipe);
        assert!(out[48_000..48_060].iter().any(|v| v.abs() > 0.1), "resume content did not continue at the timeline position");
    }

    /// The first frame index at or after from whose sample magnitude exceeds threshold.
    fn first_loud(samples: &[f32], from: usize, threshold: f32) -> usize {
        let frames = samples.len() / 2;
        (from..frames).find(|&i| samples[2 * i].abs() > threshold || samples[2 * i + 1].abs() > threshold).unwrap_or(usize::MAX)
    }
}
