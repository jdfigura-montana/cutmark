//! The status bar at the bottom of the window (docs/suite/Recorder-E3.md, section 4): the
//! state and the free disk space on the left, the camera warning, the replay buffer and the
//! Control API clients on the right, and an empty center under the stage's notch button.

use egui::text::TextWrapping;
use egui::{Color32, Galley, Pos2, Rangef, Rect, Sense, Ui, Vec2};
use std::sync::Arc;
use ui_kit::tokens::{color, space};

use crate::data_text;
use crate::engine::controller::{ControllerState, Phase};
use crate::engine::stats::GIB;

/// Diameter of the state dot.
const DOT_DIAMETER: f32 = 6.0;
/// Space between two items.
const ITEM_SPACING: f32 = 20.0;
/// The narrowest a shortened warning is worth drawing.
const MIN_WARNING_WIDTH: f32 = 80.0;
/// The camera warning's item; hovering it shows the full reason.
const CAMERA_WARNING: &str = "Camera falls back to HEVC on this GPU";

/// Draws the status bar. While idle the first item reads Ready, or Blocked when starting is
/// blocked (the reason stays in the banner strip; spec "Mapping"). `camera_note` is why the
/// camera file's codec falls back, when it does. The stage's notch button reaches into the
/// bar around `notch_x`, so no text is drawn there: the side panels can differ in width, so
/// the empty center follows the stage and not the window.
pub fn show(ui: &mut Ui, state: &ControllerState, api_clients: Option<usize>, blocked: bool, camera_note: Option<&str>, notch_x: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ui_kit::STATUS_BAR_GAP_HEIGHT), Sense::hover());
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let text = ui_kit::Theme::get(ui.ctx()).neutrals.text_tertiary;
    let gap = Rangef::new(notch_x - ui_kit::STATUS_BAR_CENTER_GAP / 2.0, notch_x + ui_kit::STATUS_BAR_CENTER_GAP / 2.0);
    let left_end = gap.min.min(rect.right() - space::SCALE_4);
    let place = |x: f32, galley: Arc<Galley>| painter.galley(Pos2::new(x, rect.center().y - galley.size().y / 2.0), galley, text);

    // The state dot and its label; a countdown is a recording that has not begun yet.
    let (dot, label) = match state.phase {
        Phase::Idle if blocked => (color::SEM_RECORD, "Blocked"),
        Phase::Idle => (color::SEM_SUCCESS, "Ready"),
        Phase::Countdown(_) => (color::SEM_RECORD, "Starting"),
        Phase::Recording => (color::SEM_RECORD, "Recording"),
        Phase::Paused => (color::SEM_METER_WARN, "Paused"),
    };
    // The left items, from the left edge. One that would reach the empty center is left out.
    let mut x = rect.left() + space::SCALE_4;
    painter.circle_filled(Pos2::new(x + DOT_DIAMETER / 2.0, rect.center().y), DOT_DIAMETER / 2.0, dot);
    x += DOT_DIAMETER + space::SCALE_8;
    // The free disk space, while idle and while a take writes.
    let free = state.take_stats.as_ref().and_then(|stats| stats.disk_free_bytes).or(state.idle_free_bytes);
    let free = format_free(free);
    let free_value = free.strip_suffix(" free").unwrap_or_default();
    for job in [data_text::with_data(label, &[], text), data_text::with_data(&free, &[free_value], text)] {
        let galley = ui.painter().layout_job(job);
        let width = galley.size().x;
        if x + width > left_end {
            break;
        }
        place(x, galley);
        x += width + ITEM_SPACING;
    }

    // The right items read left to right and end at the right edge, so they are placed from
    // the Control API clients inwards. The replay item is here because a narrow left side has
    // no room for it. The warning is shortened to the room that is left.
    let right_start = gap.max.max(rect.left() + space::SCALE_4);
    let mut right = rect.right() - space::SCALE_4;
    if let Some(clients) = api_clients {
        let count = clients.to_string();
        let galley = ui.painter().layout_job(data_text::with_data(&clients_label(clients), &[&count], text));
        let start = right - galley.size().x;
        if start >= right_start {
            place(start, galley);
            right = start - ITEM_SPACING;
        }
    }
    if state.replay_on {
        let length = format!("{} s", state.replay_length_s);
        let galley = ui.painter().layout_job(data_text::with_data(&format!("Replay {length}"), &[&length], text));
        let start = right - galley.size().x;
        if start >= right_start {
            place(start, galley);
            right = start - ITEM_SPACING;
        }
    }
    if let Some(note) = camera_note {
        let room = right - right_start;
        if room >= MIN_WARNING_WIDTH {
            let galley = layout_within(ui, CAMERA_WARNING, text, room);
            let size = galley.size();
            let area = Rect::from_min_size(Pos2::new(right - size.x, rect.center().y - size.y / 2.0), size);
            painter.galley(area.min, galley, text);
            ui.interact(area, ui.id().with("camera-warning"), Sense::hover()).on_hover_text(note);
        }
    }
}

/// One caption item on one line at most `max_width` wide, ending in an ellipsis when cut.
fn layout_within(ui: &Ui, item: &str, color: Color32, max_width: f32) -> Arc<Galley> {
    let mut job = data_text::with_data(item, &[], color);
    job.wrap = TextWrapping::truncate_at_width(max_width);
    ui.painter().layout_job(job)
}

/// The Control API item.
fn clients_label(clients: usize) -> String {
    match clients {
        0 => "Control API: no clients".to_owned(),
        1 => "Control API: 1 client".to_owned(),
        more => format!("Control API: {more} clients"),
    }
}

/// The free space item, in GB, or a dash when it is unknown.
fn format_free(free: Option<u64>) -> String {
    free.map_or_else(|| "-- free".to_owned(), |bytes| format!("{:.1} GB free", bytes as f64 / GIB as f64))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use egui::{Shape, vec2};
    use egui_kittest::Harness;

    use super::*;
    use crate::app::stage_center_x;
    use crate::config::{self, Mode, PanelWidths};

    /// Every text a status bar `width` wide paints, with the notch at `notch_x`.
    fn painted(width: f32, phase: Phase, notch_x: f32) -> Vec<(String, Rect)> {
        painted_state(width, move || ControllerState { phase, idle_free_bytes: Some(9999 * GIB), ..ControllerState::default() }, notch_x)
    }

    /// [`painted`] for any controller state.
    fn painted_state(width: f32, state: impl Fn() -> ControllerState + 'static, notch_x: f32) -> Vec<(String, Rect)> {
        let first_pass = Rc::new(Cell::new(true));
        let mut harness = Harness::builder().with_size(vec2(width, 60.0)).build_ui(move |ui| {
            // The fonts a style asks for are only there from the next pass on.
            if first_pass.replace(false) {
                ui_kit::apply(ui.ctx());
                return;
            }
            let window = ui.ctx().content_rect();
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(window));
            bar.set_clip_rect(window);
            show(&mut bar, &state(), Some(12), false, Some("Camera ISO: HEVC (AV1 1080p not supported in MP4 on this GPU)"), notch_x);
        });
        harness.run();
        let texts = harness.output().shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) => Some((text.galley.text().to_owned(), Rect::from_min_size(text.pos, text.galley.size()))),
            _ => None,
        });
        texts.collect()
    }

    /// In the narrowest window, in every mode, at the default panel widths and the widest, no
    /// status text reaches into the 120 center under the notch, overlaps another or leaves
    /// the window, and the state, the free space and the clients are all shown.
    #[test]
    fn no_status_text_reaches_the_center_in_the_narrowest_window() {
        let width = config::MIN_WINDOW_WIDTH;
        let window = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0));
        let widest = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MAX };
        let narrow_left = PanelWidths { left: config::LEFT_PANEL_MIN, right: config::RIGHT_PANEL_MAX };
        let narrow_right = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MIN };
        for mode in Mode::ALL {
            for widths in [PanelWidths::default(), widest, narrow_left, narrow_right] {
                let notch_x = stage_center_x(window, mode, widths);
                for phase in [Phase::Idle, Phase::Countdown(3), Phase::Recording, Phase::Paused] {
                    let texts = painted(width, phase, notch_x);
                    let half = ui_kit::STATUS_BAR_CENTER_GAP / 2.0;
                    for (index, (text, rect)) in texts.iter().enumerate() {
                        assert!(
                            rect.right() <= notch_x - half + 0.5 || rect.left() >= notch_x + half - 0.5,
                            "{text:?} is under the notch at {notch_x}: {rect:?}"
                        );
                        assert!(rect.left() >= 0.0 && rect.right() <= width, "{text:?} leaves the window: {rect:?}");
                        for (other, other_rect) in &texts[index + 1..] {
                            assert!(!rect.intersects(*other_rect), "{text:?} overlaps {other:?}");
                        }
                    }
                    for wanted in ["9999.0 GB free", "Control API: 12 clients"] {
                        assert!(texts.iter().any(|(text, _)| text == wanted), "{wanted} is shown in {mode:?} with {widths:?}: {texts:?}");
                    }
                    assert_eq!(texts.len(), 4, "the state, the free space, the warning and the clients are separate items: {texts:?}");
                    assert!(texts.iter().all(|(text, _)| !text.contains('\u{b7}')));
                }
            }
        }
    }

    /// The replay item is there only while the buffer is on, with the buffer's length. In the
    /// narrowest window it stays clear of the notch and of every other text.
    #[test]
    fn the_status_bar_shows_the_replay_length_while_the_buffer_is_on() {
        let width = config::MIN_WINDOW_WIDTH;
        let window = Rect::from_min_size(Pos2::ZERO, vec2(width, 600.0));
        let widest = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MAX };
        let narrow_left = PanelWidths { left: config::LEFT_PANEL_MIN, right: config::RIGHT_PANEL_MAX };
        let narrow_right = PanelWidths { left: config::LEFT_PANEL_MAX, right: config::RIGHT_PANEL_MIN };
        for mode in Mode::ALL {
            for widths in [PanelWidths::default(), widest, narrow_left, narrow_right] {
                let notch_x = stage_center_x(window, mode, widths);
                for phase in [Phase::Idle, Phase::Countdown(3), Phase::Recording, Phase::Paused] {
                    let texts = |replay_on| {
                        let state =
                            move || ControllerState { phase, replay_on, replay_length_s: 60, idle_free_bytes: Some(9999 * GIB), ..ControllerState::default() };
                        painted_state(width, state, notch_x)
                    };
                    assert!(texts(false).iter().all(|(text, _)| !text.starts_with("Replay")), "hidden while off");
                    let on = texts(true);
                    for wanted in ["Replay 60 s", "9999.0 GB free", "Control API: 12 clients"] {
                        assert!(on.iter().any(|(text, _)| text == wanted), "{wanted} is shown in {mode:?} {phase:?} with {widths:?}: {on:?}");
                    }
                    let half = ui_kit::STATUS_BAR_CENTER_GAP / 2.0;
                    for (index, (text, rect)) in on.iter().enumerate() {
                        assert!(
                            rect.right() <= notch_x - half + 0.5 || rect.left() >= notch_x + half - 0.5,
                            "{text:?} is under the notch at {notch_x}: {rect:?}"
                        );
                        assert!(rect.left() >= 0.0 && rect.right() <= width, "{text:?} leaves the window: {rect:?}");
                        assert!(on[index + 1..].iter().all(|(_, other)| !rect.intersects(*other)), "{text:?} overlaps another text: {on:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn free_space_formats_in_gb_or_dashes_when_unknown() {
        assert_eq!(format_free(None), "-- free");
        assert_eq!(format_free(Some(5 * GIB)), "5.0 GB free");
    }

    #[test]
    fn the_control_api_item_counts_its_clients() {
        assert_eq!(clients_label(0), "Control API: no clients");
        assert_eq!(clients_label(1), "Control API: 1 client");
        assert_eq!(clients_label(2), "Control API: 2 clients");
    }
}
