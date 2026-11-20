//! The Profile section of the window: profile files and every encoder option, with inline errors.
//!
//! See the recorder spec: Profiles, naming, settings.

use std::ops::RangeInclusive;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use egui::{Button, ComboBox, DragValue, Slider, TextEdit, Ui};
use media_io::AudioCodec;
use media_io::encode::{Codec as EncoderCodec, CodecSupport, ConfigError, MAX_MBPS, NumericOption, OptionRanges, amf_options, check_combo, probe_amf};

use crate::config::{AudioCodecChoice, Codec, Container, Preset, Profile, RateControl, Settings, takes_root};
use crate::engine::session::encoder_config;
use crate::profiles::{self, ProfileStore};
use crate::takes;

/// The output sizes in the size dropdown, besides Source and Custom.
const SIZES: [(u32, u32); 4] = [(3840, 2160), (2560, 1440), (1920, 1080), (1280, 720)];

/// The frame rates in the frame rate dropdown, besides Custom.
const FPS_CHOICES: [u32; 2] = [30, 60];

/// How long the next take's path in the preview line is kept before the takes folder is read again.
const PREVIEW_REFRESH: Duration = Duration::from_secs(2);

/// The size and frame rate the AMF probe ran for.
type ProbeKey = (u32, u32, u32);

/// What the Profile section changed this frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProfileOutcome {
    /// The active profile name, the project or the naming template in the settings changed, so the settings need saving and sending to the controller.
    pub active_changed: bool,
}

/// The field of a profile that an error belongs next to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Codec,
    Rate,
    Preset,
    Quality,
    MaxMbps,
    Qp,
    Mbps,
    Target,
    Keyframe,
    Size,
    Fps,
    Container,
}

/// The rate control modes of the dropdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    ConstantQuality,
    ConstantQp,
    Cbr,
    Vbr,
}

impl Mode {
    /// Every mode, in dropdown order.
    const ALL: [Self; 4] = [Self::ConstantQuality, Self::ConstantQp, Self::Cbr, Self::Vbr];

    /// The mode a rate control is in.
    fn of(rate: RateControl) -> Self {
        match rate {
            RateControl::ConstantQuality { .. } => Self::ConstantQuality,
            RateControl::ConstantQp { .. } => Self::ConstantQp,
            RateControl::Cbr { .. } => Self::Cbr,
            RateControl::VbrPeak { .. } => Self::Vbr,
        }
    }

    /// The text of the dropdown entry.
    fn label(self) -> &'static str {
        match self {
            Self::ConstantQuality => "Constant quality",
            Self::ConstantQp => "Constant QP",
            Self::Cbr => "CBR",
            Self::Vbr => "VBR",
        }
    }
}

/// The error messages of a profile, each with the field it belongs to.
type Errors = Vec<(Field, String)>;

/// Runs the AMF probe on a background thread and keeps the latest answer.
#[derive(Default)]
pub struct AmfProbe {
    /// The probe in flight with the key it runs for.
    running: Option<(ProbeKey, Receiver<Vec<CodecSupport>>)>,
    /// The last finished answer with the key it was made for.
    done: Option<(ProbeKey, Vec<CodecSupport>)>,
}

impl AmfProbe {
    /// Takes in a finished probe, and starts one for `key` when the last answer was made for another size or frame rate.
    pub fn update(&mut self, ctx: &egui::Context, key: ProbeKey) {
        if let Some((running_key, rx)) = &self.running {
            let running_key = *running_key;
            match rx.try_recv() {
                Ok(result) => {
                    self.done = Some((running_key, result));
                    self.running = None;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    log::warn!("the encoder probe ended without an answer");
                    self.done = Some((running_key, Vec::new()));
                    self.running = None;
                }
            }
        }
        if self.done.as_ref().is_some_and(|(done_key, _)| *done_key == key) {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("amf-probe".to_owned()).spawn(move || {
            // The window may be gone by now; then nobody needs the answer.
            let _ = tx.send(probe_amf(key.0, key.1, key.2));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.running = Some((key, rx)),
            Err(err) => {
                log::warn!("the encoder probe could not start: {err}");
                self.done = Some((key, Vec::new()));
            }
        }
    }

    /// Whether at least one probe has finished.
    pub fn ready(&self) -> bool {
        self.done.is_some()
    }

    /// What the last probe reported for a codec.
    fn support(&self, codec: EncoderCodec) -> Option<&CodecSupport> {
        self.done.as_ref().and_then(|(_, list)| list.iter().find(|support| support.codec == codec))
    }

    /// The codecs the last probe reported as working.
    fn usable(&self) -> Vec<EncoderCodec> {
        self.done.as_ref().map(|(_, list)| list.iter().filter(|support| support.result.is_ok()).map(|support| support.codec).collect()).unwrap_or_default()
    }

    /// The option ranges of every probed codec.
    fn all_ranges(&self) -> Vec<(EncoderCodec, OptionRanges)> {
        self.done.as_ref().map(|(_, list)| list.iter().map(|support| (support.codec, support.ranges.clone())).collect()).unwrap_or_default()
    }
}

/// The Profile section: the state of its widgets. The profiles themselves live in the [`ProfileStore`].
pub struct ProfileSection {
    /// The encoder probe.
    probe: AmfProbe,
    /// The new name while the profile is being renamed.
    renaming: Option<String>,
    /// Whether Delete was clicked once and waits for a second click.
    confirm_delete: bool,
    /// Whether the size dropdown is on Custom.
    size_custom: bool,
    /// Whether the frame rate dropdown is on Custom.
    fps_custom: bool,
    /// The text of the last file operation that failed.
    problem: Option<String>,
    /// Whether the profile being edited has an error, which blocks starting a take.
    has_errors: bool,
    /// The last preview line of the next take's path, with when and for what it was worked out.
    preview: Option<Preview>,
}

/// The preview line of the next take's path.
struct Preview {
    /// When the takes folder was read.
    at: Instant,
    /// What the line was worked out from: the root, project, template, profile and container.
    key: String,
    /// The path as shown.
    text: String,
}

/// What a click in the profile row asks for.
enum Action {
    New,
    Duplicate,
    StartRename,
    ConfirmRename,
    CancelRename,
    Delete,
}

impl ProfileSection {
    /// Loads the profile files, makes the active profile one that exists, and starts the first encoder probe.
    pub fn new(ctx: &egui::Context, store: &mut ProfileStore, settings: &mut Settings, source: (u32, u32)) -> Self {
        let mut section = Self {
            probe: AmfProbe::default(),
            renaming: None,
            confirm_delete: false,
            size_custom: false,
            fps_custom: false,
            problem: None,
            has_errors: false,
            preview: None,
        };
        section.ensure_active(store, settings);
        if let Some(profile) = store.get(&settings.active_profile) {
            let size = effective_size(profile, source);
            section.probe.update(ctx, (size.0, size.1, profile.fps));
        }
        section
    }

    /// Whether the profile being edited has an error; a take must not start then.
    pub fn blocked(&self) -> bool {
        self.has_errors
    }

    /// Makes the active profile one that exists, reading the files again first when it is not in the list.
    fn ensure_active(&mut self, store: &mut ProfileStore, settings: &mut Settings) -> bool {
        if store.get(&settings.active_profile).is_some() {
            return false;
        }
        store.flush();
        store.reload();
        if store.get(&settings.active_profile).is_some() {
            return false;
        }
        if let Some(first) = store.profiles().first() {
            settings.active_profile.clone_from(&first.name);
        }
        true
    }

    /// Makes another profile the active one.
    fn switch_to(&mut self, store: &mut ProfileStore, settings: &mut Settings, name: String, outcome: &mut ProfileOutcome) {
        store.flush();
        settings.active_profile = name;
        self.size_custom = false;
        self.fps_custom = false;
        self.renaming = None;
        self.confirm_delete = false;
        self.problem = None;
        outcome.active_changed = true;
    }

    /// Draws the section. It is read-only while a take runs, and a profile can only be switched while idle.
    pub fn show(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &mut Settings, source: (u32, u32), recording: bool, outcome: &mut ProfileOutcome) {
        if self.ensure_active(store, settings) {
            outcome.active_changed = true;
        }
        let audio = audio_codec(settings.audio.codec);
        ui.heading("Profile");
        let warn = ui.visuals().warn_fg_color;
        let error = ui.visuals().error_fg_color;
        for warning in store.warnings() {
            ui.colored_label(warn, warning);
        }
        if let Some(problem) = self.problem.as_deref().or_else(|| store.save_error()) {
            ui.colored_label(error, problem);
        }
        ui.add_enabled_ui(!recording, |ui| {
            self.picker_row(ui, store, settings, outcome);
            self.fields(ui, store, settings, source, audio, recording);
            self.naming_fields(ui, store, settings, outcome);
        });
        if recording {
            ui.colored_label(ui_kit::palette::TEXT_MUTED, "Profile settings are locked while recording");
        }
    }

    /// Draws the Project and Naming template fields, which are global settings, and the full path of the next take.
    fn naming_fields(&mut self, ui: &mut Ui, store: &ProfileStore, settings: &mut Settings, outcome: &mut ProfileOutcome) {
        ui.separator();
        let mut changed = false;
        egui::Grid::new("naming-fields").num_columns(2).show(ui, |ui| {
            ui.label("Project");
            changed |= ui.add(TextEdit::singleline(&mut settings.project).desired_width(240.0)).changed();
            ui.end_row();
            ui.label("Naming template");
            changed |= ui.add(TextEdit::singleline(&mut settings.naming_template).desired_width(320.0)).changed();
            ui.end_row();
        });
        ui.colored_label(ui_kit::palette::TEXT_MUTED, "Tokens: {project} {date} {time} {take} {profile}. Both fields apply to every profile.");
        if changed {
            outcome.active_changed = true;
        }
        let container = store.get(&settings.active_profile).map(|profile| profile.container).unwrap_or_default();
        let root = takes_root(settings);
        let key = format!("{}|{}|{}|{}|{}", root.display(), settings.project, settings.naming_template, settings.active_profile, container.extension());
        if self.preview.as_ref().is_none_or(|preview| preview.key != key || preview.at.elapsed() > PREVIEW_REFRESH) {
            let naming = takes::Naming { project: &settings.project, template: &settings.naming_template, profile: &settings.active_profile };
            let text = match takes::plan_take(&root, chrono::Local::now(), container, &naming) {
                Ok(paths) => paths.video_file.display().to_string(),
                Err(err) => format!("The takes folder could not be read: {err}"),
            };
            self.preview = Some(Preview { at: Instant::now(), key, text });
        }
        if let Some(preview) = &self.preview {
            ui.label(format!("Next take: {}", preview.text));
        }
    }

    /// Draws the profile dropdown and the New, Duplicate, Rename and Delete buttons.
    fn picker_row(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &mut Settings, outcome: &mut ProfileOutcome) {
        let mut chosen = None;
        let mut action = None;
        ui.horizontal(|ui| {
            ComboBox::from_id_salt("profile-picker").selected_text(settings.active_profile.clone()).width(240.0).show_ui(ui, |ui| {
                for profile in store.profiles() {
                    if ui.selectable_label(profile.name == settings.active_profile, &profile.name).clicked() {
                        chosen = Some(profile.name.clone());
                    }
                }
            });
            if let Some(buffer) = &mut self.renaming {
                ui.add(TextEdit::singleline(buffer).desired_width(180.0));
                if ui.button("OK").clicked() {
                    action = Some(Action::ConfirmRename);
                }
                if ui.button("Cancel").clicked() {
                    action = Some(Action::CancelRename);
                }
            } else {
                if ui.button("New").clicked() {
                    action = Some(Action::New);
                }
                if ui.button("Duplicate").clicked() {
                    action = Some(Action::Duplicate);
                }
                if ui.button("Rename").clicked() {
                    action = Some(Action::StartRename);
                }
                let delete_text = if self.confirm_delete { "Confirm delete" } else { "Delete" };
                if ui.add_enabled(store.profiles().len() > 1, Button::new(delete_text)).clicked() {
                    action = Some(Action::Delete);
                }
            }
        });
        if let Some(name) = chosen
            && name != settings.active_profile
        {
            self.switch_to(store, settings, name, outcome);
        }
        if let Some(action) = action {
            self.run_action(action, store, settings, outcome);
        }
    }

    /// Does what a click in the profile row asked for.
    fn run_action(&mut self, action: Action, store: &mut ProfileStore, settings: &mut Settings, outcome: &mut ProfileOutcome) {
        if !matches!(action, Action::Delete) {
            self.confirm_delete = false;
        }
        self.problem = None;
        store.flush();
        let Some(dir) = store.dir().map(std::path::Path::to_path_buf) else {
            self.problem = Some("The profiles folder could not be found".to_owned());
            return;
        };
        let current = settings.active_profile.clone();
        let result = match action {
            Action::New => profiles::create(&dir).map(|profile| Some(profile.name)),
            Action::Duplicate => match store.get(&current) {
                Some(profile) => profiles::duplicate(&dir, profile).map(|copy| Some(copy.name)),
                None => Ok(None),
            },
            Action::StartRename => {
                self.renaming = Some(current);
                Ok(None)
            }
            Action::CancelRename => {
                self.renaming = None;
                Ok(None)
            }
            Action::ConfirmRename => {
                let new_name = self.renaming.clone().unwrap_or_default();
                profiles::rename(&dir, &current, &new_name).map(|profile| Some(profile.name))
            }
            Action::Delete if !self.confirm_delete => {
                self.confirm_delete = true;
                Ok(None)
            }
            Action::Delete => {
                self.confirm_delete = false;
                profiles::delete(&dir, &current).map(|()| store.profiles().iter().map(|profile| profile.name.clone()).find(|name| *name != current))
            }
        };
        match result {
            Ok(Some(name)) => {
                store.reload();
                self.switch_to(store, settings, name, outcome);
            }
            Ok(None) => {}
            Err(err) => self.problem = Some(err.to_string()),
        }
    }

    /// Draws every field of the active profile and writes its edits after the debounce.
    fn fields(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &Settings, source: (u32, u32), audio: AudioCodec, recording: bool) {
        let ctx = ui.ctx().clone();
        let Some(original) = store.get(&settings.active_profile).cloned() else { return };
        let mut p = original.clone();
        let size = effective_size(&p, source);
        if !recording {
            self.probe.update(&ctx, (size.0, size.1, p.fps));
        }
        let errors = validate(&p, size, audio, &self.probe);
        self.has_errors = !errors.is_empty();
        let usable = self.probe.usable();
        let all_ranges = self.probe.all_ranges();
        let probing = !self.probe.ready();
        let mut size_custom = self.size_custom;
        let mut fps_custom = self.fps_custom;

        egui::Grid::new("profile-fields").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            row(ui, "Codec", &errors, Field::Codec, |ui| {
                let mut codec = p.codec;
                ComboBox::from_id_salt("profile-codec").selected_text(codec_label(codec)).show_ui(ui, |ui| {
                    for choice in [Codec::Av1, Codec::Hevc, Codec::H264] {
                        if choice == p.codec || usable.contains(&EncoderCodec::from(choice)) {
                            ui.selectable_value(&mut codec, choice, codec_label(choice));
                        }
                    }
                });
                if codec != p.codec {
                    p.codec = codec;
                    if let RateControl::ConstantQp { .. } = p.rate_control {
                        p.rate_control = default_rate(Mode::ConstantQp, ranges_of(&all_ranges, codec));
                    }
                }
                if probing {
                    ui.colored_label(ui_kit::palette::TEXT_MUTED, "Checking encoders...");
                }
            });

            row(ui, "Rate control", &errors, Field::Rate, |ui| {
                let mode = Mode::of(p.rate_control);
                let mut chosen = mode;
                ComboBox::from_id_salt("profile-rate").selected_text(mode.label()).show_ui(ui, |ui| {
                    for choice in Mode::ALL {
                        ui.selectable_value(&mut chosen, choice, choice.label());
                    }
                });
                if chosen != mode {
                    p.rate_control = default_rate(chosen, ranges_of(&all_ranges, p.codec));
                }
            });

            let ranges = ranges_of(&all_ranges, p.codec);
            match &mut p.rate_control {
                RateControl::ConstantQuality { quality, max_mbps } => {
                    row(ui, "Quality", &errors, Field::Quality, |ui| {
                        ui.add(Slider::new(quality, 1..=100));
                    });
                    row(ui, "Max bitrate", &errors, Field::MaxMbps, |ui| mbps_field(ui, max_mbps));
                }
                RateControl::ConstantQp { qp_i, qp_p } => {
                    let range_i = qp_range(ranges.and_then(|r| r.qp_i.as_ref()));
                    let range_p = qp_range(ranges.and_then(|r| r.qp_p.as_ref()));
                    row(ui, "QP", &errors, Field::Qp, |ui| {
                        ui.label("I");
                        ui.add(DragValue::new(qp_i).range(range_i));
                        ui.label("P");
                        ui.add(DragValue::new(qp_p).range(range_p));
                    });
                }
                RateControl::Cbr { mbps } => {
                    row(ui, "Bitrate", &errors, Field::Mbps, |ui| mbps_field(ui, mbps));
                }
                RateControl::VbrPeak { target_mbps, max_mbps } => {
                    row(ui, "Target bitrate", &errors, Field::Target, |ui| mbps_field(ui, target_mbps));
                    row(ui, "Max bitrate", &errors, Field::MaxMbps, |ui| mbps_field(ui, max_mbps));
                }
            }

            row(ui, "Preset", &errors, Field::Preset, |ui| {
                ComboBox::from_id_salt("profile-preset").selected_text(preset_label(p.preset)).show_ui(ui, |ui| {
                    for choice in [Preset::Speed, Preset::Balanced, Preset::Quality] {
                        ui.selectable_value(&mut p.preset, choice, preset_label(choice));
                    }
                });
            });

            row(ui, "Keyframe every", &errors, Field::Keyframe, |ui| {
                ui.add(DragValue::new(&mut p.keyframe_secs).range(0.0..=60.0).speed(0.1).max_decimals(1).suffix(" s"));
            });

            row(ui, "Output size", &errors, Field::Size, |ui| {
                let preset = SIZES.iter().any(|&(w, h)| p.width == w && p.height == h);
                let custom = size_custom || (!p.is_source_size() && !preset);
                let text = if p.is_source_size() {
                    "Source".to_owned()
                } else if custom {
                    "Custom".to_owned()
                } else {
                    format!("{}x{}", p.width, p.height)
                };
                ComboBox::from_id_salt("profile-size").selected_text(text).show_ui(ui, |ui| {
                    if ui.selectable_label(p.is_source_size(), "Source").clicked() {
                        (p.width, p.height) = (0, 0);
                        size_custom = false;
                    }
                    for (w, h) in SIZES {
                        if ui.selectable_label(!custom && p.width == w && p.height == h, format!("{w}x{h}")).clicked() {
                            (p.width, p.height) = (w, h);
                            size_custom = false;
                        }
                    }
                    if ui.selectable_label(custom && !p.is_source_size(), "Custom").clicked() {
                        size_custom = true;
                        if p.is_source_size() {
                            (p.width, p.height) = source;
                        }
                    }
                });
                if p.is_source_size() {
                    ui.colored_label(ui_kit::palette::TEXT_MUTED, format!("{}x{}", source.0, source.1));
                } else if custom {
                    ui.add(DragValue::new(&mut p.width).range(16..=16384));
                    ui.label("x");
                    ui.add(DragValue::new(&mut p.height).range(16..=16384));
                }
            });

            row(ui, "Frame rate", &errors, Field::Fps, |ui| {
                let preset = FPS_CHOICES.contains(&p.fps);
                let custom = fps_custom || !preset;
                let text = if custom { "Custom".to_owned() } else { format!("{} fps", p.fps) };
                ComboBox::from_id_salt("profile-fps").selected_text(text).show_ui(ui, |ui| {
                    for fps in FPS_CHOICES {
                        if ui.selectable_label(!custom && p.fps == fps, format!("{fps} fps")).clicked() {
                            p.fps = fps;
                            fps_custom = false;
                        }
                    }
                    if ui.selectable_label(custom, "Custom").clicked() {
                        fps_custom = true;
                    }
                });
                if custom {
                    ui.add(DragValue::new(&mut p.fps).range(1..=240).suffix(" fps"));
                }
            });

            row(ui, "Container", &errors, Field::Container, |ui| {
                ComboBox::from_id_salt("profile-container").selected_text(container_label(p.container)).show_ui(ui, |ui| {
                    for choice in [Container::Mkv, Container::Mp4, Container::Mov] {
                        ui.selectable_value(&mut p.container, choice, container_label(choice));
                    }
                });
            });
        });

        self.size_custom = size_custom;
        self.fps_custom = fps_custom;
        if p != original {
            store.update(p);
        }
        if self.has_errors {
            ui.colored_label(ui.visuals().error_fg_color, "Fix the errors above before starting a take");
        }
    }
}

/// Draws one labelled row of the field grid, with the errors that belong to `field` in red after the widgets.
fn row(ui: &mut Ui, label: &str, errors: &[(Field, String)], field: Field, add: impl FnOnce(&mut Ui)) {
    ui.label(label);
    ui.horizontal(|ui| {
        add(ui);
        let red = ui.visuals().error_fg_color;
        for (_, message) in errors.iter().filter(|(f, _)| *f == field) {
            ui.colored_label(red, message);
        }
    });
    ui.end_row();
}

/// Draws a bitrate field in megabits per second.
fn mbps_field(ui: &mut Ui, mbps: &mut f32) {
    ui.add(DragValue::new(mbps).range(0.0..=MAX_MBPS).speed(0.5).max_decimals(1).suffix(" Mbps"));
}

/// The output size a profile records at: its own, or the captured monitor's for Source.
fn effective_size(profile: &Profile, source: (u32, u32)) -> (u32, u32) {
    if profile.is_source_size() { source } else { (profile.width, profile.height) }
}

/// The audio codec a settings choice stands for.
fn audio_codec(choice: AudioCodecChoice) -> AudioCodec {
    match choice {
        AudioCodecChoice::Aac320 => AudioCodec::Aac { kbps: 320 },
        AudioCodecChoice::Pcm24 => AudioCodec::Pcm24,
    }
}

/// The text of a codec in the dropdown.
fn codec_label(codec: Codec) -> &'static str {
    match codec {
        Codec::Av1 => "AV1",
        Codec::Hevc => "HEVC",
        Codec::H264 => "H.264",
    }
}

/// The text of a preset in the dropdown.
fn preset_label(preset: Preset) -> &'static str {
    match preset {
        Preset::Speed => "Speed",
        Preset::Balanced => "Balanced",
        Preset::Quality => "Quality",
    }
}

/// The text of a container in the dropdown.
fn container_label(container: Container) -> &'static str {
    match container {
        Container::Mkv => "MKV",
        Container::Mp4 => "MP4",
        Container::Mov => "MOV",
    }
}

/// The probed ranges of one codec.
fn ranges_of(all: &[(EncoderCodec, OptionRanges)], codec: Codec) -> Option<&OptionRanges> {
    let wanted = EncoderCodec::from(codec);
    all.iter().find(|(known, _)| *known == wanted).map(|(_, ranges)| ranges)
}

/// The QP values a field accepts: the probed range, or the whole byte before the probe has answered.
fn qp_range(option: Option<&NumericOption>) -> RangeInclusive<u8> {
    match option {
        Some(option) => {
            let low = u8::try_from(option.min.max(0)).unwrap_or(0);
            let high = u8::try_from(option.max.max(0)).unwrap_or(u8::MAX);
            low..=high.max(low)
        }
        None => 0..=u8::MAX,
    }
}

/// A QP of the H.264 and HEVC scale (0 to 51) moved onto the probed range, which for AV1 is wider.
fn scale_qp(base: u8, option: Option<&NumericOption>) -> u8 {
    let Some(option) = option else { return base };
    let scaled = if option.max > 51 { i64::from(base) * option.max / 51 } else { i64::from(base) };
    u8::try_from(scaled.max(option.min).min(option.max)).unwrap_or(base)
}

/// The values a rate control mode starts with when it is picked.
fn default_rate(mode: Mode, ranges: Option<&OptionRanges>) -> RateControl {
    match mode {
        Mode::ConstantQuality => RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 },
        Mode::ConstantQp => {
            RateControl::ConstantQp { qp_i: scale_qp(24, ranges.and_then(|r| r.qp_i.as_ref())), qp_p: scale_qp(26, ranges.and_then(|r| r.qp_p.as_ref())) }
        }
        Mode::Cbr => RateControl::Cbr { mbps: 20.0 },
        Mode::Vbr => RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 },
    }
}

/// The field a vendor-neutral validation error is about.
fn field_of(name: &str) -> Field {
    match name {
        "size" => Field::Size,
        "frame rate" => Field::Fps,
        "keyframe_secs" => Field::Keyframe,
        "quality" => Field::Quality,
        "max_mbps" => Field::MaxMbps,
        "mbps" => Field::Mbps,
        "target_mbps" => Field::Target,
        _ => Field::Rate,
    }
}

/// Every error of a profile: the neutral limits, the GPU's own limits, and the container rules.
fn validate(profile: &Profile, size: (u32, u32), audio: AudioCodec, probe: &AmfProbe) -> Errors {
    let mut errors = Errors::new();
    let sized = Profile { width: size.0, height: size.1, ..profile.clone() };
    let config = encoder_config(&sized);
    match config.validate() {
        Err(ConfigError::Invalid { field, reason }) => errors.push((field_of(field), reason)),
        Err(other) => errors.push((Field::Rate, other.to_string())),
        Ok(()) => match probe.support(config.codec) {
            Some(support) => match &support.result {
                Err(reason) => errors.push((Field::Codec, format!("{} is not available on this GPU: {reason}", codec_label(profile.codec)))),
                Ok(()) => {
                    if let Err(err) = amf_options(config.codec, &config, &support.ranges) {
                        let field = if matches!(profile.rate_control, RateControl::ConstantQp { .. }) { Field::Qp } else { Field::Rate };
                        errors.push((field, err.to_string()));
                    }
                }
            },
            None if probe.ready() => errors.push((Field::Codec, format!("{} is not reported by this GPU", codec_label(profile.codec)))),
            None => {}
        },
    }
    if let Err(err) = check_combo(profile.container, config.codec, size.1, audio) {
        errors.push((Field::Container, err.to_string()));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qp_defaults_scale_to_the_av1_range() {
        let av1 = NumericOption { name: "qp".to_owned(), min: 0, max: 255 };
        assert_eq!(scale_qp(24, Some(&av1)), 120);
        let hevc = NumericOption { name: "qp".to_owned(), min: 0, max: 51 };
        assert_eq!(scale_qp(24, Some(&hevc)), 24);
        assert_eq!(scale_qp(26, None), 26);
    }

    #[test]
    fn switching_mode_fills_the_documented_defaults() {
        assert_eq!(default_rate(Mode::ConstantQuality, None), RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 });
        assert_eq!(default_rate(Mode::ConstantQp, None), RateControl::ConstantQp { qp_i: 24, qp_p: 26 });
        assert_eq!(default_rate(Mode::Cbr, None), RateControl::Cbr { mbps: 20.0 });
        assert_eq!(default_rate(Mode::Vbr, None), RateControl::VbrPeak { target_mbps: 20.0, max_mbps: 40.0 });
    }

    #[test]
    fn errors_land_next_to_their_field() {
        let probe = AmfProbe::default();
        let mut profile = Profile::premiere_hevc_1080p60();
        profile.rate_control = RateControl::VbrPeak { target_mbps: 50.0, max_mbps: 40.0 };
        profile.keyframe_secs = 0.1;
        let errors = validate(&profile, (1920, 1080), AudioCodec::Aac { kbps: 320 }, &probe);
        assert!(errors.iter().any(|(field, _)| *field == Field::Keyframe), "{errors:?}");
        let mut av1_mp4 = Profile::tutorial_av1_1440p60();
        av1_mp4.container = Container::Mp4;
        av1_mp4.height = 1080;
        let errors = validate(&av1_mp4, (1920, 1080), AudioCodec::Aac { kbps: 320 }, &probe);
        assert!(errors.iter().any(|(field, _)| *field == Field::Container), "{errors:?}");
        assert!(validate(&Profile::premiere_hevc_1080p60(), (1920, 1080), AudioCodec::Aac { kbps: 320 }, &probe).is_empty());
    }

    #[test]
    fn source_size_follows_the_monitor() {
        let mut profile = Profile::tutorial_av1_1440p60();
        assert_eq!(effective_size(&profile, (1920, 1080)), (2560, 1440));
        (profile.width, profile.height) = (0, 0);
        assert_eq!(effective_size(&profile, (1920, 1080)), (1920, 1080));
    }
}
