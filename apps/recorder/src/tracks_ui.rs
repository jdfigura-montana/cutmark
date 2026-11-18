//! The track list: one row per audio track with its controls and level meter, plus the WAV and format choices.
//!
//! See the recorder spec: Audio capture and tracks (Per-track controls) and UI, hotkeys, monitoring (Main window).

use std::time::{Duration, Instant};

use capture::{AudioDevice, DeviceKind};
use egui::{Response, RichText, Ui};
use ui_kit::palette::RECORD_RED;

use crate::config::{AudioCodecChoice, AudioSettings, MAX_OFFSET_MS, MAX_TRACKS, TrackSettings};
use crate::engine::session::TrackSnapshot;

/// The tooltip of every control that is locked while recording.
const LOCKED_TIP: &str = "Changes apply to the next take";
/// The tooltip of the disabled Add track button at the track limit.
const MAX_TRACKS_TIP: &str = "A take has at most 6 tracks";
/// How long a clip stays lit after it happens.
const CLIP_HOLD: Duration = Duration::from_secs(2);
/// The level shown for silence and while idle, in dBFS.
const METER_FLOOR_DBFS: f32 = -60.0;
/// How fast a shown peak falls back, in dB per second.
const METER_FALL_DB_PER_S: f32 = 40.0;
/// The lowest gain of a track, in dB.
const GAIN_MIN_DB: f32 = -30.0;
/// The highest gain of a track, in dB.
const GAIN_MAX_DB: f32 = 12.0;
/// Width of the sliders, in points.
const SLIDER_WIDTH: f32 = 130.0;
/// Width of the name field, in points.
const NAME_WIDTH: f32 = 130.0;
/// Width of the device dropdown, in points.
const DEVICE_WIDTH: f32 = 280.0;
/// Width of the level meter, in points.
const METER_WIDTH: f32 = 180.0;

/// What changed in the track panel this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TracksChanged {
    /// A setting changed value, so the settings need saving once the edit has ended.
    pub settings: bool,
    /// An edit has ended (a drag released, a field left or a click), so saving is due.
    pub commit: bool,
    /// A gain or mute changed, so the running session needs the new values.
    pub live: bool,
    /// The user asked to list the audio devices again.
    pub refresh_devices: bool,
}

impl TracksChanged {
    /// A one-step edit such as a click: the value changed and the edit is over.
    fn discrete(&mut self) {
        self.settings = true;
        self.commit = true;
    }

    /// A drag or typing edit: the value may change now, and the edit ends on release or focus loss.
    fn continuous(&mut self, response: &Response) {
        if response.changed() {
            self.settings = true;
        }
        if response.drag_stopped() || response.lost_focus() || response.clicked() {
            self.commit = true;
        }
    }
}

/// The level of one meter at one instant.
#[derive(Clone, Copy)]
struct Level {
    /// The shown peak in dBFS.
    peak_dbfs: f32,
    /// Whether the clip light is lit.
    clipped: bool,
}

/// One track's shown peak and clip hold.
struct TrackLevel {
    /// The shown peak in dBFS, falling back slowly after a loud moment.
    peak_dbfs: f32,
    /// The instant the clip light goes out, when a clip is being held.
    clip_until: Option<Instant>,
}

/// The live meters of the running take, one per track of the session, in session order.
#[derive(Default)]
pub struct LiveTrackStats {
    /// One entry per track of the take.
    tracks: Vec<TrackLevel>,
    /// When the meters were last updated, to work out how far the peaks fall.
    updated: Option<Instant>,
}

impl LiveTrackStats {
    /// Folds one session snapshot into the meters: peaks fall back slowly and a clip stays lit for two seconds.
    pub fn update(&mut self, snapshots: &[TrackSnapshot], now: Instant) {
        let fall = self.updated.map_or(0.0, |then| now.saturating_duration_since(then).as_secs_f32() * METER_FALL_DB_PER_S);
        self.tracks.resize_with(snapshots.len(), || TrackLevel { peak_dbfs: -100.0, clip_until: None });
        for (level, snapshot) in self.tracks.iter_mut().zip(snapshots) {
            level.peak_dbfs = snapshot.peak_dbfs.max(level.peak_dbfs - fall);
            if snapshot.clipped {
                level.clip_until = Some(now + CLIP_HOLD);
            }
        }
        self.updated = Some(now);
    }

    /// Forgets every meter, for when a take ends.
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.updated = None;
    }

    /// The meter of a session track at `now`; silent for a track the take does not have.
    fn level(&self, index: usize, now: Instant) -> Level {
        match self.tracks.get(index) {
            Some(level) => Level { peak_dbfs: level.peak_dbfs, clipped: level.clip_until.is_some_and(|until| now < until) },
            None => IDLE_LEVEL,
        }
    }
}

/// The meter of an idle track.
const IDLE_LEVEL: Level = Level { peak_dbfs: METER_FLOOR_DBFS, clipped: false };

/// What a row needs besides its track.
#[derive(Clone, Copy)]
struct RowInput<'a> {
    /// The track's place in the list, to keep widget IDs apart.
    index: usize,
    /// The audio devices that are present.
    devices: &'a [AudioDevice],
    /// Whether a take is running.
    recording: bool,
    /// The meter to show.
    level: Level,
}

/// Draws the track list, the WAV and format choices and the Add track and Refresh devices buttons.
///
/// While `recording` is true only gain and mute can change. `live` holds the running take's meters.
pub fn tracks_panel(ui: &mut Ui, settings: &mut AudioSettings, devices: &[AudioDevice], live: Option<&LiveTrackStats>, recording: bool) -> TracksChanged {
    let mut changed = TracksChanged::default();
    let now = Instant::now();
    ui.heading("Audio tracks");
    let mut remove_at = None;
    // A running take only has the enabled tracks, and the list cannot change while it runs.
    let mut session_index = 0;
    for (index, track) in settings.tracks.iter_mut().enumerate() {
        let level = match live {
            Some(live) if track.enabled => live.level(session_index, now),
            _ => IDLE_LEVEL,
        };
        if track.enabled {
            session_index += 1;
        }
        ui.push_id(index, |ui| {
            if track_row(ui, track, RowInput { index, devices, recording, level }, &mut changed) {
                remove_at = Some(index);
            }
        });
        ui.separator();
    }
    if let Some(index) = remove_at {
        settings.tracks.remove(index);
        changed.discrete();
    }
    if settings.tracks.is_empty() {
        ui.label("No audio tracks. Add one to record sound.");
    }

    ui.horizontal(|ui| {
        let can_add = !recording && settings.tracks.len() < MAX_TRACKS;
        let tip = if recording { LOCKED_TIP } else { MAX_TRACKS_TIP };
        if ui.add_enabled(can_add, egui::Button::new("Add track")).on_disabled_hover_text(tip).clicked() {
            settings.tracks.push(new_track(settings.tracks.len(), devices));
            changed.discrete();
        }
        if ui.button("Refresh devices").clicked() {
            changed.refresh_devices = true;
        }
    });

    ui.add_space(4.0);
    if locked(ui, recording, |ui| ui.checkbox(&mut settings.wav_sidecars, "Also save each track as a WAV file")).changed() {
        changed.discrete();
    }
    ui.horizontal(|ui| {
        ui.label("Audio format");
        for (choice, label) in [(AudioCodecChoice::Aac320, "AAC 320 kbps"), (AudioCodecChoice::Pcm24, "PCM 24-bit")] {
            let selected = settings.codec == choice;
            if locked(ui, recording, |ui| ui.radio(selected, label)).clicked() && !selected {
                settings.codec = choice;
                changed.discrete();
            }
        }
    });
    changed
}

/// Draws one track as two lines and returns true when its Remove button was clicked.
fn track_row(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, changed: &mut TracksChanged) -> bool {
    let recording = input.recording;
    let mut remove = false;
    ui.vertical(|ui| {
        ui.horizontal_wrapped(|ui| {
            if locked(ui, recording, |ui| ui.checkbox(&mut track.enabled, "On")).changed() {
                changed.discrete();
            }
            let name = locked(ui, recording, |ui| ui.add(egui::TextEdit::singleline(&mut track.name).desired_width(NAME_WIDTH)));
            changed.continuous(&name);
            device_picker(ui, track, input, changed);
            if locked(ui, recording, |ui| ui.button("Remove")).clicked() {
                remove = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = SLIDER_WIDTH;
            ui.label("Gain");
            let gain = ui.add(egui::Slider::new(&mut track.gain_db, GAIN_MIN_DB..=GAIN_MAX_DB).suffix(" dB").fixed_decimals(1));
            changed.continuous(&gain);
            if gain.changed() {
                track.gain_db = track.gain_db.clamp(GAIN_MIN_DB, GAIN_MAX_DB);
                changed.live = true;
            }
            let mute = ui.toggle_value(&mut track.muted, "Mute");
            if mute.changed() {
                changed.discrete();
                changed.live = true;
            }
            ui.label("Offset");
            let offset = locked(ui, recording, |ui| ui.add(egui::Slider::new(&mut track.offset_ms, -MAX_OFFSET_MS..=MAX_OFFSET_MS).suffix(" ms")));
            changed.continuous(&offset);
            if offset.changed() {
                track.offset_ms = track.offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
            }
            if locked(ui, recording, |ui| ui.checkbox(&mut track.in_mixdown, "In mixdown")).changed() {
                changed.discrete();
            }
            ui_kit::level_meter(ui, input.level.peak_dbfs, input.level.clipped, METER_WIDTH);
        });
    });
    remove
}

/// Draws the device dropdown of a track; a saved device that is not present shows its name in red with "(missing)".
fn device_picker(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, changed: &mut TracksChanged) {
    let found = find_device(track, input.devices);
    let found_id = found.map(|device| device.id.clone());
    let text = match found {
        Some(device) => RichText::new(device_entry(device)),
        None if track.device_name.is_empty() => RichText::new("No device selected"),
        None => RichText::new(format!("{} (missing)", track.device_name)).color(RECORD_RED),
    };
    let mut picked: Option<&AudioDevice> = None;
    locked(ui, input.recording, |ui| {
        egui::ComboBox::from_id_salt(("track-device", input.index))
            .selected_text(text)
            .width(DEVICE_WIDTH)
            .show_ui(ui, |ui| {
                for device in input.devices {
                    let selected = found_id.as_deref() == Some(device.id.as_str());
                    if ui.selectable_label(selected, device_entry(device)).clicked() {
                        picked = Some(device);
                    }
                }
            })
            .response
    });
    if let Some(device) = picked
        && found_id.as_deref() != Some(device.id.as_str())
    {
        track.device_id = device.id.clone();
        track.device_name = device.name.clone();
        track.kind = device.kind.into();
        changed.discrete();
    }
}

/// Adds a widget that is disabled while recording, with the "next take" tooltip on it.
fn locked(ui: &mut Ui, recording: bool, add: impl FnOnce(&mut Ui) -> Response) -> Response {
    let response = ui.add_enabled_ui(!recording, add).inner;
    if recording { response.on_disabled_hover_text(LOCKED_TIP) } else { response }
}

/// The device a track points at: by endpoint ID, else by exact name.
fn find_device<'a>(track: &TrackSettings, devices: &'a [AudioDevice]) -> Option<&'a AudioDevice> {
    devices.iter().find(|device| device.id == track.device_id).or_else(|| devices.iter().find(|device| device.name == track.device_name))
}

/// The dropdown label of a device.
fn device_entry(device: &AudioDevice) -> String {
    match device.kind {
        DeviceKind::Output => format!("Output (loopback): {}", device.name),
        DeviceKind::Input => format!("Input: {}", device.name),
    }
}

/// A new track on the default input device, or on the first device when there is none.
fn new_track(count: usize, devices: &[AudioDevice]) -> TrackSettings {
    let mut track = TrackSettings { name: format!("Track {}", count + 1), ..TrackSettings::default() };
    let device = devices.iter().find(|device| device.kind == DeviceKind::Input && device.is_default).or_else(|| devices.first());
    if let Some(device) = device {
        track.device_id = device.id.clone();
        track.device_name = device.name.clone();
        track.kind = device.kind.into();
    }
    track
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot with the given peak and clip state.
    fn snapshot(peak_dbfs: f32, clipped: bool) -> TrackSnapshot {
        TrackSnapshot { name: "Mic".to_owned(), peak_dbfs, clipped, packets: 0, overflows: 0, device_lost: false }
    }

    #[test]
    fn a_clip_stays_lit_for_two_seconds() {
        let start = Instant::now();
        let mut live = LiveTrackStats::default();
        live.update(&[snapshot(0.0, true)], start);
        assert!(live.level(0, start + Duration::from_millis(1900)).clipped);
        live.update(&[snapshot(-30.0, false)], start + Duration::from_millis(1900));
        assert!(live.level(0, start + Duration::from_millis(1900)).clipped);
        assert!(!live.level(0, start + Duration::from_millis(2100)).clipped);
    }

    #[test]
    fn peaks_fall_back_slowly() {
        let start = Instant::now();
        let mut live = LiveTrackStats::default();
        live.update(&[snapshot(-6.0, false)], start);
        live.update(&[snapshot(-100.0, false)], start + Duration::from_millis(500));
        let shown = live.level(0, start).peak_dbfs;
        assert!((shown - -26.0).abs() < 0.01, "{shown}");
    }

    #[test]
    fn an_unknown_track_is_silent() {
        let live = LiveTrackStats::default();
        let level = live.level(3, Instant::now());
        assert_eq!(level.peak_dbfs, METER_FLOOR_DBFS);
        assert!(!level.clipped);
    }

    #[test]
    fn a_new_track_takes_the_default_input() {
        let devices = [
            AudioDevice { id: "out".to_owned(), name: "Speakers".to_owned(), kind: DeviceKind::Output, is_default: true, sample_rate: 48_000, channels: 2 },
            AudioDevice { id: "mic".to_owned(), name: "Microphone".to_owned(), kind: DeviceKind::Input, is_default: true, sample_rate: 48_000, channels: 2 },
        ];
        let track = new_track(2, &devices);
        assert_eq!(track.name, "Track 3");
        assert_eq!(track.device_id, "mic");
        assert!(new_track(0, &[]).device_id.is_empty());
    }
}
