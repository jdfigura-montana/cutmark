//! The take part of a session: the files one take writes (the video file, the camera file, the WAV sidecars, the
//! marks and session.json). Its sink joins a capture, either from the capture's start or on running encoders.

use std::path::PathBuf;
use std::sync::Arc;

use capture::qpc_frequency;
use media_io::mux::{MuxThread, SendParameters};

use super::cam;
use super::capture_part::{Capture, CaptureEnd, CaptureStreams};
use super::clock::MediaClock;
use super::composite::CompositeBlock;
use super::marks::{MarksWarning, TakeMarks};
use super::session::{
    AppEvent, AudioStreamSpec, EngineError, SessionConfig, SessionFile, Stats, Summary, TrackPeak, VideoSetup, build_muxer, file_name, finish_marks,
};
use super::sink::{CamFile, PacketSink, TakeWriter, WavPlan};

/// A take's open files, before it records.
pub(super) struct TakeFiles {
    mux: MuxThread,
    writer: Arc<TakeWriter>,
    /// The camera file, when one was opened.
    camera_file: Option<PathBuf>,
    /// Why a camera file that should have been opened was not.
    camera_error: Option<String>,
}

impl TakeFiles {
    /// Opens the video file and, when the capture encodes a camera, the camera file. A camera file that cannot be
    /// opened never stops the take.
    pub(super) fn open(cfg: &SessionConfig, streams: &CaptureStreams) -> Result<Self, EngineError> {
        let container = cfg.profile.container;
        let video = VideoSetup { params: SendParameters(streams.video.params.0.clone()), time_base: streams.video.time_base };
        let audio = streams
            .audio
            .iter()
            .map(|spec| AudioStreamSpec {
                params: SendParameters(spec.params.0.clone()),
                time_base: spec.time_base,
                title: spec.title.clone(),
                default: spec.default,
            })
            .collect();
        let (mux, audio_streams) = build_muxer(&cfg.output_file, container, streams.fps, video, audio)?;
        let (mut camera, mut camera_error) = (None, None);
        if let Some(setup) = &streams.camera {
            let file = cam::file_path(&cfg.take_folder, &cfg.base_name, container);
            let fps = (setup.fps.0 as i32, setup.fps.1 as i32);
            match cam::open_muxer(&file, container, setup.params.0.clone(), setup.time_base, fps) {
                Ok(mux) => camera = Some(CamFile { file, mux }),
                Err(err) => camera_error = Some(format!("The camera file was skipped: {err}")),
            }
        }
        let camera_file = camera.as_ref().map(|camera| camera.file.clone());
        let wav_plan = WavPlan {
            enabled: cfg.wav_sidecars,
            take_folder: cfg.take_folder.clone(),
            base_name: cfg.base_name.clone(),
            track_names: cfg.tracks.iter().map(|track| track.name.clone()).collect(),
        };
        let writer = Arc::new(TakeWriter::new(mux.sender(), audio_streams, camera, wav_plan));
        Ok(Self { mux, writer, camera_file, camera_error })
    }

    /// Why the camera file could not be opened.
    pub(super) fn camera_error(&self) -> Option<&str> {
        self.camera_error.as_deref()
    }

    /// The take's sink. Whoever holds it must let it go before the take's files can be finished.
    pub(super) fn sink(&self) -> Arc<dyn PacketSink> {
        Arc::clone(&self.writer) as Arc<dyn PacketSink>
    }

    /// Closes and removes a camera file that no camera thread will write.
    fn discard_camera(&mut self) {
        if let Some(file) = self.camera_file.take() {
            self.writer.close_camera();
            let _ = std::fs::remove_file(file);
        }
    }

    /// Ends the files of a take that never recorded; its sink must be free.
    pub(super) fn discard(mut self) {
        self.discard_camera();
        let Self { mux, writer, .. } = self;
        drop(writer);
        let _ = mux.finish();
    }

    /// Begins the take's marks and session.json, with the take's recording time 0 at `start_qpc`. `session_file` was
    /// described from the take's config, and `camera_size` is the open camera's size.
    pub(super) fn record(
        mut self,
        cfg: &SessionConfig,
        mut session_file: SessionFile,
        capture: &Capture,
        camera_size: Option<(u32, u32)>,
        start_qpc: i64,
    ) -> Take {
        if capture.streams().camera.is_none() {
            self.discard_camera();
        }
        let notice = self.camera_error.take().or_else(|| capture.camera_notice().map(str::to_owned));
        if let Some(notice) = &notice {
            log::warn!("{notice}");
        }
        let Self { mux, writer, camera_file, .. } = self;
        // The capture's output holds the sink from here on.
        drop(writer);
        let output_file = cfg.output_file.clone();
        let fps = cfg.profile.fps;
        session_file.set_cam(camera_file.as_deref(), notice.as_deref());
        let camera_name = camera_file.as_deref().map(file_name);
        session_file.set_composite(&CompositeBlock::new(&cfg.profile, camera_size, &file_name(&output_file), camera_name.as_deref()));
        let marks_warning = MarksWarning::default();
        let clock = MediaClock::new(start_qpc, qpc_frequency());
        let marks = TakeMarks::begin(&cfg.take_folder, &cfg.base_name, &output_file, fps, clock, marks_warning.clone());
        // A session.json that cannot be written is a warning, never a reason to stop the recording.
        if let Err(err) = session_file.write() {
            log::warn!("could not write the session file: {err}");
            capture.stats().set_last_error(format!("The session file could not be written: {err}"));
        }
        Take { mux, marks: Some(marks), marks_warning, fps, session_file, camera_notice: notice, output_file }
    }
}

/// One take that is recording.
pub(super) struct Take {
    pub(super) mux: MuxThread,
    /// The marks of the take, until they are finished.
    pub(super) marks: Option<TakeMarks>,
    /// The last marks error, which never stops the recording.
    pub(super) marks_warning: MarksWarning,
    /// Frames per second of the video, for the video's length at the end.
    fps: u32,
    /// The take's session.json, rewritten at the end.
    session_file: SessionFile,
    /// Why the profile's camera file is not being recorded, for a banner.
    pub(super) camera_notice: Option<String>,
    output_file: PathBuf,
}

impl Take {
    /// Starts a take on a capture that is already running, pressed at `press_qpc`. It begins on a forced keyframe.
    /// `cfg` names the take's files; its streams must be the capture's.
    // The replay buffer's controller calls this in a later prompt; a recording starts its take with its capture.
    #[allow(dead_code)]
    pub(super) fn start_on(capture: &Capture, cfg: &SessionConfig, camera_size: Option<(u32, u32)>, press_qpc: i64) -> Result<Self, EngineError> {
        let session_file = SessionFile::begin(cfg);
        let files = TakeFiles::open(cfg, capture.streams())?;
        match capture.output().start_take(files.sink(), press_qpc) {
            Ok(start_qpc) => Ok(files.record(cfg, session_file, capture, camera_size, start_qpc)),
            Err(err) => {
                files.discard();
                Err(err)
            }
        }
    }

    /// Moves app tracks' attach and detach events into the marks and session.json. True when a mark was added.
    pub(super) fn apply_app_events(&mut self, events: Vec<AppEvent>) -> bool {
        let mut marked = false;
        for event in events {
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

    /// Finishes the take's files once the capture has let the take go, and returns the take's summary with the camera
    /// file's totals, which are none when the take had no camera file or the file could not be finished.
    pub(super) fn finish(mut self, end: CaptureEnd, stats: &Stats) -> (Result<Summary, EngineError>, Option<cam::CamSummary>) {
        // The audio thread is done, so these are the take's last app events, in before the marks are finished.
        self.apply_app_events(stats.take_app_events());
        let CaptureEnd { video, audio, camera } = end;
        let mut outcome = video.map(|mut summary| {
            summary.file = self.output_file.clone();
            summary
        });
        // A camera file that fails here never fails the take.
        let camera = match camera {
            Some(Ok(summary)) => Some(summary),
            Some(Err(err)) => {
                log::warn!("the camera file could not be finished: {err}");
                stats.set_last_error(format!("The camera file could not be finished: {err}"));
                None
            }
            None => None,
        };
        // Finish the mux thread and fold its totals into the summary.
        let Self { mux, mut marks, fps, mut session_file, .. } = self;
        if outcome.is_ok() {
            // The marks are finished before the trailer, so the chapters go into the file.
            if let (Some(marks), Ok(summary)) = (marks.take(), &outcome) {
                let wav_files = audio.as_ref().map(|finished| finished.wav_files.clone()).unwrap_or_default();
                let video_ms = u64::try_from(u128::from(summary.frames) * 1000 / u128::from(fps.max(1))).unwrap_or(u64::MAX);
                mux.set_chapters(finish_marks(marks, &wav_files, video_ms, camera.as_ref()));
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
                summary.track_peaks = stats
                    .track_names()
                    .zip(&finished.track_peaks_dbfs)
                    .map(|(name, peak_dbfs)| TrackPeak { name: name.to_owned(), peak_dbfs: *peak_dbfs })
                    .collect();
                Ok(summary)
            }
            (_, outcome) => outcome,
        };
        // The totals go into session.json; a failed write is logged and shown as a warning, and the take is still finished.
        session_file.finish(&outcome, &stats.snapshot(), camera.as_ref());
        if let Err(err) = session_file.write() {
            log::warn!("could not update the session file: {err}");
            stats.set_last_error(format!("The session file could not be updated: {err}"));
        }
        (outcome, camera)
    }
}
