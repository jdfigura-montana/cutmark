//! editor::jobs: runs import and export on worker threads, reporting progress back.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use uuid::Uuid;

use crate::audio::mix_plan::MixPlan;
use crate::audio::mixer;
use crate::io::IoError;
use crate::io::export::{ExportFormat, open_sink};
use crate::io::import::{Loaded, import_file, load_or_rebuild};
use crate::io::paths::ProjectPaths;
use crate::model::media::MediaItem;
use crate::util::db::linear_to_db;

/// Result message when an export is cancelled.
const CANCELLED: &str = "Cancelled";

/// Timeline frames rendered per mixer call and progress step during export.
const EXPORT_BLOCK_FRAMES: u64 = 8192;

/// Message a worker thread sends back to the Editor over the job channel.
#[derive(Debug)]
pub enum JobMessage {
    /// Progress of a running job, 0.0 to 1.0.
    Progress {
        /// Job id the progress belongs to.
        job: u64,
        /// Fraction done, 0.0..=1.0.
        frac: f32,
    },
    /// An import job finished.
    ImportDone {
        /// Job id.
        job: u64,
        /// The bin entry plus loaded audio, or the failure.
        result: Result<(MediaItem, Loaded), IoError>,
    },
    /// A media load (open / cache rebuild) job finished.
    LoadDone {
        /// Job id.
        job: u64,
        /// Media id the job was loading.
        media_id: Uuid,
        /// The loaded audio, or the failure.
        result: Result<Loaded, IoError>,
    },
    /// Export progress of one render pass.
    ExportProgress {
        /// Job id.
        job: u64,
        /// Render pass, 1 (peak scan) or 2 (write).
        pass: u8,
        /// Fraction of the pass done, 0.0..=1.0.
        frac: f32,
    },
    /// Pass 1 found a peak above 0 dBFS; the Editor must send an ExportDecision.
    ExportPeak {
        /// Job id.
        job: u64,
        /// Peak level in dBFS (above 0 when this fires).
        peak_db: f32,
    },
    /// The export job finished.
    ExportDone {
        /// Job id.
        job: u64,
        /// The written file, or the error message ("Cancelled" on cancel).
        result: Result<PathBuf, String>,
    },
}

/// What the Editor decided when a mix peaked above 0 dBFS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDecision {
    /// Keep exporting; the user accepted the clipping.
    Continue,
    /// Abort the export.
    Cancel,
}

/// Settings for one export job, captured when the export starts.
#[derive(Debug, Clone)]
pub struct ExportSettings {
    /// Target file path; written via `<path>.part` and renamed on finish.
    pub path: PathBuf,
    /// File kind, channel count and input sample rate.
    pub format: ExportFormat,
    /// First timeline frame to render, in samples.
    pub start: u64,
    /// One-past-the-last timeline frame to render, in samples.
    pub end: u64,
    /// When given, only these track ids stay audible (mute and solo still apply).
    pub tracks: Option<Vec<Uuid>>,
}

/// Spawns a worker thread that imports a source file into the project.
///
/// Progress and the final result are sent over `sender`.
pub fn spawn_import(
    sender: Sender<JobMessage>,
    job: u64,
    src: PathBuf,
    paths: ProjectPaths,
    media_id: Uuid,
    project_rate: u32,
) {
    thread::spawn(move || {
        let result = import_file(&src, &paths, media_id, project_rate, &mut |frac| {
            let _ = sender.send(JobMessage::Progress { job, frac });
        });
        let _ = sender.send(JobMessage::ImportDone { job, result });
    });
}

/// Spawns a worker thread that loads (or rebuilds the cache for) one media item.
///
/// Progress and the final result are sent over `sender`.
pub fn spawn_load(
    sender: Sender<JobMessage>,
    job: u64,
    item: MediaItem,
    paths: ProjectPaths,
    project_rate: u32,
) {
    let media_id = item.id;
    thread::spawn(move || {
        let result = load_or_rebuild(&paths, &item, project_rate, &mut |frac| {
            let _ = sender.send(JobMessage::Progress { job, frac });
        });
        let _ = sender.send(JobMessage::LoadDone {
            job,
            media_id,
            result,
        });
    });
}

/// Spawns a worker thread that renders `plan` to `settings.path` in two passes.
///
/// Pass 1 renders in blocks and tracks the peak; when it is above 0 dBFS the
/// job asks the Editor over `decision_rx`. Pass 2 renders again and writes
/// through an io::export sink. `cancel` aborts the job within one block.
/// Progress and the final result go over `sender`.
pub fn spawn_export(
    sender: Sender<JobMessage>,
    job: u64,
    plan: Arc<MixPlan>,
    settings: ExportSettings,
    decision_rx: Receiver<ExportDecision>,
    cancel: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        let result = run_export(&plan, &settings, &decision_rx, &cancel, &sender, job);
        let _ = sender.send(JobMessage::ExportDone { job, result });
    });
}

/// Runs the export: pass 1 peak scan, optional clip warning, pass 2 write.
///
/// Buffers are allocated once up front; the loop itself does not allocate.
fn run_export(
    plan: &MixPlan,
    settings: &ExportSettings,
    decision_rx: &Receiver<ExportDecision>,
    cancel: &AtomicBool,
    sender: &Sender<JobMessage>,
    job: u64,
) -> Result<PathBuf, String> {
    let total = settings.end.saturating_sub(settings.start);
    if total == 0 {
        return Err("empty export range".to_string());
    }
    let mut buf = vec![0.0f32; (EXPORT_BLOCK_FRAMES * 2) as usize];
    let mut mono = vec![0.0f32; EXPORT_BLOCK_FRAMES as usize];

    // Pass 1: render and track the highest absolute sample.
    let mut peak = 0.0f32;
    let mut pos = settings.start;
    while pos < settings.end {
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        let frames = (settings.end - pos).min(EXPORT_BLOCK_FRAMES) as usize;
        mixer::render(plan, pos, &mut buf[..frames * 2], false);
        for [l, r] in buf[..frames * 2].as_chunks::<2>().0 {
            peak = peak.max(l.abs()).max(r.abs());
        }
        let _ = sender.send(JobMessage::ExportProgress {
            job,
            pass: 1,
            frac: (pos - settings.start + frames as u64) as f32 / total as f32,
        });
        pos += frames as u64;
    }

    // Above 0 dBFS: warn and wait for the Editor's decision.
    if peak > 1.0 {
        let _ = sender.send(JobMessage::ExportPeak {
            job,
            peak_db: linear_to_db(peak),
        });
        match decision_rx.recv() {
            Ok(ExportDecision::Continue) => {}
            _ => return Err(CANCELLED.to_string()),
        }
    }

    // Pass 2: render the same blocks again and write them.
    let mut sink = open_sink(&settings.path, &settings.format).map_err(|e| e.to_string())?;
    let mono_out = settings.format.channels == 1;
    let mut pos = settings.start;
    while pos < settings.end {
        if cancel.load(Ordering::Relaxed) {
            sink.abort();
            return Err(CANCELLED.to_string());
        }
        let frames = (settings.end - pos).min(EXPORT_BLOCK_FRAMES) as usize;
        mixer::render(plan, pos, &mut buf[..frames * 2], false);
        let block: &[f32] = if mono_out {
            for (i, [l, r]) in buf[..frames * 2].as_chunks::<2>().0.iter().enumerate() {
                mono[i] = (l + r) * 0.5;
            }
            &mono[..frames]
        } else {
            &buf[..frames * 2]
        };
        if let Err(error) = sink.write(block) {
            sink.abort();
            return Err(error.to_string());
        }
        let _ = sender.send(JobMessage::ExportProgress {
            job,
            pass: 2,
            frac: (pos - settings.start + frames as u64) as f32 / total as f32,
        });
        pos += frames as u64;
    }
    sink.finish()
        .map(|_| settings.path.clone())
        .map_err(|e| e.to_string())
}

/// The Editor-owned job channel pair plus helpers for spawning job threads.
pub struct JobChannels {
    /// Sends job messages to the Editor.
    pub tx: Sender<JobMessage>,
    /// Receives job messages; drained by `Editor::poll_jobs`.
    pub rx: Receiver<JobMessage>,
}

impl JobChannels {
    /// Creates an unbounded channel pair.
    pub fn new() -> JobChannels {
        let (tx, rx) = crossbeam_channel::unbounded();
        JobChannels { tx, rx }
    }

    /// Spawns an import job on its own thread.
    pub fn spawn_import(
        &self,
        job: u64,
        src: PathBuf,
        paths: ProjectPaths,
        media_id: Uuid,
        project_rate: u32,
    ) {
        spawn_import(self.tx.clone(), job, src, paths, media_id, project_rate);
    }

    /// Spawns a load job on its own thread.
    pub fn spawn_load(&self, job: u64, item: MediaItem, paths: ProjectPaths, project_rate: u32) {
        spawn_load(self.tx.clone(), job, item, paths, project_rate);
    }

    /// Spawns an export job on its own thread.
    pub fn spawn_export(
        &self,
        job: u64,
        plan: Arc<MixPlan>,
        settings: ExportSettings,
        decision_rx: Receiver<ExportDecision>,
        cancel: Arc<AtomicBool>,
    ) {
        spawn_export(self.tx.clone(), job, plan, settings, decision_rx, cancel);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use tempfile::tempdir;

    use super::*;
    use crate::audio::meters::{MeterBank, StereoMeter};
    use crate::editor::action::Action;
    use crate::editor::editor::{Editor, EditorError};
    use crate::io::export::{ExportFormat, ExportKind};
    use crate::io::import::Loaded;
    use crate::model::fixtures::project_with_clips;
    use crate::model::media::Peaks;
    use crate::model::project::{BitDepth, ViewState};

    /// A 10 s poll deadline for waiting on background jobs.
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// An editor over `tracks` whose single stereo media item is filled by
    /// `fill(frame, channel)`; the media is loaded into the store.
    fn test_editor(
        tracks: &[&[(u64, u64)]],
        media_frames: u64,
        fill: impl Fn(u64, usize) -> f32,
    ) -> (Editor, tempfile::TempDir) {
        let project = project_with_clips(tracks);
        let media_id = project.media[0].id;
        let mut samples = Vec::with_capacity((media_frames * 2) as usize);
        for frame in 0..media_frames {
            for channel in 0..2 {
                samples.push(fill(frame, channel));
            }
        }
        let media: Arc<[f32]> = samples.into();
        let dir = tempdir().unwrap();
        let mut editor = Editor::with_project(
            project,
            ViewState::default(),
            ProjectPaths {
                root: dir.path().to_path_buf(),
            },
        );
        editor.media_store.insert(
            media_id,
            Loaded {
                samples: Arc::clone(&media),
                peaks: Peaks {
                    channels: 2,
                    frames: 0,
                    levels: Vec::new(),
                }
                .into(),
            },
        );
        (editor, dir)
    }

    /// Stereo float32 WAV export settings for `start..end`.
    fn wav_settings(
        editor: &Editor,
        path: PathBuf,
        channels: u16,
        start: u64,
        end: u64,
        tracks: Option<Vec<Uuid>>,
    ) -> ExportSettings {
        ExportSettings {
            path,
            format: ExportFormat {
                kind: ExportKind::Wav(BitDepth::Float32),
                channels,
                in_rate: editor.project.sample_rate,
            },
            start,
            end,
            tracks,
        }
    }

    /// The `.part` sibling of an export target.
    fn part_of(path: &Path) -> PathBuf {
        PathBuf::from(format!("{}.part", path.display()))
    }

    /// Polls until the export finished; panics after 10 s.
    fn wait_done(editor: &mut Editor) -> Result<PathBuf, String> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            editor.poll_jobs();
            if let Some(result) = editor.take_export_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "export did not finish in 10 s");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Polls until the clip warning appears; panics after 10 s.
    fn wait_peak(editor: &mut Editor) -> f32 {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            editor.poll_jobs();
            if let Some(status) = editor.export_status()
                && let Some(db) = status.peak_warning
            {
                return db;
            }
            assert!(Instant::now() < deadline, "no peak warning in 10 s");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// Renders `frames` of the editor's mix from `start` with track 1 muted
    /// when `mute_track1`, exactly as the export job would.
    fn render_all(editor: &Editor, start: u64, frames: u64, mute_track1: bool) -> Vec<f32> {
        let mut project = (*editor.project).clone();
        if mute_track1 && let Some(track) = project.tracks.get_mut(1) {
            track.mute = true;
        }
        let ids: Vec<Uuid> = project.tracks.iter().map(|t| t.id).collect();
        let meters = Arc::new(MeterBank {
            track_ids: ids,
            tracks: (0..project.tracks.len())
                .map(|_| StereoMeter::new())
                .collect(),
        });
        let store = &editor.media_store;
        let plan = MixPlan::build(
            &project,
            &|id| store.get(id).map(|l| Arc::clone(&l.samples)),
            meters,
            Arc::new(StereoMeter::new()),
        );
        let mut out = vec![0.0f32; (frames * 2) as usize];
        mixer::render(&plan, start, &mut out, false);
        out
    }

    /// Reads a float32 WAV file back as interleaved samples.
    fn read_wav(path: &Path) -> Vec<f32> {
        let mut reader = hound::WavReader::open(path).unwrap();
        assert_eq!(reader.spec().sample_format, hound::SampleFormat::Float);
        reader.samples::<f32>().map(|s| s.unwrap()).collect()
    }

    /// A deterministic stereo pattern whose peak is below 1.0.
    fn pattern(frame: u64, channel: usize) -> f32 {
        let value = (frame % 97) as f32 / 97.0 - 0.5;
        if channel == 0 { value } else { -value * 0.5 }
    }

    #[test]
    fn float32_wav_export_matches_the_mixer_exactly() {
        let (mut editor, dir) = test_editor(&[&[(0, 10_000)]], 10_000, pattern);
        let path = dir.path().join("out.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 0, 10_000, None))
            .unwrap();
        let result = wait_done(&mut editor).unwrap();
        assert_eq!(result, path);
        assert!(!part_of(&path).exists());
        assert_eq!(read_wav(&path), render_all(&editor, 0, 10_000, false));
    }

    #[test]
    fn mono_export_sums_stereo_at_half_gain() {
        let (mut editor, dir) = test_editor(&[&[(0, 10_000)]], 10_000, pattern);
        let path = dir.path().join("mono.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 1, 0, 10_000, None))
            .unwrap();
        wait_done(&mut editor).unwrap();
        let stereo = render_all(&editor, 0, 10_000, false);
        let expected: Vec<f32> = stereo
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[l, r]| (l + r) * 0.5)
            .collect();
        assert_eq!(read_wav(&path), expected);
    }

    #[test]
    fn selection_scope_exports_exactly_the_range() {
        let (mut editor, dir) = test_editor(&[&[(0, 10_000)]], 10_000, pattern);
        let path = dir.path().join("range.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 1_000, 5_000, None))
            .unwrap();
        wait_done(&mut editor).unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), 4_000, "exactly 4,000 frames");
        drop(reader);
        assert_eq!(read_wav(&path), render_all(&editor, 1_000, 4_000, false));
    }

    #[test]
    fn selected_tracks_only_matches_muting_the_others() {
        let (mut editor_a, dir_a) = test_editor(&[&[(0, 5_000)], &[(0, 5_000)]], 5_000, pattern);
        let (mut editor_b, dir_b) = test_editor(&[&[(0, 5_000)], &[(0, 5_000)]], 5_000, pattern);
        let track0 = editor_a.project.tracks[0].id;
        let track1 = editor_b.project.tracks[1].id;
        editor_b
            .apply(Action::SetTrackMute {
                id: track1,
                mute: true,
            })
            .unwrap();

        let path_a = dir_a.path().join("scoped.wav");
        editor_a
            .start_export(wav_settings(
                &editor_a,
                path_a.clone(),
                2,
                0,
                5_000,
                Some(vec![track0]),
            ))
            .unwrap();
        let path_b = dir_b.path().join("muted.wav");
        editor_b
            .start_export(wav_settings(&editor_b, path_b.clone(), 2, 0, 5_000, None))
            .unwrap();
        wait_done(&mut editor_a).unwrap();
        wait_done(&mut editor_b).unwrap();

        let expected = render_all(&editor_a, 0, 5_000, true);
        assert_eq!(read_wav(&path_a), expected);
        assert_eq!(read_wav(&path_b), expected);
        assert_ne!(
            read_wav(&path_a),
            render_all(&editor_a, 0, 5_000, false),
            "the second track must have contributed something"
        );
    }

    #[test]
    fn peak_warning_continue_writes_the_file() {
        let (mut editor, dir) = test_editor(&[&[(0, 4_000)]], 4_000, |_, _| 2.0);
        // Center pan costs 3 dB per side; +3.01 dB track volume puts the mix
        // peak at exactly 2.0.
        let track0 = editor.project.tracks[0].id;
        editor
            .apply(Action::SetTrackVolume {
                id: track0,
                db: 3.0103,
            })
            .unwrap();
        let path = dir.path().join("peak.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 0, 4_000, None))
            .unwrap();
        let db = wait_peak(&mut editor);
        assert!((db - 6.0206).abs() < 0.01, "peak 2.0 is about +6.0 dB");
        let status = editor.export_status().unwrap();
        assert_eq!(status.pass, 1);
        assert_eq!(status.peak_warning, Some(db));
        editor.export_continue();
        let result = wait_done(&mut editor).unwrap();
        assert_eq!(result, path);
        assert!(editor.export_status().is_none());
        assert!(!part_of(&path).exists());
        assert_eq!(read_wav(&path), render_all(&editor, 0, 4_000, false));
    }

    #[test]
    fn peak_warning_cancel_leaves_no_file_and_no_part() {
        let (mut editor, dir) = test_editor(&[&[(0, 4_000)]], 4_000, |_, _| 2.0);
        let path = dir.path().join("peak_cancel.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 0, 4_000, None))
            .unwrap();
        wait_peak(&mut editor);
        editor.export_cancel();
        assert_eq!(wait_done(&mut editor).unwrap_err(), "Cancelled");
        assert!(editor.export_status().is_none());
        assert!(!path.exists());
        assert!(!part_of(&path).exists());
    }

    #[test]
    fn exporting_locks_editing() {
        let (mut editor, dir) = test_editor(&[&[(0, 4_000)]], 4_000, |_, _| 2.0);
        let track0 = editor.project.tracks[0].id;
        editor
            .apply(Action::SetTrackVolume {
                id: track0,
                db: -3.0,
            })
            .unwrap();
        let path = dir.path().join("locked.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 0, 4_000, None))
            .unwrap();
        wait_peak(&mut editor);

        // The job sits waiting for a decision, so the lock is stable here.
        assert!(editor.is_locked());
        let before = editor.project.tracks.len();
        assert!(matches!(
            editor.apply(Action::AddTrack {
                index: None,
                name: None,
            }),
            Err(EditorError::Locked(_))
        ));
        assert_eq!(editor.project.tracks.len(), before, "project unchanged");
        assert!(!editor.undo());
        assert!(!editor.redo());
        editor.begin_gesture();
        assert!(editor.gesture_base.is_none());
        assert!(editor.import_files(vec![PathBuf::from("x.wav")]).is_empty());
        assert!(
            editor
                .pending_jobs()
                .iter()
                .all(|(label, _)| label == "Export")
        );

        editor.export_continue();
        wait_done(&mut editor).unwrap();
    }

    #[test]
    fn cancel_mid_pass_two_leaves_no_file() {
        // A long enough mix that pass 2 lasts seconds in debug builds, but
        // pass 1 still finishes well inside the timeout.
        let tracks: Vec<&[(u64, u64)]> = vec![&[(0, 1_500_000)], &[(0, 1_500_000)]];
        let (mut editor, dir) = test_editor(&tracks, 1_500_000, pattern);
        let path = dir.path().join("mid.wav");
        editor
            .start_export(wav_settings(&editor, path.clone(), 2, 0, 1_500_000, None))
            .unwrap();

        let deadline = Instant::now() + TIMEOUT;
        let mut cancelled = false;
        loop {
            editor.poll_jobs();
            if let Some(status) = editor.export_status()
                && status.pass == 2
            {
                editor.export_cancel();
                cancelled = true;
                break;
            }
            if editor.take_export_result().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "pass 2 never started in 10 s");
        }
        assert!(
            cancelled,
            "the job finished before pass 2 could be cancelled"
        );
        assert_eq!(wait_done(&mut editor).unwrap_err(), "Cancelled");
        assert!(!path.exists());
        assert!(!part_of(&path).exists());
    }

    #[test]
    fn default_export_path_uses_exports_folder_and_extension() {
        let (editor, dir) = test_editor(&[&[(0, 10)]], 10, pattern);
        assert_eq!(
            editor.default_export_path(ExportKind::Wav(BitDepth::Float32)),
            dir.path().join("exports").join("Test.wav")
        );
        assert_eq!(
            editor.default_export_path(ExportKind::Mp3 { kbps: 192 }),
            dir.path().join("exports").join("Test.mp3")
        );
    }
}
