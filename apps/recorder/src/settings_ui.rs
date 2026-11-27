//! The Settings dialog (spec "Settings dialog"): the sections in a list on the left and the open
//! section on the right, in the ui-kit dialog look (bg.panel frame, headline title, footer
//! button). File > Settings (Ctrl+,) and View > Appearance open it; Esc closes it. The app owns
//! the open state and everything the sections edit.

use std::collections::VecDeque;

use egui::{Align, Context, Id, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect, Sense, Ui, UiBuilder, vec2};

use ui_kit::fonts;
use ui_kit::tokens::{color, radius, space, typography};

use crate::api::ApiServer;
use crate::config::{REPLAY_LENGTHS_S, Settings, takes_root};
use crate::engine::bus::Command;
use crate::engine::stats::{GIB, disk_free_bytes};
use crate::hotkeys::Hotkeys;
use crate::platform;
use crate::takes;

/// The sections of the Settings dialog, in the order the left list shows them (spec "Settings dialog").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Section {
    /// The Hide Recorder from screen captures switch.
    #[default]
    General,
    /// The naming template and the countdown.
    Recording,
    /// The read-only hotkey status table.
    Hotkeys,
    /// The local control API.
    ControlApi,
    /// The recent commands the engine received.
    Diagnostics,
    /// The accent picker.
    Appearance,
}

impl Section {
    /// Every section in list order (spec "Settings dialog").
    const ALL: [Section; 6] = [Section::General, Section::Recording, Section::Hotkeys, Section::ControlApi, Section::Diagnostics, Section::Appearance];

    /// The label the section list shows.
    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Recording => "Recording",
            Self::Hotkeys => "Hotkeys",
            Self::ControlApi => "Control API",
            Self::Diagnostics => "Diagnostics",
            Self::Appearance => "Appearance",
        }
    }
}

/// The open state of the Settings dialog and the section its right side shows.
#[derive(Debug, Clone, Copy, Default)]
pub struct SettingsDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The section the right side shows.
    pub section: Section,
}

/// The next take the Recording section shows; the app computes it from the profile and settings.
#[derive(Debug, Clone, Default)]
pub struct NextTake {
    /// The full video file path of the next take.
    pub path: String,
}

/// The Control API state the dialog reads and edits; the app keeps it between frames.
pub struct ApiUi<'a> {
    /// The running API server, or None while it is off or could not start.
    pub server: Option<&'a ApiServer>,
    /// The token clients must send, or None when Credential Manager could not give one.
    pub token: Option<&'a str>,
    /// Whether the token is shown in full.
    pub show_token: &'a mut bool,
    /// The port as typed, applied to the settings when the field is left.
    pub port_edit: &'a mut u16,
    /// The Stream Deck row's status line.
    pub stream_deck_status: &'static str,
}

/// What the dialog's rows changed this frame, for the app to act on.
#[derive(Debug, Clone, Copy, Default)]
pub struct Outcome {
    /// A settings field changed and the app must send and save the settings.
    pub settings_changed: bool,
    /// The Hide-from-screen-captures switch flipped and the app must re-apply the exclusion.
    pub hide_from_capture_changed: bool,
    /// The token Regenerate button was pressed.
    pub regenerate_token: bool,
}

/// The width the Settings body wants, clamped to the screen.
const BODY_WIDTH: f32 = 760.0;

/// The height the Settings body wants, clamped to the screen.
const BODY_HEIGHT: f32 = 520.0;

/// Width of the section list on the left.
const SECTION_LIST_WIDTH: f32 = 200.0;

/// The margin the dialog keeps to the screen edges on every side.
const SCREEN_MARGIN: f32 = 48.0;

/// The countdown lengths the settings offer, in seconds; 0 is off.
const COUNTDOWN_CHOICES: [u8; 3] = [0, 3, 5];

/// Height of one Hotkeys table row.
const HOTKEY_ROW_HEIGHT: f32 = 32.0;

/// Height of the Hotkeys table header row.
const HOTKEY_HEADER_HEIGHT: f32 = 24.0;

/// Width of the Hotkeys table's Shortcut column.
const HOTKEY_SHORTCUT_COL: f32 = 150.0;

/// Width of the Hotkeys table's Status column.
const HOTKEY_STATUS_COL: f32 = 170.0;

/// The most asterisks the masked token shows before it truncates.
const TOKEN_MASK_MAX: usize = 24;

/// Height of the footer row that holds the Close button.
const FOOTER_ROW_HEIGHT: f32 = 32.0;

/// The 1 px hairline stroke width, as ui-kit draws every separator line.
const HAIRLINE_WIDTH: f32 = 1.0;

/// Draws the Settings dialog while it is open and reports what its rows changed this frame.
pub fn show(
    ctx: &Context,
    dialog: &mut SettingsDialog,
    settings: &mut Settings,
    next_take: &NextTake,
    hotkeys: &Hotkeys,
    recent: &VecDeque<Command>,
    api: ApiUi<'_>,
) -> Outcome {
    // Each test frame keeps only its own control rects, so the previous
    // frame's list is dropped before the sections draw.
    #[cfg(test)]
    test_clear_controls();
    // The open gate lives here, not at the call site: the app calls this every
    // frame, and egui::Modal paints the moment `show` runs, so an ungated call
    // drew the dialog even with `open` false, and Close, Esc and the backdrop
    // only flipped the flag while the modal kept being redrawn.
    if !dialog.open {
        return Outcome::default();
    }
    let mut outcome = Outcome::default();
    let mut open = dialog.open;
    let mut closed = false;
    // Fixed body size, clamped to the screen: the body never grows past the
    // clamp, so the dialog needs no clamp of its own.
    let screen = ctx.input(|input| input.content_rect());
    let body_width = BODY_WIDTH.min((screen.width() - 2.0 * SCREEN_MARGIN).max(0.0));
    let body_height = BODY_HEIGHT.min((screen.height() - 2.0 * SCREEN_MARGIN).max(0.0));
    // The dialog is drawn directly on egui::Modal, not on ui_kit::dialog:
    // ui_kit::dialog caps the modal ui with `set_max_width`, and in egui 0.36
    // that cap makes the modal's frame stretch to the cap instead of hugging
    // its content, so a fixed body still filled almost the whole screen. The
    // frame, title and footer copy ui_kit::dialog's look and close behavior.
    let frame = egui::Frame::new()
        .fill(color::BG_PANEL)
        .corner_radius(radius::PANEL)
        .inner_margin(egui::Margin::same(space::SCALE_24 as i8))
        .stroke(egui::Stroke::new(HAIRLINE_WIDTH, color::STROKE_HAIRLINE));
    let response = egui::Modal::new(Id::new("recorder-settings-dialog")).frame(frame).show(ctx, |ui| {
        ui.add(egui::Label::new(egui::RichText::new("Settings").font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY)));
        ui.add_space(space::SCALE_8);
        // The body is exactly the clamped BODY_WIDTH x BODY_HEIGHT, split by
        // hand into the section list, a hairline and the content; the hand
        // split keeps the sections a vertical column instead of one
        // horizontal row and pins the dialog to its size.
        let (body_rect, _) = ui.allocate_exact_size(vec2(body_width, body_height), Sense::hover());
        let (list_rect, hair_rect, content_rect) = split_body(body_rect, ui.spacing().item_spacing.x);
        ui.painter().rect_filled(hair_rect, 0.0, color::STROKE_SEPARATOR);
        let mut list_ui = ui.new_child(UiBuilder::new().id_salt("settings-section-list").max_rect(list_rect).layout(Layout::top_down(Align::Min)));
        list_ui.set_clip_rect(list_rect);
        for candidate in Section::ALL {
            if ui_kit::list_row(&mut list_ui, dialog.section == candidate, None, candidate.label(), None).clicked() {
                dialog.section = candidate;
            }
        }
        #[cfg(test)]
        test_record_content(content_rect);
        let mut content_ui = ui.new_child(UiBuilder::new().id_salt("settings-content").max_rect(content_rect).layout(Layout::top_down(Align::Min)));
        content_ui.set_clip_rect(content_rect);
        egui::ScrollArea::vertical().max_height(content_rect.height()).show(&mut content_ui, |ui| match dialog.section {
            Section::General => general_section(ui, settings, &mut outcome),
            Section::Recording => recording_section(ui, settings, next_take, &mut outcome),
            Section::Hotkeys => hotkey_table(ui, hotkeys),
            Section::ControlApi => api_section(ui, settings, api, &mut outcome),
            Section::Diagnostics => diagnostics_section(ui, recent),
            Section::Appearance => {
                ui_kit::accent_picker(ui);
            }
        });
        // The footer's Close button, bottom right, in an explicitly allocated
        // row: nothing in the dialog sizes itself from `available_width`, so
        // nothing can stretch the modal past its body.
        ui.add_space(space::SCALE_8);
        let (footer_rect, _) = ui.allocate_exact_size(vec2(body_width, FOOTER_ROW_HEIGHT), Sense::hover());
        let mut footer_ui = ui.new_child(UiBuilder::new().max_rect(footer_rect).layout(Layout::right_to_left(Align::Center)));
        if ui_kit::primary_button(&mut footer_ui, "Close").clicked() {
            closed = true;
        }
    });
    // Esc on the topmost modal and a click on the backdrop close it.
    if response.should_close() || closed {
        open = false;
    }
    dialog.open = open;
    outcome
}

/// Splits the body rect into the section list (left, [`SECTION_LIST_WIDTH`]
/// wide), the 1px hairline between the two and the section content (the rest).
fn split_body(body: Rect, gap: f32) -> (Rect, Rect, Rect) {
    let list = Rect::from_min_max(body.min, Pos2::new(body.left() + SECTION_LIST_WIDTH, body.bottom()));
    let hair_left = list.right() + gap;
    let hair = Rect::from_min_max(Pos2::new(hair_left, body.top()), Pos2::new(hair_left + HAIRLINE_WIDTH, body.bottom()));
    let content = Rect::from_min_max(Pos2::new(hair.right() + gap, body.top()), body.max);
    (list, hair, content)
}

/// The General section: the Hide Recorder from screen captures switch.
fn general_section(ui: &mut Ui, settings: &mut Settings, outcome: &mut Outcome) {
    let before = settings.hide_from_capture;
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Hide Recorder from screen captures",
            |ui| {
                let _switch = ui_kit::switch(ui, &mut settings.hide_from_capture);
                #[cfg(test)]
                test_record_control(TestControl::HideFromCapture, _switch.rect);
            },
            None,
        );
        ui.add_space(space::SCALE_4);
    });
    // The flip is detected by comparing the setting before and after the row.
    if settings.hide_from_capture != before {
        outcome.settings_changed = true;
        outcome.hide_from_capture_changed = true;
    }
}

/// One caption line: `type.caption` in `text.tertiary`, wrapping.
fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false).wrap());
}

/// The path shown in a folder row is middle-ellipsised past this many characters; the full
/// path is on hover.
const FOLDER_PATH_CHARS: usize = 52;

/// Height of the first-launch dialog's path row.
const LAUNCH_PATH_ROW_HEIGHT: f32 = 40.0;

/// Width the first-launch dialog's body wants, clamped to the screen.
const LAUNCH_BODY_WIDTH: f32 = 480.0;

/// The middle-ellipsised form of `text`: the first and last characters with `\u{2026}` between
/// when it is longer than `max` characters.
fn middle_ellipsis(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    let at = |start: usize, len: usize| text.chars().skip(start).take(len).collect::<String>();
    format!("{}\u{2026}{}", at(0, head), at(count - tail, tail))
}

/// The free space on the drive that holds `folder`, or a dash when it could not be read.
fn free_on_drive(folder: &std::path::Path) -> String {
    disk_free_bytes(folder).map_or_else(|| "-- free".to_owned(), |bytes| format!("{:.1} GB free", bytes as f64 / GIB as f64))
}

/// Which output folder a row edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FolderKind {
    /// The recordings root.
    Recordings,
    /// The replays root.
    Replays,
}

/// The Recording section's two output-folder rows (spec "Recorder output folders", Settings):
/// each shows the full path, middle-ellipsised with the full path on hover, the free space on
/// its drive and Browse and Open buttons; the replays row adds Use project folder and shows
/// the inside-each-project path while no replays folder was picked.
fn folder_rows(ui: &mut Ui, settings: &mut Settings, outcome: &mut Outcome) {
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        folder_row(ui, FolderKind::Recordings, "Save recordings to", settings, outcome);
        folder_row(ui, FolderKind::Replays, "Save replays to", settings, outcome);
        ui.add_space(space::SCALE_4);
    });
}

/// One output-folder row; `kind` says which setting Browse and Use project folder change.
fn folder_row(ui: &mut Ui, kind: FolderKind, label: &str, settings: &mut Settings, outcome: &mut Outcome) {
    let folder = match kind {
        FolderKind::Recordings => takes_root(settings),
        FolderKind::Replays => takes::replays_folder(settings),
    };
    let shown = match kind {
        FolderKind::Recordings => folder.display().to_string(),
        FolderKind::Replays if settings.replays_root.is_some() => folder.display().to_string(),
        FolderKind::Replays => format!("Inside each project: {}", folder.display()),
    };
    let free = free_on_drive(&folder);
    let replays_set = settings.replays_root.is_some();
    ui_kit::inspector_row(
        ui,
        label,
        |ui| {
            // Right to left, so the row reads left to right: the path, the free space, then the buttons.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if kind == FolderKind::Replays && replays_set {
                    let use_project = ui.small_button("Use project folder");
                    if use_project.clicked() {
                        settings.replays_root = None;
                        outcome.settings_changed = true;
                    }
                    #[cfg(test)]
                    test_record_control(TestControl::ReplaysUseProject, use_project.rect);
                }
                if ui.small_button("Open").clicked() {
                    platform::open_folder(&folder);
                }
                if ui.small_button("Browse").clicked() {
                    match kind {
                        FolderKind::Recordings => {
                            if let Some(picked) = platform::pick_folder("Choose the recordings folder") {
                                settings.root_folder = Some(picked);
                                outcome.settings_changed = true;
                            }
                        }
                        FolderKind::Replays => {
                            if let Some(picked) = platform::pick_folder("Choose the replays folder") {
                                settings.replays_root = Some(picked);
                                outcome.settings_changed = true;
                            }
                        }
                    }
                }
                ui.label(egui::RichText::new(free).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
                let path = ui.add(
                    egui::Label::new(
                        egui::RichText::new(middle_ellipsis(&shown, FOLDER_PATH_CHARS)).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_PRIMARY),
                    )
                    .selectable(false)
                    .truncate(),
                );
                path.on_hover_text(folder.display().to_string());
            });
        },
        None,
    );
}

/// What the first-launch dialog's buttons did this frame, for the app to act on.
#[derive(Debug, Clone, Copy, Default)]
pub struct FirstLaunch {
    /// Continue was pressed: the `folders_confirmed` flag is set and the settings must be saved.
    pub confirmed: bool,
    /// Browse picked a new recordings folder and the settings must be sent and saved.
    pub folder_changed: bool,
}

/// Draws the first-launch dialog while the recordings folder was never confirmed (spec
/// "Recorder output folders", UI 3): the question, the default path and Browse and Continue.
/// Continue sets the `folders_confirmed` flag; Browse picks a folder and writes it into the
/// settings directly. The dialog stays up until it is answered: Esc and a backdrop click do
/// not close it.
pub fn first_launch(ctx: &Context, settings: &mut Settings) -> FirstLaunch {
    #[cfg(test)]
    test_clear_controls();
    let mut outcome = FirstLaunch::default();
    if settings.folders_confirmed {
        return outcome;
    }
    let screen = ctx.input(|input| input.content_rect());
    let body_width = LAUNCH_BODY_WIDTH.min((screen.width() - 2.0 * SCREEN_MARGIN).max(0.0));
    let folder = takes_root(settings);
    // The dialog is drawn directly on egui::Modal like the Settings dialog (see show's note).
    let frame = egui::Frame::new()
        .fill(color::BG_PANEL)
        .corner_radius(radius::PANEL)
        .inner_margin(egui::Margin::same(space::SCALE_24 as i8))
        .stroke(egui::Stroke::new(HAIRLINE_WIDTH, color::STROKE_HAIRLINE));
    let response = egui::Modal::new(Id::new("recorder-first-launch-dialog")).frame(frame).show(ctx, |ui| {
        ui.add(egui::Label::new(
            egui::RichText::new("Where should CutMark save your recordings?").font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY),
        ));
        ui.add_space(space::SCALE_8);
        let (path_rect, _) = ui.allocate_exact_size(vec2(body_width, LAUNCH_PATH_ROW_HEIGHT), Sense::hover());
        let mut path_ui = ui.new_child(UiBuilder::new().max_rect(path_rect).layout(Layout::left_to_right(Align::Center)));
        let path = path_ui.add(
            egui::Label::new(
                egui::RichText::new(middle_ellipsis(&folder.display().to_string(), FOLDER_PATH_CHARS))
                    .font(fonts::mono(typography::VALUE_SIZE))
                    .color(color::TEXT_PRIMARY),
            )
            .selectable(false)
            .truncate(),
        );
        path.on_hover_text(folder.display().to_string());
        ui.add_space(space::SCALE_8);
        let (footer_rect, _) = ui.allocate_exact_size(vec2(body_width, FOOTER_ROW_HEIGHT), Sense::hover());
        let mut footer_ui = ui.new_child(UiBuilder::new().max_rect(footer_rect).layout(Layout::right_to_left(Align::Center)));
        // Right to left, so the row reads left to right: Browse, then Continue.
        let continue_button = ui_kit::primary_button(&mut footer_ui, "Continue");
        if continue_button.clicked() {
            settings.folders_confirmed = true;
            outcome.confirmed = true;
        }
        #[cfg(test)]
        test_record_control(TestControl::FirstLaunchContinue, continue_button.rect);
        if footer_ui.small_button("Browse").clicked()
            && let Some(picked) = platform::pick_folder("Choose the recordings folder")
        {
            settings.root_folder = Some(picked);
            outcome.folder_changed = true;
        }
    });
    // Answering is required: Esc and the backdrop click are ignored.
    let _ = response;
    outcome
}

/// The Recording section: the two output-folder rows, the naming template with its tokens hint
/// and the next-take path, the countdown length, the replay length and the prompter switch
/// (spec "Settings dialog", "Recorder output folders").
fn recording_section(ui: &mut Ui, settings: &mut Settings, next_take: &NextTake, outcome: &mut Outcome) {
    folder_rows(ui, settings, outcome);
    ui.add_space(space::SCALE_4);
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Naming template",
            |ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut settings.naming_template).desired_width(ui.available_width()));
                if field.changed() {
                    outcome.settings_changed = true;
                }
                #[cfg(test)]
                test_record_control(TestControl::NamingTemplate, field.rect);
            },
            None,
        );
        ui.add_space(space::SCALE_4);
    });
    caption(ui, "Tokens: {project} {date} {time} {take} {profile}.");
    let next_take_label = format!("Next take: {}", next_take.path);
    let next_take_response = ui.add(
        egui::Label::new(egui::RichText::new(next_take_label).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY))
            .selectable(false)
            .wrap(),
    );
    next_take_response.on_hover_text(&next_take.path);
    ui.add_space(space::SCALE_8);
    let before = settings.countdown_secs;
    let before_switch = settings.hide_countdown_when_linked;
    let before_replay = settings.replay_length_s;
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Countdown",
            |ui| {
                let _combo = egui::ComboBox::from_id_salt("settings-countdown").selected_text(countdown_job(settings.countdown_secs)).show_ui(ui, |ui| {
                    for secs in COUNTDOWN_CHOICES {
                        let _choice = ui.selectable_value(&mut settings.countdown_secs, secs, countdown_job(secs));
                        #[cfg(test)]
                        test_record_control(TestControl::CountdownChoice(secs), _choice.rect);
                    }
                });
                #[cfg(test)]
                test_record_control(TestControl::Countdown, _combo.response.rect);
            },
            None,
        );
        ui_kit::inspector_row(
            ui,
            "Replay length",
            |ui| {
                let _combo =
                    egui::ComboBox::from_id_salt("settings-replay-length").selected_text(replay_length_job(settings.replay_length_s)).show_ui(ui, |ui| {
                        for secs in REPLAY_LENGTHS_S {
                            let _choice = ui.selectable_value(&mut settings.replay_length_s, secs, replay_length_job(secs));
                            #[cfg(test)]
                            test_record_control(TestControl::ReplayLengthChoice(secs), _choice.rect);
                        }
                    });
                #[cfg(test)]
                test_record_control(TestControl::ReplayLength, _combo.response.rect);
            },
            None,
        );
        ui_kit::inspector_row(
            ui,
            "Hide my countdown when a prompter is linked",
            |ui| {
                let _switch = ui_kit::switch(ui, &mut settings.hide_countdown_when_linked);
                #[cfg(test)]
                test_record_control(TestControl::HideCountdown, _switch.rect);
            },
            None,
        );
        ui.add_space(space::SCALE_4);
    });
    if settings.countdown_secs != before || settings.replay_length_s != before_replay {
        outcome.settings_changed = true;
    }
    // The flip is detected by comparing the setting before and after the row.
    if settings.hide_countdown_when_linked != before_switch {
        outcome.settings_changed = true;
    }
}

/// The Control API section: the switch, the port, the listening and clients values, the
/// token with Show or Hide, Copy and Regenerate (spec "Settings dialog") and the Stream Deck
/// switch with its status line.
fn api_section(ui: &mut Ui, settings: &mut Settings, api: ApiUi<'_>, outcome: &mut Outcome) {
    let before_enabled = settings.api.enabled;
    let before_stream_deck = settings.stream_deck;
    let (listening, clients) = match api.server {
        Some(server) => (format!("127.0.0.1:{}", server.port()), server.client_count().to_string()),
        None => ("Not running".to_owned(), "0".to_owned()),
    };
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "On",
            |ui| {
                let _switch = ui_kit::switch(ui, &mut settings.api.enabled);
                #[cfg(test)]
                test_record_control(TestControl::ApiOn, _switch.rect);
            },
            None,
        );
        ui_kit::inspector_row(
            ui,
            "Port",
            |ui| {
                let response = ui.add(egui::DragValue::new(api.port_edit).range(1024..=65535).speed(0.0));
                if response.lost_focus() || response.drag_stopped() {
                    settings.api.port = *api.port_edit;
                    outcome.settings_changed = true;
                }
                #[cfg(test)]
                test_record_control(TestControl::ApiPort, response.rect);
            },
            None,
        );
        ui_kit::inspector_row(ui, "Listening", |_ui| {}, Some(listening.as_str()));
        ui_kit::inspector_row(ui, "Connected clients", |_ui| {}, Some(clients.as_str()));
        if let Some(api_token) = api.token {
            let shown = if *api.show_token { api_token.to_owned() } else { "*".repeat(api_token.chars().count().min(TOKEN_MASK_MAX)) };
            ui_kit::inspector_row(
                ui,
                "Token",
                |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(shown).monospace()).truncate());
                },
                None,
            );
            ui_kit::inspector_row(
                ui,
                "",
                |ui| {
                    if ui.button(if *api.show_token { "Hide" } else { "Show" }).clicked() {
                        *api.show_token = !*api.show_token;
                    }
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(api_token.to_owned());
                    }
                    let regenerate = ui.button("Regenerate").on_hover_text("Makes a new token and disconnects every client");
                    if regenerate.clicked() {
                        outcome.regenerate_token = true;
                    }
                    #[cfg(test)]
                    test_record_control(TestControl::Regenerate, regenerate.rect);
                },
                None,
            );
        }
        ui.add_space(space::SCALE_4);
    });
    ui.add_space(space::SCALE_8);
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Stream Deck",
            |ui| {
                let _switch = ui_kit::switch(ui, &mut settings.stream_deck);
                #[cfg(test)]
                test_record_control(TestControl::StreamDeck, _switch.rect);
            },
            Some(api.stream_deck_status),
        );
        ui.add_space(space::SCALE_4);
    });
    // The flip is detected by comparing the setting before and after the row.
    if settings.api.enabled != before_enabled || settings.stream_deck != before_stream_deck {
        outcome.settings_changed = true;
    }
}

/// The Diagnostics section: the recent commands the engine received, newest first.
fn diagnostics_section(ui: &mut Ui, recent: &VecDeque<Command>) {
    caption(ui, "Recent commands");
    if recent.is_empty() {
        ui.label("None yet. Press a hotkey.");
        return;
    }
    for command in recent.iter().rev() {
        ui_kit::list_row(ui, false, None, &format!("{:?}", command.kind), None);
    }
}

/// The read-only Hotkeys table (spec "Settings dialog"): one row per hotkey with its chord as
/// key caps, the action in words and its status. Every row is a full-width band of its own, so
/// none is cut off; the table scrolls with the section content.
fn hotkey_table(ui: &mut Ui, hotkeys: &Hotkeys) {
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        hotkey_header(ui);
        for hotkey in &hotkeys.registered {
            let conflict = hotkeys.altgr.iter().find(|candidate| candidate.action == hotkey.action);
            let (status, warn) = match conflict {
                Some(conflict) => (format!("AltGr types '{}'", conflict.typed), true),
                None => ("OK".to_owned(), false),
            };
            hotkey_row(ui, &hotkey.chord, action_label(&hotkey.action), &status, warn, None);
        }
        for hotkey in &hotkeys.refused {
            hotkey_row(ui, &hotkey.chord, action_label(&hotkey.action), "Taken by another program", true, Some(&hotkey.reason));
        }
        ui.add_space(space::SCALE_4);
    });
}

/// The Hotkeys table's column rects inside one row rect: shortcut, action, status.
fn hotkey_columns(rect: Rect) -> (Rect, Rect, Rect) {
    let inset = space::SCALE_4;
    let shortcut = Rect::from_min_max(Pos2::new(rect.left() + inset, rect.top()), Pos2::new(rect.left() + inset + HOTKEY_SHORTCUT_COL, rect.bottom()));
    let status = Rect::from_min_max(Pos2::new(rect.right() - inset - HOTKEY_STATUS_COL, rect.top()), Pos2::new(rect.right() - inset, rect.bottom()));
    let action = Rect::from_min_max(Pos2::new(shortcut.right(), rect.top()), Pos2::new(status.left(), rect.bottom()));
    (shortcut, action, status)
}

/// The Hotkeys table header: Shortcut, Action, Status.
fn hotkey_header(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOTKEY_HEADER_HEIGHT), Sense::hover());
    let (shortcut, action, status) = hotkey_columns(rect);
    let mut shortcut_ui = ui.new_child(UiBuilder::new().max_rect(shortcut).layout(Layout::left_to_right(Align::Center)));
    shortcut_ui.label(egui::RichText::new("Shortcut").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    let mut action_ui = ui.new_child(UiBuilder::new().max_rect(action).layout(Layout::left_to_right(Align::Center)));
    action_ui.label(egui::RichText::new("Action").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    let mut status_ui = ui.new_child(UiBuilder::new().max_rect(status).layout(Layout::right_to_left(Align::Center)));
    status_ui.label(egui::RichText::new("Status").font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
}

/// One Hotkeys table row: the chord as key caps, the action and a status that is `OK` in
/// `sem.meter.ok` and anything else in `sem.meter.warn`.
fn hotkey_row(ui: &mut Ui, chord: &str, action: &str, status: &str, warn: bool, tip: Option<&str>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOTKEY_ROW_HEIGHT), Sense::hover());
    ui.painter()
        .line_segment([Pos2::new(rect.left(), rect.top()), Pos2::new(rect.right(), rect.top())], egui::Stroke::new(HAIRLINE_WIDTH, color::STROKE_SEPARATOR));
    let (shortcut, action_rect, status_rect) = hotkey_columns(rect);
    let mut shortcut_ui = ui.new_child(UiBuilder::new().max_rect(shortcut).layout(Layout::left_to_right(Align::Center)));
    match parse_chord(chord) {
        Some(keys) => ui_kit::shortcuts::key_caps(&mut shortcut_ui, &keys),
        None => {
            shortcut_ui.add(egui::Label::new(egui::RichText::new(chord).monospace()).truncate());
        }
    }
    let mut action_ui = ui.new_child(UiBuilder::new().max_rect(action_rect).layout(Layout::left_to_right(Align::Center)));
    action_ui.add(egui::Label::new(egui::RichText::new(action).color(color::TEXT_PRIMARY)).wrap());
    let mut status_ui = ui.new_child(UiBuilder::new().max_rect(status_rect).layout(Layout::right_to_left(Align::Center)));
    let status_label =
        egui::RichText::new(status).font(fonts::medium(typography::CAPTION_SIZE)).color(if warn { color::SEM_METER_WARN } else { color::SEM_METER_OK });
    let status_response = status_ui.add(egui::Label::new(status_label).truncate());
    if let Some(tip) = tip {
        status_response.on_hover_text(tip);
    }
    #[cfg(test)]
    test_record_row(rect);
}

/// The human label of a hotkey action id (the Hotkeys table shows actions in words, not ids).
fn action_label(action: &str) -> &str {
    match action {
        "start_stop" => "Start or stop recording",
        "pause_resume" => "Pause or resume",
        "mark" => "Add mark",
        "mark_label" => "Add mark with label",
        "highlight_last" => "Highlight last 30 seconds",
        "mute_mic" => "Mute mic",
        other => other,
    }
}

/// Parses a chord like `Ctrl+Alt+R` or `F9` into egui's shortcut type, for `ui_kit::key_caps`;
/// None when a part is unknown, so the row falls back to the chord as plain text.
fn parse_chord(chord: &str) -> Option<KeyboardShortcut> {
    let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|part| !part.is_empty()).collect();
    let (key_text, modifier_parts) = parts.split_last()?;
    let mut modifiers = Modifiers::NONE;
    for part in modifier_parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers.ctrl = true,
            "shift" => modifiers.shift = true,
            "alt" | "altgr" => modifiers.alt = true,
            "super" | "win" | "cmd" | "meta" => modifiers.mac_cmd = true,
            _ => return None,
        }
    }
    Some(KeyboardShortcut::new(modifiers, Key::from_name(key_text)?))
}

/// The text for a countdown length in the settings.
fn countdown_label(secs: u8) -> String {
    if secs == 0 { "Off".to_owned() } else { format!("{secs} seconds") }
}

/// [`countdown_label`] as the dropdown draws it, with the number in Martian Mono.
fn countdown_job(secs: u8) -> egui::text::LayoutJob {
    crate::data_text::with_data(&countdown_label(secs), &[&secs.to_string()], egui::Color32::PLACEHOLDER)
}

/// A replay length as the dropdown draws it, with the number in Martian Mono.
fn replay_length_job(secs: u32) -> egui::text::LayoutJob {
    crate::data_text::with_data(&format!("{secs} seconds"), &[&secs.to_string()], egui::Color32::PLACEHOLDER)
}

// The interactive controls the tests record each frame, keyed by the Settings
// control they belong to; never built outside tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq)]
enum TestControl {
    /// The General section's Hide Recorder from screen captures switch.
    HideFromCapture,
    /// The Recording section's naming template field.
    NamingTemplate,
    /// The Recording section's countdown dropdown button.
    Countdown,
    /// One row of the countdown dropdown's popup, by its length in seconds.
    CountdownChoice(u8),
    /// The Recording section's replay length dropdown button.
    ReplayLength,
    /// One row of the replay length dropdown's popup, by its length in seconds.
    ReplayLengthChoice(u32),
    /// The Recording section's hide-my-countdown switch.
    HideCountdown,
    /// The Control API section's On switch.
    ApiOn,
    /// The Control API section's port value.
    ApiPort,
    /// The Control API section's Stream Deck switch.
    StreamDeck,
    /// The Control API section's token Regenerate button.
    Regenerate,
    /// The Recording section's replays Use project folder button.
    ReplaysUseProject,
    /// The first-launch dialog's Continue button.
    FirstLaunchContinue,
}

// The control rects the last frame drew, for the headless interaction tests.
#[cfg(test)]
thread_local! {
    static TEST_CONTROLS: std::cell::RefCell<Vec<(TestControl, Rect)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Drops the control rects the previous frame recorded.
#[cfg(test)]
fn test_clear_controls() {
    TEST_CONTROLS.with(|cells| cells.borrow_mut().clear());
}

/// Records one interactive control's rect.
#[cfg(test)]
fn test_record_control(control: TestControl, rect: Rect) {
    TEST_CONTROLS.with(|cells| cells.borrow_mut().push((control, rect)));
}

/// The rect the last frame recorded for one control.
#[cfg(test)]
fn test_control_rect(control: TestControl) -> Rect {
    TEST_CONTROLS
        .with(|cells| cells.borrow().iter().rev().find(|(recorded, _)| *recorded == control).map(|(_, rect)| *rect))
        .expect("the control was not drawn this frame")
}

// The content rect and the Hotkeys row rects the last frame drew, for the
// headless layout test; empty in a release build.
#[cfg(test)]
thread_local! {
    static TEST_LAYOUT: std::cell::RefCell<(Option<Rect>, Vec<Rect>)> = const { std::cell::RefCell::new((None, Vec::new())) };
}

/// Records the content rect, clearing the row rects of the previous frame.
#[cfg(test)]
fn test_record_content(content: Rect) {
    TEST_LAYOUT.with(|cells| *cells.borrow_mut() = (Some(content), Vec::new()));
}

/// Records one Hotkeys row rect.
#[cfg(test)]
fn test_record_row(row: Rect) {
    TEST_LAYOUT.with(|cells| cells.borrow_mut().1.push(row));
}

/// The content rect and row rects the last frame recorded.
#[cfg(test)]
fn test_layout() -> (Option<Rect>, Vec<Rect>) {
    TEST_LAYOUT.with(|cells| cells.borrow().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bus::CommandKind;
    use crate::hotkeys::{AltGrConflict, RefusedHotkey, RegisteredHotkey};

    /// The countdown lengths get their labels: Off for 0, "N seconds" otherwise.
    #[test]
    fn countdown_lengths_get_their_labels() {
        assert_eq!(countdown_label(0), "Off");
        assert_eq!(countdown_label(3), "3 seconds");
        assert_eq!(countdown_label(5), "5 seconds");
    }

    /// The Recorder's chord texts parse into key shortcuts; unknown parts do not.
    #[test]
    fn chords_parse_into_key_shortcuts() {
        let shortcut = parse_chord("Ctrl+Alt+R").expect("Ctrl+Alt+R parses");
        assert!(shortcut.modifiers.ctrl && shortcut.modifiers.alt && !shortcut.modifiers.shift);
        assert!(parse_chord("Shift+F9").is_some());
        assert!(parse_chord("F9").is_some());
        assert!(parse_chord("Ctrl+Nope").is_none());
    }

    /// The Settings dialog on a 2560 x 1000 screen: the dialog stays at its body size plus
    /// title and footer, and every Hotkeys row lies inside the content rect, one under the
    /// other instead of all in one horizontal row.
    #[test]
    fn the_dialog_fits_the_screen_and_hotkeys_rows_stay_in_the_content() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2560.0, 1000.0))), ..Default::default() };
        let mut settings = Settings::default();
        let next_take = NextTake { path: "C:\\Takes\\Project\\2026-11-23 101500\\take03.mkv".to_owned() };
        let hotkeys = Hotkeys {
            registered: vec![
                RegisteredHotkey { action: "start_stop".to_owned(), chord: "Ctrl+Alt+R".to_owned() },
                RegisteredHotkey { action: "pause_resume".to_owned(), chord: "Ctrl+Alt+P".to_owned() },
                RegisteredHotkey { action: "mark".to_owned(), chord: "F9".to_owned() },
                RegisteredHotkey { action: "mark_label".to_owned(), chord: "Shift+F9".to_owned() },
                RegisteredHotkey { action: "highlight_last".to_owned(), chord: "Ctrl+F9".to_owned() },
            ],
            refused: vec![RefusedHotkey { action: "mute_mic".to_owned(), chord: "Ctrl+Alt+M".to_owned(), reason: "Held by another program".to_owned() }],
            altgr: vec![AltGrConflict { action: "mark".to_owned(), chord: "F9".to_owned(), typed: '\u{e9}' }],
            problems: Vec::new(),
        };
        let recent: VecDeque<Command> = VecDeque::from(vec![Command::new(CommandKind::StartStop, 2), Command::new(CommandKind::GetStatus, 1)]);
        let mut show_token = false;
        let mut port_edit = 8765u16;

        // The height bound: the body, the headline title plus its 8px gap, one
        // button row plus its 8px gap, and the 24px modal padding per side. The
        // title is measured inside the first frame, where fonts exist.
        let title_measured = std::cell::Cell::new(None::<f32>);
        const FOOTER_HEIGHT: f32 = 48.0;
        // The 1px hairline rim adds 1px per side to the frame around the 24px
        // modal padding.
        let max_width = BODY_WIDTH + 48.0 + 2.0;

        // A warm-up frame: egui's content_rect (which the modal's backdrop
        // covers exactly) may be inset from the raw screen rect.
        let mut warmup = ctx.run_ui(raw.clone(), |_ui| {});
        warmup.textures_delta.clear();
        let screen = ctx.input(|input| input.content_rect());

        for section in Section::ALL {
            let mut dialog = SettingsDialog { open: true, section };
            let mut painted = None;
            for frame in 0..4u8 {
                let mut output = ctx.run_ui(raw.clone(), |ui| {
                    if title_measured.get().is_none() {
                        let galley =
                            ui.painter().layout_no_wrap("Settings".to_owned(), ui_kit::fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
                        title_measured.set(Some(galley.size().y));
                    }
                    let api = ApiUi {
                        server: None,
                        token: Some("0123456789abcdef0123456789abcdef"),
                        show_token: &mut show_token,
                        port_edit: &mut port_edit,
                        stream_deck_status: "Off",
                    };
                    show(ui.ctx(), &mut dialog, &mut settings, &next_take, &hotkeys, &recent, api);
                });
                if frame == 3 {
                    // The modal's Area rect covers the whole screen with its
                    // backdrop, so the dialog itself is measured as the union
                    // of its painted shapes minus that backdrop, as ui-kit's
                    // dialog test does.
                    let shapes = std::mem::take(&mut output.shapes);
                    let mut rects: Vec<Rect> = Vec::new();
                    for clipped in &shapes {
                        let rect = clipped.shape.visual_bounding_rect();
                        let nearly = screen.shrink(8.0);
                        let covers_screen =
                            rect.min.x <= nearly.min.x && rect.min.y <= nearly.min.y && rect.max.x >= nearly.max.x && rect.max.y >= nearly.max.y;
                        if !covers_screen {
                            rects.push(rect);
                        }
                    }
                    painted = rects.iter().copied().reduce(Rect::union);
                }
                output.textures_delta.clear();
            }
            let painted = painted.expect("the Settings dialog did not paint");
            // The title row also carries an item-spacing gap above and below.
            let title_height = title_measured.get().expect("the title was measured") + 2.0 * space::SCALE_8;
            let max_height = BODY_HEIGHT + title_height + FOOTER_HEIGHT + 48.0 + 2.0;
            assert!(painted.width() <= max_width + 0.5, "the {section:?} dialog is {painted:?}, wider than {max_width}");
            assert!(painted.height() <= max_height + 0.5, "the {section:?} dialog is {painted:?}, taller than {max_height}");
            if section == Section::Hotkeys {
                let (content, rows) = test_layout();
                let content = content.expect("the Hotkeys section recorded no content rect");
                assert_eq!(rows.len(), hotkeys.registered.len() + hotkeys.refused.len(), "not every Hotkeys row drew");
                assert!(content.right() <= painted.right() - 24.0 + 0.5, "the content rect grew past the dialog: {content:?} in {painted:?}");
                for row in &rows {
                    let inside = content.min.x <= row.min.x && content.min.y <= row.min.y && content.max.x >= row.max.x && content.max.y >= row.max.y;
                    assert!(inside, "the Hotkeys row {row:?} left the content rect {content:?}");
                }
                for pair in rows.windows(2) {
                    assert!(pair[1].top() >= pair[0].bottom() - 0.5, "two Hotkeys rows share a horizontal band: {rows:?}");
                }
            }
        }
    }

    /// The app starts with the dialog closed: with `open` false no frame paints the
    /// modal, and the dialog stays closed (the open state is never persisted).
    #[test]
    fn the_dialog_starts_closed_and_draws_nothing() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut warmup = ctx.run_ui(raw.clone(), |_ui| {});
        warmup.textures_delta.clear();
        let mut dialog = SettingsDialog::default();
        for frame in 0..3u8 {
            let output = dialog_frame(&ctx, &raw, &mut dialog);
            assert!(!dialog.open, "frame {frame}: the dialog opened by itself");
            assert!(output.shapes.is_empty(), "frame {frame}: the dialog painted while it is closed");
        }
    }

    /// Clicking Close closes the dialog: the open flag is false afterwards and the
    /// modal stops painting, and it stays closed in the frames after.
    #[test]
    fn clicking_close_closes_the_dialog_and_it_stays_closed() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut warmup = ctx.run_ui(raw.clone(), |_ui| {});
        warmup.textures_delta.clear();
        let screen = ctx.input(|input| input.content_rect());
        let mut dialog = SettingsDialog { open: true, section: Section::General };
        // The modal paints nothing on its very first frame; measure from the next one.
        let _ = dialog_frame(&ctx, &raw, &mut dialog);
        let drawn = dialog_frame(&ctx, &raw, &mut dialog);
        let painted = painted_dialog(&drawn, screen).expect("the open dialog did not paint");
        // The Close button sits at the right end of the footer row: the 1px hairline
        // plus the 24px modal padding in from the dialog's right edge, and half the
        // 32px footer row above the dialog's bottom edge.
        let close = Pos2::new(painted.right() - 25.0 - 30.0, painted.bottom() - 25.0 - 16.0);
        let mut click = raw.clone();
        click.events = vec![
            egui::Event::PointerMoved(close),
            egui::Event::PointerButton { pos: close, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::default() },
            egui::Event::PointerButton { pos: close, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::default() },
        ];
        dialog_frame(&ctx, &click, &mut dialog);
        assert!(!dialog.open, "clicking Close did not close the dialog");
        for frame in 0..2u8 {
            let output = dialog_frame(&ctx, &raw, &mut dialog);
            assert!(!dialog.open, "frame {frame} after Close: the dialog reopened");
            assert!(output.shapes.is_empty(), "frame {frame} after Close: the dialog painted again");
        }
    }

    /// Esc closes the dialog and it stays closed in the frames after.
    #[test]
    fn escape_closes_the_dialog_and_it_stays_closed() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut warmup = ctx.run_ui(raw.clone(), |_ui| {});
        warmup.textures_delta.clear();
        let mut dialog = SettingsDialog { open: true, section: Section::General };
        // The modal paints nothing on its very first frame; measure from the next one.
        let _ = dialog_frame(&ctx, &raw, &mut dialog);
        let drawn = dialog_frame(&ctx, &raw, &mut dialog);
        assert!(painted_dialog(&drawn, ctx.input(|input| input.content_rect())).is_some(), "the open dialog did not paint");
        let mut esc = raw.clone();
        esc.events = vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::default() }];
        dialog_frame(&ctx, &esc, &mut dialog);
        assert!(!dialog.open, "Esc did not close the dialog");
        for frame in 0..2u8 {
            let output = dialog_frame(&ctx, &raw, &mut dialog);
            assert!(!dialog.open, "frame {frame} after Esc: the dialog reopened");
            assert!(output.shapes.is_empty(), "frame {frame} after Esc: the dialog painted again");
        }
    }

    /// The union of the dialog's painted shapes without the modal's full-screen backdrop,
    /// or None when the dialog did not paint.
    fn painted_dialog(output: &egui::FullOutput, screen: Rect) -> Option<Rect> {
        let nearly = screen.shrink(8.0);
        output
            .shapes
            .iter()
            .map(|clipped| clipped.shape.visual_bounding_rect())
            .filter(|rect| {
                rect.is_finite() && !(rect.min.x <= nearly.min.x && rect.min.y <= nearly.min.y && rect.max.x >= nearly.max.x && rect.max.y >= nearly.max.y)
            })
            .reduce(Rect::union)
    }

    /// Runs one frame of the dialog on the given raw input and reports the frame output.
    fn dialog_frame(ctx: &Context, raw: &egui::RawInput, dialog: &mut SettingsDialog) -> egui::FullOutput {
        let mut settings = Settings::default();
        let hotkeys = Hotkeys { registered: Vec::new(), refused: Vec::new(), altgr: Vec::new(), problems: Vec::new() };
        let recent: VecDeque<Command> = VecDeque::new();
        let next_take = NextTake::default();
        let mut show_token = false;
        let mut port_edit = 8765u16;
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            let api = ApiUi { server: None, token: None, show_token: &mut show_token, port_edit: &mut port_edit, stream_deck_status: "Off" };
            show(ui.ctx(), dialog, &mut settings, &next_take, &hotkeys, &recent, api);
        });
        output.textures_delta.clear();
        output
    }

    /// The raw input for one frame on the test screen.
    fn test_raw() -> egui::RawInput {
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() }
    }

    /// The warmed-up state one interaction test starts from: fonts applied, the raw input and
    /// the slot the Outcome of every frame lands in.
    fn test_setup() -> (Context, egui::RawInput, std::cell::Cell<Outcome>) {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = test_raw();
        let mut warmup = ctx.run_ui(raw.clone(), |_ui| {});
        warmup.textures_delta.clear();
        (ctx, raw, std::cell::Cell::new(Outcome::default()))
    }

    /// A press and release of the primary pointer button at one position.
    fn click_at(raw: &egui::RawInput, pos: Pos2) -> egui::RawInput {
        let mut click = raw.clone();
        click.events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::default() },
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::default() },
        ];
        click
    }

    /// Runs one frame of the dialog on the given input, keeping the settings and the port
    /// buffer across frames and capturing the frame's Outcome.
    fn interact_frame(
        ctx: &Context,
        raw: &egui::RawInput,
        dialog: &mut SettingsDialog,
        settings: &mut Settings,
        port_edit: &mut u16,
        token: Option<&str>,
        outcome: &std::cell::Cell<Outcome>,
    ) -> egui::FullOutput {
        let hotkeys = Hotkeys { registered: Vec::new(), refused: Vec::new(), altgr: Vec::new(), problems: Vec::new() };
        let recent: VecDeque<Command> = VecDeque::new();
        let next_take = NextTake::default();
        let mut show_token = false;
        let mut output = ctx.run_ui(raw.clone(), |ui| {
            let api = ApiUi { server: None, token, show_token: &mut show_token, port_edit, stream_deck_status: "Off" };
            outcome.set(show(ui.ctx(), dialog, settings, &next_take, &hotkeys, &recent, api));
        });
        output.textures_delta.clear();
        output
    }

    /// The two frames the modal needs before its controls can be clicked.
    fn warm_frames(
        ctx: &Context,
        raw: &egui::RawInput,
        dialog: &mut SettingsDialog,
        settings: &mut Settings,
        port_edit: &mut u16,
        token: Option<&str>,
        outcome: &std::cell::Cell<Outcome>,
    ) {
        for _ in 0..2 {
            interact_frame(ctx, raw, dialog, settings, port_edit, token, outcome);
        }
    }

    /// Flipping the General section's Hide Recorder from screen captures switch reports both
    /// outcome flags and flips the settings field, without repeating in the frames after.
    #[test]
    fn flipping_hide_from_capture_reports_both_flags_and_flips_the_field() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::General };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let switch = test_control_rect(TestControl::HideFromCapture).center();
        let before = settings.hide_from_capture;
        interact_frame(&ctx, &click_at(&raw, switch), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let flipped = outcome.get();
        assert!(flipped.settings_changed, "the switch flip did not report settings_changed");
        assert!(flipped.hide_from_capture_changed, "the switch flip did not report hide_from_capture_changed");
        assert_eq!(settings.hide_from_capture, !before, "the switch flip did not reach the settings field");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let settled = outcome.get();
        assert!(!settled.settings_changed && !settled.hide_from_capture_changed, "the flip repeated in the frame after");
        assert_eq!(settings.hide_from_capture, !before, "the flip did not stick");
    }

    /// Typing into the Recording section's naming template reports the change and reaches the
    /// settings field.
    #[test]
    fn typing_in_the_naming_template_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::Recording };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let field = test_control_rect(TestControl::NamingTemplate).center();
        interact_frame(&ctx, &click_at(&raw, field), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let before = settings.naming_template.clone();
        let mut typing = raw.clone();
        typing.events = vec![egui::Event::Text("X".to_owned())];
        interact_frame(&ctx, &typing, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "typing did not report settings_changed");
        assert_ne!(settings.naming_template, before, "the typed text did not reach the settings field");
        assert!(settings.naming_template.contains('X'), "the typed text is missing: {}", settings.naming_template);
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(!outcome.get().settings_changed, "the change repeated in the frame after");
    }

    /// Choosing a countdown length from the dropdown reports the change and reaches the
    /// settings field.
    #[test]
    fn choosing_a_countdown_length_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::Recording };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let combo = test_control_rect(TestControl::Countdown).center();
        interact_frame(&ctx, &click_at(&raw, combo), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let three = test_control_rect(TestControl::CountdownChoice(3)).center();
        assert_eq!(settings.countdown_secs, 0, "the dialog changed the countdown on its own");
        interact_frame(&ctx, &click_at(&raw, three), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "the countdown choice did not report settings_changed");
        assert_eq!(settings.countdown_secs, 3, "the countdown choice did not reach the settings field");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(!outcome.get().settings_changed, "the change repeated in the frame after");
    }

    /// Choosing a replay length from the dropdown reports the change and reaches the settings
    /// field.
    #[test]
    fn choosing_a_replay_length_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::Recording };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let combo = test_control_rect(TestControl::ReplayLength).center();
        interact_frame(&ctx, &click_at(&raw, combo), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        for secs in REPLAY_LENGTHS_S {
            let _drawn = test_control_rect(TestControl::ReplayLengthChoice(secs));
        }
        let ninety = test_control_rect(TestControl::ReplayLengthChoice(90)).center();
        assert_eq!(settings.replay_length_s, 60, "the dialog changed the replay length on its own");
        interact_frame(&ctx, &click_at(&raw, ninety), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "the length choice did not report settings_changed");
        assert_eq!(settings.replay_length_s, 90, "the length choice did not reach the settings field");
    }

    /// Flipping the Recording section's hide-my-countdown switch reports the change and
    /// reaches the settings field.
    #[test]
    fn flipping_the_prompter_switch_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::Recording };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let switch = test_control_rect(TestControl::HideCountdown).center();
        let before = settings.hide_countdown_when_linked;
        interact_frame(&ctx, &click_at(&raw, switch), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "the switch flip did not report settings_changed");
        assert_eq!(settings.hide_countdown_when_linked, !before, "the switch flip did not reach the settings field");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert_eq!(settings.hide_countdown_when_linked, !before, "the flip did not stick");
    }

    /// Flipping the Control API section's On switch reports the change and reaches the
    /// settings field.
    #[test]
    fn flipping_the_api_on_switch_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::ControlApi };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let switch = test_control_rect(TestControl::ApiOn).center();
        let before = settings.api.enabled;
        interact_frame(&ctx, &click_at(&raw, switch), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "the switch flip did not report settings_changed");
        assert_eq!(settings.api.enabled, !before, "the switch flip did not reach the settings field");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert_eq!(settings.api.enabled, !before, "the flip did not stick");
    }

    /// The Stream Deck switch is on by default; flipping it reports the change, so the app
    /// saves it.
    #[test]
    fn flipping_the_stream_deck_switch_reports_the_change() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::ControlApi };
        let mut settings = Settings::default();
        assert!(settings.stream_deck, "the remote is on by default");
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let switch = test_control_rect(TestControl::StreamDeck).center();
        interact_frame(&ctx, &click_at(&raw, switch), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "the switch flip did not report settings_changed");
        assert!(!settings.stream_deck, "the switch flip did not reach the settings field");
        assert!(settings.api.enabled, "the Control API switch is not the Stream Deck switch");
    }

    /// Typing a new port and pressing Enter reports the change and reaches the settings field
    /// (the port is committed when the field loses focus).
    #[test]
    fn editing_the_api_port_reports_the_change_on_commit() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::ControlApi };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let port = test_control_rect(TestControl::ApiPort).center();
        interact_frame(&ctx, &click_at(&raw, port), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        // One plain frame: the click gains focus and the old value is selected.
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let mut typing = raw.clone();
        typing.events = vec![egui::Event::Text("1".to_owned())];
        interact_frame(&ctx, &typing, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let mut enter = raw.clone();
        enter.events = vec![
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::default() },
            egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::default() },
        ];
        interact_frame(&ctx, &enter, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "committing the port did not report settings_changed");
        assert_eq!(settings.api.port, 1024, "the edited port did not reach the settings field");
        assert_eq!(port_edit, 1024, "the port buffer did not take the committed value");
        // egui reports lost_focus for two frames, so the frame after Enter commits
        // once more; the value must stay the same either way.
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert_eq!(settings.api.port, 1024, "the port did not stick");
        assert_eq!(port_edit, 1024, "the port buffer did not stick");
    }

    /// Clicking the token Regenerate button reports it, without repeating in the frame after.
    #[test]
    fn clicking_regenerate_reports_it() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::ControlApi };
        let mut settings = Settings::default();
        let mut port_edit = 8765u16;
        let token = "0123456789abcdef0123456789abcdef";
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, Some(token), &outcome);
        let button = test_control_rect(TestControl::Regenerate).center();
        interact_frame(&ctx, &click_at(&raw, button), &mut dialog, &mut settings, &mut port_edit, Some(token), &outcome);
        assert!(outcome.get().regenerate_token, "the Regenerate click was not reported");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, Some(token), &outcome);
        assert!(!outcome.get().regenerate_token, "the Regenerate click repeated in the frame after");
    }

    /// Clicking the replays row's Use project folder resets the replays root to the default and
    /// reports the change, without repeating in the frame after.
    #[test]
    fn use_project_folder_resets_the_replays_root() {
        let (ctx, raw, outcome) = test_setup();
        let mut dialog = SettingsDialog { open: true, section: Section::Recording };
        let mut settings = Settings { replays_root: Some(std::path::PathBuf::from("D:\\Replays")), ..Settings::default() };
        let mut port_edit = 8765u16;
        warm_frames(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        let button = test_control_rect(TestControl::ReplaysUseProject).center();
        interact_frame(&ctx, &click_at(&raw, button), &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(outcome.get().settings_changed, "Use project folder did not report settings_changed");
        assert_eq!(settings.replays_root, None, "Use project folder did not reset the replays root");
        interact_frame(&ctx, &raw, &mut dialog, &mut settings, &mut port_edit, None, &outcome);
        assert!(!outcome.get().settings_changed, "the reset repeated in the frame after");
    }

    /// The first-launch dialog: Continue sets the folders_confirmed flag, the dialog stops
    /// drawing afterwards and nothing else closed it.
    #[test]
    fn the_first_launch_dialog_confirms_and_then_stays_closed() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0))), ..Default::default() };
        let mut settings = Settings::default();
        let frame = |ctx: &Context, raw: &egui::RawInput, settings: &mut Settings| {
            let mut output = ctx.run_ui(raw.clone(), |ui| {
                first_launch(ui.ctx(), settings);
            });
            output.textures_delta.clear();
            output
        };
        // The modal paints nothing on its very first frame; the controls are clickable from the next one.
        frame(&ctx, &raw, &mut settings);
        frame(&ctx, &raw, &mut settings);
        assert!(!settings.folders_confirmed, "the dialog confirmed itself without a click");
        let continue_at = test_control_rect(TestControl::FirstLaunchContinue).center();
        frame(&ctx, &click_at(&raw, continue_at), &mut settings);
        assert!(settings.folders_confirmed, "Continue did not set the folders_confirmed flag");
        frame(&ctx, &raw, &mut settings);
        let confirmed = TEST_CONTROLS.with(|cells| cells.borrow().iter().all(|(control, _)| *control != TestControl::FirstLaunchContinue));
        assert!(confirmed, "the first-launch dialog drew again after Continue");
    }

    /// A long path is middle-ellipsised and keeps both ends; a short one passes through.
    #[test]
    fn middle_ellipsis_keeps_both_ends_of_a_long_path() {
        let long = "C:\\Users\\JD\\Videos\\Cutmark\\My Show\\2026-11-27 101500";
        let shown = middle_ellipsis(long, 24);
        let shown: Vec<char> = shown.chars().collect();
        assert_eq!(shown.len(), 24, "the ellipsised path is not the asked length: {shown:?}");
        let middle = shown.iter().position(|c| *c == '\u{2026}').expect("the ellipsis is there");
        let long: Vec<char> = long.chars().collect();
        assert_eq!(shown[..middle], long[..middle], "the head was cut: {:?}", shown.iter().collect::<String>());
        let tail = long.len() - (shown.len() - middle - 1);
        assert_eq!(shown[middle + 1..], long[tail..], "the tail was cut: {:?}", shown.iter().collect::<String>());
        assert_eq!(middle_ellipsis("C:\\Takes", 24), "C:\\Takes", "a short path was changed");
    }
}
