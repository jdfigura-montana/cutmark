//! editor::record: arm state, the audio input stream behind it, the app's
//! audio settings and the recording flow: start, stop, cancel, placing the
//! finished take on the armed track and the editing lock while a take runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer, RingBuffer};
use uuid::Uuid;

use crate::audio::engine::{AudioError, latency_samples};
use crate::audio::input::{self, InputPick, InputStream};
use crate::audio::meters::MeterReading;
use crate::audio::player::Command;
use crate::editor::action::Action;
use crate::editor::editor::{Editor, EditorError};
use crate::editor::jobs::{TakeJob, TakeOutcome};
use crate::editor::message::Message;
use crate::io::config::{self, AppConfig, RecordChannels};
use crate::io::import::Loaded;
use crate::io::paths::{self, conformed_path, peaks_path};
use crate::io::record_writer::RecordWriter;
use crate::model::media::MediaItem;

/// Test double for the system's input devices. An Editor holding one lists
/// these devices and never opens a real one, so tests do not depend on the
/// machine's audio hardware. It also plays the part of the open input stream
/// (its format, measured latency and dropped-frame count); the test pushes
/// the take's samples with `Editor::push_input_frames`.
#[derive(Debug, Clone)]
pub struct FakeInput {
    /// Name of the Windows default input, if any.
    pub default: Option<String>,
    /// Every input device as (name, channel count).
    pub devices: Vec<(String, u16)>,
    /// Opening the input fails (as if no device could be found).
    pub open_fails: bool,
    /// The open input reports a lost device on the next tick.
    pub lost: bool,
    /// Sample rate of the fake input stream, in Hz.
    pub rate: u32,
    /// Channel count of the fake input stream.
    pub channels: u16,
    /// The input latency the fake stream measured, in nanoseconds.
    pub latency_ns: u64,
    /// Whole frames the fake stream reports as dropped (ring overflow).
    pub dropped_frames: u64,
}

impl Default for FakeInput {
    /// No devices, a healthy 48 kHz mono stream, no latency, nothing dropped.
    fn default() -> Self {
        FakeInput { default: None, devices: Vec::new(), open_fails: false, lost: false, rate: 48_000, channels: 1, latency_ns: 0, dropped_frames: 0 }
    }
}

/// True once the "device not found" notice was shown in this launch.
static FALLBACK_NOTED: AtomicBool = AtomicBool::new(false);

/// Message for changing the audio settings while a take runs.
const SETTINGS_LOCKED: &str = "Can't change this while recording";

/// The ring buffer between the input stream's callback and the take writer.
///
/// Created when a track is armed (and again whenever the input is reopened).
pub(super) struct Ring {
    /// Identifies this ring: a finished take job hands its consumer back with
    /// the id it was started with, and the Editor only keeps it when this same
    /// ring is still open.
    id: Uuid,
    /// The reading end. None while a take job holds it.
    consumer: Option<Consumer<f32>>,
    /// Test double only: the writing end, which the real input callback owns.
    #[cfg(test)]
    producer: Option<Producer<f32>>,
}

impl Ring {
    /// A ring around `consumer`. `producer` is kept only in tests, where it
    /// feeds synthetic frames; the real one lives in the input callback.
    fn new(consumer: Consumer<f32>, producer: Option<Producer<f32>>) -> Ring {
        #[cfg(not(test))]
        drop(producer);
        Ring {
            id: Uuid::new_v4(),
            consumer: Some(consumer),
            #[cfg(test)]
            producer,
        }
    }
}

/// The take being recorded, or (after Stop) being saved. While the Editor
/// holds one, editing is locked.
pub(super) struct Recording {
    /// The armed track the take is recorded on.
    track_id: Uuid,
    /// Timeline position where recording started (the playhead), in samples.
    start: u64,
    /// Take latency `L` in samples at the project rate, measured at the start.
    latency: u64,
    /// Job id of the take writer.
    job: u64,
    /// The ring the consumer belongs to.
    ring_id: Uuid,
    /// Device sample rate in Hz, to turn dropped frames into milliseconds.
    device_rate: u32,
    /// Channels of the take file: 2 for a Stereo take, else 1.
    channels: u16,
    /// Frames written to the take file so far, at the project rate.
    frames: u64,
    /// Peak entries `[minL, maxL, minR, maxR]` per 1,024 frames, for the live view.
    peaks: Vec<[f32; 4]>,
    /// Whole frames the input dropped, read when the take was stopped.
    dropped: u64,
    /// True once Stop or Esc was pressed: the take is on its way to the project.
    stopping: bool,
    /// Tells the writer job to finish the take and keep it.
    finish: Arc<AtomicBool>,
    /// Tells the writer job to discard the take.
    cancel: Arc<AtomicBool>,
}

/// What a running take looks like from outside, for drawing the live clip.
#[derive(Debug, Clone, Copy)]
pub struct RecordStatus<'a> {
    /// The track being recorded on.
    pub track_id: Uuid,
    /// Timeline position where the take starts, in samples.
    pub start: u64,
    /// Frames written so far, at the project rate.
    pub frames: u64,
    /// Channels of the take: 2 draws two lanes (L above R), 1 draws one.
    pub channels: u16,
    /// `[minL, maxL, minR, maxR]` per 1,024 frames written so far (mono: R = L).
    pub peaks: &'a [[f32; 4]],
}

/// Ring buffer size in samples: two seconds of whole device frames.
fn ring_capacity(rate: u32, channels: u16) -> usize {
    2usize.saturating_mul(rate as usize).saturating_mul(usize::from(channels))
}

/// Milliseconds of audio `frames` device frames make at `rate` Hz, rounded up
/// so any loss shows as at least 1 ms.
fn dropped_ms(frames: u64, rate: u32) -> u64 {
    frames.saturating_mul(1_000).div_ceil(u64::from(rate.max(1)))
}

/// What the Editor starts with: the app config and where it is saved, plus the
/// input test double. Test builds never touch the real config file or any
/// real input device; the running app reads the config once.
pub(super) fn initial_state() -> (AppConfig, Option<PathBuf>, Option<FakeInput>) {
    if cfg!(test) { (AppConfig::default(), None, Some(FakeInput::default())) } else { (config::load(), config::default_path(), None) }
}

/// The input channels a Record channels setting records.
pub fn input_pick(channels: RecordChannels) -> InputPick {
    match channels {
        RecordChannels::Mono1 => InputPick::Mono(0),
        RecordChannels::Mono2 => InputPick::Mono(1),
        RecordChannels::Stereo => InputPick::Stereo,
    }
}

/// The "device not found" notice, at most once per launch: only when a device
/// name was configured, the stream fell back to the default input, and `noted`
/// was not set yet. Sets `noted` when it returns the notice.
fn fallback_notice(wanted: Option<&str>, fell_back: bool, noted: &AtomicBool) -> Option<String> {
    let name = wanted?;
    if !fell_back || noted.swap(true, Ordering::Relaxed) {
        return None;
    }
    Some(format!("Input device {name} not found. Using the default input."))
}

impl Editor {
    /// The app audio settings.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// The track armed for recording, if any.
    pub fn armed_track(&self) -> Option<Uuid> {
        self.armed_track
    }

    /// Arms `track_id`, or disarms it when it is the armed track.
    ///
    /// Only one track is armed at a time: arming moves the arm. Arming opens
    /// the input stream (and fills the track's header meter from it); when it
    /// cannot open, the track stays disarmed and the reason goes to the
    /// message dialog. Arm state is not saved, not undoable and never dirty.
    /// Does nothing while a take is recording.
    pub fn toggle_arm(&mut self, track_id: Uuid) {
        if self.recording.is_some() {
            return;
        }
        if self.armed_track == Some(track_id) {
            self.disarm();
            return;
        }
        if !self.project.tracks.iter().any(|t| t.id == track_id) {
            return;
        }
        // Another track is armed: the input is already open, the arm moves.
        if self.armed_track.is_some() {
            self.armed_track = Some(track_id);
            return;
        }
        match self.open_input() {
            Ok(()) => self.armed_track = Some(track_id),
            Err(error) => self.job_errors.push(Message::recording(format!("Could not arm the track: {error}"))),
        }
    }

    /// Disarms the track and closes the input stream and its ring.
    pub(super) fn disarm(&mut self) {
        self.armed_track = None;
        self.input = None;
        self.ring = None;
    }

    /// Disarms when the armed track no longer exists (deleted, or removed by
    /// undo or redo).
    pub(super) fn tidy_armed_track(&mut self) {
        if let Some(id) = self.armed_track
            && !self.project.tracks.iter().any(|t| t.id == id)
        {
            self.disarm();
        }
    }

    /// Per-frame check: a lost input device disarms the track. While a take
    /// is recording, the take is stopped first and keeps what was captured.
    pub(super) fn input_tick(&mut self) {
        let lost = match &self.fake_input {
            Some(fake) => fake.lost && self.armed_track.is_some(),
            None => self.input.as_ref().is_some_and(InputStream::errored),
        };
        if !lost {
            return;
        }
        let live_take = self.recording.as_ref().is_some_and(|rec| !rec.stopping);
        if live_take {
            self.stop_recording();
            self.job_errors.push(Message::recording("Recording stopped: input device lost."));
        } else {
            self.job_errors.push(Message::recording("Input device lost."));
        }
        self.disarm();
    }

    /// Takes the input meter's reading (both sides), emptying it.
    pub(super) fn take_input_reading(&self) -> [MeterReading; 2] {
        [self.input_meter.l.take(), self.input_meter.r.take()]
    }

    /// Opens the input stream for the current settings, with a fresh ring
    /// buffer of two seconds of device frames whose reading end stays on the
    /// Editor until a take starts. With a test double nothing is opened.
    /// Queues the "device not found" notice (once per launch) when the saved
    /// device was replaced by the default input.
    fn open_input(&mut self) -> Result<(), AudioError> {
        // Any ring of an earlier stream is gone before a new one is made.
        self.ring = None;
        if let Some(fake) = &self.fake_input {
            if fake.open_fails {
                return Err(AudioError::NoInputDevice);
            }
            let (producer, consumer) = RingBuffer::new(ring_capacity(fake.rate, fake.channels));
            self.ring = Some(Ring::new(consumer, Some(producer)));
            return Ok(());
        }
        // Start from silence: nothing left over from an earlier arm.
        self.take_input_reading();
        let device = self.config.input_device.as_deref();
        // The ring must exist before the stream does: size it for the device
        // that will be opened.
        let (_, rate, channels) = input::probe_input(device)?;
        let (producer, consumer) = RingBuffer::new(ring_capacity(rate, channels));
        let stream = InputStream::open(device, input_pick(self.config.record_channels), Arc::clone(&self.input_meter), producer)?;
        if let Some(notice) = fallback_notice(device, stream.fell_back(), &FALLBACK_NOTED) {
            self.job_errors.push(Message::recording(notice));
        }
        self.ring = Some(Ring::new(consumer, None));
        self.input = Some(stream);
        Ok(())
    }

    /// Saves new audio settings and, when a track is armed, reopens the input
    /// with them.
    ///
    /// The settings are saved first (an error leaves everything as it was).
    /// If the reopen then fails, the track is disarmed and the error returned;
    /// the saved settings stay. Refused while a take is recording.
    pub fn set_config(&mut self, config: AppConfig) -> Result<(), EditorError> {
        if self.recording.is_some() {
            return Err(EditorError::Locked(SETTINGS_LOCKED.to_string()));
        }
        let config = config.clamped();
        let path = self.config_path.clone().ok_or_else(config::no_config_folder)?;
        config::save_to(&config, &path)?;
        self.config = config;
        if self.armed_track.is_some() {
            // The old stream must be closed before the device opens again.
            self.input = None;
            if let Err(error) = self.open_input() {
                self.disarm();
                return Err(EditorError::Input(format!("Could not reopen the input: {error}. The track was disarmed.")));
            }
        }
        Ok(())
    }

    /// Names of the input devices Windows lists. Slow (it probes every
    /// device): call when a dialog opens, not every frame.
    pub fn input_device_names(&self) -> Vec<String> {
        match &self.fake_input {
            Some(fake) => fake.devices.iter().map(|(name, _)| name.clone()).collect(),
            None => input::input_device_names(),
        }
    }

    /// Name of the Windows default input device, if there is one.
    pub fn default_input_name(&self) -> Option<String> {
        match &self.fake_input {
            Some(fake) => fake.default.clone(),
            None => input::default_input_name(),
        }
    }

    /// Input channel count of the device that would be opened for `device`
    /// (a name; None, or a name that is not found, means the Windows default).
    pub fn input_device_channels(&self, device: Option<&str>) -> Option<u16> {
        match &self.fake_input {
            Some(fake) => {
                let known = |name: &str| fake.devices.iter().any(|(d, _)| d == name);
                let name = device.filter(|name| known(name)).or(fake.default.as_deref())?;
                fake.devices.iter().find(|(d, _)| d == name).map(|(_, channels)| *channels)
            }
            None => input::device_channels(device),
        }
    }

    /// Test seam: where `set_config` saves.
    #[cfg(test)]
    pub fn set_config_path(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// Test seam: replaces the input test double.
    #[cfg(test)]
    pub fn set_fake_input(&mut self, fake: FakeInput) {
        self.fake_input = Some(fake);
    }

    /// Test seam: pushes interleaved device samples into the ring as the
    /// input callback would while recording. The samples must fit in the ring.
    #[cfg(test)]
    pub fn push_input_frames(&mut self, samples: &[f32]) {
        let producer = self.ring.as_mut().and_then(|ring| ring.producer.as_mut()).expect("an armed track with a test ring");
        let (_, rest) = producer.push_partial_slice(samples);
        assert!(rest.is_empty(), "the test ring is full");
    }
}

/// Recording: start, stop, cancel and placing the finished take.
impl Editor {
    /// True from `start_recording` until the take has been placed (or
    /// discarded), including the moment after Stop while the file is saved.
    /// Editing is locked for the whole time (see `check_unlocked`).
    pub fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// True while a take is being captured: from `start_recording` until Stop
    /// or Esc. After that the take is only being saved (`is_recording` stays
    /// true until it is placed), and nothing is left to ask the user.
    pub fn take_is_live(&self) -> bool {
        self.recording.as_ref().is_some_and(|rec| !rec.stopping)
    }

    /// The running take, for drawing the live clip: where it starts, how many
    /// frames are written and the peak entries so far.
    pub fn recording_status(&self) -> Option<RecordStatus<'_>> {
        self.recording.as_ref().map(|rec| RecordStatus {
            track_id: rec.track_id,
            start: rec.start,
            frames: rec.frames,
            channels: rec.channels,
            peaks: &rec.peaks,
        })
    }

    /// Where the view's playhead sits while a take exists: the end of what is
    /// recorded so far (record start + frames) while it is live, the record
    /// start once Stop or Esc returned it. None when nothing is recording.
    pub fn record_playhead(&self) -> Option<u64> {
        self.recording.as_ref().map(|rec| if rec.stopping { rec.start } else { rec.start.saturating_add(rec.frames) })
    }

    /// Starts recording on the armed track from the playhead, with every
    /// other track playing along (overdub).
    ///
    /// Refused with `Locked` during an export or an earlier take, and with
    /// `Record("Arm a track first")` when no track is armed. Stops a bin
    /// preview or scrub, measures the take latency from the stream timestamps
    /// (plus the Latency offset setting), opens the take file in media/ and
    /// starts the writer job, then switches capture on and starts playback.
    /// The player runs open-ended: it keeps advancing past the last clip
    /// (silence) until Stop.
    pub fn start_recording(&mut self) -> Result<(), EditorError> {
        self.check_unlocked(false)?;
        let Some(track_id) = self.armed_track else {
            return Err(EditorError::Record("Arm a track first".to_string()));
        };
        let not_ready = || EditorError::Record("The input is not ready yet".to_string());
        let (device_rate, device_channels, input_latency_ns) = self.input_format().ok_or_else(not_ready)?;
        if !self.ring.as_ref().is_some_and(|ring| ring.consumer.is_some()) {
            return Err(not_ready());
        }

        let project_rate = self.project.sample_rate;
        let file_name = paths::record_file_name(&self.paths.media(), chrono::Local::now());
        let mut part = self.paths.media_file(&file_name)?.into_os_string();
        part.push(".part");
        let writer = RecordWriter::create(&PathBuf::from(part), device_rate, device_channels, self.config.record_channels, project_rate)?;
        let Some((ring_id, consumer)) = self.ring.as_mut().and_then(|ring| ring.consumer.take().map(|consumer| (ring.id, consumer))) else {
            writer.abort();
            return Err(not_ready());
        };

        // A bin preview stops here; a scrub is ended by the Play below.
        self.stop_preview();
        let start = self.view.playhead;
        let output_ns = self.audio.as_ref().map_or(0, |engine| engine.shared().output_latency_ns.load(Ordering::Relaxed));
        let latency = latency_samples(output_ns, input_latency_ns, self.config.latency_offset_ms, project_rate);

        let job = self.next_job;
        self.next_job += 1;
        let finish = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.spawn_record(
            job,
            TakeJob {
                writer,
                consumer,
                paths: self.paths.clone(),
                file_name,
                media_id: Uuid::new_v4(),
                project_rate,
                latency,
                finish: Arc::clone(&finish),
                cancel: Arc::clone(&cancel),
            },
        );
        self.recording = Some(Recording {
            track_id,
            start,
            latency,
            job,
            ring_id,
            device_rate,
            channels: if self.config.record_channels == RecordChannels::Stereo { 2 } else { 1 },
            frames: 0,
            peaks: Vec::new(),
            dropped: 0,
            stopping: false,
            finish,
            cancel,
        });

        // Capture on first, then playback: the take starts a moment before
        // the first overdub sample is heard, which the latency accounts for.
        if let Some(stream) = &self.input {
            stream.reset_dropped();
            stream.set_recording(true);
        }
        if let Some(engine) = &self.audio {
            engine.send(Command::Record { from: start });
        }
        Ok(())
    }

    /// Ends the take and keeps it: capture and playback stop (the playhead
    /// returns like any Stop) and the writer finishes the file. The clip
    /// appears when the writer is done (`poll_jobs`). Does nothing when not
    /// recording or when the take was already stopped.
    pub fn stop_recording(&mut self) {
        self.end_take(false);
    }

    /// Ends the take and discards it: nothing is added to the project and the
    /// take file is deleted. Does nothing when not recording or when the take
    /// was already stopped (Esc after Stop cannot undo the Stop).
    pub fn cancel_recording(&mut self) {
        self.end_take(true);
    }

    /// Shared body of stop and cancel.
    fn end_take(&mut self, discard: bool) {
        if !self.recording.as_ref().is_some_and(|rec| !rec.stopping) {
            return;
        }
        let dropped = self.captured_dropped();
        // Capture off before the writer is told to finish, so its last drain
        // sees everything the input pushed.
        self.halt_capture();
        let mut start = None;
        if let Some(rec) = self.recording.as_mut() {
            rec.stopping = true;
            rec.dropped = dropped;
            let flag = if discard { &rec.cancel } else { &rec.finish };
            flag.store(true, Ordering::Release);
            start = Some(rec.start);
        }
        // Like any Stop, the playhead returns to where the take started.
        if let Some(start) = start {
            self.view.playhead = start;
        }
    }

    /// Capture off and playback stopped (the normal Stop return).
    fn halt_capture(&self) {
        if let Some(stream) = &self.input {
            stream.set_recording(false);
        }
        if let Some(engine) = &self.audio {
            engine.send(Command::Stop);
        }
    }

    /// Device rate, device channels and measured input latency of the open
    /// input (the test double supplies them in tests).
    fn input_format(&self) -> Option<(u32, u16, u64)> {
        match (&self.fake_input, &self.input) {
            (Some(fake), _) => Some((fake.rate, fake.channels, fake.latency_ns)),
            (None, Some(stream)) => Some((stream.device_rate(), stream.device_channels(), stream.latency_ns())),
            (None, None) => None,
        }
    }

    /// Whole frames the input has dropped since the take started.
    fn captured_dropped(&self) -> u64 {
        match (&self.fake_input, &self.input) {
            (Some(fake), _) => fake.dropped_frames,
            (None, Some(stream)) => stream.dropped_frames(),
            (None, None) => 0,
        }
    }

    /// A writer-job progress message: frames written and new peak entries.
    pub(super) fn record_progress(&mut self, job: u64, frames: u64, mut peaks: Vec<[f32; 4]>) {
        if let Some(rec) = self.recording.as_mut().filter(|rec| rec.job == job) {
            rec.frames = frames;
            rec.peaks.append(&mut peaks);
        }
    }

    /// The writer job ended: the ring comes back and the take is placed on its
    /// track as one "Record" undo step, or dropped with a message.
    pub(super) fn record_done(&mut self, job: u64, consumer: Consumer<f32>, outcome: TakeOutcome) {
        let Some(rec) = self.recording.take_if(|rec| rec.job == job) else {
            return;
        };
        // The ring goes back to the stream it belongs to, unless that stream
        // is gone (disarmed) or was reopened with a ring of its own.
        if let Some(ring) = self.ring.as_mut()
            && ring.id == rec.ring_id
            && ring.consumer.is_none()
        {
            ring.consumer = Some(consumer);
        }
        // A writer that ended by itself (an error) leaves capture and
        // playback running: stop them too.
        if !rec.stopping {
            self.halt_capture();
        }
        match outcome {
            TakeOutcome::Finished { item, loaded } => self.place_take(&rec, item, loaded),
            TakeOutcome::TooShort => self.job_errors.push(Message::recording("Recording too short.")),
            TakeOutcome::Cancelled => {}
            TakeOutcome::Failed(message) => self.job_errors.push(Message::recording(format!("The recording failed: {message}"))),
        }
    }

    /// Adds the finished take as one "Record" step: bin item "Take <n>" plus a
    /// clip at the record start that hides the first `latency` samples. Takes
    /// the files away again when the project refuses it.
    fn place_take(&mut self, rec: &Recording, item: MediaItem, loaded: Loaded) {
        let (id, file_name) = (item.id, item.file_name.clone());
        // In the store before the edit publishes its mix plan, like an import.
        self.media_store.insert(id, loaded);
        let action = Action::RecordTake { item, track_id: rec.track_id, start: rec.start, src_in: rec.latency };
        match self.apply(action) {
            Ok(()) if rec.dropped > 0 => {
                let ms = dropped_ms(rec.dropped, rec.device_rate);
                self.job_errors.push(Message::recording(format!("The recording dropped {ms} ms of audio.")));
            }
            Ok(()) => {}
            Err(error) => {
                let _ = std::fs::remove_file(self.paths.media().join(file_name));
                let _ = std::fs::remove_file(conformed_path(&self.paths, id));
                let _ = std::fs::remove_file(peaks_path(&self.paths, id));
                self.job_errors.push(Message::recording(error.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::action::Action;
    use crate::editor::history::History;
    use crate::editor::jobs::{ExportSettings, JobMessage};
    use crate::editor::message::texts;
    use crate::io::export::{ExportFormat, ExportKind};
    use crate::io::paths::ProjectPaths;
    use crate::model::fixtures::{project_with_clips, spans, srcs};
    use crate::model::project::{BitDepth, ViewState};
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    /// An editor over `count` empty tracks; no device is ever opened.
    fn editor_with_tracks(count: usize) -> Editor {
        let tracks: Vec<&[(u64, u64)]> = vec![&[]; count];
        Editor::with_project(project_with_clips(&tracks), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") })
    }

    fn track_id(editor: &Editor, index: usize) -> Uuid {
        editor.project.tracks[index].id
    }

    #[test]
    fn record_channels_map_to_input_picks() {
        assert_eq!(input_pick(RecordChannels::Mono1), InputPick::Mono(0));
        assert_eq!(input_pick(RecordChannels::Mono2), InputPick::Mono(1));
        assert_eq!(input_pick(RecordChannels::Stereo), InputPick::Stereo);
    }

    #[test]
    fn arming_sets_the_armed_track_and_toggling_again_disarms() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        assert_eq!(editor.armed_track(), None);
        editor.toggle_arm(a);
        assert_eq!(editor.armed_track(), Some(a));
        editor.toggle_arm(a);
        assert_eq!(editor.armed_track(), None);
        editor.toggle_arm(b);
        assert_eq!(editor.armed_track(), Some(b));
    }

    #[test]
    fn only_one_track_is_armed_at_a_time() {
        let mut editor = editor_with_tracks(3);
        let ids: Vec<Uuid> = (0..3).map(|i| track_id(&editor, i)).collect();
        editor.toggle_arm(ids[0]);
        editor.toggle_arm(ids[2]);
        assert_eq!(editor.armed_track(), Some(ids[2]));
        editor.toggle_arm(ids[1]);
        assert_eq!(editor.armed_track(), Some(ids[1]));
        // Toggling a track that is not armed never disarms the armed one.
        editor.toggle_arm(ids[0]);
        assert_eq!(editor.armed_track(), Some(ids[0]));
    }

    #[test]
    fn an_unknown_track_cannot_be_armed() {
        let mut editor = editor_with_tracks(1);
        editor.toggle_arm(Uuid::new_v4());
        assert_eq!(editor.armed_track(), None);
    }

    #[test]
    fn deleting_the_armed_track_disarms() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        editor.toggle_arm(a);
        editor.apply(Action::DeleteTrack { id: b }).unwrap();
        assert_eq!(editor.armed_track(), Some(a), "another track's delete");
        editor.apply(Action::DeleteTrack { id: a }).unwrap();
        assert_eq!(editor.armed_track(), None);
    }

    #[test]
    fn undo_and_redo_that_remove_the_armed_track_disarm() {
        let mut editor = editor_with_tracks(1);
        editor.apply(Action::AddTrack { index: None, name: None }).unwrap();
        let added = track_id(&editor, 1);

        // Undo removes the added track.
        editor.toggle_arm(added);
        assert!(editor.undo());
        assert_eq!(editor.armed_track(), None, "undo of the add");

        // Redo brings it back; delete + undo + arm + redo removes it again.
        assert!(editor.redo());
        editor.apply(Action::DeleteTrack { id: added }).unwrap();
        assert!(editor.undo());
        editor.toggle_arm(added);
        assert_eq!(editor.armed_track(), Some(added));
        assert!(editor.redo());
        assert_eq!(editor.armed_track(), None, "redo of the delete");
    }

    #[test]
    fn arming_never_marks_the_project_dirty_or_adds_history() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        let before = (editor.history.undo.len(), editor.history.redo.len());
        let project = Arc::clone(&editor.project);
        editor.toggle_arm(a);
        editor.toggle_arm(b);
        editor.toggle_arm(b);
        assert!(!editor.dirty);
        assert_eq!((editor.history.undo.len(), editor.history.redo.len()), before);
        assert_eq!(editor.undo_label(), None);
        assert!(Arc::ptr_eq(&project, &editor.project), "project untouched");
    }

    #[test]
    fn an_open_failure_leaves_the_track_disarmed_and_reports_it() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.set_fake_input(FakeInput { open_fails: true, ..FakeInput::default() });
        editor.toggle_arm(id);
        assert_eq!(editor.armed_track(), None);
        let messages = editor.take_job_errors();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].text.starts_with("Could not arm the track: "), "{messages:?}");
        assert_eq!(messages[0].title, "Recording");
    }

    #[test]
    fn a_lost_input_device_disarms_with_a_message() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);
        editor.tick();
        assert_eq!(editor.armed_track(), Some(id), "a healthy input stays armed");
        assert!(editor.take_job_errors().is_empty());

        editor.set_fake_input(FakeInput { lost: true, ..FakeInput::default() });
        editor.tick();
        assert_eq!(editor.armed_track(), None);
        assert_eq!(editor.take_job_errors(), vec![Message::recording("Input device lost.")]);
        // Nothing armed: the lost flag alone says nothing.
        editor.tick();
        assert!(editor.take_job_errors().is_empty());
    }

    #[test]
    fn the_armed_track_meter_reads_the_input_not_its_own_playback() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        editor.publish(); // builds the meter bank
        editor.toggle_arm(a);

        // Playback level on both tracks, a different input level.
        for meter in &editor.meters.tracks {
            meter.l.add(0.2, 0.04, 1, false);
            meter.r.add(0.2, 0.04, 1, false);
        }
        editor.input_meter.l.add(0.9, 0.81, 1, false);
        editor.input_meter.r.add(1.0, 1.0, 1, true);

        let (tracks, _master) = editor.read_meters();
        let reading = |id: Uuid| tracks.iter().find(|(t, _)| *t == id).unwrap().1;
        let armed = reading(a);
        assert_eq!((armed[0].0, armed[0].3), (0.9, false), "input left");
        assert_eq!((armed[1].0, armed[1].3), (1.0, true), "input right, clipped");
        let other = reading(b);
        assert_eq!((other[0].0, other[1].0), (0.2, 0.2), "unarmed keeps playback");

        // Both accumulators were emptied.
        let (tracks, _) = editor.read_meters();
        assert!(tracks.iter().all(|(_, [l, r])| l.0 == 0.0 && r.0 == 0.0));
    }

    #[test]
    fn a_disarmed_track_reads_its_own_meter_again() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.publish();
        editor.toggle_arm(id);
        editor.toggle_arm(id);
        editor.meters.tracks[0].l.add(0.3, 0.09, 1, false);
        editor.input_meter.l.add(0.9, 0.81, 1, false);
        let (tracks, _) = editor.read_meters();
        assert_eq!(tracks[0].1[0].0, 0.3);
    }

    #[test]
    fn set_config_writes_through_to_the_injected_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut editor = editor_with_tracks(1);
        editor.set_config_path(path.clone());
        let config = AppConfig {
            input_device: Some("Line In".to_string()),
            record_channels: RecordChannels::Stereo,
            latency_offset_ms: 250, // clamped
        };
        editor.set_config(config).unwrap();
        let want = AppConfig { input_device: Some("Line In".to_string()), record_channels: RecordChannels::Stereo, latency_offset_ms: 100 };
        assert_eq!(editor.config(), &want);
        assert_eq!(config::load_from(&path), want);
        assert!(!editor.dirty, "settings are not part of the project");
    }

    #[test]
    fn set_config_without_a_config_path_fails_and_changes_nothing() {
        let mut editor = editor_with_tracks(1);
        let config = AppConfig { record_channels: RecordChannels::Mono2, ..AppConfig::default() };
        assert!(editor.set_config(config).is_err());
        assert_eq!(editor.config(), &AppConfig::default());
    }

    #[test]
    fn set_config_with_an_armed_track_reopens_and_a_failed_reopen_disarms() {
        let dir = tempdir().unwrap();
        let mut editor = editor_with_tracks(1);
        editor.set_config_path(dir.path().join("config.json"));
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);

        editor.set_config(AppConfig::default()).unwrap();
        assert_eq!(editor.armed_track(), Some(id), "reopen worked");

        editor.set_fake_input(FakeInput { open_fails: true, ..FakeInput::default() });
        let config = AppConfig { record_channels: RecordChannels::Stereo, ..AppConfig::default() };
        let error = editor.set_config(config.clone()).unwrap_err().to_string();
        assert!(error.contains("The track was disarmed."), "{error}");
        assert_eq!(editor.armed_track(), None);
        assert_eq!(editor.config(), &config, "the settings were still saved");
    }

    #[test]
    fn a_failed_save_leaves_an_armed_track_armed() {
        let dir = tempdir().unwrap();
        let mut editor = editor_with_tracks(1);
        // The "file" is a directory, so the rename over it fails.
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        editor.set_config_path(path);
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);
        assert!(editor.set_config(AppConfig::default()).is_err());
        assert_eq!(editor.armed_track(), Some(id));
        assert_eq!(editor.config(), &AppConfig::default());
    }

    #[test]
    fn the_fallback_notice_shows_once_and_only_for_a_named_device() {
        let noted = AtomicBool::new(false);
        // No saved name, or no fallback: nothing, and the flag stays unset.
        assert_eq!(fallback_notice(None, true, &noted), None);
        assert_eq!(fallback_notice(Some("USB Mic"), false, &noted), None);
        assert!(!noted.load(Ordering::Relaxed));
        // First fallback: the notice with the saved name.
        assert_eq!(fallback_notice(Some("USB Mic"), true, &noted).as_deref(), Some("Input device USB Mic not found. Using the default input."));
        // Every later fallback in the same launch is silent.
        assert_eq!(fallback_notice(Some("USB Mic"), true, &noted), None);
        assert_eq!(fallback_notice(Some("Other"), true, &noted), None);
    }

    #[test]
    fn the_device_lists_come_from_the_test_double() {
        let mut editor = editor_with_tracks(1);
        assert!(editor.input_device_names().is_empty());
        assert_eq!(editor.default_input_name(), None);
        assert_eq!(editor.input_device_channels(None), None);

        editor.set_fake_input(FakeInput {
            default: Some("Mic".to_string()),
            devices: vec![("Mic".to_string(), 2), ("Line".to_string(), 1)],
            ..FakeInput::default()
        });
        assert_eq!(editor.input_device_names(), vec!["Mic", "Line"]);
        assert_eq!(editor.default_input_name().as_deref(), Some("Mic"));
        assert_eq!(editor.input_device_channels(None), Some(2));
        assert_eq!(editor.input_device_channels(Some("Line")), Some(1));
        // A name that is not found uses the default, like the real stream.
        assert_eq!(editor.input_device_channels(Some("Gone")), Some(2));
    }

    // ---- Recording ----

    /// How long a test waits for the writer thread before it fails.
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// The only message the lock returns while a take runs.
    const LOCKED: &str = "Recording in progress";

    /// A project folder on disk with the given tracks and clips; the first
    /// track is armed on the test input `fake`.
    fn recording_editor_with(tracks: &[&[(u64, u64)]], fake: FakeInput) -> (Editor, tempfile::TempDir) {
        let parent = tempdir().unwrap();
        let paths = paths::create_layout(parent.path(), "Rec").unwrap();
        let mut editor = Editor::with_project(project_with_clips(tracks), ViewState::default(), paths);
        editor.set_fake_input(fake);
        let first = editor.project.tracks[0].id;
        editor.toggle_arm(first);
        assert_eq!(editor.armed_track(), Some(first));
        (editor, parent)
    }

    /// Like [`recording_editor_with`] with the default 48 kHz mono test input.
    fn recording_editor(tracks: &[&[(u64, u64)]]) -> (Editor, tempfile::TempDir) {
        recording_editor_with(tracks, FakeInput::default())
    }

    /// Polls the editor's jobs until `done` is true; fails after 10 s.
    fn wait_until(editor: &mut Editor, what: &str, done: impl Fn(&Editor) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            editor.poll_jobs();
            if done(editor) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Pushes `frames` mono frames of `value` and waits until the writer has
    /// written `total` frames (the take's frame count so far).
    fn push_mono(editor: &mut Editor, frames: usize, value: f32, total: u64) {
        editor.push_input_frames(&vec![value; frames]);
        wait_until(editor, "the writer to catch up", |e| e.recording_status().is_some_and(|s| s.frames == total));
    }

    /// Stops the take and waits until it has been placed (or dropped).
    fn finish_take(editor: &mut Editor) {
        editor.stop_recording();
        wait_until(editor, "the take to finish", |e| !e.is_recording());
    }

    /// Names of the files in a project folder, sorted.
    fn files_in(dir: PathBuf) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// The bin item with this display name.
    fn bin_item(editor: &Editor, name: &str) -> MediaItem {
        editor.project.media.iter().find(|item| item.display_name == name).cloned().unwrap_or_else(|| panic!("no bin item named {name}"))
    }

    /// `Err(Locked)` with the recording message?
    fn is_locked<T>(result: &Result<T, EditorError>) -> bool {
        matches!(result, Err(EditorError::Locked(message)) if message == LOCKED)
    }

    #[test]
    fn the_ring_holds_two_seconds_of_device_frames() {
        assert_eq!(ring_capacity(48_000, 1), 96_000);
        assert_eq!(ring_capacity(44_100, 2), 176_400);
        assert_eq!(ring_capacity(0, 2), 0);
        assert_eq!(ring_capacity(u32::MAX, u16::MAX), { 2usize.saturating_mul(u32::MAX as usize).saturating_mul(usize::from(u16::MAX)) });
    }

    #[test]
    fn dropped_frames_become_milliseconds_rounded_up() {
        assert_eq!(dropped_ms(0, 48_000), 0);
        assert_eq!(dropped_ms(960, 48_000), 20);
        assert_eq!(dropped_ms(1, 48_000), 1, "any loss shows as at least 1 ms");
        assert_eq!(dropped_ms(48_001, 48_000), 1_001);
        assert_eq!(dropped_ms(10, 0), 10_000, "a zero rate cannot divide by zero");
    }

    #[test]
    fn arming_opens_a_ring_and_disarming_closes_it() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        let id = editor.armed_track().unwrap();
        assert!(editor.ring.as_ref().is_some_and(|r| r.consumer.is_some()));
        editor.toggle_arm(id);
        assert!(editor.ring.is_none());
        editor.toggle_arm(id);
        assert!(editor.ring.is_some(), "arming again opens a fresh ring");
    }

    #[test]
    fn a_finished_take_is_one_record_step_with_the_latency_hidden() {
        // L = 10 ms offset (480) + 5 ms measured input latency (240) = 720.
        let fake = FakeInput { latency_ns: 5_000_000, ..FakeInput::default() };
        let (mut editor, _dir) = recording_editor_with(&[&[], &[]], fake);
        editor.config.latency_offset_ms = 10;
        let armed = editor.armed_track().unwrap();
        editor.set_playhead(1_000);

        editor.start_recording().unwrap();
        assert!(editor.is_recording());
        let status = editor.recording_status().unwrap();
        assert_eq!((status.track_id, status.start, status.frames), (armed, 1_000, 0));

        push_mono(&mut editor, 4_800, 0.5, 4_800);
        let status = editor.recording_status().unwrap();
        assert_eq!(status.peaks.len(), 4, "one entry per 1,024 frames");
        assert!(status.peaks.iter().all(|p| *p == [0.5; 4]), "{:?}", status.peaks);
        assert!(editor.take_job_errors().is_empty());

        finish_take(&mut editor);

        // One undo step, labeled "Record".
        assert_eq!(editor.history.undo.len(), 1);
        assert_eq!(editor.undo_label(), Some("Record"));
        assert!(editor.dirty);
        // The bin item and the one clip.
        let take = bin_item(&editor, "Take 1");
        assert_eq!((take.channels, take.length, take.original_rate), (1, 4_800, 48_000));
        assert_eq!(editor.project.media.len(), 2, "the fixture's item plus the take");
        assert!(editor.project.tracks[1].clips.is_empty(), "only the armed track");
        let clips = &editor.project.tracks[0].clips;
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!((clip.media_id, clip.start, clip.src_in, clip.src_out), (take.id, 1_000, 720, 4_800));
        // The take is a real WAV in media/, with its cache, and the audio is loaded.
        assert!(take.file_name.starts_with("rec-") && take.file_name.ends_with(".wav"));
        assert_eq!(files_in(editor.paths.media()), std::slice::from_ref(&take.file_name));
        assert_eq!(files_in(editor.paths.cache()).len(), 2);
        let loaded = editor.media(take.id).expect("the take is loaded");
        assert_eq!(loaded.samples.len(), 4_800);
        assert!((loaded.samples[0] - 0.5).abs() < 1e-5);
        // Recording is over, the track stays armed, nothing to report.
        assert!(!editor.is_recording());
        assert!(editor.recording_status().is_none());
        assert_eq!(editor.armed_track(), Some(armed));
        assert!(editor.take_job_errors().is_empty());
    }

    #[test]
    fn the_second_take_is_take_2_and_the_ring_is_reused() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.set_playhead(500);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 2_000, 0.25, 2_000);
        finish_take(&mut editor);

        editor.set_playhead(10_000);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 3_000, 0.25, 3_000);
        finish_take(&mut editor);

        let (first, second) = (bin_item(&editor, "Take 1"), bin_item(&editor, "Take 2"));
        assert_ne!(first.file_name, second.file_name, "two files, even in one second");
        assert_eq!(editor.history.undo.len(), 2);
        let clips = &editor.project.tracks[0].clips;
        assert_eq!(clips.iter().map(|c| (c.start, c.media_id)).collect::<Vec<_>>(), [(500, first.id), (10_000, second.id)]);
    }

    #[test]
    fn undo_removes_the_clip_and_the_bin_item_and_save_cleans_the_file_up() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 2_400, 0.5, 2_400);
        finish_take(&mut editor);
        let take = bin_item(&editor, "Take 1");
        let file = editor.paths.media().join(&take.file_name);
        let cache = conformed_path(&editor.paths, take.id);

        assert!(editor.undo());
        assert_eq!(editor.project.media.len(), 1, "the bin item is gone");
        assert!(editor.project.tracks[0].clips.is_empty(), "and so is the clip");

        // Redo could bring the take back, so Save keeps its files...
        editor.save().unwrap();
        assert!(file.is_file() && cache.is_file());
        assert!(editor.redo());
        assert_eq!(editor.project.tracks[0].clips.len(), 1);
        assert!(editor.undo());

        // ...and once history can no longer reach it, Save deletes them.
        editor.history = History::new();
        editor.save().unwrap();
        assert!(!file.exists(), "the take file is cleaned up");
        assert!(!cache.exists() && !peaks_path(&editor.paths, take.id).exists());
    }

    #[test]
    fn cancel_adds_nothing_and_leaves_no_files() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        let before = Arc::clone(&editor.project);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 4_800, 0.5, 4_800);
        assert_eq!(files_in(editor.paths.media()).len(), 1, "the .part exists");

        editor.cancel_recording();
        wait_until(&mut editor, "the take to be discarded", |e| !e.is_recording());

        assert!(Arc::ptr_eq(&editor.project, &before), "the project is untouched");
        assert!(editor.history.undo.is_empty());
        assert!(!editor.dirty);
        assert!(files_in(editor.paths.media()).is_empty(), "no .part, no wav");
        assert!(files_in(editor.paths.cache()).is_empty());
        assert!(editor.take_job_errors().is_empty());

        // The ring came back: the next take works.
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        finish_take(&mut editor);
        assert_eq!(bin_item(&editor, "Take 1").length, 1_000);
    }

    #[test]
    fn cancel_after_stop_cannot_undo_the_stop() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        editor.stop_recording();
        editor.cancel_recording();
        wait_until(&mut editor, "the take to finish", |e| !e.is_recording());
        assert_eq!(bin_item(&editor, "Take 1").length, 1_000);
        // And with nothing recording, both are plain no-ops.
        editor.stop_recording();
        editor.cancel_recording();
        assert!(!editor.is_recording());
    }

    #[test]
    fn a_take_no_longer_than_its_latency_is_dropped() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.config.latency_offset_ms = 10; // 480 samples
        let before = Arc::clone(&editor.project);

        editor.start_recording().unwrap();
        push_mono(&mut editor, 480, 0.5, 480);
        finish_take(&mut editor);

        assert_eq!(editor.take_job_errors(), [Message::recording("Recording too short.")], "a Recording message");
        assert!(Arc::ptr_eq(&editor.project, &before));
        assert!(editor.history.undo.is_empty());
        assert!(files_in(editor.paths.media()).is_empty(), "the file is deleted");
        assert!(files_in(editor.paths.cache()).is_empty(), "and no cache was built");
        assert!(!editor.is_recording());

        // One frame longer than L has one audible frame.
        editor.start_recording().unwrap();
        push_mono(&mut editor, 481, 0.5, 481);
        finish_take(&mut editor);
        let clip = &editor.project.tracks[0].clips[0];
        assert_eq!((clip.src_in, clip.src_out), (480, 481));
        assert!(editor.take_job_errors().is_empty());
    }

    #[test]
    fn stopping_at_once_is_too_short_too() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        finish_take(&mut editor);
        assert_eq!(texts(editor.take_job_errors()), ["Recording too short."]);
        assert!(editor.history.undo.is_empty());
        assert!(files_in(editor.paths.media()).is_empty());
    }

    #[test]
    fn recording_needs_an_armed_track() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        let id = editor.armed_track().unwrap();
        editor.toggle_arm(id);
        let next_job = editor.next_job;

        let error = editor.start_recording().unwrap_err();
        assert!(matches!(error, EditorError::Record(_)));
        assert_eq!(error.to_string(), "Arm a track first");
        assert!(!editor.is_recording());
        assert_eq!(editor.next_job, next_job, "no job was started");
        assert!(files_in(editor.paths.media()).is_empty());
    }

    #[test]
    fn a_take_that_cannot_be_started_leaves_the_ring_alone() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        std::fs::remove_dir(editor.paths.media()).unwrap();
        let error = editor.start_recording().unwrap_err();
        assert!(matches!(error, EditorError::Io(_)), "{error}");
        assert!(!editor.is_recording());

        std::fs::create_dir(editor.paths.media()).unwrap();
        editor.start_recording().expect("the ring consumer is still here");
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        finish_take(&mut editor);
        assert_eq!(bin_item(&editor, "Take 1").length, 1_000);
    }

    #[test]
    fn dropped_frames_are_reported_after_the_take() {
        // 960 frames at 48 kHz = 20 ms.
        let fake = FakeInput { dropped_frames: 960, ..FakeInput::default() };
        let (mut editor, _dir) = recording_editor_with(&[&[]], fake);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 2_000, 0.5, 2_000);
        finish_take(&mut editor);
        assert_eq!(editor.take_job_errors(), [Message::recording("The recording dropped 20 ms of audio.")]);
        assert_eq!(bin_item(&editor, "Take 1").length, 2_000, "the take is kept");
    }

    #[test]
    fn a_lost_device_keeps_what_was_captured() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        let id = editor.armed_track().unwrap();
        editor.start_recording().unwrap();
        push_mono(&mut editor, 2_400, 0.5, 2_400);

        editor.fake_input.as_mut().unwrap().lost = true;
        editor.tick();
        assert_eq!(editor.armed_track(), None, "disarmed as for any lost input");
        assert!(editor.ring.is_none());
        wait_until(&mut editor, "the take to finish", |e| !e.is_recording());

        assert_eq!(editor.take_job_errors(), [Message::recording("Recording stopped: input device lost.")], "one message, not two");
        let take = bin_item(&editor, "Take 1");
        assert_eq!(take.length, 2_400);
        assert_eq!(editor.project.tracks[0].clips.len(), 1);
        assert_eq!(editor.undo_label(), Some("Record"));

        // Arming again opens a new ring and the next take works.
        editor.fake_input.as_mut().unwrap().lost = false;
        editor.toggle_arm(id);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        finish_take(&mut editor);
        let messages = editor.take_job_errors();
        assert!(messages.is_empty(), "{messages:?}");
        assert_eq!(bin_item(&editor, "Take 2").length, 1_000);
    }

    #[test]
    fn a_device_lost_after_stop_is_the_plain_lost_message() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        editor.stop_recording();
        editor.fake_input.as_mut().unwrap().lost = true;
        editor.tick();
        wait_until(&mut editor, "the take to finish", |e| !e.is_recording());
        assert_eq!(editor.take_job_errors(), [Message::recording("Input device lost.")]);
        assert_eq!(bin_item(&editor, "Take 1").length, 1_000, "the take is kept");
    }

    #[test]
    fn a_failed_writer_reports_the_error_and_adds_nothing() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        let rec = editor.recording.as_ref().unwrap();
        let (job, cancel) = (rec.job, Arc::clone(&rec.cancel));

        // The writer thread says it failed (a full disk, say) while the take
        // is still recording.
        let consumer = RingBuffer::<f32>::new(1).1;
        editor.jobs.tx.send(JobMessage::RecordDone { job, consumer, outcome: TakeOutcome::Failed("disk full".to_string()) }).unwrap();
        editor.poll_jobs();

        assert!(!editor.is_recording(), "capture is over");
        assert_eq!(editor.take_job_errors(), [Message::recording("The recording failed: disk full")]);
        assert!(editor.history.undo.is_empty());
        assert_eq!(editor.project.media.len(), 1);
        assert!(editor.ring.as_ref().is_some_and(|r| r.consumer.is_some()));

        // The real writer thread is still running; end it and check it cleans up.
        cancel.store(true, Ordering::Release);
        let media = editor.paths.media();
        let deadline = Instant::now() + TIMEOUT;
        while !files_in(media.clone()).is_empty() {
            assert!(Instant::now() < deadline, "the .part was not deleted");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn a_take_overwrites_what_it_lands_on() {
        let (mut editor, _dir) = recording_editor(&[&[(0, 100_000)]]);
        editor.config.latency_offset_ms = 10; // 480 samples hidden
        editor.set_playhead(20_000);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 4_800, 0.5, 4_800);
        finish_take(&mut editor);

        // The take's clip covers 20,000..24,320 (4,800 - 480 audible frames).
        assert_eq!(spans(&editor.project, 0), [(0, 20_000), (20_000, 24_320), (24_320, 100_000)]);
        assert_eq!(srcs(&editor.project, 0), [(0, 20_000), (480, 4_800), (24_320, 100_000)]);
        assert!(editor.undo(), "one step undoes the whole take");
        assert_eq!(spans(&editor.project, 0), [(0, 100_000)]);
    }

    #[test]
    fn a_retake_at_the_same_position_keeps_the_rest_of_the_first_take() {
        // Recording again from the exact start of an earlier take: the new
        // clip covers the first part of the old one, which keeps its tail.
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 3_000, 0.5, 3_000);
        finish_take(&mut editor);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.25, 1_000);
        finish_take(&mut editor);

        assert!(editor.take_job_errors().is_empty());
        assert_eq!(spans(&editor.project, 0), [(0, 1_000), (1_000, 3_000)]);
        assert_eq!(srcs(&editor.project, 0), [(0, 1_000), (1_000, 3_000)]);
        let (first, second) = (bin_item(&editor, "Take 1"), bin_item(&editor, "Take 2"));
        let clips = &editor.project.tracks[0].clips;
        assert_eq!((clips[0].media_id, clips[1].media_id), (second.id, first.id));
        assert!(editor.undo(), "the retake is one undo step");
        assert_eq!(spans(&editor.project, 0), [(0, 3_000)]);
    }

    #[test]
    fn the_view_playhead_follows_the_take_and_returns_to_its_start_on_stop() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.set_playhead(1_000);
        assert!(editor.record_playhead().is_none());
        editor.start_recording().unwrap();
        assert!(editor.take_is_live());
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 1_000, "nothing recorded yet");

        push_mono(&mut editor, 4_800, 0.5, 4_800);
        editor.poll_transport();
        assert_eq!(editor.record_playhead(), Some(5_800));
        assert_eq!(editor.view.playhead, 5_800, "record start + frames");
        let status = editor.recording_status().unwrap();
        assert_eq!(status.channels, 1, "a mono take has one lane");

        // Stop: the playhead is back at the record start at once, and stays
        // there while the take is saved and after it is placed.
        editor.stop_recording();
        assert!(!editor.take_is_live(), "no longer being captured");
        assert!(editor.is_recording(), "but not placed yet");
        assert_eq!(editor.view.playhead, 1_000);
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 1_000);
        wait_until(&mut editor, "the take to finish", |e| !e.is_recording());
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 1_000);
        assert!(editor.record_playhead().is_none());
    }

    #[test]
    fn esc_returns_the_playhead_to_the_record_start_too() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        editor.set_playhead(2_000);
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        editor.poll_transport();
        assert_eq!(editor.view.playhead, 3_000);
        editor.cancel_recording();
        assert_eq!(editor.view.playhead, 2_000);
        wait_until(&mut editor, "the take to be discarded", |e| !e.is_recording());
        assert_eq!(editor.view.playhead, 2_000);
    }

    #[test]
    fn a_stereo_take_reports_two_lanes() {
        let fake = FakeInput { channels: 2, ..FakeInput::default() };
        let (mut editor, _dir) = recording_editor_with(&[&[]], fake);
        editor.config.record_channels = RecordChannels::Stereo;
        editor.start_recording().unwrap();
        assert_eq!(editor.recording_status().unwrap().channels, 2);
        editor.cancel_recording();
        wait_until(&mut editor, "the take to be discarded", |e| !e.is_recording());
    }

    #[test]
    fn a_stereo_take_keeps_both_channels() {
        let fake = FakeInput { channels: 2, ..FakeInput::default() };
        let (mut editor, _dir) = recording_editor_with(&[&[]], fake);
        editor.config.record_channels = RecordChannels::Stereo;
        editor.start_recording().unwrap();
        let frames: Vec<f32> = (0..2_000).flat_map(|_| [0.25, -0.5]).collect();
        editor.push_input_frames(&frames);
        wait_until(&mut editor, "the writer to catch up", |e| e.recording_status().is_some_and(|s| s.frames == 2_000));
        assert_eq!(editor.recording_status().unwrap().peaks[0], [0.25, 0.25, -0.5, -0.5]);
        finish_take(&mut editor);

        let take = bin_item(&editor, "Take 1");
        assert_eq!((take.channels, take.length), (2, 2_000));
        let samples = &editor.media(take.id).unwrap().samples;
        assert!((samples[0] - 0.25).abs() < 1e-5 && (samples[1] + 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_device_at_another_rate_is_resampled_to_the_project_rate() {
        let fake = FakeInput { rate: 44_100, ..FakeInput::default() };
        let (mut editor, _dir) = recording_editor_with(&[&[]], fake);
        editor.start_recording().unwrap();
        editor.push_input_frames(&vec![0.25; 44_100]);
        // The resampler holds frames back until it is flushed, so wait on the
        // ring being drained rather than on a frame count.
        wait_until(&mut editor, "the ring to drain", |e| e.recording_status().is_some_and(|s| s.frames > 40_000));
        finish_take(&mut editor);
        let take = bin_item(&editor, "Take 1");
        assert!(take.length.abs_diff(48_000) <= 2, "one second at 44.1 kHz is one second at 48 kHz, got {}", take.length);
        assert_eq!(take.original_rate, 48_000, "the file is written at the project rate");
    }

    #[test]
    fn editing_is_locked_while_recording_except_adding_a_marker() {
        let (mut editor, _dir) = recording_editor(&[&[(0, 1_000)], &[]]);
        let track = editor.project.tracks[1].id;
        editor.apply(Action::SetTrackMute { id: track, mute: true }).unwrap();
        editor.copy(0, 500, &[editor.project.tracks[0].id]);
        editor.start_recording().unwrap();
        let before = Arc::clone(&editor.project);
        let history = editor.history.undo.len();

        // Every edit is refused and changes nothing.
        let add_track = Action::AddTrack { index: None, name: None };
        assert!(is_locked(&editor.apply(add_track)));
        assert!(is_locked(&editor.apply(Action::DeleteTrack { id: track })));
        assert!(is_locked(&editor.apply(Action::SetMasterVolume { db: -6.0 })));
        assert!(is_locked(&editor.paste_insert_at(0, 0)), "paste goes through commit");
        assert!(is_locked(&editor.split()));
        assert!(is_locked(&editor.cut(0, 500, vec![editor.project.tracks[0].id], false)));
        assert!(is_locked(&editor.preview(Action::SetMasterVolume { db: -3.0 })));
        assert!(Arc::ptr_eq(&editor.project, &before));
        assert_eq!(editor.history.undo.len(), history);

        // Undo and redo, gestures, import and previews are no-ops.
        assert!(!editor.undo());
        assert!(!editor.redo());
        editor.begin_gesture();
        assert!(editor.gesture_base.is_none());
        assert!(editor.import_files(vec![PathBuf::from("x.wav")]).is_empty());
        editor.relink(editor.project.media[0].id, PathBuf::from("x.wav"));
        editor.preview_media(editor.project.media[0].id);
        assert!(editor.pending_jobs().is_empty(), "no job was started");
        assert_eq!(editor.previewing(), None);

        // Adding a marker is the one edit that works.
        editor.set_playhead(700);
        editor.add_marker_at_playhead().unwrap();
        assert_eq!(editor.project.markers.len(), 1);
        assert_eq!(editor.history.undo.len(), history + 1);
        assert!(editor.dirty);

        editor.cancel_recording();
        wait_until(&mut editor, "the take to be discarded", |e| !e.is_recording());
        assert!(editor.apply(Action::SetMasterVolume { db: -6.0 }).is_ok(), "unlocked again");
    }

    #[test]
    fn transport_arming_and_settings_are_locked_while_recording() {
        let (mut editor, _dir) = recording_editor(&[&[(0, 1_000)], &[]]);
        let (armed, other) = (editor.project.tracks[0].id, editor.project.tracks[1].id);
        let config = editor.config().clone();
        editor.set_playhead(5_000);
        editor.start_recording().unwrap();

        // Transport: nothing moves the playhead.
        editor.seek(9_999);
        editor.jump_start();
        editor.jump_end();
        editor.scrub_start(7_000);
        editor.scrub_to(8_000);
        editor.scrub_end();
        editor.toggle_pause();
        editor.play_range(0, 100);
        editor.jump_next_marker();
        assert_eq!(editor.view.playhead, 5_000);
        assert!(editor.is_recording());

        // Arming and the settings.
        editor.toggle_arm(other);
        editor.toggle_arm(armed);
        assert_eq!(editor.armed_track(), Some(armed));
        let error = editor.set_config(AppConfig::default()).unwrap_err();
        assert!(matches!(&error, EditorError::Locked(m) if m == "Can't change this while recording"));
        assert_eq!(editor.config(), &config);

        // Nothing else can start.
        assert!(is_locked(&editor.start_recording()));
        let export = ExportSettings {
            path: editor.paths.exports().join("x.wav"),
            format: ExportFormat { kind: ExportKind::Wav(BitDepth::Float32), channels: 2, in_rate: 48_000 },
            start: 0,
            end: 1_000,
            tracks: None,
        };
        assert!(is_locked(&editor.start_export(export)));
        let parent = editor.paths.root.parent().unwrap().to_path_buf();
        assert!(is_locked(&editor.save_as(&parent, "Other")));
        assert!(editor.pending_jobs().is_empty());

        editor.cancel_recording();
        wait_until(&mut editor, "the take to be discarded", |e| !e.is_recording());
    }

    #[test]
    fn space_and_the_stop_button_end_the_take_and_keep_it() {
        for use_stop_button in [false, true] {
            let (mut editor, _dir) = recording_editor(&[&[]]);
            editor.start_recording().unwrap();
            push_mono(&mut editor, 1_000, 0.5, 1_000);
            if use_stop_button {
                editor.stop();
            } else {
                editor.toggle_play();
            }
            // Stopped, but still locked until the take is placed.
            assert!(editor.is_recording());
            assert!(is_locked(&editor.apply(Action::SetMasterVolume { db: -6.0 })));
            wait_until(&mut editor, "the take to finish", |e| !e.is_recording());
            assert_eq!(bin_item(&editor, "Take 1").length, 1_000);
            assert_eq!(editor.undo_label(), Some("Record"));
        }
    }

    #[test]
    fn save_and_autosave_work_while_recording_and_cleanup_waits() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        std::fs::write(editor.paths.media().join("junk.wav"), b"x").unwrap();
        editor.autosave_interval = Duration::ZERO;
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);

        // Autosave keeps running: a marker makes the project dirty.
        editor.add_marker_at_playhead().unwrap();
        editor.tick();
        assert!(!files_in(editor.paths.autosave()).is_empty(), "autosave wrote");

        // Save works, but the unused-media cleanup waits: the take's own
        // files (and this junk) must survive until the take is placed.
        editor.save().unwrap();
        assert!(editor.paths.project_json().is_file());
        assert!(!editor.dirty);
        let names = files_in(editor.paths.media());
        assert_eq!(names.len(), 2, "junk.wav and the .part: {names:?}");
        assert!(names.iter().any(|n| n == "junk.wav"));
        assert!(names.iter().any(|n| n.ends_with(".wav.part")));

        finish_take(&mut editor);
        editor.save().unwrap();
        let take = bin_item(&editor, "Take 1");
        assert_eq!(files_in(editor.paths.media()), std::slice::from_ref(&take.file_name));
    }

    #[test]
    fn dropping_the_editor_mid_take_ends_the_writer_and_deletes_the_part() {
        let (mut editor, _dir) = recording_editor(&[&[]]);
        let media = editor.paths.media();
        editor.start_recording().unwrap();
        push_mono(&mut editor, 1_000, 0.5, 1_000);
        assert_eq!(files_in(media.clone()).len(), 1);
        drop(editor);
        let deadline = Instant::now() + TIMEOUT;
        while !files_in(media.clone()).is_empty() {
            assert!(Instant::now() < deadline, "the .part was left behind");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
