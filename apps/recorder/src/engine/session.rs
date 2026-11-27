//! The recording session: session config, live stats, and the start, pause,
//! resume and stop calls. A session is a capture part ([`Capture`]) with the take part ([`Take`]) recording it, the
//! replay buffer's rings, or both.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

use ::marks::NewMark;
use capture::{AudioDevice, CaptureError, qpc_frequency, qpc_now};
use media_io::AudioCodec;
use media_io::MediaError;
use media_io::encode::{ComboError, EncoderConfig, check_combo};
use media_io::mux::{MuxThread, Muxer, SendParameters};

use super::cam::{self, CamCounters, CamSnapshot, CamSummary};
use super::capture_part::{Capture, CaptureStreams, PreparedCapture};
use super::clock::MediaClock;
use super::composite::CompositeBlock;
use super::input_track::{self, InputTrack};
use super::marks::{self, MarksWarning, MkvChapter, TakeMarks, wav_length_ms};
use super::replay::{CamStream, ReplayRings, ReplaySnapshot, RingSink};
use super::share::{SharedCapture, Sharing};
use super::sink::{CaptureOutput, PacketSink};
use super::take_part::{Take, TakeFiles};
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
    /// The QPC time of the first main video frame the latest take received; 0 until it has one.
    pub(super) first_video_qpc: AtomicU64,
    /// Whether the take is paused, for the snapshot; the capture goes on and its output leaves the pause out.
    pub(super) paused: AtomicBool,
    /// The text of the last frame error, if any.
    pub(super) last_error: Mutex<Option<String>>,
    /// Total audio frames handed to the audio encoders across all streams.
    pub(super) audio_frames: AtomicU64,
    /// Bytes of audio data written to the WAV sidecars so far.
    pub(super) wav_bytes: AtomicU64,
    /// The counters of the camera ISO file, written by the camera thread.
    pub(super) cam: CamCounters,
    /// Where the frame, byte and audio counters stood when the take started: they count for the whole capture, and a
    /// take is the difference.
    base: TakeBase,
    /// The video thread's average and 95th percentile time per frame in ms, over the whole capture.
    encode_ms: Mutex<(f64, f64)>,
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
            first_video_qpc: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            last_error: Mutex::new(None),
            audio_frames: AtomicU64::new(0),
            wav_bytes: AtomicU64::new(0),
            cam: CamCounters::new(),
            base: TakeBase::default(),
            encode_ms: Mutex::new((0.0, 0.0)),
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

    /// Starts the counters of a take that joins a running capture: they count from here on.
    pub(super) fn begin_take(&self) {
        let move_up = |base: &AtomicU64, now: &AtomicU64| base.store(now.load(Ordering::Relaxed), Ordering::Relaxed);
        move_up(&self.base.frames, &self.frames);
        move_up(&self.base.duplicated, &self.duplicated);
        move_up(&self.base.dropped, &self.dropped);
        move_up(&self.base.bytes_written, &self.bytes_written);
        move_up(&self.base.audio_frames, &self.audio_frames);
        move_up(&self.base.wav_bytes, &self.wav_bytes);
        self.cam.begin_take();
        // Reading a meter resets it, so the take does not show peaks from before it.
        for slot in &self.tracks {
            let _ = slot.snapshot();
        }
        self.paused.store(false, Ordering::Relaxed);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = None;
        }
    }

    /// The take's frames, repeated frames and dropped frames.
    pub(super) fn take_frames(&self) -> (u64, u64, u64) {
        (since(&self.frames, &self.base.frames), since(&self.duplicated, &self.base.duplicated), since(&self.dropped, &self.base.dropped))
    }

    /// The WAV bytes of the take.
    fn take_wav_bytes(&self) -> u64 {
        since(&self.wav_bytes, &self.base.wav_bytes)
    }

    /// Records the video thread's frame times.
    pub(super) fn set_encode_times(&self, average_ms: f64, p95_ms: f64) {
        if let Ok(mut slot) = self.encode_ms.lock() {
            *slot = (average_ms, p95_ms);
        }
    }

    /// The video thread's average and 95th percentile time per frame in ms.
    pub(super) fn encode_times(&self) -> (f64, f64) {
        self.encode_ms.lock().map(|slot| *slot).unwrap_or_default()
    }

    /// The apps the app tracks have a capture of right now, as attach events at time 0 that get no mark.
    pub(super) fn attached_apps(&self, tracks: &[TrackSpec]) -> Vec<AppEvent> {
        tracks
            .iter()
            .zip(&self.tracks)
            .enumerate()
            .filter(|(_, (_, slot))| slot.attached.load(Ordering::Relaxed))
            .filter_map(|(index, (track, _))| {
                let app = track.app.as_ref()?;
                Some(AppEvent { track: index, app_name: app.app_name.clone(), except: app.except, attached: true, t_ms: 0, mark: false })
            })
            .collect()
    }

    /// The track names, in track order.
    pub(super) fn track_names(&self) -> impl Iterator<Item = &str> {
        self.tracks.iter().map(|slot| slot.name.as_str())
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
        let (frames, duplicated, dropped) = self.take_frames();
        StatsSnapshot {
            frames,
            duplicated,
            dropped,
            frames_received: self.frames_received.load(Ordering::Relaxed),
            bytes_written: since(&self.bytes_written, &self.base.bytes_written),
            started_qpc: self.started_qpc.load(Ordering::Relaxed),
            paused: self.paused.load(Ordering::Relaxed),
            last_error: self.last_error.lock().ok().and_then(|slot| slot.clone()),
            audio_frames: since(&self.audio_frames, &self.base.audio_frames),
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

/// A counter's growth since its base.
fn since(counter: &AtomicU64, base: &AtomicU64) -> u64 {
    counter.load(Ordering::Relaxed).saturating_sub(base.load(Ordering::Relaxed))
}

/// The capture's counters as they were when the take started; zero for a take that started with the capture.
#[derive(Default)]
struct TakeBase {
    frames: AtomicU64,
    duplicated: AtomicU64,
    dropped: AtomicU64,
    bytes_written: AtomicU64,
    audio_frames: AtomicU64,
    wav_bytes: AtomicU64,
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
    /// Where the thread's packets go, and what decides the frames its take wants as keyframes.
    pub(super) output: Arc<CaptureOutput>,
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
    pub(super) fn app_event(&mut self, event: &AppEvent) {
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

/// How a take ended: its totals, and its camera file's, which are none when it had no camera file or the file could not
/// be finished.
pub(super) type StoppedTake = (Result<Summary, EngineError>, Option<CamSummary>);

/// The Recorder's session: the one capture, shared by the take and the replay buffer.
pub(super) type RecordingSession = Sharing<LiveCapture>;

/// A running capture and what every listener of it is made from.
pub(super) struct LiveCapture {
    capture: Capture,
    /// What the capture was opened with, its output size resolved. Every take on it has these streams.
    cfg: SessionConfig,
    /// The open camera's size as it was at the capture's start, for the composite block.
    camera_size: Option<(u32, u32)>,
    /// The replay buffer, while it is on.
    rings: Option<Arc<RingSink>>,
    /// The input logger of a capture whose profile tracks the cursor.
    input: Option<InputTrack>,
    /// Why the cursor is burned in although the profile tracks it.
    input_failure: Option<String>,
    /// Toasts about the input track that the controller has not picked up.
    input_notices: Vec<String>,
}

/// A capture ready to start, with what a [`LiveCapture`] keeps of it.
struct Opened {
    prepared: PreparedCapture,
    cfg: SessionConfig,
    camera_size: Option<(u32, u32)>,
    input: Option<InputTrack>,
    input_failure: Option<String>,
}

impl Opened {
    /// Checks the config and opens the capture's encoders.
    fn new(mut cfg: SessionConfig) -> Result<Self, EngineError> {
        resolve_source_size(&mut cfg)?;
        let encoder = encoder_config(&cfg.profile);
        encoder.validate().map_err(media_io::encode::EncoderError::Config)?;
        check_combo(cfg.profile.container, encoder.codec, cfg.profile.height, cfg.audio_codec)?;
        media_io::init()?;
        let cam_plan = cam::plan(&cfg);
        // The composite block needs the camera's size as it is at the start.
        let camera_size = crate::webcam::shared().map(|share| share.capture.format()).map(|format| (format.width, format.height));
        let stats = Arc::new(Stats::new(&cfg.tracks));
        // The logger starts before the encoders open: if it cannot, the capture still draws the cursor.
        let plan = input_track::plan_cursor(cfg.profile.cursor, || InputTrack::start_logger(cfg.monitor, &cfg.profile));
        cfg.cursor = plan.burn_in;
        let prepared = PreparedCapture::open(&cfg, cam_plan, stats)?;
        Ok(Self { prepared, cfg, camera_size, input: plan.track, input_failure: plan.failure })
    }
}

/// How far back a replay clip can start: the buffer's length and the keyframe interval it can run over.
fn replay_reach(cfg: &SessionConfig, length: Duration) -> Duration {
    length + Duration::from_secs_f32(cfg.profile.keyframe_secs.max(0.0))
}

/// An empty replay buffer for the streams of a capture opened with `cfg`.
fn new_rings(cfg: &SessionConfig, streams: &CaptureStreams, camera_size: Option<(u32, u32)>, length: Duration) -> Arc<RingSink> {
    let camera = streams.camera.as_ref().map(|camera| CamStream {
        params: SendParameters(camera.params.0.clone()),
        time_base: camera.time_base,
        fps: camera.fps,
        size: camera_size.unwrap_or_default(),
    });
    Arc::new(RingSink::new(ReplayRings::new(cfg, streams.video_setup(), camera, streams.audio_specs(), length)))
}

impl LiveCapture {
    /// The capture's config with the tracks' gains and mutes as they are now; they may have changed since it started.
    fn live_config(&self) -> SessionConfig {
        let mut cfg = self.cfg.clone();
        for (index, track) in cfg.tracks.iter_mut().enumerate() {
            if let Some((gain_db, muted)) = self.capture.stats().track_controls(index) {
                (track.gain_db, track.muted) = (gain_db, muted);
            }
        }
        cfg
    }

    /// Gives the input track a take that began at `start_qpc`, when the cursor is tracked.
    fn begin_input_take(&self, take_folder: &Path, base_name: &str, start_qpc: i64) {
        if let Some(input) = &self.input {
            input.core().begin_take(&input_track::events_path(take_folder, base_name), start_qpc);
        }
    }

    /// Tells the input track about the take's first video frame once there is one, which creates the take's events
    /// file. Calls after that do nothing.
    fn pass_first_frame(stats: &Stats, input: &InputTrack) {
        let first = stats.first_video_qpc.load(Ordering::Relaxed);
        if first != 0 {
            input.core().first_frame(first as i64);
        }
    }

    /// The time of `qpc` on the capture's clock in ms. That clock never pauses, so neither does the replay buffer's.
    fn buffer_ms(&self, qpc: i64) -> u64 {
        MediaClock::new(self.capture.start_qpc(), qpc_frequency()).media_ms(qpc)
    }
}

impl SharedCapture for LiveCapture {
    type Config = SessionConfig;
    type Take = Take;
    type Stopped = StoppedTake;

    /// The capture's encoders open, the take builds its files around their streams, then every stream starts from one
    /// instant with the take already on it.
    fn open_for_take(cfg: SessionConfig) -> Result<(Self, Take), EngineError> {
        let Opened { mut prepared, cfg, camera_size, input, input_failure } = Opened::new(cfg)?;
        let session_file = SessionFile::begin(&cfg);
        let files = match TakeFiles::open(&cfg, prepared.streams()) {
            Ok(files) => files,
            Err(err) => {
                prepared.abort();
                return Err(err);
            }
        };
        // With no file to write, a recording does not encode the camera at all.
        if let Some(notice) = files.camera_error() {
            prepared.drop_camera(notice.to_owned());
        }
        let capture = match prepared.begin(Some(files.sink()), None) {
            Ok(capture) => capture,
            Err(err) => {
                files.discard();
                return Err(err);
            }
        };
        let take = files.record(&cfg, session_file, &capture, camera_size, capture.start_qpc());
        let input_notices = input_failure.iter().cloned().collect();
        let live = Self { capture, cfg, camera_size, rings: None, input, input_failure, input_notices };
        live.begin_input_take(&live.cfg.take_folder, &live.cfg.base_name, live.capture.start_qpc());
        Ok((live, take))
    }

    fn open_for_replay(cfg: SessionConfig, length: Duration) -> Result<Self, EngineError> {
        let Opened { prepared, cfg, camera_size, input, input_failure } = Opened::new(cfg)?;
        if let Some(input) = &input {
            input.core().set_replay(Some(replay_reach(&cfg, length)));
        }
        let rings = new_rings(&cfg, prepared.streams(), camera_size, length);
        let capture = prepared.begin(None, Some(Arc::clone(&rings) as Arc<dyn PacketSink>))?;
        let input_notices = input_failure.iter().cloned().collect();
        Ok(Self { capture, cfg, camera_size, rings: Some(rings), input, input_failure, input_notices })
    }

    /// Only the take's file names come from `cfg`: its streams, tracks and profile are the running capture's.
    fn join_take(&mut self, cfg: SessionConfig, press_qpc: i64) -> Result<Take, EngineError> {
        let mut take_cfg = self.live_config();
        take_cfg.output_file = cfg.output_file;
        take_cfg.take_folder = cfg.take_folder;
        take_cfg.base_name = cfg.base_name;
        let take = Take::start_on(&self.capture, &take_cfg, self.camera_size, press_qpc)?;
        // The take's recording time 0, which `start_on` left in the counters.
        let start_qpc = self.capture.stats().started_qpc.load(Ordering::Relaxed) as i64;
        self.input_notices.extend(self.input_failure.clone());
        self.begin_input_take(&take_cfg.take_folder, &take_cfg.base_name, start_qpc);
        Ok(take)
    }

    fn leave_take(&mut self, take: Take, press_qpc: i64) -> StoppedTake {
        let end_qpc = self.capture.output().pause(press_qpc);
        let end = self.capture.leave_take(press_qpc);
        if let Some(input) = &self.input {
            // A very short take's first frame may only have left the encoder just now.
            Self::pass_first_frame(self.capture.stats(), input);
            input.core().finish_take(end_qpc.unwrap_or(press_qpc));
        }
        take.finish(end, self.capture.stats())
    }

    /// The capture stops first, then the take's mux thread.
    fn close(self, take: Option<Take>) -> Option<StoppedTake> {
        let stats = Arc::clone(self.capture.stats());
        let end = self.capture.stop();
        if let Some(input) = &self.input {
            Self::pass_first_frame(&stats, input);
            input.core().finish_take(qpc_now());
        }
        take.map(|take| take.finish(end, &stats))
    }

    fn add_rings(&mut self, length: Duration) -> Result<(), EngineError> {
        let rings = new_rings(&self.cfg, self.capture.streams(), self.camera_size, length);
        self.capture.output().set_rings(Some(Arc::clone(&rings) as Arc<dyn PacketSink>));
        self.rings = Some(rings);
        if let Some(input) = &self.input {
            input.core().set_replay(Some(replay_reach(&self.cfg, length)));
        }
        Ok(())
    }

    fn drop_rings(&mut self) {
        self.capture.output().set_rings(None);
        self.rings = None;
        if let Some(input) = &self.input {
            input.core().set_replay(None);
        }
    }

    fn is_dead(&self) -> bool {
        self.capture.is_dead()
    }
}

impl Sharing<LiveCapture> {
    /// Starts a recording right now.
    pub(super) fn start(&mut self, cfg: SessionConfig) -> Result<(), EngineError> {
        self.start_take(cfg, qpc_now())
    }

    /// Finishes the take's files and returns the summary of the take; the camera file's totals are dropped, see
    /// [`RecordingSession::stop_with_cam`].
    pub(super) fn stop(&mut self) -> Result<Summary, EngineError> {
        self.stop_with_cam().0
    }

    /// Finishes the take's files and returns the summary of the take with the camera ISO file's totals. The capture
    /// goes on while the replay buffer is on.
    pub(super) fn stop_with_cam(&mut self) -> StoppedTake {
        let press_qpc = qpc_now();
        // The buffer gets the marks of the app events that are still queued.
        self.apply_app_events();
        self.stop_take(press_qpc).unwrap_or_else(|| (Err(EngineError::Failed("no take is running".into())), None))
    }

    /// What the running capture was opened with.
    pub(super) fn capture_config(&self) -> Option<&SessionConfig> {
        self.capture().map(|live| &live.cfg)
    }

    /// Why the profile's camera file is not being recorded in this take, if it asked for one.
    pub(super) fn cam_notice(&self) -> Option<&str> {
        self.take().and_then(|take| take.camera_notice.as_deref())
    }

    /// The take's marks, until the take is stopped.
    pub(super) fn marks_mut(&mut self) -> Option<&mut TakeMarks> {
        self.take_mut().and_then(|take| take.marks.as_mut())
    }

    /// Keeps a mark made at `press_qpc` in the replay buffer; `build` gets the time on the buffer's clock in ms.
    /// Nothing happens while the buffer is off.
    pub(super) fn buffer_mark(&self, press_qpc: i64, build: impl FnOnce(u64) -> NewMark) {
        let Some(live) = self.capture() else { return };
        let Some(rings) = &live.rings else { return };
        rings.rings().push_mark(build(live.buffer_ms(press_qpc)));
    }

    /// What a save pressed at `press_qpc` writes: a snapshot of the replay buffer and the config of its streams. The
    /// buffer and a running take go on untouched. None while the buffer is off or holds no video yet.
    pub(super) fn replay_snapshot(&self, press_qpc: i64) -> Option<(ReplaySnapshot, SessionConfig)> {
        let live = self.capture()?;
        let mut snapshot = live.rings.as_ref()?.rings().snapshot(live.buffer_ms(press_qpc))?;
        if let Some(input) = &live.input {
            let (from, to) = snapshot.qpc_span(live.capture.start_qpc(), qpc_frequency());
            snapshot.input = Some(input.core().clip(from, to));
        }
        Some((snapshot, live.live_config()))
    }

    /// Moves the app tracks' attach and detach events into the replay buffer's marks, and into the take's marks and
    /// session.json while one runs.
    ///
    /// The controller calls this while a capture runs, so the marks appear when they happen. True when the take got a mark.
    pub(super) fn apply_app_events(&mut self) -> bool {
        let Some(live) = self.capture() else { return false };
        let events = live.capture.stats().take_app_events();
        for event in events.iter().filter(|event| event.mark) {
            let (kind, label) = marks::app_text(&event.app_name, event.except, event.attached);
            // An event's own time is on the take's clock; the buffer takes the time it is picked up.
            self.buffer_mark(qpc_now(), |t_ms| marks::system_mark(kind, &label, t_ms));
        }
        self.take_mut().is_some_and(|take| take.apply_app_events(events))
    }

    /// Passes the take's first video frame on to the input track, and returns the toasts it has raised since the last
    /// call.
    pub(super) fn take_input_notices(&mut self) -> Vec<String> {
        let Some(live) = self.capture_mut() else { return Vec::new() };
        let mut notices = std::mem::take(&mut live.input_notices);
        if let Some(input) = &live.input {
            LiveCapture::pass_first_frame(live.capture.stats(), input);
            notices.extend(input.core().take_notices());
        }
        notices
    }

    /// A handle to the take's last marks error, which stays readable after the take is stopped.
    pub(super) fn marks_warning(&self) -> MarksWarning {
        self.take().map(|take| take.marks_warning.clone()).unwrap_or_default()
    }

    /// Pauses the take at the moment of the press; the capture and the replay buffer go on.
    pub(super) fn pause(&self, press_qpc: i64) {
        let Some(live) = self.capture() else { return };
        live.capture.stats().paused.store(true, Ordering::Relaxed);
        let from = live.capture.output().pause(press_qpc);
        if let Some(input) = &live.input {
            input.core().pause(from.unwrap_or(press_qpc));
        }
    }

    /// Continues the take on a forced keyframe, with no gap in its frame indexes.
    pub(super) fn resume(&self, press_qpc: i64) {
        let Some(live) = self.capture() else { return };
        if let Some(to) = live.capture.output().resume(press_qpc)
            && let Some(input) = &live.input
        {
            input.core().resume(to);
        }
        live.capture.stats().paused.store(false, Ordering::Relaxed);
    }

    /// Sets a track's gain in dB right away; `index` counts the tracks of the capture, in order.
    pub(super) fn set_track_gain_db(&self, index: usize, db: f32) {
        if db.is_finite()
            && let Some(live) = self.capture()
        {
            live.capture.stats().set_track_gain_db(index, db);
        }
    }

    /// Mutes or unmutes a track right away; `index` counts the tracks of the capture, in order.
    pub(super) fn set_track_muted(&self, index: usize, muted: bool) {
        if let Some(live) = self.capture() {
            live.capture.stats().set_track_muted(index, muted);
        }
    }

    /// A snapshot of the live counters; all zero without a capture.
    pub(super) fn stats(&self) -> StatsSnapshot {
        self.capture().map_or_else(|| Stats::new(&[]).snapshot(), |live| live.capture.stats().snapshot())
    }

    /// The bytes written to the take's files and the mux queue depth.
    pub(super) fn counters(&self) -> TakeCounters {
        let (Some(live), Some(take)) = (self.capture(), self.take()) else { return TakeCounters::default() };
        let stats = live.capture.stats();
        TakeCounters {
            mux_bytes: take.mux.bytes_written(),
            cam_bytes: stats.cam.bytes(),
            wav_bytes: stats.take_wav_bytes(),
            queue_depth: take.mux.queue_depth(),
        }
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
