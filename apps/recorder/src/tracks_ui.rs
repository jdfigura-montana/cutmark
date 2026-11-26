//! The Audio tool's panel: one inspector group per track, the Add track and Refresh devices
//! buttons and the Output group; plus the audio rows the Sources panel shows (spec "Setup mode").
//!
//! See the recorder spec: Audio capture and tracks (Per-track controls).

use std::time::{Duration, Instant};

use capture::{AudioDevice, DeviceKind};
use egui::{ComboBox, Response, RichText, Ui};
use ui_kit::tokens::{color, space};
use ui_kit::{SliderKind, inspector_group, inspector_row, switch};

use crate::config::{AudioCodecChoice, AudioSettings, MAX_OFFSET_MS, MAX_TRACKS, TrackSettings};
use crate::engine::session::TrackSnapshot;
use crate::tools_ui::slider_row;

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
/// Width of the compact level meter slot in the Sources panel, in points.
const SOURCE_METER_SPAN: f32 = 160.0;
/// The gap left for the peak value at a Sources row's right edge, in points.
const SOURCE_METER_GAP: f32 = 64.0;
/// The entries of the Audio format choice.
const FORMAT_LABELS: [&str; 2] = ["AAC 320 kbps", "PCM 24-bit"];
/// The tooltips of the Audio format entries.
const FORMAT_TIPS: [&str; 2] = ["MPEG-4 AAC at 320 kbps", "Uncompressed PCM, 24-bit"];

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

/// Draws the Audio tool (spec "Setup mode"): one inspector group per track, then the Add track
/// and Refresh devices buttons, then the Output group with the WAV sidecars and the audio format.
///
/// While `recording` is true only gain and mute can change. `live` holds the running take's meters.
pub fn audio_panel(ui: &mut Ui, settings: &mut AudioSettings, devices: &[AudioDevice], live: Option<&LiveTrackStats>, recording: bool) -> TracksChanged {
    let mut changed = TracksChanged::default();
    let now = Instant::now();
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
        let mut remove = false;
        ui.push_id(index, |ui| {
            inspector_group(ui, |ui| track_group(ui, track, RowInput { index, devices, recording, level }, &mut changed, &mut remove));
        });
        if remove {
            remove_at = Some(index);
        }
    }
    if let Some(index) = remove_at {
        settings.tracks.remove(index);
        changed.discrete();
    }
    if settings.tracks.is_empty() {
        ui.weak("No audio tracks. Add one to record sound.");
    }
    ui.add_space(space::SCALE_4);
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
    ui.add_space(space::SCALE_8);
    output_group(ui, settings, recording, &mut changed);
    changed
}

/// Draws one track's inspector group (spec "Setup mode": On, Name, Device, Gain, Mute, Offset,
/// In mixdown, the level meter, Remove) and reports the edits through `changed`;
/// sets `*remove` when the Remove button was clicked.
fn track_group(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, changed: &mut TracksChanged, remove: &mut bool) {
    let recording = input.recording;
    let on_text = if track.enabled { "On" } else { "Off" };
    inspector_row(
        ui,
        "On",
        |ui| {
            let mut on = track.enabled;
            if locked(ui, recording, |ui| switch(ui, &mut on)).changed() {
                track.enabled = on;
                changed.discrete();
            }
        },
        Some(on_text),
    );
    inspector_row(
        ui,
        "Name",
        |ui| {
            let name = locked(ui, recording, |ui| ui.add(egui::TextEdit::singleline(&mut track.name).desired_width(ui.available_width())));
            changed.continuous(&name);
        },
        None,
    );
    device_row(ui, track, input, changed);
    // The unit is in the label: "-30.0 dB" is wider than the value column.
    let gain_text = format!("{:.1}", track.gain_db);
    slider_row(ui, "Gain (dB)", &gain_text, |slider_ui, switch_ui| {
        let mut gain = track.gain_db;
        if ui_kit::slider(slider_ui, &mut gain, GAIN_MIN_DB..=GAIN_MAX_DB, SliderKind::Accent).changed() {
            track.gain_db = gain.clamp(GAIN_MIN_DB, GAIN_MAX_DB);
            changed.settings = true;
            changed.live = true;
        }
        let mute = switch(switch_ui, &mut track.muted);
        if mute.changed() {
            changed.discrete();
            changed.live = true;
        }
    });
    // No row value: the slider has its own value box, which can be typed into.
    inspector_row(
        ui,
        "Offset",
        |ui| {
            ui.spacing_mut().slider_width = SLIDER_WIDTH;
            let offset = locked(ui, recording, |ui| ui.add(egui::Slider::new(&mut track.offset_ms, -MAX_OFFSET_MS..=MAX_OFFSET_MS).suffix(" ms")));
            changed.continuous(&offset);
            if offset.changed() {
                track.offset_ms = track.offset_ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS);
            }
        },
        None,
    );
    inspector_row(
        ui,
        "In mixdown",
        |ui| {
            if locked(ui, recording, |ui| switch(ui, &mut track.in_mixdown)).changed() {
                changed.discrete();
            }
            let width = (ui.available_width() - ui.spacing().item_spacing.x).max(60.0);
            ui_kit::level_meter(ui, input.level.peak_dbfs, input.level.clipped, width);
        },
        None,
    );
    inspector_row(
        ui,
        "",
        |ui| {
            if ui.button("Remove").clicked() {
                *remove = true;
            }
        },
        None,
    );
}

/// Draws the device dropdown of a track; a saved device that is not present shows its name in
/// red with "(missing)".
fn device_row(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, changed: &mut TracksChanged) {
    let found = find_device(track, input.devices);
    let found_id = found.map(|device| device.id.clone());
    let text = match found {
        Some(device) => RichText::new(device_entry(device)),
        None if track.device_name.is_empty() => RichText::new("No device selected"),
        None => RichText::new(format!("{} (missing)", track.device_name)).color(color::SEM_RECORD),
    };
    let mut picked: Option<&AudioDevice> = None;
    inspector_row(
        ui,
        "Device",
        |ui| {
            locked(ui, input.recording, |ui| {
                ComboBox::from_id_salt(("track-device", input.index))
                    .selected_text(text.clone())
                    .width(ui.available_width())
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
        },
        None,
    );
    if let Some(device) = picked
        && found_id.as_deref() != Some(device.id.as_str())
    {
        track.device_id = device.id.clone();
        track.device_name = device.name.clone();
        track.kind = device.kind.into();
        changed.discrete();
    }
}

/// Draws the Output group (spec "Setup mode"): whether each track is also saved as a WAV file,
/// and the audio format as a segmented choice.
fn output_group(ui: &mut Ui, settings: &mut AudioSettings, recording: bool, changed: &mut TracksChanged) {
    inspector_group(ui, |ui| {
        let wav_text = if settings.wav_sidecars { "On" } else { "Off" };
        inspector_row(
            ui,
            "Also save each track as a WAV file",
            |ui| {
                let mut on = settings.wav_sidecars;
                if locked(ui, recording, |ui| switch(ui, &mut on)).changed() {
                    settings.wav_sidecars = on;
                    changed.discrete();
                }
            },
            Some(wav_text),
        );
        inspector_row(
            ui,
            "Audio format",
            |ui| {
                let mut index = match settings.codec {
                    AudioCodecChoice::Aac320 => 0,
                    AudioCodecChoice::Pcm24 => 1,
                };
                locked(ui, recording, |ui| ui_kit::segmented_filled(ui, "audio-format", &mut index, &FORMAT_LABELS, &FORMAT_TIPS));
                let picked = if index == 0 { AudioCodecChoice::Aac320 } else { AudioCodecChoice::Pcm24 };
                if picked != settings.codec {
                    settings.codec = picked;
                    changed.discrete();
                }
            },
            None,
        );
    });
}

/// Draws the Sources panel's audio rows (spec "Setup mode"): one list row per enabled track
/// with its name, a compact level meter and the peak value. Info only; nothing is editable here.
pub fn sources_rows(ui: &mut Ui, settings: &AudioSettings, live: Option<&LiveTrackStats>) {
    let now = Instant::now();
    let mut session_index = 0;
    let mut rows = 0;
    for track in &settings.tracks {
        if !track.enabled {
            continue;
        }
        let level = match live {
            Some(live) => live.level(session_index, now),
            None => IDLE_LEVEL,
        };
        session_index += 1;
        rows += 1;
        let peak_text = format!("{:.0} dB", level.peak_dbfs.max(METER_FLOOR_DBFS));
        let response = ui_kit::list_row(ui, false, None, &track.name, Some(&peak_text));
        // The compact level meter is painted into a slot of the row, left of the peak value.
        let rect = response.rect;
        let meter_rect = egui::Rect::from_min_max(
            egui::Pos2::new(rect.right() - SOURCE_METER_SPAN, rect.top() + space::SCALE_4),
            egui::Pos2::new(rect.right() - SOURCE_METER_GAP, rect.bottom() - space::SCALE_4),
        );
        let mut meter_ui = ui.new_child(egui::UiBuilder::new().max_rect(meter_rect));
        ui_kit::level_meter(&mut meter_ui, level.peak_dbfs, level.clipped, meter_rect.width());
    }
    if rows == 0 {
        ui.weak("No enabled audio tracks.");
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
    use ui_kit::tokens::size;

    use super::*;
    use crate::config;
    use crate::tools_ui::row_checks::{assert_no_overlap, controls, painted, panel_harness};

    /// A snapshot with the given peak and clip state.
    fn snapshot(peak_dbfs: f32, clipped: bool) -> TrackSnapshot {
        TrackSnapshot { name: "Mic".to_owned(), peak_dbfs, clipped, packets: 0, overflows: 0, device_lost: false }
    }

    /// A real track group paints no text over another at the narrowest right panel, and its Gain
    /// row keeps label, slider, mute switch and value apart, at both ends of the gain and offset
    /// ranges.
    #[test]
    fn track_rows_do_not_overlap_at_the_narrowest_panel() {
        for (gain_db, shown, offset_ms) in [(GAIN_MIN_DB, "-30.0", -MAX_OFFSET_MS), (GAIN_MAX_DB, "12.0", MAX_OFFSET_MS)] {
            let mut track = TrackSettings { name: "Mic".to_owned(), gain_db, offset_ms, ..TrackSettings::default() };
            let harness = panel_harness(config::RIGHT_PANEL_MIN, move |ui| {
                let input = RowInput { index: 0, devices: &[], recording: false, level: IDLE_LEVEL };
                inspector_group(ui, |ui| track_group(ui, &mut track, input, &mut TracksChanged::default(), &mut false));
            });
            let texts = painted(&harness);
            let label = texts.iter().find(|painted| painted.text == "Gain (dB)").unwrap().rect;
            let row = label.center().y - size::INSPECTOR_ROW_HEIGHT / 2.0..=label.center().y + size::INSPECTOR_ROW_HEIGHT / 2.0;
            let in_row: Vec<_> = texts.iter().filter(|painted| row.contains(&painted.rect.center().y)).collect();
            assert_eq!(in_row.len(), 2, "the label and the value");
            assert!(in_row.iter().any(|painted| painted.text == shown), "the value {shown} is shown whole");
            let controls: Vec<_> = controls(&harness).into_iter().filter(|rect| row.contains(&rect.center().y)).collect();
            assert_eq!(controls.len(), 2, "the slider and the mute switch");
            assert_no_overlap(&texts.iter().collect::<Vec<_>>(), &controls, config::RIGHT_PANEL_MIN);
        }
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
