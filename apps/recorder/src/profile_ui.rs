//! The Profile section of the window: profile files and every encoder option, with inline errors.
//!
//! See the recorder spec: Profiles, naming, settings.

use std::ops::RangeInclusive;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use egui::{Button, ComboBox, DragValue, RichText, TextEdit, Ui};
use media_io::AudioCodec;
use media_io::encode::{Codec as EncoderCodec, CodecSupport, ConfigError, MAX_MBPS, NumericOption, OptionRanges, amf_options, check_combo_with, probe_amf};
use ui_kit::tokens::space;
use ui_kit::{inspector_group, inspector_row};

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

/// The line shown under a constant quality max bitrate field.
const SOFT_MAX_HINT: &str = "On AMD the max is a soft limit; use VBR for a firm budget.";

/// The line shown under the container picker while MKV is chosen.
const MKV_HINT: &str = "MKV: Windows Explorer shows no video details for MKV files. Use MP4 for the widest compatibility.";

/// The width of the profile dropdown, in the Output group and the preview toolbar.
const PICKER_WIDTH: f32 = 220.0;

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
pub enum Field {
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

    /// Whether the last probe found that the GPU codes AV1 at exactly 1080 lines.
    fn aligned_1080(&self) -> bool {
        self.support(EncoderCodec::Av1).is_some_and(|support| support.aligned_1080.is_some())
    }

    /// The probed options of one codec, once a probe has answered and listed it.
    pub fn ranges(&self, codec: Codec) -> Option<OptionRanges> {
        self.support(EncoderCodec::from(codec)).map(|support| support.ranges.clone())
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
    /// The last next-take answer, with when and for what it was worked out.
    preview: Option<Preview>,
}

/// The next take's answer for the preview toolbar.
struct Preview {
    /// When the takes folder was read.
    at: Instant,
    /// What the answer was worked out from: the root, project, template, profile and container.
    key: String,
    /// The path as shown, or the error message when the takes folder could not be read.
    text: String,
    /// The take's short label ("takeNN"), or None when the folder could not be read.
    label: Option<String>,
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

    /// Draws the Output tool's groups (spec "Setup mode"): the Profile actions, the Encode group
    /// and the Format group. Everything is disabled while a take runs, as the profiles are today.
    pub fn output_panel(
        &mut self,
        ui: &mut Ui,
        store: &mut ProfileStore,
        settings: &mut Settings,
        source: (u32, u32),
        recording: bool,
        outcome: &mut ProfileOutcome,
    ) {
        if self.ensure_active(store, settings) {
            outcome.active_changed = true;
        }
        let audio = audio_codec(settings.audio.codec);
        let warn = ui.visuals().warn_fg_color;
        let error = ui.visuals().error_fg_color;
        for warning in store.warnings() {
            ui.colored_label(warn, warning);
        }
        if let Some(problem) = self.problem.as_deref().or_else(|| store.save_error()) {
            ui.colored_label(error, problem);
        }
        ui.add_enabled_ui(!recording, |ui| {
            self.profile_group(ui, store, settings, outcome);
            ui.add_space(space::SCALE_8);
            self.edit_fields(ui, store, settings, source, audio, recording);
        });
        if recording {
            ui.colored_label(ui_kit::palette::TEXT_MUTED, "Profile settings are locked while recording");
        }
    }

    /// The Profile group: the picker and the New, Duplicate, Rename and Delete actions, with
    /// today's inline rename and two-click Confirm delete.
    fn profile_group(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &mut Settings, outcome: &mut ProfileOutcome) {
        inspector_group(ui, |ui| {
            inspector_row(
                ui,
                "Profile",
                |ui| {
                    self.picker(ui, store, settings, outcome);
                },
                None,
            );
            let mut action = None;
            ui.add_space(space::SCALE_4);
            if let Some(buffer) = &mut self.renaming {
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(buffer).desired_width(140.0));
                    if ui.button("OK").clicked() {
                        action = Some(Action::ConfirmRename);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(Action::CancelRename);
                    }
                });
            } else {
                ui.horizontal(|ui| {
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
                });
            }
            ui.add_space(space::SCALE_4);
            if let Some(action) = action {
                self.run_action(action, store, settings, outcome);
            }
        });
    }

    /// Draws the profile dropdown; switching to another profile reports through `outcome`.
    pub fn picker(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &mut Settings, outcome: &mut ProfileOutcome) {
        let mut chosen = None;
        ComboBox::from_id_salt("profile-picker").selected_text(settings.active_profile.clone()).width(PICKER_WIDTH).show_ui(ui, |ui| {
            for profile in store.profiles() {
                if ui.selectable_label(profile.name == settings.active_profile, &profile.name).clicked() {
                    chosen = Some(profile.name.clone());
                }
            }
        });
        if let Some(name) = chosen
            && name != settings.active_profile
        {
            self.switch_to(store, settings, name, outcome);
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

    /// The next take for the preview toolbar: its short label ("takeNN" when the take name ends
    /// with one) and the full video file path, or an error message in the path's place.
    ///
    /// The takes folder is read again when something on the line changed or two seconds passed.
    pub fn next_take(&mut self, store: &ProfileStore, settings: &Settings) -> (Option<String>, String) {
        let container = store.get(&settings.active_profile).map(|profile| profile.container).unwrap_or_default();
        let root = takes_root(settings);
        let key = format!("{}|{}|{}|{}|{}", root.display(), settings.project, settings.naming_template, settings.active_profile, container.extension());
        if self.preview.as_ref().is_none_or(|preview| preview.key != key || preview.at.elapsed() > PREVIEW_REFRESH) {
            let naming = takes::Naming { project: &settings.project, template: &settings.naming_template, profile: &settings.active_profile };
            let (text, label) = match takes::plan_take(&root, chrono::Local::now(), container, &naming) {
                Ok(paths) => (paths.video_file.display().to_string(), Some(take_label(&paths.base_name).to_owned())),
                Err(err) => (format!("The takes folder could not be read: {err}"), None),
            };
            self.preview = Some(Preview { at: Instant::now(), key, text, label });
        }
        let mut answer = (None, String::new());
        if let Some(preview) = self.preview.as_ref() {
            answer = (preview.label.clone(), preview.text.clone());
        }
        answer
    }

    /// Draws the Encode and Format groups for the active profile and writes its edits after the debounce.
    /// Draws the Encode and Format groups for the active profile and writes its edits after the debounce.
    fn edit_fields(&mut self, ui: &mut Ui, store: &mut ProfileStore, settings: &Settings, source: (u32, u32), audio: AudioCodec, recording: bool) {
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

        // Encode group (spec "Setup mode": codec, rate control and its fields, preset, keyframe interval).
        inspector_group(ui, |ui| {
            inspector_row(
                ui,
                "Codec",
                |ui| {
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
                },
                None,
            );
            error_line(ui, &errors, Field::Codec);
            let ranges = ranges_of(&all_ranges, p.codec);
            rate_control_editor(ui, "profile", &mut p.rate_control, ranges, &errors);

            inspector_row(
                ui,
                "Preset",
                |ui| {
                    ComboBox::from_id_salt("profile-preset").selected_text(preset_label(p.preset)).show_ui(ui, |ui| {
                        for choice in [Preset::Speed, Preset::Balanced, Preset::Quality] {
                            ui.selectable_value(&mut p.preset, choice, preset_label(choice));
                        }
                    });
                },
                None,
            );
            error_line(ui, &errors, Field::Preset);

            let keyframe_text = format!("{:.1} s", p.keyframe_secs);
            inspector_row(
                ui,
                "Keyframe every",
                |ui| {
                    ui.add(DragValue::new(&mut p.keyframe_secs).range(0.0..=60.0).speed(0.1).max_decimals(1).suffix(" s"));
                },
                Some(&keyframe_text),
            );
            error_line(ui, &errors, Field::Keyframe);
        });
        ui.add_space(space::SCALE_8);

        // Format group (spec "Setup mode": output size, frame rate, container).
        inspector_group(ui, |ui| {
            inspector_row(
                ui,
                "Output size",
                |ui| {
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
                },
                None,
            );
            error_line(ui, &errors, Field::Size);

            inspector_row(
                ui,
                "Frame rate",
                |ui| {
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
                },
                None,
            );
            error_line(ui, &errors, Field::Fps);

            inspector_row(
                ui,
                "Container",
                |ui| {
                    ComboBox::from_id_salt("profile-container").selected_text(container_label(p.container)).show_ui(ui, |ui| {
                        for choice in [Container::Mkv, Container::Mp4, Container::Mov] {
                            ui.selectable_value(&mut p.container, choice, container_label(choice));
                        }
                    });
                },
                None,
            );
            if p.container == Container::Mkv {
                ui.add_space(space::SCALE_4);
                ui.label(RichText::new(MKV_HINT).small().weak());
                ui.add_space(space::SCALE_4);
            }
            error_line(ui, &errors, Field::Container);
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

/// Draws the rate control picker and the fields of the chosen mode as inspector rows, and returns
/// whether the rate control changed.
///
/// A new mode starts with its documented defaults, `ranges` are the probed options of the codec (none before the
/// probe has answered) and `errors` are drawn in red under the field they belong to. `id_salt` keeps the picker's
/// id apart when more than one editor is on screen.
pub fn rate_control_editor(ui: &mut Ui, id_salt: &str, rate: &mut RateControl, ranges: Option<&OptionRanges>, errors: &[(Field, String)]) -> bool {
    let before = *rate;
    let mode = Mode::of(*rate);
    let mut chosen = mode;
    inspector_row(
        ui,
        "Rate control",
        |ui| {
            ComboBox::from_id_salt(format!("{id_salt}-rate")).selected_text(mode.label()).show_ui(ui, |ui| {
                for choice in Mode::ALL {
                    ui.selectable_value(&mut chosen, choice, choice.label());
                }
            });
        },
        None,
    );
    if chosen != mode {
        *rate = default_rate(chosen, ranges);
    }
    error_line(ui, errors, Field::Rate);

    match rate {
        RateControl::ConstantQuality { quality, max_mbps } => {
            let quality_text = format!("{quality}");
            inspector_row(
                ui,
                "Quality",
                |ui| {
                    ui.add(egui::Slider::new(quality, 1..=100));
                },
                Some(&quality_text),
            );
            error_line(ui, errors, Field::Quality);
            let max_text = format!("{max_mbps:.1} Mbps");
            inspector_row(ui, "Max bitrate", |ui| mbps_field(ui, max_mbps), Some(&max_text));
            error_line(ui, errors, Field::MaxMbps);
            ui.add_space(space::SCALE_4);
            ui.label(RichText::new(SOFT_MAX_HINT).small().weak());
            ui.add_space(space::SCALE_4);
        }
        RateControl::ConstantQp { qp_i, qp_p } => {
            let range_i = qp_range(ranges.and_then(|r| r.qp_i.as_ref()));
            let range_p = qp_range(ranges.and_then(|r| r.qp_p.as_ref()));
            let qp_text = format!("I {qp_i} / P {qp_p}");
            inspector_row(
                ui,
                "QP",
                |ui| {
                    ui.label("I");
                    ui.add(DragValue::new(qp_i).range(range_i));
                    ui.label("P");
                    ui.add(DragValue::new(qp_p).range(range_p));
                },
                Some(&qp_text),
            );
            error_line(ui, errors, Field::Qp);
        }
        RateControl::Cbr { mbps } => {
            let mbps_text = format!("{mbps:.1} Mbps");
            inspector_row(ui, "Bitrate", |ui| mbps_field(ui, mbps), Some(&mbps_text));
            error_line(ui, errors, Field::Mbps);
        }
        RateControl::VbrPeak { target_mbps, max_mbps } => {
            let target_text = format!("{target_mbps:.1} Mbps");
            inspector_row(ui, "Target bitrate", |ui| mbps_field(ui, target_mbps), Some(&target_text));
            error_line(ui, errors, Field::Target);
            let max_text = format!("{max_mbps:.1} Mbps");
            inspector_row(ui, "Max bitrate", |ui| mbps_field(ui, max_mbps), Some(&max_text));
            error_line(ui, errors, Field::MaxMbps);
        }
    }
    *rate != before
}

/// The errors of a rate control for a codec: the neutral limits, then the encoder's own when its probed `ranges` are known.
pub fn rate_errors(codec: Codec, rate: RateControl, ranges: Option<&OptionRanges>) -> Vec<(Field, String)> {
    let mut errors = Errors::new();
    let config = encoder_config(&Profile { codec, rate_control: rate, ..Profile::premiere_hevc_1080p60() });
    match config.validate() {
        Err(ConfigError::Invalid { field, reason }) => errors.push((field_of(field), reason)),
        Err(other) => errors.push((Field::Rate, other.to_string())),
        Ok(()) => {
            if let Some(ranges) = ranges
                && let Err(err) = amf_options(config.codec, &config, ranges)
            {
                let field = if matches!(rate, RateControl::ConstantQp { .. }) { Field::Qp } else { Field::Rate };
                errors.push((field, err.to_string()));
            }
        }
    }
    errors
}

/// What the camera file's codec line says: the profile's codec, or HEVC with the reason when the container
/// cannot hold the profile's codec at the camera's height on this GPU.
pub fn iso_codec_text(profile: &Profile, camera_height: u32, probe: &AmfProbe) -> String {
    let codec = EncoderCodec::from(profile.codec);
    let fallback =
        profile.codec == Codec::Av1 && check_combo_with(profile.container, codec, camera_height, AudioCodec::Aac { kbps: 320 }, probe.aligned_1080()).is_err();
    if fallback {
        format!("HEVC (AV1 {camera_height}p not supported in {} on this GPU)", container_label(profile.container))
    } else {
        codec_label(profile.codec).to_owned()
    }
}

/// Draws the errors that belong to `field` as red lines inside the current inspector group.
fn error_line(ui: &mut Ui, errors: &[(Field, String)], field: Field) {
    let red = ui.visuals().error_fg_color;
    let hits: Vec<&String> = errors.iter().filter(|(f, _)| *f == field).map(|(_, message)| message).collect();
    if hits.is_empty() {
        return;
    }
    ui.add_space(space::SCALE_4);
    for message in hits {
        ui.colored_label(red, message.clone());
    }
    ui.add_space(space::SCALE_4);
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

/// The short label of a take name: its trailing "takeNN" part, or the whole name when the
/// naming template does not end with one (the preview toolbar's next-take caption).
fn take_label(base_name: &str) -> &str {
    let digits = base_name.bytes().rev().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let head = base_name.len() - digits;
        if base_name[..head].ends_with("take") {
            return &base_name[head - 4..];
        }
    }
    base_name
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
    if let Err(err) = check_combo_with(profile.container, config.codec, size.1, audio, probe.aligned_1080()) {
        errors.push((Field::Container, err.to_string()));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_take_label_is_the_trailing_take_number() {
        assert_eq!(take_label("2026-11-23_14-30-05_take07"), "take07");
        assert_eq!(take_label("take12"), "take12");
        assert_eq!(take_label("Show AV1 #12"), "Show AV1 #12");
        assert_eq!(take_label("My Show_07"), "My Show_07");
    }

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
    fn the_camera_file_names_its_codec_and_the_reason_for_a_fallback() {
        let probe = AmfProbe::default();
        let mut profile = Profile::tutorial_av1_1440p60();
        profile.container = Container::Mp4;
        assert_eq!(iso_codec_text(&profile, 1080, &probe), "HEVC (AV1 1080p not supported in MP4 on this GPU)");
        assert_eq!(iso_codec_text(&profile, 720, &probe), "AV1");
        profile.container = Container::Mkv;
        assert_eq!(iso_codec_text(&profile, 1080, &probe), "AV1");
        profile.codec = Codec::Hevc;
        profile.container = Container::Mp4;
        assert_eq!(iso_codec_text(&profile, 1080, &probe), "HEVC");
    }

    #[test]
    fn a_rate_control_error_is_found_without_a_probe() {
        let bad = RateControl::VbrPeak { target_mbps: 50.0, max_mbps: 40.0 };
        assert!(!rate_errors(Codec::Hevc, bad, None).is_empty());
        assert!(rate_errors(Codec::Hevc, default_rate(Mode::Vbr, None), None).is_empty());
    }

    #[test]
    fn source_size_follows_the_monitor() {
        let mut profile = Profile::tutorial_av1_1440p60();
        assert_eq!(effective_size(&profile, (1920, 1080)), (2560, 1440));
        (profile.width, profile.height) = (0, 0);
        assert_eq!(effective_size(&profile, (1920, 1080)), (1920, 1080));
    }
}
