//! editor::jobs: runs import and export on worker threads, reporting progress back.

use std::path::PathBuf;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use uuid::Uuid;

use crate::io::IoError;
use crate::io::import::{Loaded, import_file, load_or_rebuild};
use crate::io::paths::ProjectPaths;
use crate::model::media::MediaItem;

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
}
