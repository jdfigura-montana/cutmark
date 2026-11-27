//! Where a take's encoded output goes: the one trait the video, audio and camera threads write through, and the
//! writer that puts it into the take's files.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ffmpeg_next::Packet;
use ffmpeg_next::packet::Flags;
use media_io::mux::{MuxSender, MuxThread};
use media_io::{EncodedPacket, MediaError};

use super::audio::{OUT_CHANNELS, SidecarWav};
use super::session::{EngineError, Stats};
use super::take_gate::{FramePlan, TakeGate};

/// The muxer stream index of the main video stream, which the session adds first.
const VIDEO_STREAM: usize = 0;

/// The muxer stream index of the camera's video stream, the only stream of its file.
const CAM_STREAM: usize = 0;

/// Receives every encoded output of the stream threads.
///
/// The video methods belong to the video thread, the camera methods to the camera thread, and the audio and PCM
/// methods to the audio thread.
pub(super) trait PacketSink: Send + Sync {
    /// Takes one main video packet.
    fn video(&self, packet: EncodedPacket) -> Result<(), MediaError>;

    /// Takes one camera ISO packet.
    fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError>;

    /// Says that no more camera packets come, and finishes the camera file while the other streams go on. Returns how
    /// the file ended; None when there is no camera file, or it was already finished.
    fn close_camera(&self) -> Option<CamFileEnd>;

    /// Packet bytes written to the camera file so far.
    fn camera_bytes(&self) -> u64;

    /// Takes one packet of the audio stream in `slot`: the mixdown first when there is one, then one per track stream.
    fn audio(&self, slot: usize, packet: EncodedPacket) -> Result<(), EngineError>;

    /// Gets ready for PCM; the audio thread calls it once, before the first block.
    fn open_pcm(&self) -> Result<(), EngineError>;

    /// Takes interleaved stereo samples of a track that start at the 48 kHz frame `start` on the take's timeline.
    /// Returns the bytes of audio data written.
    fn pcm(&self, track: usize, start: u64, samples: &[f32]) -> Result<u64, EngineError>;

    /// Makes the PCM taken so far safe against a crash.
    fn flush_pcm(&self);

    /// Ends the PCM output and returns the files written, which a second call no longer lists.
    fn close_pcm(&self) -> Vec<PathBuf>;
}

/// How a take's camera file ended.
pub(super) struct CamFileEnd {
    /// The camera file.
    pub(super) file: PathBuf,
    /// The bytes written to it, or why it could not be finished.
    pub(super) bytes: Result<u64, String>,
}

/// A camera file being written.
pub(super) struct CamFile {
    /// The camera file.
    pub(super) file: PathBuf,
    /// The thread that writes it.
    pub(super) mux: MuxThread,
}

/// The open camera file and the writer's end of its mux queue.
struct CamOutput {
    file: CamFile,
    sender: MuxSender,
}

/// What the take writer needs to create the WAV sidecars.
pub(super) struct WavPlan {
    /// Whether every track is written as a WAV sidecar.
    pub(super) enabled: bool,
    /// The take folder the sidecars are written into.
    pub(super) take_folder: PathBuf,
    /// The base name for the sidecar file names.
    pub(super) base_name: String,
    /// The track names, in track order.
    pub(super) track_names: Vec<String>,
}

/// Writes a take: packets into the mux threads of the video and camera files, PCM into the WAV sidecars.
///
/// A mux thread finishes only once every sender of its queue is gone. The main file's sender lasts as long as the
/// writer, so every holder of the writer must let it go before that mux thread is finished.
pub(super) struct TakeWriter {
    /// The main file's queue.
    main: MuxSender,
    /// The main file's stream index per audio slot.
    audio_streams: Vec<usize>,
    /// The camera file; None when the take has none, or once it is closed.
    camera: Mutex<Option<CamOutput>>,
    wav_plan: WavPlan,
    /// One sidecar per track in track order while they are open, otherwise none.
    wavs: Mutex<Vec<SidecarWav>>,
}

/// Locks a mutex, also after a thread panicked while holding it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl TakeWriter {
    /// A writer over the take's mux queues; the WAV sidecars are created by [`PacketSink::open_pcm`].
    pub(super) fn new(main: MuxSender, audio_streams: Vec<usize>, camera: Option<CamFile>, wav_plan: WavPlan) -> Self {
        let camera = camera.map(|file| CamOutput { sender: file.mux.sender(), file });
        Self { main, audio_streams, camera: Mutex::new(camera), wav_plan, wavs: Mutex::new(Vec::new()) }
    }
}

impl PacketSink for TakeWriter {
    fn video(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        self.main.send(VIDEO_STREAM, packet)
    }

    fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        lock(&self.camera).as_ref().map_or(Ok(()), |camera| camera.sender.send(CAM_STREAM, packet))
    }

    fn close_camera(&self) -> Option<CamFileEnd> {
        let CamOutput { file, sender } = lock(&self.camera).take()?;
        // The mux thread finishes only once every sender of its queue is gone.
        drop(sender);
        let bytes = file.mux.finish().map(|summary| summary.bytes_written).map_err(|err| err.to_string());
        Some(CamFileEnd { file: file.file, bytes })
    }

    fn camera_bytes(&self) -> u64 {
        lock(&self.camera).as_ref().map_or(0, |camera| camera.file.mux.bytes_written())
    }

    fn audio(&self, slot: usize, packet: EncodedPacket) -> Result<(), EngineError> {
        let stream = self.audio_streams.get(slot).copied().ok_or(EngineError::Failed("an audio stream index is missing".into()))?;
        Ok(self.main.send(stream, packet)?)
    }

    fn open_pcm(&self) -> Result<(), EngineError> {
        let plan = &self.wav_plan;
        if !plan.enabled {
            return Ok(());
        }
        let _ = std::fs::create_dir_all(&plan.take_folder);
        let wavs = plan
            .track_names
            .iter()
            .enumerate()
            .map(|(index, name)| SidecarWav::create(&plan.take_folder, &plan.base_name, index, name))
            .collect::<Result<Vec<_>, _>>()?;
        *lock(&self.wavs) = wavs;
        Ok(())
    }

    fn pcm(&self, track: usize, _start: u64, samples: &[f32]) -> Result<u64, EngineError> {
        // A recording has no gaps, so each block follows the one before it and its position is not needed.
        match lock(&self.wavs).get_mut(track) {
            Some(wav) => {
                wav.write(samples)?;
                Ok(samples.len() as u64 * SidecarWav::BYTES_PER_SAMPLE)
            }
            None => Ok(0),
        }
    }

    fn flush_pcm(&self) {
        for wav in lock(&self.wavs).iter_mut() {
            wav.flush();
        }
    }

    fn close_pcm(&self) -> Vec<PathBuf> {
        let wavs = std::mem::take(&mut *lock(&self.wavs));
        wavs.into_iter().map(SidecarWav::close).collect()
    }
}

/// The one sink of a running capture, with up to two listeners. The replay rings get everything, on the capture's own
/// clock. The take gets what its gate lets through, restamped to the take's clock. With neither everything is dropped.
///
/// The take's writer is never called under the lock, apart from creating its WAV files, so a slow disk does not make
/// a pause or another stream wait.
pub(super) struct CaptureOutput {
    /// Where the bytes and audio frames the take received are counted.
    stats: Arc<Stats>,
    /// How many audio streams the capture encodes.
    audio_streams: u64,
    routing: Mutex<Routing>,
}

/// The gate and the take, changed together.
struct Routing {
    gate: TakeGate,
    take: Option<Arc<dyn PacketSink>>,
    /// Whether the audio thread is ready for PCM, so a take that joins later must open its own.
    pcm_open: bool,
    /// The replay rings, while the buffer is on.
    rings: Option<Arc<dyn PacketSink>>,
    /// The highest linear level of each track while the take was on, pauses included.
    peaks: Vec<f32>,
    /// How the take's camera file ended, when the camera thread closed it.
    camera_end: Option<(PathBuf, Result<u64, String>)>,
}

/// What is left of a take that has left its capture.
pub(super) struct LeftTake {
    /// The take's sink; None when no take was on.
    pub(super) take: Option<Arc<dyn PacketSink>>,
    /// The highest level of each track during the take, in dBFS, in track order; -100.0 for silence.
    pub(super) peaks_dbfs: Vec<f32>,
    /// How the take's camera file ended, if the camera thread closed it first.
    pub(super) camera_end: Option<CamFileEnd>,
}

/// Where one output goes.
struct Routed<T> {
    rings: Option<Arc<dyn PacketSink>>,
    /// The take and what its gate chose.
    take: Option<(Arc<dyn PacketSink>, T)>,
}

impl CaptureOutput {
    /// The output of a capture with no take.
    pub(super) fn new(stats: Arc<Stats>, audio_streams: usize, gate: TakeGate) -> Self {
        let routing = Routing { gate, take: None, pcm_open: false, rings: None, peaks: Vec::new(), camera_end: None };
        Self { stats, audio_streams: audio_streams as u64, routing: Mutex::new(routing) }
    }

    /// Starts or stops feeding the replay rings. A take is not affected.
    pub(super) fn set_rings(&self, rings: Option<Arc<dyn PacketSink>>) {
        lock(&self.routing).rings = rings;
    }

    /// See [`TakeGate::drained`].
    pub(super) fn drained(&self, camera: bool, tracks: usize) -> bool {
        lock(&self.routing).gate.drained(camera, self.audio_streams as usize, tracks)
    }

    /// Lets the take go, so its files can be finished, and makes the gate ready for the next one.
    pub(super) fn leave_take(&self) -> LeftTake {
        let mut routing = lock(&self.routing);
        routing.gate.end_take();
        let peaks = std::mem::take(&mut routing.peaks);
        LeftTake {
            take: routing.take.take(),
            peaks_dbfs: peaks.into_iter().map(|peak| if peak > 0.0 { 20.0 * peak.log10() } else { -100.0 }).collect(),
            camera_end: routing.camera_end.take().map(|(file, bytes)| CamFileEnd { file, bytes }),
        }
    }

    /// Gives the capture its take before any stream has started: the take holds everything from the start.
    pub(super) fn begin_take(&self, take: Arc<dyn PacketSink>) {
        let mut routing = lock(&self.routing);
        routing.gate.begin_with_capture();
        routing.take = Some(take);
    }

    /// Starts a take on the running encoders, pressed at `press_qpc`, and returns the QPC time its recording time 0
    /// maps to: that of the frame the next forced keyframe is on.
    pub(super) fn start_take(&self, take: Arc<dyn PacketSink>, press_qpc: i64) -> Result<i64, EngineError> {
        let mut routing = lock(&self.routing);
        if routing.take.is_some() {
            return Err(EngineError::Failed("a take is already running".into()));
        }
        if routing.pcm_open {
            take.open_pcm()?;
        }
        routing.take = Some(take);
        let frame = routing.gate.open(press_qpc);
        Ok(routing.gate.frame_qpc(frame))
    }

    /// Pauses the take; the capture goes on.
    pub(super) fn pause(&self, press_qpc: i64) {
        lock(&self.routing).gate.close(press_qpc);
    }

    /// Resumes the take on a forced keyframe.
    pub(super) fn resume(&self, press_qpc: i64) {
        let mut routing = lock(&self.routing);
        if routing.take.is_some() {
            routing.gate.open(press_qpc);
        }
    }

    /// See [`TakeGate::video_frame`].
    pub(super) fn video_frame(&self, frame: u64) -> FramePlan {
        lock(&self.routing).gate.video_frame(frame)
    }

    /// See [`TakeGate::video_failed`].
    pub(super) fn video_failed(&self, frame: u64) {
        lock(&self.routing).gate.video_failed(frame);
    }

    /// See [`TakeGate::camera_frame`].
    pub(super) fn camera_frame(&self, tick: u64) -> FramePlan {
        lock(&self.routing).gate.camera_frame(tick)
    }

    /// See [`TakeGate::camera_failed`].
    pub(super) fn camera_failed(&self, tick: u64) {
        lock(&self.routing).gate.camera_failed(tick);
    }

    /// See [`TakeGate::take_ms`].
    pub(super) fn take_ms(&self, capture_100ns: i64) -> u64 {
        lock(&self.routing).gate.take_ms(capture_100ns)
    }

    /// Runs the gate, which follows every stream also while there is no take, and returns the rings with what the gate
    /// chose for the take, if there is one and it wants the output.
    fn route<T>(&self, choose: impl FnOnce(&mut TakeGate) -> Option<T>) -> Routed<T> {
        let mut routing = lock(&self.routing);
        let chosen = choose(&mut routing.gate);
        Routed { rings: routing.rings.clone(), take: routing.take.clone().zip(chosen) }
    }

    fn take(&self) -> Option<Arc<dyn PacketSink>> {
        lock(&self.routing).take.clone()
    }
}

/// A packet moved back by `offset` on the take's clock. One that does not move is passed on as it is.
fn moved(packet: EncodedPacket, offset: i64, duration: i64) -> Option<EncodedPacket> {
    if offset == 0 {
        return Some(packet);
    }
    copied(&packet, offset, duration)
}

/// A copy of a packet's payload and key flag, with its time stamps moved back by `offset`.
fn copied(packet: &EncodedPacket, offset: i64, duration: i64) -> Option<EncodedPacket> {
    let mut copy = Packet::copy(packet.data()?);
    copy.set_pts(packet.pts().map(|pts| pts - offset));
    copy.set_dts(packet.dts().map(|dts| dts - offset));
    copy.set_duration(duration);
    if packet.is_key() {
        copy.set_flags(Flags::KEY);
    }
    Some(EncodedPacket::from_raw_packet(copy))
}

/// Gives the rings their packet through `send`, and returns the one the take still needs: the rings get a copy only
/// when the take wants the packet too.
fn feed_rings<E>(
    rings: Option<Arc<dyn PacketSink>>,
    packet: EncodedPacket,
    take_wants_it: bool,
    send: impl FnOnce(&dyn PacketSink, EncodedPacket) -> Result<(), E>,
) -> Result<Option<EncodedPacket>, E> {
    let Some(rings) = rings else {
        return Ok(take_wants_it.then_some(packet));
    };
    if !take_wants_it {
        return send(rings.as_ref(), packet).map(|()| None);
    }
    if let Some(copy) = copied(&packet, 0, 0) {
        send(rings.as_ref(), copy)?;
    }
    Ok(Some(packet))
}

impl PacketSink for CaptureOutput {
    fn video(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        let (pts, key) = (packet.pts().unwrap_or(-1), packet.is_key());
        let Routed { rings, take } = self.route(|gate| gate.video_packet(pts, key));
        let packet = feed_rings(rings, packet, take.is_some(), |rings, packet| rings.video(packet))?;
        let Some(((take, offset), packet)) = take.zip(packet) else {
            return Ok(());
        };
        self.stats.bytes_written.fetch_add(packet.size() as u64, Ordering::Relaxed);
        // A video frame lasts one tick of its encoder's time base.
        moved(packet, offset, 1).map_or(Ok(()), |packet| take.video(packet))
    }

    fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        let (pts, key) = (packet.pts().unwrap_or(-1), packet.is_key());
        let Routed { rings, take } = self.route(|gate| gate.camera_packet(pts, key));
        let packet = feed_rings(rings, packet, take.is_some(), |rings, packet| rings.camera(packet))?;
        let Some(((take, offset), packet)) = take.zip(packet) else {
            return Ok(());
        };
        moved(packet, offset, 1).map_or(Ok(()), |packet| take.camera(packet))
    }

    fn close_camera(&self) -> Option<CamFileEnd> {
        let end = self.take().and_then(|take| take.close_camera())?;
        lock(&self.routing).camera_end = Some((end.file.clone(), end.bytes.clone()));
        Some(end)
    }

    fn camera_bytes(&self) -> u64 {
        self.take().map_or(0, |take| take.camera_bytes())
    }

    fn audio(&self, slot: usize, packet: EncodedPacket) -> Result<(), EngineError> {
        // A priming packet starts before time 0, so a missing pts cannot be told apart from it by its sign.
        let Some(pts) = packet.pts() else {
            return Ok(());
        };
        let Routed { rings, take } = self.route(|gate| gate.audio_packet(slot, pts));
        let packet = feed_rings(rings, packet, take.is_some(), |rings, packet| rings.audio(slot, packet))?;
        let Some(((take, (out, duration)), packet)) = take.zip(packet) else {
            return Ok(());
        };
        moved(packet, pts - out, duration).map_or(Ok(()), |packet| take.audio(slot, packet))
    }

    fn open_pcm(&self) -> Result<(), EngineError> {
        let mut routing = lock(&self.routing);
        routing.pcm_open = true;
        routing.take.as_ref().map_or(Ok(()), |take| take.open_pcm())
    }

    fn pcm(&self, track: usize, start: u64, samples: &[f32]) -> Result<u64, EngineError> {
        let channels = usize::from(OUT_CHANNELS);
        let frames = samples.len() / channels;
        let peak = samples.iter().fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        let (rings, take) = {
            let mut routing = lock(&self.routing);
            let parts = routing.gate.pcm(start, frames);
            routing.gate.pcm_seen(track, start + frames as u64);
            let take = routing.take.clone();
            if take.is_some() {
                if routing.peaks.len() <= track {
                    routing.peaks.resize(track + 1, 0.0);
                }
                routing.peaks[track] = routing.peaks[track].max(peak);
            }
            (routing.rings.clone(), take.map(|take| (take, parts)))
        };
        if let Some(rings) = rings {
            rings.pcm(track, start, samples)?;
        }
        let Some((take, parts)) = take else {
            return Ok(0);
        };
        let mut bytes = 0;
        for (Range { start: from, end: to }, take_start) in parts {
            if track == 0 {
                self.stats.audio_frames.fetch_add((to - from) as u64 * self.audio_streams, Ordering::Relaxed);
            }
            bytes += take.pcm(track, take_start, &samples[from * channels..to * channels])?;
        }
        Ok(bytes)
    }

    fn flush_pcm(&self) {
        if let Some(take) = self.take() {
            take.flush_pcm();
        }
    }

    fn close_pcm(&self) -> Vec<PathBuf> {
        self.take().map_or_else(Vec::new, |take| take.close_pcm())
    }
}

#[cfg(test)]
#[path = "take_gate_tests.rs"]
mod gate_tests;

#[cfg(test)]
pub(super) mod fake {
    use std::collections::BTreeMap;

    use super::*;

    /// One output of a take.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(in super::super) enum Output {
        Video,
        Camera,
        Audio(usize),
        Pcm(usize),
    }

    /// A sink that writes nothing and keeps what it receives: packets per output with their pts, frames for PCM with
    /// the sample each block starts at.
    #[derive(Default)]
    pub(in super::super) struct FakeSink {
        received: Mutex<BTreeMap<Output, u64>>,
        stamps: Mutex<BTreeMap<Output, Vec<i64>>>,
    }

    impl FakeSink {
        pub(in super::super) fn received(&self) -> BTreeMap<Output, u64> {
            lock(&self.received).clone()
        }

        /// The pts of every packet of an output, or the start of every PCM block, in the order received.
        pub(in super::super) fn stamps(&self, output: Output) -> Vec<i64> {
            lock(&self.stamps).get(&output).cloned().unwrap_or_default()
        }

        fn add(&self, output: Output, count: u64, stamp: i64) {
            *lock(&self.received).entry(output).or_default() += count;
            lock(&self.stamps).entry(output).or_default().push(stamp);
        }
    }

    impl PacketSink for FakeSink {
        fn video(&self, packet: EncodedPacket) -> Result<(), MediaError> {
            self.add(Output::Video, 1, packet.pts().unwrap_or_default());
            Ok(())
        }

        fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError> {
            self.add(Output::Camera, 1, packet.pts().unwrap_or_default());
            Ok(())
        }

        fn close_camera(&self) -> Option<CamFileEnd> {
            None
        }

        fn camera_bytes(&self) -> u64 {
            0
        }

        fn audio(&self, slot: usize, packet: EncodedPacket) -> Result<(), EngineError> {
            self.add(Output::Audio(slot), 1, packet.pts().unwrap_or_default());
            Ok(())
        }

        fn open_pcm(&self) -> Result<(), EngineError> {
            Ok(())
        }

        fn pcm(&self, track: usize, start: u64, samples: &[f32]) -> Result<u64, EngineError> {
            self.add(Output::Pcm(track), samples.len() as u64 / 2, start as i64);
            Ok(0)
        }

        fn flush_pcm(&self) {}

        fn close_pcm(&self) -> Vec<PathBuf> {
            Vec::new()
        }
    }
}
