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
use media_io::encode::{ComboError, EncoderConfig, check_combo};
use media_io::mux::{MuxSender, MuxThread, Muxer, SendParameters};

use super::audio::{self, AudioMessage, AudioStart};
use super::cam::{self, CamCounters, CamHandle, CamSnapshot, CamSummary};
use super::clock::MediaClock;
use super::composite::CompositeBlock;
use super::marks::{MarksWarning, MkvChapter, TakeMarks, wav_length_ms};
use super::video;
use crate::config::{Container, Profile};

/// Why starting, pausing or stopping a recording failed.
#[derive(Debug, Error)]
pub enum EngineError {
    /// A capture call failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// A media IO call failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// The encoder rejected its configuration or could not be opened.
    #[error(transparent)]
    Encoder(#[from] media_io::encode::EncoderError),
    /// The profile's container cannot hold its codecs.
    #[error(transparent)]
    Combo(#[from] ComboError),
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
    /// The app this track records or leaves out; None for a device track. Such a track has no device of its own.
    pub app: Option<AppSpec>,
}

/// The app an app track records, or a desktop-except track leaves out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSpec {
    /// The executable's file name in lowercase, for example `discord.exe`.
    pub exe: String,
    /// The app's name for display.
    pub app_name: String,
    /// True for a desktop-except track.
    pub except: bool,
}

/// How many of an app's running instances a desktop-except capture leaves out; both zero for every other capture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Instances {
    /// The instances that are left out.
    pub excluded: usize,
    /// The instances that were running when the capture started.
    pub total: usize,
}

/// An app track gaining or losing its capture during a take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppEvent {
    /// The track, counted in track order.
    pub(super) track: usize,
    /// The app's name for display.
    pub(super) app_name: String,
    /// True for a desktop-except track, where attached means the app is being left out.
    pub(super) except: bool,
    /// True when the track attached, false when it detached.
    pub(super) attached: bool,
    /// The moment on the take's clock, in ms.
    pub(super) t_ms: u64,
    /// False for an app already attached when the take started, which gets no mark.
    pub(super) mark: bool,
}

/// Everything the video thread needs to record one take.
#[derive(Clone)]
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
    /// Bytes of audio data written to the WAV sidecars so far.
    pub(super) wav_bytes: AtomicU64,
    /// The counters of the camera ISO file, written by the camera thread.
    pub(super) cam: CamCounters,
    /// One entry per audio track, folded in by the audio thread.
    tracks: Vec<TrackSlot>,
    /// App attach and detach events the session has not applied yet, oldest first.
    app_events: Mutex<Vec<AppEvent>>,
}

impl Stats {
    pub(super) fn new(tracks: &[TrackSpec]) -> Self {
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
            wav_bytes: AtomicU64::new(0),
            cam: CamCounters::new(),
            tracks: tracks
                .iter()
                .map(|track| TrackSlot {
                    name: track.name.clone(),
                    meter: Mutex::new(TrackMeter { peak: 0.0, clipped: false }),
                    packets: AtomicU64::new(0),
                    overflows: AtomicU64::new(0),
                    device_lost: AtomicBool::new(false),
                    attached: AtomicBool::new(track.device.is_some()),
                    instances: Mutex::new(Instances::default()),
                    gain_db_bits: AtomicU32::new(track.gain_db.to_bits()),
                    muted: AtomicBool::new(track.muted),
                })
                .collect(),
            app_events: Mutex::new(Vec::new()),
        }
    }

    /// Queues an app event for the session.
    pub(super) fn push_app_event(&self, event: AppEvent) {
        if let Ok(mut events) = self.app_events.lock() {
            events.push(event);
        }
    }

    /// Takes the queued app events, oldest first.
    pub(super) fn take_app_events(&self) -> Vec<AppEvent> {
        self.app_events.lock().map(|mut events| std::mem::take(&mut *events)).unwrap_or_default()
    }

    /// Records whether a track has a running capture.
    pub(super) fn set_track_attached(&self, index: usize, attached: bool) {
        if let Some(slot) = self.tracks.get(index) {
            slot.attached.store(attached, Ordering::Relaxed);
        }
    }

    /// Records how many app instances a track's capture leaves out.
    pub(super) fn set_track_instances(&self, index: usize, instances: Instances) {
        if let Some(Ok(mut slot)) = self.tracks.get(index).map(|slot| slot.instances.lock()) {
            *slot = instances;
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
            cam: self.cam.snapshot(),
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
    /// The camera ISO file's counters, or none while the take has no camera file.
    pub cam: Option<CamSnapshot>,
    /// One entry per audio track, in track order.
    pub tracks: Vec<TrackSnapshot>,
}

/// The byte and queue counters of a take's files at one instant.
#[derive(Debug, Clone, Copy, Default)]
pub struct TakeCounters {
    /// Packet bytes the mux thread has written to the video file.
    pub mux_bytes: u64,
    /// Packet bytes the camera's mux thread has written to the camera file.
    pub cam_bytes: u64,
    /// Audio bytes written to the WAV sidecars.
    pub wav_bytes: u64,
    /// Packets waiting for the mux thread.
    pub queue_depth: usize,
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
    /// Whether the track has a running capture: its device, or its app while that runs.
    pub attached: bool,
    /// The app instances a desktop-except track leaves out.
    pub instances: Instances,
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
    /// Whether the track has a running capture.
    attached: AtomicBool,
    /// The app instances the capture leaves out.
    instances: Mutex<Instances>,
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
            attached: self.attached.load(Ordering::Relaxed),
            instances: self.instances.lock().map(|instances| *instances).unwrap_or_default(),
        }
    }
}

/// The highest output level one audio track reached over a whole take.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPeak {
    /// The track name.
    pub name: String,
    /// The peak in dBFS, held for the whole take; -100.0 for silence.
    pub peak_dbfs: f32,
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
    /// The average time the video thread spent per frame to convert, send and drain, in milliseconds.
    pub encode_avg_ms: f64,
    /// The 95th percentile of that per-frame time, in milliseconds.
    pub encode_p95_ms: f64,
    /// The peak level of each audio track over the whole take, in track order.
    pub track_peaks: Vec<TrackPeak>,
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

/// The take's session.json: what was recorded with, written at start and again at stop with the totals.
pub(super) struct SessionFile {
    /// Where the file is written: `{base_name}_session.json` in the take folder.
    path: PathBuf,
    /// The JSON written to the file.
    doc: serde_json::Value,
}

impl SessionFile {
    /// Describes the take from its config: the app, the profile and the audio tracks with their devices.
    pub(super) fn begin(cfg: &SessionConfig) -> Self {
        let tracks: Vec<serde_json::Value> = cfg.tracks.iter().map(track_entry).collect();
        let doc = serde_json::json!({
            "app": "CutMark Recorder",
            "version": env!("CARGO_PKG_VERSION"),
            "take": cfg.base_name,
            "started_at": chrono::Local::now().to_rfc3339(),
            "video_file": cfg.output_file.file_name().map(|name| name.to_string_lossy().into_owned()),
            "cam_file": serde_json::Value::Null,
            "cam_notice": serde_json::Value::Null,
            "profile": serde_json::to_value(&cfg.profile).unwrap_or(serde_json::Value::Null),
            "audio": {
                "codec": format!("{:?}", cfg.audio_codec),
                "mixdown": cfg.mixdown,
                "track_streams": cfg.track_streams,
                "wav_sidecars": cfg.wav_sidecars,
                "tracks": tracks,
            },
            "result": serde_json::Value::Null,
        });
        Self { path: cfg.take_folder.join(format!("{}_session.json", cfg.base_name)), doc }
    }

    /// Adds an attach or detach time to the app track's entry.
    fn app_event(&mut self, event: &AppEvent) {
        let key = if event.attached { "attached_ms" } else { "detached_ms" };
        if let Some(times) = self.doc.pointer_mut(&format!("/audio/tracks/{}/{key}", event.track)).and_then(serde_json::Value::as_array_mut) {
            times.push(event.t_ms.into());
        }
    }

    /// Lists the camera ISO file, or the reason the take has none although the profile asked for one.
    pub(super) fn set_cam(&mut self, file: Option<&Path>, notice: Option<&str>) {
        if let Some(doc) = self.doc.as_object_mut() {
            doc.insert("cam_file".to_owned(), serde_json::json!(file.and_then(Path::file_name).map(|name| name.to_string_lossy().into_owned())));
            doc.insert("cam_notice".to_owned(), serde_json::json!(notice));
        }
    }

    /// Adds the composite block: what the main video holds and how an editor rebuilds the look.
    pub(super) fn set_composite(&mut self, block: &CompositeBlock) {
        if let Some(doc) = self.doc.as_object_mut() {
            doc.insert("composite".to_owned(), serde_json::to_value(block).unwrap_or(serde_json::Value::Null));
        }
    }

    /// Records how many ms before the main video the camera file starts; negative when it starts later. It goes on the
    /// composite block's camera layer, or into a `camera_iso` object when the block has no such layer because the
    /// layout hides the camera. Only a saved replay has one. Call it after [`SessionFile::set_composite`].
    pub(super) fn set_camera_start_offset(&mut self, file: &str, offset_ms: i64) {
        let layers = self.doc.pointer_mut("/composite/layers").and_then(serde_json::Value::as_array_mut);
        let mut on_a_layer = false;
        for layer in layers.into_iter().flatten().filter_map(serde_json::Value::as_object_mut) {
            if layer.get("source").and_then(serde_json::Value::as_str) == Some("cam") {
                layer.insert("start_offset_ms".to_owned(), serde_json::json!(offset_ms));
                on_a_layer = true;
            }
        }
        if !on_a_layer && let Some(doc) = self.doc.as_object_mut() {
            doc.insert("camera_iso".to_owned(), serde_json::json!({ "file": file, "start_offset_ms": offset_ms }));
        }
    }

    /// Marks the take as a saved replay of the buffer: `length` is the replay length setting, `started_at` the clip's start.
    pub(super) fn set_replay(&mut self, started_at: chrono::DateTime<chrono::Local>, length: std::time::Duration) {
        if let Some(doc) = self.doc.as_object_mut() {
            doc.insert("started_at".to_owned(), serde_json::json!(started_at.to_rfc3339()));
            doc.insert("capture".to_owned(), serde_json::json!("replay"));
            doc.insert("replay_length_s".to_owned(), serde_json::json!(length.as_secs()));
        }
    }

    /// Writes the file through a temp file in the same folder, then renames it over the target.
    pub(super) fn write(&self) -> Result<(), String> {
        crate::config::write_json(&self.path, &self.doc).map_err(|err| err.to_string())
    }

    /// Fills in how the take ended: the totals, with the camera file's, when it finished, or the error and the counters so far when it did not.
    pub(super) fn finish(&mut self, outcome: &Result<Summary, EngineError>, so_far: &StatsSnapshot, cam: Option<&CamSummary>) {
        let result = match outcome {
            Ok(summary) => serde_json::json!({
                "finished": true,
                "ended_at": chrono::Local::now().to_rfc3339(),
                "duration_ms": summary.duration_ms,
                "frames": summary.frames,
                "duplicated_frames": summary.duplicated,
                "dropped_frames": summary.dropped,
                "bytes_written": summary.bytes_written,
                "average_mbps": summary.average_mbps,
                "packets": summary.packets,
                "wav_files": summary.wav_files.iter().filter_map(|wav| wav.file_name()).map(|name| name.to_string_lossy().into_owned()).collect::<Vec<_>>(),
                "cam": cam.map(|cam| serde_json::json!({
                    "frames": cam.frames,
                    "duplicated_frames": cam.duplicated,
                    "dropped_frames": cam.dropped,
                    "bytes_written": cam.bytes_written,
                    "duration_ms": cam.duration_ms,
                    "encode_avg_ms": cam.encode_avg_ms,
                    "encode_p95_ms": cam.encode_p95_ms,
                    "lost": cam.lost,
                })),
            }),
            Err(err) => serde_json::json!({
                "finished": false,
                "ended_at": chrono::Local::now().to_rfc3339(),
                "error": err.to_string(),
                "frames": so_far.frames,
                "duplicated_frames": so_far.duplicated,
                "dropped_frames": so_far.dropped,
                "bytes_written": so_far.bytes_written,
            }),
        };
        if let Some(doc) = self.doc.as_object_mut() {
            doc.insert("result".to_owned(), result);
        }
    }
}

/// A track's entry in session.json; an app track also names its app and has lists of its attach and detach times.
fn track_entry(track: &TrackSpec) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "name": track.name,
        "device_id": track.device_id,
        "device_name": track.device_name,
        "device_found": track.device.is_some(),
        "sample_rate": track.device.as_ref().map(|device| device.sample_rate),
        "channels": track.device.as_ref().map(|device| device.channels),
        "gain_db": track.gain_db,
        "muted": track.muted,
        "offset_ms": track.offset_ms,
        "in_mixdown": track.in_mixdown,
    });
    if let (Some(app), Some(fields)) = (&track.app, entry.as_object_mut()) {
        fields.insert("kind".to_owned(), if app.except { "desktop_except" } else { "app" }.into());
        fields.insert("exe".to_owned(), app.exe.clone().into());
        fields.insert("app_name".to_owned(), app.app_name.clone().into());
        fields.insert("attached_ms".to_owned(), serde_json::Value::Array(Vec::new()));
        fields.insert("detached_ms".to_owned(), serde_json::Value::Array(Vec::new()));
    }
    entry
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
    /// The take's session.json, rewritten at stop.
    session_file: SessionFile,
    /// The camera ISO thread, until stop() finishes it.
    cam: Option<CamHandle>,
    /// Why the profile's camera file is not being recorded, for a banner.
    cam_notice: Option<String>,
}

impl RecordingSession {
    /// Starts a recording: the video and audio threads set up, the session
    /// builds the muxer around their streams, then every stream starts from
    /// one instant.
    pub fn start(mut cfg: SessionConfig) -> Result<Self, EngineError> {
        resolve_source_size(&mut cfg)?;
        let container = cfg.profile.container;
        let encoder = encoder_config(&cfg.profile);
        encoder.validate().map_err(media_io::encode::EncoderError::Config)?;
        check_combo(container, encoder.codec, cfg.profile.height, cfg.audio_codec)?;
        media_io::init()?;
        let output_file = cfg.output_file.clone();
        let (take_folder, base_name) = (cfg.take_folder.clone(), cfg.base_name.clone());
        let fps = cfg.profile.fps;
        let mut session_file = SessionFile::begin(&cfg);
        // The camera file is decided before the config moves to the video thread.
        let cam_plan = cam::plan(&cfg);
        // So is what the composite block needs: the profile and the camera's size as they are at the start.
        let profile = cfg.profile.clone();
        let cam_size = crate::webcam::shared().map(|share| share.capture.format()).map(|format| (format.width, format.height));
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
        let mux = match build_muxer(&output_file, container, fps, setup, audio_setup.streams) {
            Ok((mux, streams)) => (mux, streams),
            Err(err) => {
                drop(start_tx);
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                wind_up(&stop_flag, &rx, handle);
                return Err(err);
            }
        };
        // The camera file has its own encoder and mux thread; a camera file that cannot start never stops the recording.
        let (cam, cam_notice) = match cam_plan {
            Ok(Some(plan)) => match cam::spawn(plan, Arc::clone(&stats), Arc::clone(&stop_flag)) {
                Ok(handle) => (Some(handle), None),
                Err(notice) => (None, Some(notice)),
            },
            Ok(None) => (None, None),
            Err(notice) => (None, Some(notice)),
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
            if let Some(cam) = cam {
                cam.abort();
            }
            return Err(EngineError::Failed("the video thread ended before recording started".into()));
        }
        let audio_sent = audio_start_tx.send(AudioStart { start_qpc: start_100ns, mux: mux.0.sender(), streams: mux.1 });
        drop(audio_start_tx);
        if audio_sent.is_err() {
            wind_up(&stop_flag, &rx, handle);
            let _ = mux.0.finish();
            if let Some(cam) = cam {
                cam.abort();
            }
            return Err(EngineError::Failed("the audio thread ended before recording started".into()));
        }
        // The camera thread starts from the same instant as every other stream.
        let (cam, cam_notice) = match cam {
            Some(handle) if !handle.begin(start_qpc) => {
                handle.abort();
                (None, Some("The camera file was skipped: its thread ended before recording started.".to_owned()))
            }
            other => (other, cam_notice),
        };
        if let Some(notice) = &cam_notice {
            log::warn!("{notice}");
        }
        session_file.set_cam(cam.as_ref().map(CamHandle::file), cam_notice.as_deref());
        let cam_file = cam.as_ref().map(|handle| file_name(handle.file()));
        session_file.set_composite(&CompositeBlock::new(&profile, cam_size, &file_name(&output_file), cam_file.as_deref()));
        let marks_warning = MarksWarning::default();
        let marks = TakeMarks::begin(&take_folder, &base_name, &output_file, fps, MediaClock::new(start_qpc, qpc_freq), marks_warning.clone());
        // A session.json that cannot be written is a warning, never a reason to stop the recording.
        if let Err(err) = session_file.write() {
            log::warn!("could not write the session file: {err}");
            stats.set_last_error(format!("The session file could not be written: {err}"));
        }
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
            session_file,
            cam,
            cam_notice,
        })
    }

    /// Why the profile's camera file is not being recorded in this take, if it asked for one.
    pub fn cam_notice(&self) -> Option<&str> {
        self.cam_notice.as_deref()
    }

    /// The take's marks, until the take is stopped.
    pub fn marks_mut(&mut self) -> Option<&mut TakeMarks> {
        self.marks.as_mut()
    }

    /// Moves the app tracks' attach and detach events into the marks and session.json.
    ///
    /// The controller calls this while a take runs, so the marks appear when they happen. True when a mark was added.
    pub fn apply_app_events(&mut self) -> bool {
        let mut marked = false;
        for event in self.stats.take_app_events() {
            self.session_file.app_event(&event);
            if event.mark
                && let Some(marks) = self.marks.as_mut()
            {
                marks.app(&event.app_name, event.except, event.attached, event.t_ms);
                marked = true;
            }
        }
        marked
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

    /// The bytes written to the take's files and the mux queue depth, safe to call from any thread at any time.
    pub fn counters(&self) -> TakeCounters {
        TakeCounters {
            mux_bytes: self.mux.as_ref().map_or(0, MuxThread::bytes_written),
            cam_bytes: self.stats.cam.bytes(),
            wav_bytes: self.stats.wav_bytes.load(Ordering::Relaxed),
            queue_depth: self.mux.as_ref().map_or(0, MuxThread::queue_depth),
        }
    }

    /// Finishes the file and returns the summary of the take; the camera file's totals are dropped, see [`RecordingSession::stop_with_cam`].
    pub fn stop(self) -> Result<Summary, EngineError> {
        self.stop_with_cam().0
    }

    /// Finishes the files and returns the summary of the take with the camera ISO file's totals, which are none when the
    /// take had no camera file or the file could not be finished. The audio thread stops first, then the video thread,
    /// the camera thread, and the mux thread.
    pub fn stop_with_cam(mut self) -> (Result<Summary, EngineError>, Option<CamSummary>) {
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
        // The audio thread is done, so these are the take's last app events, in before the marks are finished.
        self.apply_app_events();
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
        // The camera thread ends with the stop flag and finishes its own file; a camera file that fails here never fails the take.
        let cam = match self.cam.take().map(CamHandle::finish) {
            Some(Ok(summary)) => Some(summary),
            Some(Err(err)) => {
                log::warn!("the camera file could not be finished: {err}");
                self.stats.set_last_error(format!("The camera file could not be finished: {err}"));
                None
            }
            None => None,
        };
        // Finish the mux thread and fold its totals into the summary.
        if outcome.is_ok()
            && let Some(mux) = self.mux.take()
        {
            // The marks are finished before the trailer, so the chapters go into the file.
            if let (Some(marks), Ok(summary)) = (self.marks.take(), &outcome) {
                let wav_files = audio.as_ref().map(|finished| finished.wav_files.clone()).unwrap_or_default();
                let video_ms = u64::try_from(u128::from(summary.frames) * 1000 / u128::from(self.fps.max(1))).unwrap_or(u64::MAX);
                mux.set_chapters(finish_marks(marks, &wav_files, video_ms, cam.as_ref()));
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
                summary.track_peaks = self
                    .stats
                    .tracks
                    .iter()
                    .zip(&finished.track_peaks_dbfs)
                    .map(|(slot, peak_dbfs)| TrackPeak { name: slot.name.clone(), peak_dbfs: *peak_dbfs })
                    .collect();
                Ok(summary)
            }
            (_, outcome) => outcome,
        };
        // The totals go into session.json; a failed write is logged and shown as a warning, and the take is still finished.
        self.session_file.finish(&outcome, &self.stats.snapshot(), cam.as_ref());
        if let Err(err) = self.session_file.write() {
            log::warn!("could not update the session file: {err}");
            self.stats.set_last_error(format!("The session file could not be updated: {err}"));
        }
        (outcome, cam)
    }
}

/// Ends a take's marks once its other files are written: lists the WAV and camera files as media with their lengths,
/// and returns the MKV chapters.
pub(super) fn finish_marks(mut marks: TakeMarks, wav_files: &[PathBuf], video_ms: u64, cam: Option<&CamSummary>) -> Vec<MkvChapter> {
    let mut durations_ms = vec![video_ms];
    durations_ms.extend(wav_files.iter().map(|wav| wav_length_ms(wav).unwrap_or(video_ms)));
    if let Some(cam) = cam {
        marks.set_cam(cam.file.clone(), cam.duration_ms);
    }
    marks.finish(wav_files, &durations_ms)
}

/// A path's file name as text; empty when it has none.
pub(super) fn file_name(path: &Path) -> String {
    path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The encoder tag every take's container carries.
pub(super) const ENCODER_TAG: &str = concat!("CutMark Recorder ", env!("CARGO_PKG_VERSION"));

/// The take name a container carries as its title: the file name without its extension.
pub(super) fn take_title(file: &Path) -> String {
    file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Creates the muxer, adds the video stream then the audio streams, starts it,
/// and returns the mux thread plus the stream index per audio stream.
pub(super) fn build_muxer(
    output_file: &Path,
    container: Container,
    fps: u32,
    setup: VideoSetup,
    audio: Vec<AudioStreamSpec>,
) -> Result<(MuxThread, Vec<usize>), EngineError> {
    let mut muxer = Muxer::create(output_file, container)?;
    muxer.set_tags(&take_title(output_file), ENCODER_TAG);
    // The video stream is stream 0; the mixdown and per-track streams follow.
    muxer.add_video_titled(setup.params.0, setup.time_base, (fps as i32, 1), "Video")?;
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

/// Replaces a profile's Source output size (0x0) with the size of the captured monitor, rounded down to even numbers.
fn resolve_source_size(cfg: &mut SessionConfig) -> Result<(), EngineError> {
    if !cfg.profile.is_source_size() {
        return Ok(());
    }
    let monitor = capture::list_monitors()?.into_iter().nth(cfg.monitor).ok_or(EngineError::MonitorIndex(cfg.monitor))?;
    cfg.profile.width = u32::try_from(monitor.width).unwrap_or(0) & !1;
    cfg.profile.height = u32::try_from(monitor.height).unwrap_or(0) & !1;
    Ok(())
}

/// Maps a profile onto the media-io encoder configuration: codec, size, frame rate, rate control, preset and keyframe interval.
///
/// A profile with the Source size must have it resolved first.
pub(crate) fn encoder_config(profile: &Profile) -> EncoderConfig {
    EncoderConfig {
        codec: profile.codec.into(),
        width: profile.width,
        height: profile.height,
        fps_num: profile.fps,
        fps_den: 1,
        rate_control: profile.rate_control,
        preset: profile.preset,
        keyframe_secs: profile.keyframe_secs,
    }
}

#[cfg(test)]
mod app_track_tests {
    use super::*;

    #[test]
    fn session_json_names_an_app_track_and_lists_its_attach_and_detach_times() {
        let mut track = TrackSpec {
            name: "Discord".to_owned(),
            device: None,
            device_id: String::new(),
            device_name: String::new(),
            gain_db: 0.0,
            muted: false,
            offset_ms: 0,
            in_mixdown: true,
            app: Some(AppSpec { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned(), except: false }),
        };
        let app = track_entry(&track);
        if let Some(spec) = track.app.as_mut() {
            spec.except = true;
        }
        assert_eq!(track_entry(&track)["kind"], "desktop_except");
        track.app = None;
        assert!(track_entry(&track).get("kind").is_none());
        let mut file = SessionFile { path: PathBuf::new(), doc: serde_json::json!({ "audio": { "tracks": [app] } }) };
        for (attached, t_ms) in [(true, 0), (false, 4_000), (true, 9_000)] {
            file.app_event(&AppEvent { track: 0, app_name: "Discord".to_owned(), except: false, attached, t_ms, mark: t_ms > 0 });
        }
        let entry = &file.doc["audio"]["tracks"][0];
        assert_eq!((&entry["kind"], &entry["exe"], &entry["app_name"]), (&"app".into(), &"discord.exe".into(), &"Discord".into()));
        assert_eq!(entry["attached_ms"], serde_json::json!([0, 9_000]));
        assert_eq!(entry["detached_ms"], serde_json::json!([4_000]));
    }
}
