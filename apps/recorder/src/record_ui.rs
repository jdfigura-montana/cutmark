//! The Record mode's Live panel (spec "Record mode") and the recording
//! pill's label. The take controls are in the stage's bottom bar
//! ([`crate::bar_ui`]).
//!
//! See the recorder spec: docs/suite/recorder-phase-13-ui.md.

use egui::{Align, Key, KeyboardShortcut, Layout, Modifiers, RichText, Ui, vec2};
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{inspector_group, inspector_row};

use crate::config::AudioSettings;
use crate::engine::controller::{ControllerState, Phase};
use crate::engine::session::Summary;
use crate::marks_ui;
use crate::tracks_ui::{self, LiveTrackStats};

/// Left padding of a panel header's title, the same inset the app's other
/// headers use.
const HEADER_PAD: f32 = 6.0;
/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// The recording pill's text: REC while a take runs, PAUSED while it is paused.
pub(crate) fn pill_label(paused: bool) -> &'static str {
    if paused { "PAUSED" } else { "REC" }
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

    /// The recording pill reads REC while recording and PAUSED while paused.
    #[test]
    fn the_rec_pill_reads_paused_while_paused() {
        assert_eq!(pill_label(false), "REC");
        assert_eq!(pill_label(true), "PAUSED");
    }
}
