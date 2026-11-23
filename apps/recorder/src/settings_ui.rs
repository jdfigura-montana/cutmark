//! The Settings dialog (spec "Settings dialog"): the sections in a list on the left and the open
//! section on the right, in the ui-kit dialog look (bg.panel frame, headline title, footer
//! button). File > Settings (Ctrl+,) and View > Appearance open it; Esc closes it. The app owns
//! the open state and everything the sections edit.

use std::collections::VecDeque;

use egui::{Align, Context, Id, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect, Sense, Ui, UiBuilder, vec2};

use ui_kit::fonts;
use ui_kit::tokens::{color, radius, space, typography};

use crate::api::ApiServer;
use crate::config::Settings;
use crate::engine::bus::Command;
use crate::hotkeys::Hotkeys;

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
    /// The accent picker and the glass switch.
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
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Hide Recorder from screen captures",
            |ui| {
                if ui_kit::switch(ui, &mut settings.hide_from_capture).changed() {
                    outcome.settings_changed = true;
                    outcome.hide_from_capture_changed = true;
                }
            },
            None,
        );
        ui.add_space(space::SCALE_4);
    });
}

/// One caption line: `type.caption` in `text.tertiary`, wrapping.
fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false).wrap());
}

/// The Recording section: the naming template with its tokens hint and the next-take path,
/// the countdown length and the prompter switch (spec "Settings dialog").
fn recording_section(ui: &mut Ui, settings: &mut Settings, next_take: &NextTake, outcome: &mut Outcome) {
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Naming template",
            |ui| {
                if ui.add(egui::TextEdit::singleline(&mut settings.naming_template).desired_width(ui.available_width())).changed() {
                    outcome.settings_changed = true;
                }
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
    ui_kit::inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui_kit::inspector_row(
            ui,
            "Countdown",
            |ui| {
                egui::ComboBox::from_id_salt("settings-countdown").selected_text(countdown_label(settings.countdown_secs)).show_ui(ui, |ui| {
                    for secs in COUNTDOWN_CHOICES {
                        ui.selectable_value(&mut settings.countdown_secs, secs, countdown_label(secs));
                    }
                });
            },
            None,
        );
        ui_kit::inspector_row(
            ui,
            "Hide my countdown when a prompter is linked",
            |ui| {
                if ui_kit::switch(ui, &mut settings.hide_countdown_when_linked).changed() {
                    outcome.settings_changed = true;
                }
            },
            None,
        );
        ui.add_space(space::SCALE_4);
    });
    if settings.countdown_secs != before {
        outcome.settings_changed = true;
    }
}

/// The Control API section: the switch, the port, the listening and clients values and the
/// token with Show or Hide, Copy and Regenerate (spec "Settings dialog").
fn api_section(ui: &mut Ui, settings: &mut Settings, api: ApiUi<'_>, outcome: &mut Outcome) {
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
                if ui_kit::switch(ui, &mut settings.api.enabled).changed() {
                    outcome.settings_changed = true;
                }
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
                    if ui.button("Regenerate").on_hover_text("Makes a new token and disconnects every client").clicked() {
                        outcome.regenerate_token = true;
                    }
                },
                None,
            );
        }
        ui.add_space(space::SCALE_4);
    });
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
                    let api = ApiUi { server: None, token: Some("0123456789abcdef0123456789abcdef"), show_token: &mut show_token, port_edit: &mut port_edit };
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
            let api = ApiUi { server: None, token: None, show_token: &mut show_token, port_edit: &mut port_edit };
            show(ui.ctx(), dialog, &mut settings, &next_take, &hotkeys, &recent, api);
        });
        output.textures_delta.clear();
        output
    }
}
