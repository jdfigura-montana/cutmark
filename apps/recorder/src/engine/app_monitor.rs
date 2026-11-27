//! The idle watch of app and desktop-except tracks: once a second their apps are looked for, and their
//! captures run for the meters only. It writes nothing and adds no marks.
//!
//! See the per-app audio spec: Tracks, attach and detach.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use capture::{AudioApp, list_audio_apps, qpc_now};

use super::audio::{WindowsApps, spawn_app_lister};
use super::audio_track::{AppSystem, TrackLink, app_pids};
use super::session::{AppSpec, Instances};

/// How often the monitor drains its captures.
const TICK: Duration = Duration::from_millis(20);
/// The level of silence in dBFS, the floor of every idle meter.
pub(super) const SILENCE_DBFS: f32 = -100.0;

/// One track to watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WatchedTrack {
    /// The track, counted in settings order.
    pub(super) index: usize,
    /// The app it records or leaves out.
    pub(super) app: AppSpec,
}

/// What an idle app or desktop-except track is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct IdleAppTrack {
    /// The track, counted in settings order.
    pub index: usize,
    /// The app the track records or leaves out, so a reading is never taken for a newly picked app.
    pub app: AppSpec,
    /// The app instances a desktop-except track leaves out.
    pub instances: Instances,
    /// Whether the app is being captured, or for a desktop-except track being left out.
    pub attached: bool,
    /// The peak since the last reading in dBFS, -100.0 for silence.
    pub peak_dbfs: f32,
}

/// The links of the watched tracks, with the peak each has heard since the last reading.
struct AppWatch {
    tracks: Vec<(WatchedTrack, TrackLink, f32)>,
}

impl AppWatch {
    fn new(watched: &[WatchedTrack]) -> Self {
        Self { tracks: watched.iter().map(|track| (track.clone(), TrackLink::new(&track.app.exe, track.app.except), 0.0)).collect() }
    }

    /// Attaches, detaches, switches or restarts every link. With no take running, a change is not an event.
    fn poll(&mut self, listing: &[AudioApp], system: &mut dyn AppSystem) {
        for (_, link, _) in &mut self.tracks {
            let running = app_pids(listing, link.exe());
            link.poll(running, system);
        }
    }

    /// Reads every queued packet for its peak and drops it.
    fn drain(&mut self, now_100ns: i64, system: &mut dyn AppSystem) {
        for (_, link, peak) in &mut self.tracks {
            link.drain(now_100ns, system, &mut |packet| *peak = packet.samples.iter().fold(*peak, |peak, sample| peak.max(sample.abs())));
        }
    }

    /// Puts the state of every track into `readings`, keeping the louder of a peak not read yet and the new one.
    fn publish(&mut self, readings: &mut Vec<IdleAppTrack>) {
        let unread = std::mem::take(readings);
        readings.extend(self.tracks.iter_mut().map(|(track, link, peak)| {
            let heard = std::mem::take(peak);
            let dbfs = if heard > 0.0 { (20.0 * heard.log10()).max(SILENCE_DBFS) } else { SILENCE_DBFS };
            let before = unread.iter().find(|reading| reading.index == track.index).map_or(SILENCE_DBFS, |reading| reading.peak_dbfs);
            let (app, instances) = (track.app.clone(), link.instances());
            IdleAppTrack { index: track.index, app, instances, attached: link.attached(), peak_dbfs: dbfs.max(before) }
        }));
    }
}

/// The running monitor thread; dropping it stops the thread and its captures.
pub(super) struct AppMonitor {
    watched: Vec<WatchedTrack>,
    stop: Arc<AtomicBool>,
    readings: Arc<Mutex<Vec<IdleAppTrack>>>,
}

impl AppMonitor {
    /// Starts watching the tracks. A thread that cannot start leaves the readings empty.
    pub(super) fn start(watched: Vec<WatchedTrack>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let readings = Arc::new(Mutex::new(Vec::new()));
        let (tracks, thread_stop, thread_readings) = (watched.clone(), Arc::clone(&stop), Arc::clone(&readings));
        let spawned = std::thread::Builder::new().name("app-monitor".to_owned()).spawn(move || watch(&tracks, &thread_stop, &thread_readings));
        if let Err(err) = spawned {
            log::warn!("the app monitor thread could not be started, idle app tracks will not attach: {err}");
        }
        Self { watched, stop, readings }
    }

    /// The tracks being watched.
    pub(super) fn watched(&self) -> &[WatchedTrack] {
        &self.watched
    }

    /// The latest state of every track, empty until the first listing; reading the peaks resets them.
    pub(super) fn read(&self) -> Vec<IdleAppTrack> {
        let mut readings = self.readings.lock().unwrap_or_else(PoisonError::into_inner);
        let latest = readings.clone();
        for track in readings.iter_mut() {
            track.peak_dbfs = SILENCE_DBFS;
        }
        latest
    }
}

impl Drop for AppMonitor {
    fn drop(&mut self) {
        // Not joined: a slow app listing must not hold up the controller.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The monitor thread's loop.
fn watch(watched: &[WatchedTrack], stop: &AtomicBool, readings: &Mutex<Vec<IdleAppTrack>>) {
    let mut watch = AppWatch::new(watched);
    // The first listing is read at once; the lister then reads one a second.
    let mut first = list_audio_apps(true).map_err(|err| log::debug!("the running apps could not be listed: {err}")).ok();
    let listings = spawn_app_lister();
    let mut listed = false;
    while !stop.load(Ordering::Relaxed) {
        watch.drain(qpc_now(), &mut WindowsApps);
        if let Some(listing) = first.take().or_else(|| listings.try_iter().last()) {
            watch.poll(&listing, &mut WindowsApps);
            listed = true;
        }
        if listed {
            watch.publish(&mut readings.lock().unwrap_or_else(PoisonError::into_inner));
        }
        std::thread::sleep(TICK);
    }
}

#[cfg(test)]
mod tests {
    use capture::AudioPacket;

    use super::super::audio_track::fake::FakeSystem;
    use super::*;

    #[test]
    fn an_idle_attach_sets_attached_and_feeds_only_the_meter() {
        let app = AppSpec { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned(), except: false };
        let mut watch = AppWatch::new(&[WatchedTrack { index: 2, app: app.clone() }]);
        let system = FakeSystem::default();
        *system.instances.borrow_mut() = vec![40];
        let mut apps = system.clone();
        let mut readings = Vec::new();
        watch.poll(&[], &mut apps);
        watch.publish(&mut readings);
        assert_eq!(readings, [IdleAppTrack { index: 2, app, instances: Instances::default(), attached: false, peak_dbfs: SILENCE_DBFS }]);

        let running = [AudioApp { exe: "discord.exe".to_owned(), display_name: "Discord".to_owned(), pids: vec![40, 41], active: true }];
        watch.poll(&running, &mut apps);
        let packet = AudioPacket { qpc_100ns: 0, frames: 480, channels: 2, sample_rate: 48_000, samples: vec![-0.5; 960], discontinuity: false, silent: false };
        assert!(system.latest_sender().is_some_and(|sender| sender.send(packet).is_ok()));
        watch.drain(0, &mut apps);
        watch.publish(&mut readings);
        // A peak that was not read yet is kept.
        watch.publish(&mut readings);
        assert!(readings[0].attached);
        assert!((readings[0].peak_dbfs + 6.02).abs() < 0.1, "peak was {}", readings[0].peak_dbfs);
    }
}
