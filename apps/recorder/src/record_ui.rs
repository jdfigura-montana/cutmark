//! The transport deck over the preview and the Record mode's Live panel
//! (spec "Setup mode": the deck; spec "Record mode": the rim, the pill and
//! the Live panel).
//!
//! See the recorder spec: docs/suite/recorder-phase-13-ui.md.

use egui::{
    Align, Color32, CornerRadius, Id, Key, KeyboardShortcut, Layout, Modifiers, Rect, Response, RichText, Sense, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{inspector_group, inspector_row};

use crate::app::{Chords, send};
use crate::config::AudioSettings;
use crate::engine::bus::{BusSender, CommandKind, MarkKind};
use crate::engine::controller::{ControllerState, Phase};
use crate::engine::session::Summary;
use crate::marks_ui;
use crate::tracks_ui::{self, LiveTrackStats};

/// Left padding of the capsule's content (the dot or square).
const CAPSULE_PAD_LEFT: f32 = 14.0;
/// Right padding of the capsule's content (the label).
const CAPSULE_PAD_RIGHT: f32 = 18.0;
/// Side of the white dot or square inside the record capsule.
const CAPSULE_GLYPH: f32 = 10.0;
/// Corner radius of the capsule's white square, the stop glyph.
const SQUARE_RADIUS: f32 = 2.5;
/// Opacity of the capsule while it is disabled (spec "States": disabled at 40%).
const DISABLED_OPACITY: f32 = 0.4;
/// Gap between the capsule's glyph and its label.
const CAPSULE_GAP: f32 = space::SCALE_8;
/// Left padding of a panel header's title, the same inset the app's other
/// headers use.
const HEADER_PAD: f32 = 6.0;
/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// The idle deck's timecode (spec "Setup mode").
const IDLE_TIMECODE: &str = "00:00:00";
/// The idle deck's timecode's frame fraction.
const IDLE_TIMECODE_FRACTION: &str = ":00";
/// Gaps between the deck's items (spec "Setup mode"): the timecode sits 12 px
/// from the deck's left edge and 12 px from the first button, and the caption
/// 12 px after the last button.
const DECK_GAP: f32 = space::SCALE_12;
/// Gap between the deck's buttons and the capsule (spec "Setup mode").
const DECK_BUTTON_GAP: f32 = space::SCALE_4;
/// Space after the deck's caption; the space scale has no 14 px step.
const DECK_CAPTION_TAIL: f32 = 14.0;

/// What the deck needs from the app for one frame.
pub struct Deck<'a> {
    /// The controller phase.
    pub phase: Phase,
    /// The frame rate of the running take, for the timecode.
    pub fps: u32,
    /// The frames captured so far, while a take runs.
    pub frames: Option<u64>,
    /// The countdown setting in seconds; 0 is off.
    pub countdown_secs: u8,
    /// The hotkey chords the tooltips show.
    pub chords: &'a Chords,
}

/// The deck over the preview (spec "Setup mode" and "Record mode"), centered
/// at the bottom of the preview panel. Idle it shows the timecode at zero,
/// Mark, the Record capsule, a disabled Pause and the countdown caption;
/// while a take runs it shows the running timecode, Mark, the Stop capsule,
/// Pause or Resume and Mute mic. Every button sends the command the menu and
/// the hotkeys send, and every tooltip names the hotkey.
pub fn deck(ui: &Ui, content_rect: Rect, id_salt: &str, d: Deck<'_>, bus: &BusSender) {
    ui_kit::transport_deck(ui, content_rect, Id::new(id_salt), false, |ui| {
        deck_contents(ui, d, &mut |kind| send(bus, kind));
    });
}

/// The deck's buttons and captions, reporting every click through `send`.
/// Split from [`deck`] so a test can draw the deck without a command bus.
///
/// Everything sits on one center line: the contents fill the deck button
/// height in a centered row, so the timecode, the buttons, the capsule and
/// the caption all center on the deck's center. The gaps are the deck's own
/// (12 px around the timecode and before the caption, 4 px between the
/// buttons and the capsule, 14 px after the caption), so egui's item spacing
/// is off inside the row.
fn deck_contents(ui: &mut Ui, d: Deck<'_>, send: &mut dyn FnMut(CommandKind)) {
    let running = d.phase.has_take();
    let height = size::TRANSPORT_BUTTON;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), height), Layout::left_to_right(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(DECK_GAP);
        let timecode = match (running, d.frames) {
            (true, Some(frames)) => {
                let (main, fraction) = take_timecode(frames, d.fps);
                ui_kit::timecode(ui, &main, &fraction)
            }
            _ => ui_kit::timecode(ui, IDLE_TIMECODE, IDLE_TIMECODE_FRACTION),
        };
        note_deck_item(timecode.response.rect);
        ui.add_space(DECK_GAP);
        let mark = ui_kit::deck_button(ui, ui_kit::icons::regular::FLAG, &tip("Add mark", &d.chords.mark));
        if mark.clicked() {
            send(CommandKind::Mark(MarkKind::Silent));
        }
        note_deck_item(mark.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let start_stop = if running { "Stop recording" } else { "Start recording" };
        let capsule = record_capsule(ui, running, &tip(start_stop, &d.chords.start_stop));
        if capsule.clicked() {
            send(CommandKind::StartStop);
        }
        note_deck_item(capsule.rect);
        ui.add_space(DECK_BUTTON_GAP);
        let (icon, action) = if d.phase == Phase::Paused { (ui_kit::icons::fill::PLAY, "Resume") } else { (ui_kit::icons::fill::PAUSE, "Pause") };
        ui.add_enabled_ui(running, |ui| {
            let pause = ui_kit::deck_button(ui, icon, &tip(action, &d.chords.pause_resume));
            if pause.clicked() {
                send(CommandKind::PauseResume);
            }
            note_deck_item(pause.rect);
        });
        if running {
            ui.add_space(DECK_BUTTON_GAP);
            let mute = ui_kit::deck_button(ui, ui_kit::icons::regular::MICROPHONE_SLASH, &tip("Mute mic", &d.chords.mute_mic));
            if mute.clicked() {
                // The mic is assumed to sit on track 0, like the hotkey does.
                send(CommandKind::Mute { track: 0 });
            }
            note_deck_item(mute.rect);
        }
        let caption = countdown_caption(d.phase, d.countdown_secs);
        if !caption.is_empty() {
            ui.add_space(DECK_GAP);
            let label = ui.add(
                egui::Label::new(RichText::new(caption).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false),
            );
            note_deck_item(label.rect);
            ui.add_space(DECK_CAPTION_TAIL);
        }
    });
}

/// The label the record capsule reports: Record while idle, Stop while a take runs.
fn capsule_label(recording: bool) -> &'static str {
    if recording { "Stop" } else { "Record" }
}

/// The recording pill's text: REC while a take runs, PAUSED while it is paused.
pub(crate) fn pill_label(paused: bool) -> &'static str {
    if paused { "PAUSED" } else { "REC" }
}

// The rects of the last record capsule a test drew: the outer rect, the
// glyph rect and the label rect (test hook for the containment test).
#[cfg(test)]
thread_local! {
    static LAST_CAPSULE_RECTS: std::cell::RefCell<Option<(Rect, Rect, Rect)>> = const { std::cell::RefCell::new(None) };
}

/// The record capsule (spec "Setup mode"): one `sem.record` capsule measured
/// from its content (dot, gap, label) plus 14 px left and 18 px right padding
/// at the deck button height with a true capsule radius (height / 2), so the
/// fill always covers the white dot and the white "Record" label while idle
/// and the white rounded square and "Stop" while a take runs. Hover and
/// pressed take the derived `sem.record` shades, disabled fades to 40%, the
/// focus ring follows the capsule, and the dot and the label sit vertically
/// centered on the rect, on the deck buttons' center line. ui-kit's
/// `play_button` has no labelled capsule, so the app draws this one from the
/// tokens.
pub fn record_capsule(ui: &mut Ui, recording: bool, tooltip: &str) -> Response {
    let height = size::TRANSPORT_BUTTON;
    let label = capsule_label(recording);
    // White like the Play capsule's record glyph: there is no text.on_record token.
    let label_galley = ui.painter().layout_no_wrap(label.to_owned(), ui_kit::fonts::semibold(typography::BODY_SIZE), Color32::WHITE);
    let width = CAPSULE_PAD_LEFT + CAPSULE_GLYPH + CAPSULE_GAP + label_galley.size().x + CAPSULE_PAD_RIGHT;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let response = response.on_hover_text(tooltip);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, label));
    // The hover and pressed shades come from the same derivation the accent's
    // shades come from; the base fill stays the exact token value.
    let record = ui_kit::accent::derive(color::SEM_RECORD);
    let enabled = ui.is_enabled();
    let content = if enabled { Color32::WHITE } else { faded(Color32::WHITE) };
    let fill = if !enabled {
        faded(color::SEM_RECORD)
    } else if response.is_pointer_button_down_on() {
        record.pressed
    } else if response.hovered() {
        record.hover
    } else {
        color::SEM_RECORD
    };
    let dot_rect = Rect::from_min_size(pos2(rect.left() + CAPSULE_PAD_LEFT, rect.center().y - CAPSULE_GLYPH / 2.0), vec2(CAPSULE_GLYPH, CAPSULE_GLYPH));
    let label_pos = pos2(dot_rect.right() + CAPSULE_GAP, rect.center().y - label_galley.size().y / 2.0);
    #[cfg(test)]
    let label_rect = Rect::from_min_size(label_pos, label_galley.size());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::from(height / 2.0), fill);
        if recording {
            painter.rect_filled(dot_rect, CornerRadius::from(SQUARE_RADIUS), content);
        } else {
            painter.circle_filled(dot_rect.center(), CAPSULE_GLYPH / 2.0, content);
        }
        painter.galley(label_pos, label_galley, Color32::PLACEHOLDER);
    }
    if response.has_focus() {
        ui_kit::focus_ring(ui, rect, CornerRadius::from(height / 2.0));
    }
    #[cfg(test)]
    LAST_CAPSULE_RECTS.with(|slot| *slot.borrow_mut() = Some((rect, dot_rect, label_rect)));
    response
}

/// Fades a fully opaque token color to [`DISABLED_OPACITY`] alpha (spec
/// "States": disabled at 40%).
fn faded(color: Color32) -> Color32 {
    color.gamma_multiply(DISABLED_OPACITY)
}

// The rects of the deck's items, recorded for the center-line test (test hook).
#[cfg(test)]
thread_local! {
    static DECK_ITEM_RECTS: std::cell::RefCell<Vec<Rect>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Notes one deck item's rect for the center-line test; a no-op outside tests.
#[cfg(not(test))]
fn note_deck_item(_: Rect) {}

/// Notes one deck item's rect for the center-line test (test builds only).
#[cfg(test)]
fn note_deck_item(rect: Rect) {
    DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().push(rect));
}

/// The running take's timecode: hours, minutes, seconds and the frame within
/// the second, from the frames the take has captured (paused time removed).
fn take_timecode(frames: u64, fps: u32) -> (String, String) {
    let fps = u64::from(fps.max(1));
    let seconds = frames / fps;
    (format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60), format!(":{:02}", frames % fps))
}

/// The caption on the deck: the countdown setting while idle, "Starting in N"
/// while the countdown runs, nothing while a take records (spec "Mapping").
fn countdown_caption(phase: Phase, countdown_secs: u8) -> String {
    match phase {
        Phase::Countdown(left) => format!("Starting in {left}"),
        Phase::Idle => match countdown_secs {
            0 => String::new(),
            secs => format!("{secs} s countdown"),
        },
        Phase::Recording | Phase::Paused => String::new(),
    }
}

/// An action tooltip with its hotkey chord, when one is registered.
fn tip(action: &str, chord: &str) -> String {
    if chord.is_empty() { action.to_owned() } else { format!("{action} ({chord})") }
}

/// The Record mode's right Live panel (spec "Record mode"): the levels, the
/// running take's encoder stats, the marks, the last take's summary while
/// idle, and the mark hotkeys. `live` holds the running take's meters.
pub fn live_panel(ui: &mut Ui, state: &ControllerState, audio: &AudioSettings, live: Option<&LiveTrackStats>) {
    live_header(ui, state);
    ui.add_space(space::SCALE_8);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        caption(ui, "Levels");
        // One row per enabled track with its level meter and peak; these are
        // the same rows the Sources panel shows (spec "Record mode").
        tracks_ui::sources_rows(ui, audio, live);
        ui.add_space(space::SCALE_8);
        if state.phase.has_take() {
            caption(ui, "Encoder");
            encoder_group(ui, state);
            ui.add_space(space::SCALE_8);
        }
        caption(ui, "Marks");
        // Newest first, so the newest mark is the selected row at the top.
        marks_ui::rows(ui, &state.recent_marks);
        ui.add_space(space::SCALE_8);
        if state.phase == Phase::Idle
            && let Some(summary) = &state.summary
        {
            caption(ui, "Last take");
            last_take_group(ui, summary);
            ui.add_space(space::SCALE_8);
        }
        footer(ui);
    });
}

/// The Live panel header (44 tall): "Live" and the take name in `sem.record`,
/// with the hairline along the bottom of the row.
fn live_header(ui: &mut Ui, state: &ControllerState) {
    let row = vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.add(egui::Label::new(RichText::new("Live").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY)).selectable(false));
        if state.phase.has_take()
            && let Some(take) = &state.take
        {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&take.base_name).font(ui_kit::fonts::medium(typography::BODY_SIZE)).color(color::SEM_RECORD))
                        .selectable(false)
                        .truncate(),
                );
            });
        }
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// A group caption ("Levels", "Encoder") in the caption style.
fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false));
}

/// One read-only inspector row with its value.
fn row(ui: &mut Ui, label: &str, value: &str) {
    inspector_row(ui, label, |_ui| {}, Some(value));
}

/// The Live panel's Encoder group: the running take's stats, the rows of the
/// old recording panel (spec "Mapping").
fn encoder_group(ui: &mut Ui, state: &ControllerState) {
    let (Some(take), Some(stats)) = (&state.take, &state.stats) else { return };
    let elapsed = stats.frames / u64::from(state.fps.max(1));
    inspector_group(ui, |ui| {
        row(ui, "Time", &format_hms(elapsed));
        row(ui, "File", &take.base_name);
        row(ui, "Folder", &take.folder.display().to_string());
        row(ui, "Size", &format!("{:.1} MB", stats.bytes_written as f64 / 1_000_000.0));
        row(ui, "Bitrate", &format!("{:.1} Mbps", state.mbps));
        row(ui, "Frames", &stats.frames.to_string());
        row(ui, "Duplicated", &stats.duplicated.to_string());
        row(ui, "Dropped", &stats.dropped.to_string());
        row(ui, "Captured", &stats.frames_received.to_string());
    });
}

/// The Live panel's Last take group, shown while idle (spec "Mapping": the
/// last take summary stays in the Record Live panel until R8's Review panel).
fn last_take_group(ui: &mut Ui, summary: &Summary) {
    inspector_group(ui, |ui| {
        row(ui, "File", &summary.file.display().to_string());
        row(ui, "Frames", &summary.frames.to_string());
        row(ui, "Duplicated", &summary.duplicated.to_string());
        row(ui, "Dropped", &summary.dropped.to_string());
        row(ui, "Size", &format!("{:.1} MB", summary.bytes_written as f64 / 1_000_000.0));
        row(ui, "Duration", &format_hms(summary.duration_ms / 1000));
        row(ui, "Average bitrate", &format!("{:.2} Mbps", summary.average_mbps));
    });
}

/// The Live panel's footer: the mark hotkeys as key caps (spec "Record mode").
fn footer(ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui_kit::shortcuts::key_caps(ui, &KeyboardShortcut::new(Modifiers::NONE, Key::F9));
        ui.label(RichText::new("mark").font(ui_kit::fonts::medium(typography::BODY_SIZE)).color(color::TEXT_SECONDARY));
    });
    ui.horizontal(|ui| {
        ui_kit::shortcuts::key_caps(ui, &KeyboardShortcut::new(Modifiers::SHIFT, Key::F9));
        ui.label(RichText::new("mark with label").font(ui_kit::fonts::medium(typography::BODY_SIZE)).color(color::TEXT_SECONDARY));
    });
}

/// Formats a number of seconds as HH:MM:SS.
fn format_hms(total_seconds: u64) -> String {
    format!("{:02}:{:02}:{:02}", total_seconds / 3600, total_seconds / 60 % 60, total_seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The deck's capsule reads Record while idle and Stop while a take runs.
    #[test]
    fn the_deck_capsule_reads_record_when_idle_and_stop_while_recording() {
        assert_eq!(capsule_label(false), "Record");
        assert_eq!(capsule_label(true), "Stop");
    }

    /// The recording pill reads REC while recording and PAUSED while paused.
    #[test]
    fn the_rec_pill_reads_paused_while_paused() {
        assert_eq!(pill_label(false), "REC");
        assert_eq!(pill_label(true), "PAUSED");
    }

    /// The countdown caption follows the setting and the running countdown.
    #[test]
    fn the_countdown_caption_follows_the_setting_and_the_countdown() {
        assert_eq!(countdown_caption(Phase::Idle, 0), "");
        assert_eq!(countdown_caption(Phase::Idle, 3), "3 s countdown");
        assert_eq!(countdown_caption(Phase::Countdown(2), 5), "Starting in 2");
        assert_eq!(countdown_caption(Phase::Recording, 3), "");
    }

    /// The running timecode splits the captured frames into time and frame.
    #[test]
    fn the_running_timecode_counts_time_and_frames() {
        assert_eq!(take_timecode(0, 60), ("00:00:00".to_owned(), ":00".to_owned()));
        // One hour, one second and one frame at 60 fps.
        assert_eq!(take_timecode(60 * 3601 + 1, 60), ("01:00:01".to_owned(), ":01".to_owned()));
    }

    /// The deck draws headlessly, idle, recording and paused, without panicking.
    #[test]
    fn the_deck_draws_while_idle_and_while_recording() {
        let chords = Chords {
            start_stop: "F10".to_owned(),
            pause_resume: String::new(),
            mark: "F9".to_owned(),
            mark_label: "Shift+F9".to_owned(),
            mute_mic: String::new(),
        };
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        for (phase, frames) in [(Phase::Idle, None), (Phase::Recording, Some(60 * 3601 + 1)), (Phase::Paused, Some(120))] {
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                let d = Deck { phase, fps: 60, frames, countdown_secs: 3, chords: &chords };
                deck_contents(ui, d, &mut |_| {});
            });
            output.textures_delta.clear();
        }
    }

    /// The deck's items all sit on one center line (spec "Setup mode"): in a
    /// headless frame, every deck item's rect center y is within 1 px of the
    /// deck's center y, while idle, recording and paused.
    #[test]
    fn the_deck_items_sit_on_one_center_line() {
        let chords = Chords {
            start_stop: "F10".to_owned(),
            pause_resume: String::new(),
            mark: "F9".to_owned(),
            mark_label: "Shift+F9".to_owned(),
            mute_mic: String::new(),
        };
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        for (phase, frames) in [(Phase::Idle, None), (Phase::Recording, Some(60 * 3601 + 1)), (Phase::Paused, Some(120))] {
            DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().clear());
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
            let mut deck_rect = Rect::NOTHING;
            let mut output = ctx.run_ui(input.clone(), |ui| {
                let d = Deck { phase, fps: 60, frames, countdown_secs: 3, chords: &chords };
                let deck = ui_kit::transport_deck(ui, Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0)), Id::new("deck-center-line"), false, |ui| {
                    deck_contents(ui, d, &mut |_| {});
                });
                deck_rect = deck.response.rect;
            });
            output.textures_delta.clear();
            DECK_ITEM_RECTS.with(|slot| slot.borrow_mut().clear());
            let mut output = ctx.run_ui(input, |ui| {
                let d = Deck { phase, fps: 60, frames, countdown_secs: 3, chords: &chords };
                let deck = ui_kit::transport_deck(ui, Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0)), Id::new("deck-center-line"), false, |ui| {
                    deck_contents(ui, d, &mut |_| {});
                });
                deck_rect = deck.response.rect;
            });
            output.textures_delta.clear();
            let items = DECK_ITEM_RECTS.with(|slot| slot.borrow().clone());
            assert!(!items.is_empty(), "the deck drew its items");
            for rect in items {
                assert!((rect.center().y - deck_rect.center().y).abs() <= 1.0, "the deck item {rect:?} must sit on the deck's center line {deck_rect:?}");
            }
        }
    }

    /// The capsule's fill covers its whole rect with the dot (or stop square)
    /// and the label inside, both vertically centered, while idle and while a
    /// take runs.
    #[test]
    fn the_capsule_rect_contains_the_dot_and_the_label() {
        let ctx = egui::Context::default();
        ui_kit::apply(&ctx);
        for recording in [false, true] {
            LAST_CAPSULE_RECTS.with(|slot| *slot.borrow_mut() = None);
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 100.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| {
                record_capsule(ui, recording, "Start recording");
            });
            output.textures_delta.clear();
            let (rect, glyph, label) = LAST_CAPSULE_RECTS.with(|slot| (*slot.borrow()).expect("the capsule recorded its rects"));
            assert!(rect.contains_rect(glyph), "the glyph {glyph:?} must sit inside the capsule {rect:?}");
            assert!(rect.contains_rect(label), "the label {label:?} must sit inside the capsule {rect:?}");
            assert!((glyph.center().y - rect.center().y).abs() < 0.5, "the glyph must be vertically centered on the rect");
            assert!((label.center().y - rect.center().y).abs() < 0.5, "the label must be vertically centered on the rect");
            assert_eq!(rect.height(), size::TRANSPORT_BUTTON, "the capsule is the deck button height");
        }
    }
}
