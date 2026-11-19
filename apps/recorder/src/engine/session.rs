//! The recording session: session config, live stats, and the start, pause,
//! resume and stop calls that drive the video thread.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use thiserror::Error;

use capture::{AudioDevice, CaptureError, qpc_frequency, qpc_now};
use media_io::AudioCodec;
use media_io::MediaError;
use media_io::encode::RateControl as EncoderRateControl;
use media_io::encode::{Codec as EncoderCodec, EncoderConfig, Preset};
use media_io::mux::{Container as MuxContainer, MuxSender, MuxThread, Muxer, SendParameters};

use super::audio::{self, AudioMessage, AudioStart};
use super::clock::MediaClock;
use super::marks::{MarksWarning, TakeMarks, wav_length_ms};
use super::video;
use crate::config::{Codec, Container, Profile, RateControl};

/// Why starting, pausing or stopping a recording failed.
#[derive(Debug, Error)]
pub enum EngineError {
    /// A capture call failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A media IO call failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The requested monitor index does not exist.
    #[error("no monitor with index {0}")]
    MonitorIndex(usize),
    /// A container or codec that arrives in a later phase.
    #[error("{0}")]
    NotYet(&'static str),
    /// Something failed inside the engine that has no tighter category.
    #[error("{0}")]
    Failed(String),
}

/// One audio track: a device, its processing settings, and its stream routing.
#[derive(Clone)]
pub struct TrackSpec {
    /// The track name, used as the stream title and in the WAV file name.
    pub name: String,
    /// The device captured for this track; None when it was missing at start, and the track records silence until it returns.
    pub device: Option<AudioDevice>,
    /// The Windows endpoint ID of the device, used to find it again after it is lost.
    pub device_id: String,
    /// The friendly name of the device, the fallback when the ID no longer matches.
    pub device_name: String,
    /// The track's output gain in dB.
    pub gain_db: f32,
    /// Whether the track's output is muted.
    pub muted: bool,
    /// The track's sync offset in ms, -500 to 500.
    pub offset_ms: i32,
    /// Whether the track takes part in the mixdown stream.
    pub in_mixdown: bool,
}

/// Everything the video thread needs to record one take.
pub struct SessionConfig {
    /// DXGI adapter index for the shared D3D11 device.
    pub adapter: usize,
    /// Index into the list returned by `capture::list_monitors()`.
    pub monitor: usize,
    /// Draw the mouse cursor into the captured frames.
    pub cursor: bool,
    /// The capture and encode profile in use.
    pub profile: Profile,
    /// Where the recording is written.
    pub output_file: PathBuf,
    /// One entry per audio track, in stream order.
    pub tracks: Vec<TrackSpec>,
    /// The codec every audio stream is encoded with; AAC 320 kbps by default.
    pub audio_codec: AudioCodec,
    /// Whether the sum of the in-mixdown tracks becomes its own stream.
    pub mixdown: bool,
    /// Whether every track also gets its own stream in the video file.
    pub track_streams: bool,
    /// Whether every track is written as a WAV sidecar.
    pub wav_sidecars: bool,
    /// The take folder the WAV sidecars are written into.
    pub take_folder: PathBuf,
    /// The base name for the WAV sidecar file names.
    pub base_name: String,
}

/// Live counters of a running session, shared with the UI thread through an Arc.
pub struct Stats {
    /// Frames written to the encoder.
    pub(super) frames: AtomicU64,
    /// Recorded frames that repeated the previous source frame.
    pub(super) duplicated: AtomicU64,
    /// Frames that never reached the encoder.
    pub(super) dropped: AtomicU64,
    /// Frames the capture received, whether the pacer used them or not.
    pub(super) frames_received: AtomicU64,
    /// Encoded bytes handed to the muxer.
    pub(super) bytes_written: AtomicU64,
    /// The QPC value at recording time 0.
    pub(super) started_qpc: AtomicU64,
    /// Whether the session is paused; the video thread watches it every tick.
    pub(super) paused: AtomicBool,
    /// The text of the last frame error, if any.
    pub(super) last_error: Mutex<Option<String>>,
    /// Total audio frames handed to the audio encoders across all streams.
    pub(super) audio_frames: AtomicU64,
    /// One entry per audio track, folded in by the audio thread.
    tracks: Vec<TrackSlot>,
}

impl Stats {
    fn new(tracks: &[TrackSpec]) -> Self {
        Self {
            frames: AtomicU64::new(0),
            duplicated: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            frames_received: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            started_qpc: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            last_error: Mutex::new(None),
            audio_frames: AtomicU64::new(0),
            tracks: tracks
                .iter()
                .map(|track| TrackSlot {
                    name: track.name.clone(),
                    meter: Mutex::new(TrackMeter { peak: 0.0, clipped: false }),
                    packets: AtomicU64::new(0),
                    overflows: AtomicU64::new(0),
                    device_lost: AtomicBool::new(false),
                    gain_db_bits: AtomicU32::new(track.gain_db.to_bits()),
                    muted: AtomicBool::new(track.muted),
                })
                .collect(),
        }
    }

    /// Reads every counter into a plain snapshot.
    pub(super) fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            frames: self.frames.load(Ordering::Relaxed),
            duplicated: self.duplicated.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            frames_received: self.frames_received.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            started_qpc: self.started_qpc.load(Ordering::Relaxed),
            paused: self.paused.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|slot| slot.clone()),
            audio_frames: self.audio_frames.load(Ordering::Relaxed),
            tracks: self.tracks.iter().map(TrackSlot::snapshot).collect(),
        }
    }

    /// Folds one tick of per-track audio state into the live counters.
    pub(super) fn fold_audio_track(&self, index: usize, peak: f32, clipped: bool, packets: u64, overflows: u64, device_lost: bool) {
        let Some(slot) = self.tracks.get(index) else {
            return;
        };
        slot.packets.store(packets, Ordering::Relaxed);
        slot.overflows.store(overflows, Ordering::Relaxed);
        slot.device_lost.store(device_lost, Ordering::Relaxed);
        if let Ok(mut meter) = slot.meter.lock() {
            if peak > meter.peak {
                meter.peak = peak;
            }
            meter.clipped |= clipped;
        }
    }

    /// Sets a track's live gain in dB; the audio thread applies it on its next tick.
    fn set_track_gain_db(&self, index: usize, db: f32) {
        if let Some(slot) = self.tracks.get(index) {
            slot.gain_db_bits.store(db.to_bits(), Ordering::Relaxed);
        }
    }

    /// Sets a track's live mute; the audio thread applies it on its next tick.
    fn set_track_muted(&self, index: usize, muted: bool) {
        if let Some(slot) = self.tracks.get(index) {
            slot.muted.store(muted, Ordering::Relaxed);
        }
    }

    /// A track's live gain in dB and mute, as the UI last set them.
    pub(super) fn track_controls(&self, index: usize) -> Option<(f32, bool)> {
        let slot = self.tracks.get(index)?;
        Some((f32::from_bits(slot.gain_db_bits.load(Ordering::Relaxed)), slot.muted.load(Ordering::Relaxed)))
    }

    /// Records the text of the latest frame error.
    pub(super) fn set_last_error(&self, text: String) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(text);
        }
    }
}

/// A plain copy of the live counters at one instant.
pub struct StatsSnapshot {
    /// Frames written to the encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous source frame.
    pub duplicated: u64,
    /// Frames that never reached the encoder.
    pub dropped: u64,
    /// Frames the capture received, whether the pacer used them or not.
    pub frames_received: u64,
    /// Encoded bytes handed to the muxer.
    pub bytes_written: u64,
    /// The QPC value at recording time 0.
    pub started_qpc: u64,
    /// Whether the session is paused right now.
    pub paused: bool,
    /// The text of the last frame error, if any.
    pub last_error: Option<String>,
    /// Total audio frames handed to the audio encoders across all streams.
    pub audio_frames: u64,
    /// One entry per audio track, in track order.
    pub tracks: Vec<TrackSnapshot>,
}

/// One audio track's counters at one instant.
pub struct TrackSnapshot {
    /// The track name.
    pub name: String,
    /// The output peak in dBFS since the last read; -100.0 for silence.
    pub peak_dbfs: f32,
    /// Whether the output clipped since the last read.
    pub clipped: bool,
    /// Packets the capture produced so far, including dropped ones.
    pub packets: u64,
    /// Packets dropped because the capture queue was full.
    pub overflows: u64,
    /// Whether the track's device was lost.
    pub device_lost: bool,
}

/// One audio track's live state, shared between the audio thread and snapshots.
struct TrackSlot {
    /// The track name.
    name: String,
    /// The output peak and clip state since the last snapshot read.
    meter: Mutex<TrackMeter>,
    /// Packets the capture produced so far, including dropped ones.
    packets: AtomicU64,
    /// Packets dropped because the capture queue was full.
    overflows: AtomicU64,
    /// Whether the endpoint was invalidated.
    device_lost: AtomicBool,
    /// The track's live gain in dB, as f32 bits, set by the UI and applied by the audio thread.
    gain_db_bits: AtomicU32,
    /// Whether the track is muted right now, set by the UI and applied by the audio thread.
    muted: AtomicBool,
}

/// One track's output meter, reset by every snapshot read.
struct TrackMeter {
    /// The linear output peak since the last read.
    peak: f32,
    /// Whether the output clipped since the last read.
    clipped: bool,
}

impl TrackSlot {
    /// Reads and resets the slot's meter into a plain snapshot.
    fn snapshot(&self) -> TrackSnapshot {
        let (peak, clipped) = match self.meter.lock() {
            Ok(mut meter) => {
                let peak = meter.peak;
                let clipped = meter.clipped;
                meter.peak = 0.0;
                meter.clipped = false;
                (peak, clipped)
            }
            Err(_) => (0.0, false),
        };
        let peak_dbfs = if peak > 0.0 { 20.0 * peak.log10() } else { -100.0 };
        TrackSnapshot {
            name: self.name.clone(),
            peak_dbfs,
            clipped,
            packets: self.packets.load(Ordering::Relaxed),
            overflows: self.overflows.load(Ordering::Relaxed),
            device_lost: self.device_lost.load(Ordering::Relaxed),
        }
    }
}

/// The totals of one finished take.
pub struct Summary {
    /// The file the recording was written to.
    pub file: PathBuf,
    /// Frames written to the encoder.
    pub frames: u64,
    /// Recorded frames that repeated the previous source frame.
    pub duplicated: u64,
    /// Frames that never reached the encoder.
    pub dropped: u64,
    /// The size of the finished file, read back by stop().
    pub bytes_written: u64,
    /// Packets written, one entry per stream index.
    pub packets: Vec<u64>,
    /// Recording time in milliseconds.
    pub duration_ms: u64,
    /// Average bitrate of the finished file, in megabits per second.
    pub average_mbps: f64,
    /// The WAV sidecar files written for the take.
    pub wav_files: Vec<PathBuf>,
}

/// Messages from the video thread back to the session.
pub(super) enum ThreadMessage {
    /// The video thread's setup ended: the encoder facts on success, or the
    /// error that ended the setup.
    Setup(Result<VideoSetup, EngineError>),
    /// The take ended, either with a summary or with the error that ended it.
    Finished(Result<Summary, EngineError>),
}

/// The encoder facts the session needs to add the video stream to the muxer.
pub(super) struct VideoSetup {
    /// The encoder's codec parameters, including extradata.
    pub(super) params: SendParameters,
    /// The time base the encoder stamps its packets in.
    pub(super) time_base: (i32, i32),
}

/// The command that releases a stream thread to start pacing.
pub(super) struct StartCommand {
    /// The QPC value every stream counts its recording time 0 from.
    pub(super) start_qpc: i64,
    /// The queue the thread writes its packets into.
    pub(super) mux: MuxSender,
}

/// One audio stream to add to the muxer between the video stream and start.
pub(super) struct AudioStreamSpec {
    /// The encoder's codec parameters, including extradata.
    pub(super) params: SendParameters,
    /// The time base the encoder stamps its packets in.
    pub(super) time_base: (i32, i32),
    /// The stream title: the track name, or Mixdown for the mixdown stream.
    pub(super) title: String,
    /// Whether this stream is the default one.
    pub(super) default: bool,
}

/// One running recording: talks to the video thread through atomics and a channel.
pub struct RecordingSession {
    stats: Arc<Stats>,
    stop_flag: Arc<AtomicBool>,
    rx: Receiver<ThreadMessage>,
    handle: Option<std::thread::JoinHandle<()>>,
    audio_rx: Receiver<AudioMessage>,
    audio_handle: Option<std::thread::JoinHandle<()>>,
    mux: Option<MuxThread>,
    /// The marks of the take, until stop() finishes them.
    marks: Option<TakeMarks>,
    /// The last marks error, which never stops the recording.
    marks_warning: MarksWarning,
    /// Frames per second of the video, for the video's length at stop.
    fps: u32,
}

impl RecordingSession {
    /// Starts a recording: the video and audio threads set up, the session
    /// builds the muxer around their streams, then every stream starts from
    /// one instant.
    pub fn start(cfg: SessionConfig) -> Result<Self, EngineError> {
        if cfg.profile.container != Container::Mkv {
            return Err(EngineError::NotYet("MP4 and MOV arrive in phase 4"));
        }
        media_io::init()?;
        let output_file = cfg.output_file.clone();
        let (take_folder, base_name) = (cfg.take_folder.clone(), cfg.base_name.clone());
        let fps = cfg.profile.fps;
        let stats = Arc::new(Stats::new(&cfg.tracks));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel::<StartCommand>();
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stop_flag);
        let job = audio::AudioJob {
            tracks: cfg.tracks.clone(),
            codec: cfg.audio_codec,
            mixdown: cfg.mixdown,
            track_streams: cfg.track_streams,
            wav_sidecars: cfg.wav_sidecars,
            take_folder: cfg.take_folder.clone(),
            base_name: cfg.base_name.clone(),
        };
        let handle = std::thread::Builder::new()
            .name("video".to_owned())
            .spawn(move || video::run(cfg, thread_stats, thread_stop, tx, start_rx))
            .map_err(|err| EngineError::Failed(format!("the video thread could not be started: {err}")))?;
        // The audio thread reports the encoder facts of its streams, or its
        // setup error, while the video thread sets up in parallel.
        let (audio_tx, audio_rx) = mpsc::channel();
        let (audio_start_tx, audio_start_rx) = mpsc::channel::<AudioStart>();
        let audio_stats = Arc::clone(&stats);
        let audio_stop = Arc::clone(&stop_flag);
        let audio_handle =
            match std::thread::Builder::new().name("audio".to_owned()).spawn(move || audio::run(job, audio_stats, audio_stop, audio_tx, audio_start_rx)) {
                Ok(handle) => handle,
                Err(err) => {
                    wind_up(&stop_flag, &rx, handle);
                    return Err(EngineError::Failed(format!("the audio thread could not be started: {err}")));
                }
            };
        // The video thread reports the encoder facts, or its setup error.
        let setup = match rx.recv() {
            Ok(ThreadMessage::Setup(Ok(setup))) => setup,
            Ok(ThreadMessage::Setup(Err(err))) => {
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                let _ = handle.join();
                return Err(err);
            }
            Ok(ThreadMessage::Finished(..)) | Err(_) => {
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                let _ = handle.join();
                return Err(EngineError::Failed("the video thread ended before reporting setup".into()));
            }
        };
        // The audio thread reports its stream facts, or its setup error.
        let audio_setup = match audio_rx.recv() {
            Ok(AudioMessage::Setup(Ok(setup))) => setup,
            Ok(AudioMessage::Setup(Err(err))) | Ok(AudioMessage::Finished(Err(err))) => {
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                wind_up(&stop_flag, &rx, handle);
                return Err(err);
            }
            Ok(AudioMessage::Finished(Ok(_))) | Err(_) => {
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                wind_up(&stop_flag, &rx, handle);
                return Err(EngineError::Failed("the audio thread ended before reporting setup".into()));
            }
        };
        let mux = match build_muxer(&output_file, fps, setup, audio_setup.streams) {
            Ok((mux, streams)) => (mux, streams),
            Err(err) => {
                drop(start_tx);
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                wind_up(&stop_flag, &rx, handle);
                return Err(err);
            }
        };
        // One start instant for every stream: the video thread only begins
        // pacing, and the audio thread only begins filling, once they receive
        // their commands.
        let start_qpc = qpc_now();
        stats.started_qpc.store(start_qpc as u64, Ordering::Relaxed);
        // The audio thread works in 100 ns units, the video pacer in QPC ticks.
        let qpc_freq = qpc_frequency();
        let start_100ns = (start_qpc / qpc_freq) * 10_000_000 + (start_qpc % qpc_freq) * 10_000_000 / qpc_freq;
        let sent = start_tx.send(StartCommand { start_qpc, mux: mux.0.sender() });
        drop(start_tx);
        if sent.is_err() {
            wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
            wind_up(&stop_flag, &rx, handle);
            let _ = mux.0.finish();
            return Err(EngineError::Failed("the video thread ended before recording started".into()));
        }
        let audio_sent = audio_start_tx.send(AudioStart { start_qpc: start_100ns, mux: mux.0.sender(), streams: mux.1 });
        drop(audio_start_tx);
        if audio_sent.is_err() {
            wind_up(&stop_flag, &rx, handle);
            let _ = mux.0.finish();
            return Err(EngineError::Failed("the audio thread ended before recording started".into()));
        }
        let marks_warning = MarksWarning::default();
        let marks = TakeMarks::begin(&take_folder, &base_name, &output_file, fps, MediaClock::new(start_qpc, qpc_freq), marks_warning.clone());
        Ok(Self {
            stats,
            stop_flag,
            rx,
            handle: Some(handle),
            audio_rx,
            audio_handle: Some(audio_handle),
            mux: Some(mux.0),
            marks: Some(marks),
            marks_warning,
            fps,
        })
    }

    /// The take's marks, until the take is stopped.
    pub fn marks_mut(&mut self) -> Option<&mut TakeMarks> {
        self.marks.as_mut()
    }

    /// A handle to the take's last marks error, which stays readable after the take is stopped.
    pub fn marks_warning(&self) -> MarksWarning {
        self.marks_warning.clone()
    }

    /// Stops feeding the pacer; recording continues from the same frame on resume.
    pub fn pause(&self) {
        self.stats.paused.store(true, Ordering::Relaxed);
    }

    /// Continues recording with no gap in the frame indexes.
    pub fn resume(&self) {
        self.stats.paused.store(false, Ordering::Relaxed);
    }

    /// Sets a track's gain in dB right away; `index` counts the tracks of this take, in order.
    pub fn set_track_gain_db(&self, index: usize, db: f32) {
        if db.is_finite() {
            self.stats.set_track_gain_db(index, db);
        }
    }

    /// Mutes or unmutes a track right away; `index` counts the tracks of this take, in order.
    pub fn set_track_muted(&self, index: usize, muted: bool) {
        self.stats.set_track_muted(index, muted);
    }

    /// A snapshot of the live counters, safe to call from the UI thread at any time.
    pub fn stats(&self) -> StatsSnapshot {
        self.stats.snapshot()
    }

    /// Finishes the file and returns the summary of the take. The audio
    /// thread stops first, then the video thread, then the mux thread.
    pub fn stop(mut self) -> Result<Summary, EngineError> {
        self.stop_flag.store(true, Ordering::Relaxed);
        // The audio thread stops first so every stream and WAV ends at the
        // video's length.
        let audio = match self.audio_rx.recv() {
            Ok(AudioMessage::Finished(result)) => result,
            Ok(AudioMessage::Setup(_)) | Err(_) => Err(EngineError::Failed("the audio thread ended without reporting a summary".into())),
        };
        if let Some(handle) = self.audio_handle.take() {
            let _ = handle.join();
        }
        let mut outcome = Err(EngineError::Failed("the video thread ended without reporting a summary".into()));
        while let Ok(message) = self.rx.recv() {
            if let ThreadMessage::Finished(result) = message {
                outcome = result;
                break;
            }
        }
        // The video thread is done, so the queue holds its last packets.
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Finish the mux thread and fold its totals into the summary.
        if outcome.is_ok()
            && let Some(mux) = self.mux.take()
        {
            // The marks are finished before the trailer, so the chapters go into the file.
            if let (Some(marks), Ok(summary)) = (self.marks.take(), &outcome) {
                let wav_files = audio.as_ref().map(|finished| finished.wav_files.clone()).unwrap_or_default();
                let video_ms = u64::try_from(u128::from(summary.frames) * 1000 / u128::from(self.fps.max(1))).unwrap_or(u64::MAX);
                let mut durations_ms = vec![video_ms];
                durations_ms.extend(wav_files.iter().map(|wav| wav_length_ms(wav).unwrap_or(video_ms)));
                mux.set_chapters(marks.finish(&wav_files, &durations_ms));
            }
            outcome = match mux.finish() {
                Ok(mux_summary) => outcome.map(|mut summary| {
                    let seconds = summary.duration_ms as f64 / 1000.0;
                    summary.bytes_written = mux_summary.bytes_written;
                    summary.packets = mux_summary.packets;
                    summary.average_mbps = if seconds > 0.0 { summary.bytes_written as f64 * 8.0 / seconds / 1_000_000.0 } else { 0.0 };
                    summary
                }),
                Err(err) => Err(err.into()),
            };
        }
        // The WAV paths land in the summary; an audio failure surfaces only
        // when the video side finished.
        outcome = match (audio, outcome) {
            (Err(err), Ok(_)) => Err(err),
            (Ok(finished), Ok(mut summary)) => {
                summary.wav_files = finished.wav_files;
                Ok(summary)
            }
            (_, outcome) => outcome,
        };
        outcome
    }
}

/// Creates the muxer, adds the video stream then the audio streams, starts it,
/// and returns the mux thread plus the stream index per audio stream.
fn build_muxer(output_file: &Path, fps: u32, setup: VideoSetup, audio: Vec<AudioStreamSpec>) -> Result<(MuxThread, Vec<usize>), EngineError> {
    let mut muxer = Muxer::create(output_file, MuxContainer::Mkv)?;
    // The video stream is stream 0; the mixdown and per-track streams follow.
    muxer.add_video(setup.params.0, setup.time_base, (fps as i32, 1))?;
    let mut indexes = Vec::with_capacity(audio.len());
    for spec in audio {
        let index = muxer.add_audio(spec.params.0, spec.time_base, &spec.title, spec.default)?;
        indexes.push(index);
    }
    muxer.start()?;
    let thread = MuxThread::start(muxer)?;
    Ok((thread, indexes))
}

/// Shuts the audio thread down after a failed startup and waits for it.
fn wind_up_audio(stop_flag: &Arc<AtomicBool>, start_tx: std::sync::mpsc::Sender<AudioStart>, rx: &Receiver<AudioMessage>, handle: std::thread::JoinHandle<()>) {
    stop_flag.store(true, Ordering::Relaxed);
    drop(start_tx);
    while let Ok(message) = rx.recv() {
        if matches!(message, AudioMessage::Finished(_)) {
            break;
        }
    }
    let _ = handle.join();
}

/// Shuts the video thread down after a failed startup and waits for it.
fn wind_up(stop_flag: &Arc<AtomicBool>, rx: &Receiver<ThreadMessage>, handle: std::thread::JoinHandle<()>) {
    stop_flag.store(true, Ordering::Relaxed);
    while let Ok(message) = rx.recv() {
        if matches!(message, ThreadMessage::Finished(_)) {
            break;
        }
    }
    let _ = handle.join();
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        // A dropped session asks the video and audio threads to wind down; the
        // file is only finished properly through stop().
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// Maps a profile onto the media-io encoder configuration (phase 2: AMF only).
pub(super) fn encoder_config(profile: &Profile) -> EncoderConfig {
    EncoderConfig {
        codec: match profile.codec {
            Codec::Av1 => EncoderCodec::Av1,
            Codec::Hevc => EncoderCodec::Hevc,
            Codec::H264 => EncoderCodec::H264,
        },
        width: profile.width,
        height: profile.height,
        fps_num: profile.fps,
        fps_den: 1,
        rate_control: match profile.rate_control {
            RateControl::Qvbr { quality, max_mbps } => EncoderRateControl::Qvbr { quality, max_mbps: max_mbps as u32 },
            RateControl::Vbr { target_mbps, max_mbps } => EncoderRateControl::VbrPeak { target_mbps: target_mbps as u32, max_mbps: max_mbps as u32 },
            RateControl::Cbr { mbps } => EncoderRateControl::Cbr { mbps: mbps as u32 },
            RateControl::Cqp { qp_i, qp_p } => EncoderRateControl::Cqp { qp_i, qp_p },
        },
        preset: Preset::Balanced,
        keyframe_seconds: 2,
    }
}
