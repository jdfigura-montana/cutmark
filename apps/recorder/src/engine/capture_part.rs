//! The capture part of a session: the capture, the compositor and the encoders on their threads. It runs on one clock
//! from its start to its stop, with or without a take; a take joins it through its [`CaptureOutput`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use capture::{qpc_frequency, qpc_now};
use media_io::mux::SendParameters;

use super::audio::{self, AudioMessage, AudioStart, AudioSummary};
use super::cam::{self, CamHandle, CamPlan, CamSetup, CamSummary};
use super::session::{AudioStreamSpec, EngineError, SessionConfig, StartCommand, Stats, Summary, ThreadMessage, VideoSetup};
use super::sink::{CaptureOutput, LeftTake, PacketSink};
use super::take_gate::TakeGate;
use super::video;

/// How long a take that leaves a running capture waits for the encoders to hand out its last output.
const DRAIN_LIMIT: Duration = Duration::from_secs(3);

/// How often that wait looks again.
const DRAIN_POLL: Duration = Duration::from_millis(5);

/// What a take needs to know about the running encoders to open its files.
pub(super) struct CaptureStreams {
    /// The main video's frames per second.
    pub(super) fps: u32,
    /// The main video encoder.
    pub(super) video: VideoSetup,
    /// The audio encoders in stream order.
    pub(super) audio: Vec<AudioStreamSpec>,
    /// The camera encoder; None when no camera is being encoded.
    pub(super) camera: Option<CamSetup>,
}

impl CaptureStreams {
    /// A copy of the main video encoder's facts.
    pub(super) fn video_setup(&self) -> VideoSetup {
        VideoSetup { params: SendParameters(self.video.params.0.clone()), time_base: self.video.time_base }
    }

    /// A copy of the audio encoders' facts, in stream order.
    pub(super) fn audio_specs(&self) -> Vec<AudioStreamSpec> {
        self.audio
            .iter()
            .map(|spec| AudioStreamSpec {
                params: SendParameters(spec.params.0.clone()),
                time_base: spec.time_base,
                title: spec.title.clone(),
                default: spec.default,
            })
            .collect()
    }
}

/// The threads of a capture.
struct Threads {
    stats: Arc<Stats>,
    stop_flag: Arc<AtomicBool>,
    rx: Receiver<ThreadMessage>,
    video: Option<JoinHandle<()>>,
    audio_rx: Receiver<AudioMessage>,
    audio: Option<JoinHandle<()>>,
    camera: Option<CamHandle>,
}

impl Drop for Threads {
    fn drop(&mut self) {
        // A dropped capture asks its threads to wind down; a take's files are only finished properly through stop().
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

/// A capture whose encoders are open and whose threads wait for the start.
pub(super) struct PreparedCapture {
    threads: Threads,
    start_tx: Sender<StartCommand>,
    audio_start_tx: Sender<AudioStart>,
    streams: CaptureStreams,
    camera_notice: Option<String>,
}

/// A running capture.
pub(super) struct Capture {
    threads: Threads,
    streams: CaptureStreams,
    /// Why a camera that should be encoded is not.
    camera_notice: Option<String>,
    output: Arc<CaptureOutput>,
    /// The QPC value of the instant every stream started from.
    start_qpc: i64,
}

/// How the capture's part of a take ended.
pub(super) struct CaptureEnd {
    /// The video thread's totals of the take.
    pub(super) video: Result<Summary, EngineError>,
    /// The audio thread's totals of the take.
    pub(super) audio: Result<AudioSummary, EngineError>,
    /// The camera file's totals; None without a camera.
    pub(super) camera: Option<Result<CamSummary, String>>,
}

impl PreparedCapture {
    /// Starts the video, audio and camera threads and waits until their encoders are open. A camera that cannot be
    /// encoded never stops the capture.
    pub(super) fn open(cfg: &SessionConfig, camera_plan: Result<Option<CamPlan>, String>, stats: Arc<Stats>) -> Result<Self, EngineError> {
        let fps = cfg.profile.fps;
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel::<StartCommand>();
        let thread_stats = Arc::clone(&stats);
        let thread_stop = Arc::clone(&stop_flag);
        let job = audio::AudioJob { tracks: cfg.tracks.clone(), codec: cfg.audio_codec, mixdown: cfg.mixdown, track_streams: cfg.track_streams };
        let video_cfg = cfg.clone();
        let handle = std::thread::Builder::new()
            .name("video".to_owned())
            .spawn(move || video::run(video_cfg, thread_stats, thread_stop, tx, start_rx))
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
                    drop(start_tx);
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
        let audio_error = match audio_rx.recv() {
            Ok(AudioMessage::Setup(Ok(audio_setup))) => Ok(audio_setup),
            Ok(AudioMessage::Setup(Err(err))) | Ok(AudioMessage::Finished(Err(err))) => Err(err),
            Ok(AudioMessage::Finished(Ok(_))) | Err(_) => Err(EngineError::Failed("the audio thread ended before reporting setup".into())),
        };
        let audio_setup = match audio_error {
            Ok(audio_setup) => audio_setup,
            Err(err) => {
                drop(start_tx);
                wind_up_audio(&stop_flag, audio_start_tx, &audio_rx, audio_handle);
                wind_up(&stop_flag, &rx, handle);
                return Err(err);
            }
        };
        // The camera has its own encoder thread.
        let (camera, camera_setup, camera_notice) = match camera_plan {
            Ok(Some(plan)) => match cam::spawn(plan, Arc::clone(&stats), Arc::clone(&stop_flag)) {
                Ok((handle, camera_setup)) => (Some(handle), Some(camera_setup), None),
                Err(notice) => (None, None, Some(notice)),
            },
            Ok(None) => (None, None, None),
            Err(notice) => (None, None, Some(notice)),
        };
        let threads = Threads { stats, stop_flag, rx, video: Some(handle), audio_rx, audio: Some(audio_handle), camera };
        let streams = CaptureStreams { fps, video: setup, audio: audio_setup.streams, camera: camera_setup };
        Ok(Self { threads, start_tx, audio_start_tx, streams, camera_notice })
    }

    /// The facts of the open encoders.
    pub(super) fn streams(&self) -> &CaptureStreams {
        &self.streams
    }

    /// Ends the camera thread before the start, for a first take that could not open its camera file.
    pub(super) fn drop_camera(&mut self, notice: String) {
        if let Some(camera) = self.threads.camera.take() {
            camera.abort();
        }
        self.streams.camera = None;
        self.camera_notice = Some(notice);
    }

    /// Ends a capture that never started, and waits for its threads.
    pub(super) fn abort(self) {
        let Self { mut threads, start_tx, audio_start_tx, .. } = self;
        drop(start_tx);
        if let Some(handle) = threads.audio.take() {
            wind_up_audio(&threads.stop_flag, audio_start_tx, &threads.audio_rx, handle);
        }
        if let Some(handle) = threads.video.take() {
            wind_up(&threads.stop_flag, &threads.rx, handle);
        }
        if let Some(camera) = threads.camera.take() {
            camera.abort();
        }
    }

    /// Releases the threads from one start instant: the video thread only begins pacing, and the audio thread only
    /// begins filling, once they receive their commands. A `take` or the replay `rings` given here hold everything from
    /// that instant.
    pub(super) fn begin(self, take: Option<Arc<dyn PacketSink>>, rings: Option<Arc<dyn PacketSink>>) -> Result<Capture, EngineError> {
        let Self { mut threads, start_tx, audio_start_tx, mut streams, mut camera_notice } = self;
        let start_qpc = qpc_now();
        threads.stats.started_qpc.store(start_qpc as u64, Ordering::Relaxed);
        // The audio thread works in 100 ns units, the video pacer in QPC ticks.
        let qpc_freq = qpc_frequency();
        let start_100ns = (start_qpc / qpc_freq) * 10_000_000 + (start_qpc % qpc_freq) * 10_000_000 / qpc_freq;
        let camera_rate = streams.camera.as_ref().map(|camera| camera.fps);
        let gate = TakeGate::new(start_qpc, qpc_freq, streams.fps, camera_rate);
        let output = Arc::new(CaptureOutput::new(Arc::clone(&threads.stats), streams.audio.len(), gate));
        output.set_rings(rings);
        if let Some(take) = take {
            output.begin_take(take);
        }
        let video_ended = start_tx.send(StartCommand { start_qpc, output: Arc::clone(&output) }).is_err();
        drop(start_tx);
        let audio_ended = !video_ended && audio_start_tx.send(AudioStart { start_qpc: start_100ns, output: Arc::clone(&output) }).is_err();
        if video_ended || audio_ended {
            // The take must be free of the capture before its files can be finished.
            drop(output.leave_take());
            if let Some(handle) = threads.audio.take() {
                wind_up_audio(&threads.stop_flag, audio_start_tx, &threads.audio_rx, handle);
            } else {
                drop(audio_start_tx);
            }
            if let Some(handle) = threads.video.take() {
                wind_up(&threads.stop_flag, &threads.rx, handle);
            }
            if let Some(camera) = threads.camera.take() {
                camera.abort();
            }
            let thread = if video_ended { "video" } else { "audio" };
            return Err(EngineError::Failed(format!("the {thread} thread ended before recording started")));
        }
        drop(audio_start_tx);
        // The camera thread starts from the same instant as every other stream.
        if let Some(camera) = threads.camera.take() {
            if camera.begin(StartCommand { start_qpc, output: Arc::clone(&output) }) {
                threads.camera = Some(camera);
            } else {
                camera.abort();
                streams.camera = None;
                camera_notice = Some("The camera file was skipped: its thread ended before recording started.".to_owned());
            }
        }
        Ok(Capture { threads, streams, camera_notice, output, start_qpc })
    }
}

impl Capture {
    /// The facts of the running encoders.
    pub(super) fn streams(&self) -> &CaptureStreams {
        &self.streams
    }

    /// Why a camera that should be encoded is not.
    pub(super) fn camera_notice(&self) -> Option<&str> {
        self.camera_notice.as_deref()
    }

    /// The live counters.
    pub(super) fn stats(&self) -> &Arc<Stats> {
        &self.threads.stats
    }

    /// Where the takes of this capture join it, pause and resume.
    pub(super) fn output(&self) -> &CaptureOutput {
        &self.output
    }

    /// The QPC value of the capture clock's 0, which the replay buffer's times count from.
    pub(super) fn start_qpc(&self) -> i64 {
        self.start_qpc
    }

    /// Whether a stream thread ended although the capture was not stopped.
    pub(super) fn is_dead(&self) -> bool {
        self.threads.video.as_ref().is_none_or(JoinHandle::is_finished) || self.threads.audio.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Ends the take, pressed at `press_qpc`, while the threads go on: waits until the encoders have handed out all of
    /// it, lets the take's sink go and builds the totals the threads would report at a stop.
    pub(super) fn leave_take(&self, press_qpc: i64) -> CaptureEnd {
        let stats = self.stats();
        self.output.pause(press_qpc);
        let tracks = stats.track_names().count();
        let camera_on = self.streams.camera.is_some();
        let deadline = Instant::now() + DRAIN_LIMIT;
        while !self.output.drained(camera_on && stats.cam.is_running(), tracks) {
            if self.is_dead() || Instant::now() >= deadline {
                log::warn!("the take was closed before every stream reached its end");
                break;
            }
            std::thread::sleep(DRAIN_POLL);
        }
        let LeftTake { take, peaks_dbfs, camera_end } = self.output.leave_take();
        let mut wav_files = Vec::new();
        let mut camera = None;
        if let Some(take) = take {
            // A stream thread may still be writing with its own hold on the sink; the files close after it.
            while Arc::strong_count(&take) > 1 && Instant::now() < deadline {
                std::thread::sleep(DRAIN_POLL);
            }
            wav_files = take.close_pcm();
            camera = camera_end.or_else(|| take.close_camera()).zip(self.streams.camera.as_ref()).map(|(end, setup)| {
                let (encode_avg_ms, encode_p95_ms) = stats.cam.encode_times();
                end.bytes.map(|bytes_written| CamSummary {
                    file: end.file,
                    frames: 0,
                    duplicated: 0,
                    dropped: 0,
                    bytes_written,
                    duration_ms: 0,
                    fps: f64::from(setup.fps.0) / f64::from(setup.fps.1.max(1)),
                    encode_avg_ms,
                    encode_p95_ms,
                    lost: stats.cam.is_lost(),
                })
            });
        }
        let (encode_avg_ms, encode_p95_ms) = stats.encode_times();
        let video = Ok(Summary {
            file: PathBuf::new(),
            frames: 0,
            duplicated: 0,
            dropped: 0,
            bytes_written: 0,
            packets: Vec::new(),
            duration_ms: 0,
            average_mbps: 0.0,
            wav_files: Vec::new(),
            encode_avg_ms,
            encode_p95_ms,
            track_peaks: Vec::new(),
        });
        let audio = Ok(AudioSummary { wav_files, track_peaks_dbfs: peaks_dbfs.clone() });
        self.count_take(CaptureEnd { video, audio, camera }, peaks_dbfs)
    }

    /// Puts the take's own counts and track peaks into the totals, because the threads count for the whole capture.
    fn count_take(&self, mut end: CaptureEnd, peaks_dbfs: Vec<f32>) -> CaptureEnd {
        let stats = self.stats();
        if let Ok(summary) = &mut end.video {
            (summary.frames, summary.duplicated, summary.dropped) = stats.take_frames();
            summary.duration_ms = summary.frames * 1000 / u64::from(self.streams.fps.max(1));
        }
        if let Ok(audio) = &mut end.audio {
            audio.track_peaks_dbfs = peaks_dbfs;
            audio.track_peaks_dbfs.resize(stats.track_names().count(), -100.0);
        }
        if let (Some(Ok(summary)), Some(setup)) = (&mut end.camera, &self.streams.camera) {
            (summary.frames, summary.duplicated, summary.dropped) = stats.cam.take_frames();
            summary.duration_ms = summary.frames.saturating_mul(1000).saturating_mul(u64::from(setup.fps.1)) / u64::from(setup.fps.0.max(1));
        }
        end
    }

    /// Ends the threads and waits for them. The audio thread stops first, so every stream and WAV ends at the video's
    /// length, then the video thread and the camera thread. Then the capture lets the take go.
    pub(super) fn stop(mut self) -> CaptureEnd {
        let threads = &mut self.threads;
        threads.stop_flag.store(true, Ordering::Relaxed);
        let audio = match threads.audio_rx.recv() {
            Ok(AudioMessage::Finished(result)) => result,
            Ok(AudioMessage::Setup(_)) | Err(_) => Err(EngineError::Failed("the audio thread ended without reporting a summary".into())),
        };
        if let Some(handle) = threads.audio.take() {
            let _ = handle.join();
        }
        let mut video = Err(EngineError::Failed("the video thread ended without reporting a summary".into()));
        while let Ok(message) = threads.rx.recv() {
            if let ThreadMessage::Finished(result) = message {
                video = result;
                break;
            }
        }
        // The video thread is done, so the queue holds its last packets.
        if let Some(handle) = threads.video.take() {
            let _ = handle.join();
        }
        // The camera thread ends with the stop flag and finishes the camera file.
        let camera = threads.camera.take().map(CamHandle::finish);
        let left = self.output.leave_take();
        self.count_take(CaptureEnd { video, audio, camera }, left.peaks_dbfs)
    }
}

/// Shuts the audio thread down after a failed startup and waits for it.
fn wind_up_audio(stop_flag: &AtomicBool, start_tx: Sender<AudioStart>, rx: &Receiver<AudioMessage>, handle: JoinHandle<()>) {
    stop_flag.store(true, Ordering::Relaxed);
    drop(start_tx);
    while let Ok(message) = rx.recv() {
        if matches!(message, AudioMessage::Finished(_)) {
            break;
        }
    }
    let _ = handle.join();
}

/// Shuts the video thread down after a failed startup and waits for it; its start sender must be dropped already.
fn wind_up(stop_flag: &AtomicBool, rx: &Receiver<ThreadMessage>, handle: JoinHandle<()>) {
    stop_flag.store(true, Ordering::Relaxed);
    while let Ok(message) = rx.recv() {
        if matches!(message, ThreadMessage::Finished(_)) {
            break;
        }
    }
    let _ = handle.join();
}
