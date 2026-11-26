//! The Audio tool's panel: one inspector group per track, the Add track and Refresh devices
//! buttons and the Output group; plus the audio rows the Sources panel shows (spec "Setup mode").
//!
//! See the recorder spec: Audio capture and tracks (Per-track controls).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use capture::{AppTarget, AudioApp, AudioDevice, DeviceKind, list_audio_apps};
use egui::{ComboBox, Response, RichText, Ui};
use ui_kit::tokens::{color, space};
use ui_kit::{SliderKind, inspector_group, inspector_row, switch};

use crate::config::{AudioCodecChoice, AudioSettings, MAX_OFFSET_MS, MAX_TRACKS, TrackKind, TrackSettings};
use crate::engine::app_monitor::IdleAppTrack;
use crate::engine::session::{Instances, TrackSnapshot};
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
/// The entries of a track's Source choice, in [`source_index`] order.
const SOURCE_LABELS: [&str; 3] = ["Device", "App", "Desktop except"];
/// The Source choice of an app track.
const SOURCE_APP: usize = 1;
/// The Source choice of a desktop-except track.
const SOURCE_EXCEPT: usize = 2;
/// What a Sources row shows in place of the peak while its app is not attached.
const WAITING_TEXT: &str = "waiting";
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
    /// What the track's capture is doing.
    status: AppStatus,
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
        self.tracks.resize_with(snapshots.len(), || TrackLevel { peak_dbfs: -100.0, clip_until: None, status: AppStatus::default() });
        for (level, snapshot) in self.tracks.iter_mut().zip(snapshots) {
            level.peak_dbfs = snapshot.peak_dbfs.max(level.peak_dbfs - fall);
            level.status = AppStatus { attached: snapshot.attached, instances: snapshot.instances };
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

    /// What a session track's capture is doing; None before the take has reported it.
    fn status(&self, index: usize) -> Option<AppStatus> {
        self.tracks.get(index).map(|level| level.status)
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

/// What an app or desktop-except track's capture is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct AppStatus {
    /// Whether the app is being captured, or for a desktop-except track being left out.
    attached: bool,
    /// The app instances a desktop-except track leaves out.
    instances: Instances,
}

/// What the window keeps about app and desktop-except tracks between frames.
#[derive(Default)]
pub struct AppSources {
    /// The idle watch's readings; empty while a take runs.
    idle: Vec<IdleAppTrack>,
    /// The apps an app track was attached to at some point, to tell "closed" from "waiting".
    seen: BTreeSet<String>,
    /// The open app dropdown.
    picker: AppPicker,
}

/// The app dropdown that is open, with the listing read when it opened.
#[derive(Default)]
struct AppPicker {
    /// The track whose dropdown is open.
    open_row: Option<usize>,
    /// Whether the list was expanded to all running apps.
    show_all: bool,
    /// The listed apps, the ones making sound first.
    apps: Vec<AudioApp>,
}

impl AppSources {
    /// Takes in this frame's state: the idle watch's readings, or `live` while a take runs.
    pub fn observe(&mut self, tracks: &[TrackSettings], idle: &[IdleAppTrack], live: Option<&LiveTrackStats>) {
        self.idle = idle.to_vec();
        let mut session_index = 0;
        for (index, track) in tracks.iter().enumerate() {
            if !track.enabled {
                continue;
            }
            if let Some((exe, _, false)) = app_of(&track.kind)
                && self.status(index, track, session_index, live).is_some_and(|status| status.attached)
            {
                self.seen.insert(exe.to_owned());
            }
            session_index += 1;
        }
    }

    /// The idle reading of a settings track, only while it still names the same app.
    fn idle_reading(&self, index: usize, track: &TrackSettings) -> Option<&IdleAppTrack> {
        let (exe, _, except) = app_of(&track.kind)?;
        let exe = AppTarget::new(exe);
        self.idle.iter().find(|reading| reading.index == index && reading.app.except == except && reading.app.exe == exe.exe())
    }

    /// What an app track's capture is doing; None for a device or disabled track, or before it is known.
    fn status(&self, index: usize, track: &TrackSettings, session_index: usize, live: Option<&LiveTrackStats>) -> Option<AppStatus> {
        if !track.enabled || app_of(&track.kind).is_none() {
            return None;
        }
        match live {
            Some(live) => live.status(session_index),
            None => self.idle_reading(index, track).map(|reading| AppStatus { attached: reading.attached, instances: reading.instances }),
        }
    }

    /// The meter of a settings track: the take's, or while idle the watch's for an app track.
    fn level(&self, index: usize, track: &TrackSettings, session_index: usize, live: Option<&LiveTrackStats>, now: Instant) -> Level {
        if !track.enabled {
            return IDLE_LEVEL;
        }
        match live {
            Some(live) => live.level(session_index, now),
            None => self.idle_reading(index, track).map_or(IDLE_LEVEL, |reading| Level { peak_dbfs: reading.peak_dbfs, clipped: false }),
        }
    }
}

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
    /// What the capture of an app or desktop-except track is doing, when known.
    status: Option<AppStatus>,
}

/// Draws the Audio tool (spec "Setup mode"): one inspector group per track, then the Add track
/// and Refresh devices buttons, then the Output group with the WAV sidecars and the audio format.
///
/// While `recording` is true only gain and mute can change. `live` holds the running take's meters.
pub fn audio_panel(
    ui: &mut Ui,
    settings: &mut AudioSettings,
    devices: &[AudioDevice],
    apps: &mut AppSources,
    live: Option<&LiveTrackStats>,
    recording: bool,
) -> TracksChanged {
    let mut changed = TracksChanged::default();
    let now = Instant::now();
    let mut remove_at = None;
    // A running take only has the enabled tracks, and the list cannot change while it runs.
    let mut session_index = 0;
    for (index, track) in settings.tracks.iter_mut().enumerate() {
        let level = apps.level(index, track, session_index, live, now);
        let status = apps.status(index, track, session_index, live);
        if track.enabled {
            session_index += 1;
        }
        let mut remove = false;
        ui.push_id(index, |ui| {
            let input = RowInput { index, devices, recording, level, status };
            inspector_group(ui, |ui| track_group(ui, track, input, apps, &mut changed, &mut remove));
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
        let menu = ui.add_enabled_ui(can_add, |ui| {
            ui.menu_button("Add track", |ui| {
                let count = settings.tracks.len();
                let mut added = None;
                if ui.button("Device track").clicked() {
                    added = Some(new_track(count, devices));
                }
                if ui.button("App track").clicked() {
                    added = Some(new_app_track(count, SOURCE_APP));
                }
                if ui.button("Desktop except an app").clicked() {
                    added = Some(new_app_track(count, SOURCE_EXCEPT));
                }
                if let Some(track) = added {
                    settings.tracks.push(track);
                    changed.discrete();
                    ui.close();
                }
            })
            .response
        });
        menu.inner.on_disabled_hover_text(tip);
        if ui.button("Refresh devices").clicked() {
            changed.refresh_devices = true;
        }
    });
    ui.add_space(space::SCALE_8);
    output_group(ui, settings, recording, &mut changed);
    changed
}

/// Draws one track's inspector group (spec "Setup mode": On, Name, Source, Device or App, Gain, Mute,
/// Offset, In mixdown, the level meter, Remove) and reports the edits through `changed`;
/// sets `*remove` when the Remove button was clicked.
fn track_group(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, apps: &mut AppSources, changed: &mut TracksChanged, remove: &mut bool) {
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
    let source_changed = source_row(ui, track, input, changed);
    if let Some((exe, app_name, except)) = app_of(&track.kind) {
        // A reading taken before a source change this frame belongs to the old source.
        if let Some(status) = input.status.filter(|_| !source_changed)
            && !exe.is_empty()
        {
            caption_line(ui, &source_caption(app_name, except, status.attached, apps.seen.contains(exe)), color::TEXT_TERTIARY);
            if let Some(warning) = except.then(|| instances_warning(app_name, status.instances)).flatten() {
                caption_line(ui, &warning, color::SEM_METER_WARN);
            }
        }
        app_row(ui, track, input, &mut apps.picker, changed);
    } else {
        device_row(ui, track, input, changed);
    }
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

/// Draws the Source row: its label, then the choice on a line of its own, which is too wide to sit
/// beside the label in a narrow panel. Returns whether the source changed.
fn source_row(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, changed: &mut TracksChanged) -> bool {
    inspector_row(ui, "Source", |_| {}, None);
    let before = source_index(&track.kind);
    let mut source = before;
    locked(ui, input.recording, |ui| ui_kit::segmented(ui, egui::Id::new(("track-source", input.index)), &mut source, &SOURCE_LABELS));
    ui.add_space(space::SCALE_4);
    if source == before || input.recording {
        return false;
    }
    set_source(track, source, input.devices);
    changed.discrete();
    true
}

/// One line of small text under the Source row.
fn caption_line(ui: &mut Ui, text: &str, text_color: egui::Color32) {
    ui.add(egui::Label::new(RichText::new(text).small().color(text_color)).wrap());
    ui.add_space(space::SCALE_4);
}

/// What an app or desktop-except track is doing, in words (per-app audio spec "UI").
/// `was_attached` tells an app that closed from one that has not opened yet.
fn source_caption(app_name: &str, except: bool, attached: bool, was_attached: bool) -> String {
    match (except, attached) {
        (false, true) => "Attached".to_owned(),
        (false, false) if was_attached => format!("{app_name} closed"),
        (false, false) => format!("Waiting for {app_name} to open"),
        (true, true) => format!("Excluding {app_name}"),
        (true, false) => format!("{app_name} not running, capturing all desktop audio"),
    }
}

/// The warning of a desktop-except track when Windows leaves out only some of the app's instances.
fn instances_warning(app_name: &str, instances: Instances) -> Option<String> {
    let Instances { excluded, total } = instances;
    (excluded < total).then(|| format!("{excluded} of {total} instances of {app_name} left out; the others are still heard"))
}

/// Draws the app dropdown of an app or desktop-except track. The apps are listed when it opens.
fn app_row(ui: &mut Ui, track: &mut TrackSettings, input: RowInput<'_>, picker: &mut AppPicker, changed: &mut TracksChanged) {
    let Some((exe, app_name, _)) = app_of(&track.kind) else {
        return;
    };
    let text = if exe.is_empty() { "No app selected".to_owned() } else { app_entry(app_name, exe) };
    let current = exe.to_owned();
    let mut picked: Option<(String, String)> = None;
    inspector_row(
        ui,
        "App",
        |ui| {
            locked(ui, input.recording, |ui| {
                let combo = ComboBox::from_id_salt(("track-app", input.index))
                    .selected_text(text)
                    .width(ui.available_width())
                    .truncate()
                    // "Show all running apps" must not close the list.
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show_ui(ui, |ui| {
                        if picker.open_row != Some(input.index) {
                            *picker = AppPicker { open_row: Some(input.index), show_all: false, apps: running_apps(false) };
                        }
                        if picker.apps.is_empty() {
                            ui.weak("No apps found");
                        }
                        for app in &picker.apps {
                            if app_option(ui, app, app.exe == current) {
                                picked = Some((app.exe.clone(), app.display_name.clone()));
                                ui.close();
                            }
                        }
                        if !picker.show_all && ui.button("Show all running apps").clicked() {
                            picker.show_all = true;
                            picker.apps = running_apps(true);
                        }
                    });
                if combo.inner.is_none() && picker.open_row == Some(input.index) {
                    picker.open_row = None;
                }
                combo.response
            });
        },
        None,
    );
    if let Some((new_exe, new_name)) = picked
        && let TrackKind::App { exe, app_name } | TrackKind::DesktopExcept { exe, app_name } = &mut track.kind
        && *exe != new_exe
    {
        *exe = new_exe;
        *app_name = new_name;
        changed.discrete();
    }
}

/// One entry of the app dropdown, with a small dot while the app is making sound. Returns whether it was clicked.
fn app_option(ui: &mut Ui, app: &AudioApp, selected: bool) -> bool {
    ui.horizontal(|ui| {
        let (slot, _) = ui.allocate_exact_size(egui::Vec2::splat(space::SCALE_8), egui::Sense::hover());
        if app.active {
            ui.painter().circle_filled(slot.center(), space::SCALE_8 / 2.0, color::SEM_SUCCESS);
        }
        ui.selectable_label(selected, app_entry(&app.display_name, &app.exe)).clicked()
    })
    .inner
}

/// The running apps, the ones making sound first.
fn running_apps(include_all: bool) -> Vec<AudioApp> {
    // The listing sets up COM for itself, which the window's thread has already set up another way.
    let listing = std::thread::spawn(move || list_audio_apps(include_all)).join();
    let mut apps = match listing {
        Ok(Ok(apps)) => apps,
        Ok(Err(err)) => {
            log::warn!("the running apps could not be listed: {err}");
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    active_first(&mut apps);
    apps
}

/// Moves the apps that are making sound to the front, keeping the listing's order otherwise.
fn active_first(apps: &mut [AudioApp]) {
    apps.sort_by_key(|app| !app.active);
}

/// The label of an app: its display name and executable.
fn app_entry(display_name: &str, exe: &str) -> String {
    format!("{display_name} ({exe})")
}

/// The executable, display name and "except" flag of an app or desktop-except track; None for a device track.
fn app_of(kind: &TrackKind) -> Option<(&str, &str, bool)> {
    match kind {
        TrackKind::App { exe, app_name } => Some((exe, app_name, false)),
        TrackKind::DesktopExcept { exe, app_name } => Some((exe, app_name, true)),
        TrackKind::Input | TrackKind::Output => None,
    }
}

/// The Source choice a track kind shows as.
fn source_index(kind: &TrackKind) -> usize {
    match kind {
        TrackKind::Input | TrackKind::Output => 0,
        TrackKind::App { .. } => SOURCE_APP,
        TrackKind::DesktopExcept { .. } => SOURCE_EXCEPT,
    }
}

/// Switches a track's source. The picked app stays between App and Desktop except, and the saved
/// device stays while an app source is chosen.
fn set_source(track: &mut TrackSettings, source: usize, devices: &[AudioDevice]) {
    let (exe, app_name) = app_of(&track.kind).map(|(exe, name, _)| (exe.to_owned(), name.to_owned())).unwrap_or_default();
    track.kind = match source {
        SOURCE_APP => TrackKind::App { exe, app_name },
        SOURCE_EXCEPT => TrackKind::DesktopExcept { exe, app_name },
        _ => match find_device(track, devices) {
            Some(device) => device.kind.into(),
            None => TrackKind::Input,
        },
    };
    // A track that never had a device gets the one a new track would.
    if source_index(&track.kind) == 0 && track.device_id.is_empty() && track.device_name.is_empty() {
        let device = new_track(0, devices);
        track.device_id = device.device_id;
        track.device_name = device.device_name;
        track.kind = device.kind;
    }
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
/// with its name (the app's for an app or desktop-except track), a compact level meter and the
/// peak value, or "waiting" while its app is not attached. Info only; nothing is editable here.
pub fn sources_rows(ui: &mut Ui, settings: &AudioSettings, apps: &AppSources, live: Option<&LiveTrackStats>) {
    let now = Instant::now();
    let mut session_index = 0;
    let mut rows = 0;
    for (index, track) in settings.tracks.iter().enumerate() {
        if !track.enabled {
            continue;
        }
        let level = apps.level(index, track, session_index, live, now);
        let attached = apps.status(index, track, session_index, live).is_some_and(|status| status.attached);
        session_index += 1;
        rows += 1;
        let (name, trailing) = source_row_text(track, attached, level.peak_dbfs);
        // The row's trailing text is drawn in text.tertiary.
        let response = ui_kit::list_row(ui, false, None, name, Some(&trailing));
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

/// The name and trailing text of a Sources row.
fn source_row_text(track: &TrackSettings, attached: bool, peak_dbfs: f32) -> (&str, String) {
    let app = app_of(&track.kind);
    let name = match app {
        Some((_, app_name, _)) if !app_name.is_empty() => app_name,
        _ => &track.name,
    };
    let trailing = if app.is_some() && !attached { WAITING_TEXT.to_owned() } else { format!("{:.0} dB", peak_dbfs.max(METER_FLOOR_DBFS)) };
    (name, trailing)
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

/// A new app or desktop-except track with no app picked yet; `source` is its Source choice.
fn new_app_track(count: usize, source: usize) -> TrackSettings {
    let (exe, app_name) = (String::new(), String::new());
    let kind = if source == SOURCE_EXCEPT { TrackKind::DesktopExcept { exe, app_name } } else { TrackKind::App { exe, app_name } };
    TrackSettings { name: format!("Track {}", count + 1), kind, ..TrackSettings::default() }
}

#[cfg(test)]
mod tests {
    use ui_kit::tokens::size;

    use super::*;
    use crate::config;
    use crate::engine::session::AppSpec;
    use crate::tools_ui::row_checks::{assert_no_overlap, controls, painted, panel_harness};

    /// A snapshot with the given peak and clip state.
    fn snapshot(peak_dbfs: f32, clipped: bool) -> TrackSnapshot {
        TrackSnapshot {
            name: "Mic".to_owned(),
            peak_dbfs,
            clipped,
            packets: 0,
            overflows: 0,
            device_lost: false,
            attached: true,
            instances: Instances::default(),
        }
    }

    /// A real track group paints no text over another at the narrowest right panel, and its Gain
    /// row keeps label, slider, mute switch and value apart, at both ends of the gain and offset
    /// ranges.
    #[test]
    fn track_rows_do_not_overlap_at_the_narrowest_panel() {
        for (gain_db, shown, offset_ms) in [(GAIN_MIN_DB, "-30.0", -MAX_OFFSET_MS), (GAIN_MAX_DB, "12.0", MAX_OFFSET_MS)] {
            let mut track = TrackSettings { name: "Mic".to_owned(), gain_db, offset_ms, ..TrackSettings::default() };
            let harness = panel_harness(config::RIGHT_PANEL_MIN, move |ui| {
                let input = RowInput { index: 0, devices: &[], recording: false, level: IDLE_LEVEL, status: None };
                inspector_group(ui, |ui| track_group(ui, &mut track, input, &mut AppSources::default(), &mut TracksChanged::default(), &mut false));
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

    #[test]
    fn each_state_has_its_caption() {
        assert_eq!(source_caption("Discord", false, true, true), "Attached");
        assert_eq!(source_caption("Discord", false, false, false), "Waiting for Discord to open");
        assert_eq!(source_caption("Discord", false, false, true), "Discord closed");
        assert_eq!(source_caption("Spotify", true, true, false), "Excluding Spotify");
        assert_eq!(source_caption("Spotify", true, false, false), "Spotify not running, capturing all desktop audio");
        assert_eq!(source_caption("Spotify", true, false, true), "Spotify not running, capturing all desktop audio");
    }

    #[test]
    fn the_instances_warning_shows_only_when_some_are_still_heard() {
        assert_eq!(instances_warning("Spotify", Instances { excluded: 1, total: 1 }), None);
        assert_eq!(instances_warning("Spotify", Instances::default()), None);
        assert_eq!(
            instances_warning("Spotify", Instances { excluded: 1, total: 2 }).as_deref(),
            Some("1 of 2 instances of Spotify left out; the others are still heard")
        );
    }

    /// An app track on Discord.
    fn discord_track() -> TrackSettings {
        let kind = TrackKind::App { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned() };
        TrackSettings { name: "Chat".to_owned(), kind, ..TrackSettings::default() }
    }

    #[test]
    fn an_idle_reading_counts_only_for_its_own_app_and_an_attach_is_remembered() {
        let app = AppSpec { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned(), except: false };
        let reading = IdleAppTrack { index: 0, app, instances: Instances::default(), attached: true, peak_dbfs: -12.0 };
        let mut apps = AppSources::default();
        apps.observe(&[discord_track()], &[reading], None);
        assert!(apps.seen.contains("discord.exe"));
        assert!(apps.status(0, &discord_track(), 0, None).is_some_and(|status| status.attached));
        assert_eq!(apps.level(0, &discord_track(), 0, None, Instant::now()).peak_dbfs, -12.0);
        let mut other = discord_track();
        other.kind = TrackKind::App { exe: "spotify.exe".to_owned(), app_name: "Spotify".to_owned() };
        assert_eq!(apps.status(0, &other, 0, None), None);
    }

    #[test]
    fn a_sources_row_names_the_app_and_says_waiting_until_attached() {
        assert_eq!(source_row_text(&discord_track(), false, -100.0), ("Discord", "waiting".to_owned()));
        assert_eq!(source_row_text(&discord_track(), true, -12.0), ("Discord", "-12 dB".to_owned()));
        assert_eq!(source_row_text(&TrackSettings::default(), false, -100.0), ("Track", "-60 dB".to_owned()));
    }

    #[test]
    fn switching_the_source_keeps_the_app_and_the_device() {
        let mut track = TrackSettings { device_id: "mic".to_owned(), ..discord_track() };
        set_source(&mut track, SOURCE_EXCEPT, &[]);
        assert_eq!(app_of(&track.kind), Some(("discord.exe", "Discord", true)));
        set_source(&mut track, 0, &[]);
        assert_eq!(track.kind, TrackKind::Input);
        assert_eq!(track.device_id, "mic");
    }

    /// The widest app track group stays inside the narrowest right panel, the Source choice included.
    #[test]
    fn an_app_track_group_fits_the_narrowest_panel() {
        let mut track = discord_track();
        track.kind = TrackKind::DesktopExcept { exe: "discord.exe".to_owned(), app_name: "Discord".to_owned() };
        let harness = panel_harness(config::RIGHT_PANEL_MIN, move |ui| {
            let status = Some(AppStatus { attached: true, instances: Instances { excluded: 1, total: 2 } });
            let input = RowInput { index: 0, devices: &[], recording: false, level: IDLE_LEVEL, status };
            inspector_group(ui, |ui| track_group(ui, &mut track, input, &mut AppSources::default(), &mut TracksChanged::default(), &mut false));
        });
        let texts = painted(&harness);
        for wanted in ["Source", "Device", "App", "Desktop except", "Excluding Discord"] {
            assert!(texts.iter().any(|painted| painted.text == wanted), "{wanted} is painted");
        }
        assert!(texts.iter().all(|painted| painted.rect.right() <= config::RIGHT_PANEL_MIN), "nothing leaves the panel");
        assert_no_overlap(&texts.iter().collect::<Vec<_>>(), &[], config::RIGHT_PANEL_MIN);
    }

    #[test]
    fn apps_making_sound_are_listed_first() {
        let app = |exe: &str, active| AudioApp { exe: exe.to_owned(), display_name: exe.to_owned(), pids: vec![1], active };
        let mut apps = [app("a.exe", false), app("b.exe", true), app("c.exe", false), app("d.exe", true)];
        active_first(&mut apps);
        assert_eq!(apps.map(|app| app.exe), ["b.exe", "d.exe", "a.exe", "c.exe"]);
    }
}
