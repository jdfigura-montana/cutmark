//! editor::record: arm state, the audio input stream behind it and the app's
//! audio settings. Arming opens the input and meters it; nothing is recorded
//! yet (phase 12).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use uuid::Uuid;

use crate::audio::engine::AudioError;
use crate::audio::input::{self, InputPick, InputStream};
use crate::audio::meters::MeterReading;
use crate::editor::editor::{Editor, EditorError};
use crate::io::config::{self, AppConfig, RecordChannels};

/// Test double for the system's input devices. An Editor holding one lists
/// these devices and never opens a real one, so tests do not depend on the
/// machine's audio hardware.
#[derive(Debug, Clone, Default)]
pub struct FakeInput {
    /// Name of the Windows default input, if any.
    pub default: Option<String>,
    /// Every input device as (name, channel count).
    pub devices: Vec<(String, u16)>,
    /// Opening the input fails (as if no device could be found).
    pub open_fails: bool,
    /// The open input reports a lost device on the next tick.
    pub lost: bool,
}

/// True once the "device not found" notice was shown in this launch.
static FALLBACK_NOTED: AtomicBool = AtomicBool::new(false);

/// What the Editor starts with: the app config and where it is saved, plus the
/// input test double. Test builds never touch the real config file or any
/// real input device; the running app reads the config once.
pub(super) fn initial_state() -> (AppConfig, Option<PathBuf>, Option<FakeInput>) {
    if cfg!(test) {
        (AppConfig::default(), None, Some(FakeInput::default()))
    } else {
        (config::load(), config::default_path(), None)
    }
}

/// The input channels a Record channels setting records.
pub fn input_pick(channels: RecordChannels) -> InputPick {
    match channels {
        RecordChannels::Mono1 => InputPick::Mono(0),
        RecordChannels::Mono2 => InputPick::Mono(1),
        RecordChannels::Stereo => InputPick::Stereo,
    }
}

/// The "device not found" notice, at most once per launch: only when a device
/// name was configured, the stream fell back to the default input, and `noted`
/// was not set yet. Sets `noted` when it returns the notice.
fn fallback_notice(wanted: Option<&str>, fell_back: bool, noted: &AtomicBool) -> Option<String> {
    let name = wanted?;
    if !fell_back || noted.swap(true, Ordering::Relaxed) {
        return None;
    }
    Some(format!(
        "Input device {name} not found. Using the default input."
    ))
}

impl Editor {
    /// The app audio settings.
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    /// The track armed for recording, if any.
    pub fn armed_track(&self) -> Option<Uuid> {
        self.armed_track
    }

    /// Arms `track_id`, or disarms it when it is the armed track.
    ///
    /// Only one track is armed at a time: arming moves the arm. Arming opens
    /// the input stream (and fills the track's header meter from it); when it
    /// cannot open, the track stays disarmed and the reason goes to the
    /// message dialog. Arm state is not saved, not undoable and never dirty.
    pub fn toggle_arm(&mut self, track_id: Uuid) {
        if self.armed_track == Some(track_id) {
            self.disarm();
            return;
        }
        if !self.project.tracks.iter().any(|t| t.id == track_id) {
            return;
        }
        // Another track is armed: the input is already open, the arm moves.
        if self.armed_track.is_some() {
            self.armed_track = Some(track_id);
            return;
        }
        match self.open_input() {
            Ok(()) => self.armed_track = Some(track_id),
            Err(error) => self
                .job_errors
                .push(format!("Could not arm the track: {error}")),
        }
    }

    /// Disarms the track and closes the input stream.
    pub(super) fn disarm(&mut self) {
        self.armed_track = None;
        self.input = None;
    }

    /// Disarms when the armed track no longer exists (deleted, or removed by
    /// undo or redo).
    pub(super) fn tidy_armed_track(&mut self) {
        if let Some(id) = self.armed_track
            && !self.project.tracks.iter().any(|t| t.id == id)
        {
            self.disarm();
        }
    }

    /// Per-frame check: a lost input device disarms the track.
    pub(super) fn input_tick(&mut self) {
        let lost = match &self.fake_input {
            Some(fake) => fake.lost && self.armed_track.is_some(),
            None => self.input.as_ref().is_some_and(InputStream::errored),
        };
        if lost {
            self.disarm();
            self.job_errors.push("Input device lost.".to_string());
        }
    }

    /// Takes the input meter's reading (both sides), emptying it.
    pub(super) fn take_input_reading(&self) -> [MeterReading; 2] {
        [self.input_meter.l.take(), self.input_meter.r.take()]
    }

    /// Opens the input stream for the current settings. With a test double
    /// nothing is opened. Queues the "device not found" notice (once per
    /// launch) when the saved device was replaced by the default input.
    fn open_input(&mut self) -> Result<(), AudioError> {
        if let Some(fake) = &self.fake_input {
            return if fake.open_fails {
                Err(AudioError::NoInputDevice)
            } else {
                Ok(())
            };
        }
        // Start from silence: nothing left over from an earlier arm.
        self.take_input_reading();
        let stream = InputStream::open(
            self.config.input_device.as_deref(),
            input_pick(self.config.record_channels),
            Arc::clone(&self.input_meter),
            // Placeholder ring: nothing is recorded yet (recording is off), and
            // 12.2 creates the real ring whose consumer the take job keeps.
            rtrb::RingBuffer::new(0).0,
        )?;
        if let Some(notice) = fallback_notice(
            self.config.input_device.as_deref(),
            stream.fell_back(),
            &FALLBACK_NOTED,
        ) {
            self.job_errors.push(notice);
        }
        self.input = Some(stream);
        Ok(())
    }

    /// Saves new audio settings and, when a track is armed, reopens the input
    /// with them.
    ///
    /// The settings are saved first (an error leaves everything as it was).
    /// If the reopen then fails, the track is disarmed and the error returned;
    /// the saved settings stay.
    pub fn set_config(&mut self, config: AppConfig) -> Result<(), EditorError> {
        let config = config.clamped();
        let path = self
            .config_path
            .clone()
            .ok_or_else(config::no_config_folder)?;
        config::save_to(&config, &path)?;
        self.config = config;
        if self.armed_track.is_some() {
            // The old stream must be closed before the device opens again.
            self.input = None;
            if let Err(error) = self.open_input() {
                self.disarm();
                return Err(EditorError::Input(format!(
                    "Could not reopen the input: {error}. The track was disarmed."
                )));
            }
        }
        Ok(())
    }

    /// Names of the input devices Windows lists. Slow (it probes every
    /// device): call when a dialog opens, not every frame.
    pub fn input_device_names(&self) -> Vec<String> {
        match &self.fake_input {
            Some(fake) => fake.devices.iter().map(|(name, _)| name.clone()).collect(),
            None => input::input_device_names(),
        }
    }

    /// Name of the Windows default input device, if there is one.
    pub fn default_input_name(&self) -> Option<String> {
        match &self.fake_input {
            Some(fake) => fake.default.clone(),
            None => input::default_input_name(),
        }
    }

    /// Input channel count of the device that would be opened for `device`
    /// (a name; None, or a name that is not found, means the Windows default).
    pub fn input_device_channels(&self, device: Option<&str>) -> Option<u16> {
        match &self.fake_input {
            Some(fake) => {
                let known = |name: &str| fake.devices.iter().any(|(d, _)| d == name);
                let name = device
                    .filter(|name| known(name))
                    .or(fake.default.as_deref())?;
                fake.devices
                    .iter()
                    .find(|(d, _)| d == name)
                    .map(|(_, channels)| *channels)
            }
            None => input::device_channels(device),
        }
    }

    /// Test seam: where `set_config` saves.
    #[cfg(test)]
    pub fn set_config_path(&mut self, path: PathBuf) {
        self.config_path = Some(path);
    }

    /// Test seam: replaces the input test double.
    #[cfg(test)]
    pub fn set_fake_input(&mut self, fake: FakeInput) {
        self.fake_input = Some(fake);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::action::Action;
    use crate::io::paths::ProjectPaths;
    use crate::model::fixtures::project_with_clips;
    use crate::model::project::ViewState;
    use tempfile::tempdir;

    /// An editor over `count` empty tracks; no device is ever opened.
    fn editor_with_tracks(count: usize) -> Editor {
        let tracks: Vec<&[(u64, u64)]> = vec![&[]; count];
        Editor::with_project(
            project_with_clips(&tracks),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        )
    }

    fn track_id(editor: &Editor, index: usize) -> Uuid {
        editor.project.tracks[index].id
    }

    #[test]
    fn record_channels_map_to_input_picks() {
        assert_eq!(input_pick(RecordChannels::Mono1), InputPick::Mono(0));
        assert_eq!(input_pick(RecordChannels::Mono2), InputPick::Mono(1));
        assert_eq!(input_pick(RecordChannels::Stereo), InputPick::Stereo);
    }

    #[test]
    fn arming_sets_the_armed_track_and_toggling_again_disarms() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        assert_eq!(editor.armed_track(), None);
        editor.toggle_arm(a);
        assert_eq!(editor.armed_track(), Some(a));
        editor.toggle_arm(a);
        assert_eq!(editor.armed_track(), None);
        editor.toggle_arm(b);
        assert_eq!(editor.armed_track(), Some(b));
    }

    #[test]
    fn only_one_track_is_armed_at_a_time() {
        let mut editor = editor_with_tracks(3);
        let ids: Vec<Uuid> = (0..3).map(|i| track_id(&editor, i)).collect();
        editor.toggle_arm(ids[0]);
        editor.toggle_arm(ids[2]);
        assert_eq!(editor.armed_track(), Some(ids[2]));
        editor.toggle_arm(ids[1]);
        assert_eq!(editor.armed_track(), Some(ids[1]));
        // Toggling a track that is not armed never disarms the armed one.
        editor.toggle_arm(ids[0]);
        assert_eq!(editor.armed_track(), Some(ids[0]));
    }

    #[test]
    fn an_unknown_track_cannot_be_armed() {
        let mut editor = editor_with_tracks(1);
        editor.toggle_arm(Uuid::new_v4());
        assert_eq!(editor.armed_track(), None);
    }

    #[test]
    fn deleting_the_armed_track_disarms() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        editor.toggle_arm(a);
        editor.apply(Action::DeleteTrack { id: b }).unwrap();
        assert_eq!(editor.armed_track(), Some(a), "another track's delete");
        editor.apply(Action::DeleteTrack { id: a }).unwrap();
        assert_eq!(editor.armed_track(), None);
    }

    #[test]
    fn undo_and_redo_that_remove_the_armed_track_disarm() {
        let mut editor = editor_with_tracks(1);
        editor
            .apply(Action::AddTrack {
                index: None,
                name: None,
            })
            .unwrap();
        let added = track_id(&editor, 1);

        // Undo removes the added track.
        editor.toggle_arm(added);
        assert!(editor.undo());
        assert_eq!(editor.armed_track(), None, "undo of the add");

        // Redo brings it back; delete + undo + arm + redo removes it again.
        assert!(editor.redo());
        editor.apply(Action::DeleteTrack { id: added }).unwrap();
        assert!(editor.undo());
        editor.toggle_arm(added);
        assert_eq!(editor.armed_track(), Some(added));
        assert!(editor.redo());
        assert_eq!(editor.armed_track(), None, "redo of the delete");
    }

    #[test]
    fn arming_never_marks_the_project_dirty_or_adds_history() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        let before = (editor.history.undo.len(), editor.history.redo.len());
        let project = Arc::clone(&editor.project);
        editor.toggle_arm(a);
        editor.toggle_arm(b);
        editor.toggle_arm(b);
        assert!(!editor.dirty);
        assert_eq!(
            (editor.history.undo.len(), editor.history.redo.len()),
            before
        );
        assert_eq!(editor.undo_label(), None);
        assert!(Arc::ptr_eq(&project, &editor.project), "project untouched");
    }

    #[test]
    fn an_open_failure_leaves_the_track_disarmed_and_reports_it() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.set_fake_input(FakeInput {
            open_fails: true,
            ..FakeInput::default()
        });
        editor.toggle_arm(id);
        assert_eq!(editor.armed_track(), None);
        let messages = editor.take_job_errors();
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].starts_with("Could not arm the track: "),
            "{messages:?}"
        );
    }

    #[test]
    fn a_lost_input_device_disarms_with_a_message() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);
        editor.tick();
        assert_eq!(
            editor.armed_track(),
            Some(id),
            "a healthy input stays armed"
        );
        assert!(editor.take_job_errors().is_empty());

        editor.set_fake_input(FakeInput {
            lost: true,
            ..FakeInput::default()
        });
        editor.tick();
        assert_eq!(editor.armed_track(), None);
        assert_eq!(
            editor.take_job_errors(),
            vec!["Input device lost.".to_string()]
        );
        // Nothing armed: the lost flag alone says nothing.
        editor.tick();
        assert!(editor.take_job_errors().is_empty());
    }

    #[test]
    fn the_armed_track_meter_reads_the_input_not_its_own_playback() {
        let mut editor = editor_with_tracks(2);
        let (a, b) = (track_id(&editor, 0), track_id(&editor, 1));
        editor.publish(); // builds the meter bank
        editor.toggle_arm(a);

        // Playback level on both tracks, a different input level.
        for meter in &editor.meters.tracks {
            meter.l.add(0.2, 0.04, 1, false);
            meter.r.add(0.2, 0.04, 1, false);
        }
        editor.input_meter.l.add(0.9, 0.81, 1, false);
        editor.input_meter.r.add(1.0, 1.0, 1, true);

        let (tracks, _master) = editor.read_meters();
        let reading = |id: Uuid| tracks.iter().find(|(t, _)| *t == id).unwrap().1;
        let armed = reading(a);
        assert_eq!((armed[0].0, armed[0].3), (0.9, false), "input left");
        assert_eq!(
            (armed[1].0, armed[1].3),
            (1.0, true),
            "input right, clipped"
        );
        let other = reading(b);
        assert_eq!(
            (other[0].0, other[1].0),
            (0.2, 0.2),
            "unarmed keeps playback"
        );

        // Both accumulators were emptied.
        let (tracks, _) = editor.read_meters();
        assert!(tracks.iter().all(|(_, [l, r])| l.0 == 0.0 && r.0 == 0.0));
    }

    #[test]
    fn a_disarmed_track_reads_its_own_meter_again() {
        let mut editor = editor_with_tracks(1);
        let id = track_id(&editor, 0);
        editor.publish();
        editor.toggle_arm(id);
        editor.toggle_arm(id);
        editor.meters.tracks[0].l.add(0.3, 0.09, 1, false);
        editor.input_meter.l.add(0.9, 0.81, 1, false);
        let (tracks, _) = editor.read_meters();
        assert_eq!(tracks[0].1[0].0, 0.3);
    }

    #[test]
    fn set_config_writes_through_to_the_injected_path() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut editor = editor_with_tracks(1);
        editor.set_config_path(path.clone());
        let config = AppConfig {
            input_device: Some("Line In".to_string()),
            record_channels: RecordChannels::Stereo,
            latency_offset_ms: 250, // clamped
        };
        editor.set_config(config).unwrap();
        let want = AppConfig {
            input_device: Some("Line In".to_string()),
            record_channels: RecordChannels::Stereo,
            latency_offset_ms: 100,
        };
        assert_eq!(editor.config(), &want);
        assert_eq!(config::load_from(&path), want);
        assert!(!editor.dirty, "settings are not part of the project");
    }

    #[test]
    fn set_config_without_a_config_path_fails_and_changes_nothing() {
        let mut editor = editor_with_tracks(1);
        let config = AppConfig {
            record_channels: RecordChannels::Mono2,
            ..AppConfig::default()
        };
        assert!(editor.set_config(config).is_err());
        assert_eq!(editor.config(), &AppConfig::default());
    }

    #[test]
    fn set_config_with_an_armed_track_reopens_and_a_failed_reopen_disarms() {
        let dir = tempdir().unwrap();
        let mut editor = editor_with_tracks(1);
        editor.set_config_path(dir.path().join("config.json"));
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);

        editor.set_config(AppConfig::default()).unwrap();
        assert_eq!(editor.armed_track(), Some(id), "reopen worked");

        editor.set_fake_input(FakeInput {
            open_fails: true,
            ..FakeInput::default()
        });
        let config = AppConfig {
            record_channels: RecordChannels::Stereo,
            ..AppConfig::default()
        };
        let error = editor.set_config(config.clone()).unwrap_err().to_string();
        assert!(error.contains("The track was disarmed."), "{error}");
        assert_eq!(editor.armed_track(), None);
        assert_eq!(editor.config(), &config, "the settings were still saved");
    }

    #[test]
    fn a_failed_save_leaves_an_armed_track_armed() {
        let dir = tempdir().unwrap();
        let mut editor = editor_with_tracks(1);
        // The "file" is a directory, so the rename over it fails.
        let path = dir.path().join("config.json");
        std::fs::create_dir(&path).unwrap();
        editor.set_config_path(path);
        let id = track_id(&editor, 0);
        editor.toggle_arm(id);
        assert!(editor.set_config(AppConfig::default()).is_err());
        assert_eq!(editor.armed_track(), Some(id));
        assert_eq!(editor.config(), &AppConfig::default());
    }

    #[test]
    fn the_fallback_notice_shows_once_and_only_for_a_named_device() {
        let noted = AtomicBool::new(false);
        // No saved name, or no fallback: nothing, and the flag stays unset.
        assert_eq!(fallback_notice(None, true, &noted), None);
        assert_eq!(fallback_notice(Some("USB Mic"), false, &noted), None);
        assert!(!noted.load(Ordering::Relaxed));
        // First fallback: the notice with the saved name.
        assert_eq!(
            fallback_notice(Some("USB Mic"), true, &noted).as_deref(),
            Some("Input device USB Mic not found. Using the default input.")
        );
        // Every later fallback in the same launch is silent.
        assert_eq!(fallback_notice(Some("USB Mic"), true, &noted), None);
        assert_eq!(fallback_notice(Some("Other"), true, &noted), None);
    }

    #[test]
    fn the_device_lists_come_from_the_test_double() {
        let mut editor = editor_with_tracks(1);
        assert!(editor.input_device_names().is_empty());
        assert_eq!(editor.default_input_name(), None);
        assert_eq!(editor.input_device_channels(None), None);

        editor.set_fake_input(FakeInput {
            default: Some("Mic".to_string()),
            devices: vec![("Mic".to_string(), 2), ("Line".to_string(), 1)],
            ..FakeInput::default()
        });
        assert_eq!(editor.input_device_names(), vec!["Mic", "Line"]);
        assert_eq!(editor.default_input_name().as_deref(), Some("Mic"));
        assert_eq!(editor.input_device_channels(None), Some(2));
        assert_eq!(editor.input_device_channels(Some("Line")), Some(1));
        // A name that is not found uses the default, like the real stream.
        assert_eq!(editor.input_device_channels(Some("Gone")), Some(2));
    }
}
