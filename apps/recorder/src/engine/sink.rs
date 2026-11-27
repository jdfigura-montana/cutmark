//! Where a take's encoded output goes: the one trait the video, audio and camera threads write through, and the
//! writer that puts it into the take's files.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use media_io::mux::MuxSender;
use media_io::{EncodedPacket, MediaError};

use super::audio::SidecarWav;
use super::session::EngineError;

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

    /// Says that no more camera packets come, so the camera file can be finished while the other streams go on.
    fn close_camera(&self);

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
    /// The camera file's queue; None when the take has no camera file, or once it is closed.
    camera: Mutex<Option<MuxSender>>,
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
    pub(super) fn new(main: MuxSender, audio_streams: Vec<usize>, camera: Option<MuxSender>, wav_plan: WavPlan) -> Self {
        Self { main, audio_streams, camera: Mutex::new(camera), wav_plan, wavs: Mutex::new(Vec::new()) }
    }
}

impl PacketSink for TakeWriter {
    fn video(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        self.main.send(VIDEO_STREAM, packet)
    }

    fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        lock(&self.camera).as_ref().map_or(Ok(()), |camera| camera.send(CAM_STREAM, packet))
    }

    fn close_camera(&self) {
        lock(&self.camera).take();
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

    /// A sink that writes nothing and counts what it receives: packets per output, frames for PCM.
    #[derive(Default)]
    pub(in super::super) struct FakeSink {
        received: Mutex<BTreeMap<Output, u64>>,
    }

    impl FakeSink {
        pub(in super::super) fn received(&self) -> BTreeMap<Output, u64> {
            lock(&self.received).clone()
        }

        fn add(&self, output: Output, count: u64) {
            *lock(&self.received).entry(output).or_default() += count;
        }
    }

    impl PacketSink for FakeSink {
        fn video(&self, _packet: EncodedPacket) -> Result<(), MediaError> {
            self.add(Output::Video, 1);
            Ok(())
        }

        fn camera(&self, _packet: EncodedPacket) -> Result<(), MediaError> {
            self.add(Output::Camera, 1);
            Ok(())
        }

        fn close_camera(&self) {}

        fn audio(&self, slot: usize, _packet: EncodedPacket) -> Result<(), EngineError> {
            self.add(Output::Audio(slot), 1);
            Ok(())
        }

        fn open_pcm(&self) -> Result<(), EngineError> {
            Ok(())
        }

        fn pcm(&self, track: usize, _start: u64, samples: &[f32]) -> Result<u64, EngineError> {
            self.add(Output::Pcm(track), samples.len() as u64 / 2);
            Ok(0)
        }

        fn flush_pcm(&self) {}

        fn close_pcm(&self) -> Vec<PathBuf> {
            Vec::new()
        }
    }
}
