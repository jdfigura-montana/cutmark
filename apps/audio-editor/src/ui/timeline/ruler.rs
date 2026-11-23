//! ui::timeline::ruler: the timecode ruler with markers.

use crate::ui::timeline::metrics;
use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::snap;
use crate::model::marker::Marker;

use ui_kit::MarkerKind;
use ui_kit::widgets;

use super::{Geom, tick_step};

/// Key under which the marker interaction state is carried between frames.
const MARKER_STATE_KEY: &str = "timeline_marker_state";

/// Opacity of the faint line under an unselected marker.
const MARKER_LINE_ALPHA: f32 = 0.35;

/// Extra grab area on each side of a marker triangle, in points.
const MARKER_GRAB_PX: f32 = 5.0;

/// A primary press on a marker triangle, kept until release.
#[derive(Clone, Copy)]
struct MarkerPress {
    /// The pressed marker.
    id: Uuid,
    /// Pointer position at press time.
    pos: egui::Pos2,
}

/// A marker drag in progress: the marker and its last good position.
#[derive(Clone, Copy)]
struct MarkerDrag {
    /// The dragged marker.
    id: Uuid,
    /// Last position the preview accepted, in samples.
    last_good: u64,
}

/// A marker context menu shown at a fixed screen position.
#[derive(Clone, Copy)]
struct MarkerMenu {
    /// The right-clicked marker.
    id: Uuid,
    /// Screen position of the menu's top left corner.
    pos: egui::Pos2,
}

/// Marker interaction state carried between frames.
#[derive(Clone, Default)]
struct MarkerState {
    /// Press on a marker triangle, when the primary button is down.
    press: Option<MarkerPress>,
    /// Active marker drag, when any.
    drag: Option<MarkerDrag>,
    /// Open marker context menu, if any.
    menu: Option<MarkerMenu>,
}

/// Reads the marker interaction state from egui's temp storage.
fn marker_state(ctx: &egui::Context) -> MarkerState {
    ctx.data(|d| d.get_temp(egui::Id::new(MARKER_STATE_KEY)).unwrap_or_default())
}

/// Writes the marker interaction state back to egui's temp storage.
fn save_marker_state(ctx: &egui::Context, state: &MarkerState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(MARKER_STATE_KEY), state.clone()));
}

/// The marker under the pointer: the triangle plus `MARKER_GRAB_PX` on each
/// side, and the ruler height only. Markers cannot overlap, so a scan suffices.
fn marker_hit(markers: &[Marker], g: &Geom, ruler_rect: egui::Rect, p: egui::Pos2) -> Option<Uuid> {
    if !ruler_rect.contains(p) {
        return None;
    }
    markers
        .iter()
        .find(|m| {
            let x = g.sample_to_x(m.position);
            (p.x - x).abs() <= metrics::MARKER_TRI_W + MARKER_GRAB_PX
        })
        .map(|m| m.id)
}

/// Handles all marker mouse input on the ruler.
///
/// A click selects the marker; a drag previews `MoveMarker` at the snapped
/// pointer position (Alt bypasses snapping), keeping the last good position
/// when the preview fails; Esc cancels; a right-click opens the marker menu.
///
/// Returns true when a marker interaction occupies the pointer this frame, so
/// the plain ruler click (playhead) must not also fire.
pub fn handle_markers(ui: &egui::Ui, editor: &mut Editor, g: &Geom, ruler_rect: egui::Rect) -> bool {
    let mut state = marker_state(ui.ctx());
    let (pressed, down, released, secondary, pos, alt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_released(),
            i.pointer.secondary_pressed(),
            i.pointer.latest_pos(),
            i.modifiers.alt,
        )
    });
    let mut blocked = state.drag.is_some() || state.menu.is_some();

    // Esc during a drag cancels; the key is consumed so the global Esc
    // handler does not also clear the selection.
    if state.drag.is_some() && crate::shortcuts::pressed(ui.ctx(), "cancel_gesture") {
        editor.cancel_gesture();
        state.drag = None;
        state.press = None;
        save_marker_state(ui.ctx(), &state);
        return true;
    }

    if released {
        if state.press.take().is_some() {
            blocked = true;
            if state.drag.take().is_some() {
                editor.end_gesture();
            }
            // A click that never became a drag selected the marker at press time.
        }
    } else if secondary
        && let Some(p) = pos
        && let Some(id) = marker_hit(&editor.project.markers, g, ruler_rect, p)
    {
        state.menu = Some(MarkerMenu { id, pos: p });
        blocked = true;
    } else if down && let (Some(press), Some(p)) = (state.press, pos) {
        let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
        if dist >= metrics::DRAG_START_PX
            && state.drag.is_none()
            && let Some(base) = editor.project.markers.iter().find(|m| m.id == press.id).map(|m| m.position)
        {
            editor.begin_gesture();
            state.drag = Some(MarkerDrag { id: press.id, last_good: base });
        }
        if let Some(drag) = state.drag {
            let raw = g.x_to_sample(p.x);
            let target = if editor.view.snap && !alt {
                let candidates = editor.snap_candidates(&[], Some(drag.id));
                snap::snap(raw, &candidates, g.spp, metrics::SNAP_PX).unwrap_or(raw)
            } else {
                raw
            };
            if editor.preview(Action::MoveMarker { id: drag.id, pos: target }).is_ok() {
                state.drag = Some(MarkerDrag { id: drag.id, last_good: target });
            } else {
                // Another marker sits there: keep the last good position.
                let last = drag.last_good;
                let _ = editor.preview(Action::MoveMarker { id: drag.id, pos: last });
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    } else if pressed
        && let Some(p) = pos
        && let Some(id) = marker_hit(&editor.project.markers, g, ruler_rect, p)
    {
        editor.select_marker(Some(id));
        state.press = Some(MarkerPress { id, pos: p });
        blocked = true;
    } else if !down
        && let Some(p) = pos
        && marker_hit(&editor.project.markers, g, ruler_rect, p).is_some()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    save_marker_state(ui.ctx(), &state);
    blocked
}

/// Key under which the scrub drag state is carried between frames.
const SCRUB_STATE_KEY: &str = "timeline_scrub_state";

/// Distance from a lane edge (in points) inside which the view auto-scrolls
/// while scrubbing.
const SCRUB_EDGE_PX: f32 = 20.0;

/// Auto-scroll speed at the very edge while scrubbing, in points per second.
const SCRUB_EDGE_SPEED_PX: f32 = 600.0;

/// A scrub drag in progress: whether the pointer is down on the ruler and the
/// last target sample sent to the engine.
#[derive(Clone, Copy, Default)]
struct ScrubState {
    /// Scrubbing is active (pointer pressed on the ruler).
    active: bool,
    /// Last sample sent with `scrub_to`.
    last: u64,
}

/// Reads the scrub state from egui's temp storage.
fn scrub_state(ctx: &egui::Context) -> ScrubState {
    ctx.data(|d| d.get_temp(egui::Id::new(SCRUB_STATE_KEY)).unwrap_or_default())
}

/// True while the user drags on the ruler (an audible scrub is active).
pub fn is_scrubbing(ctx: &egui::Context) -> bool {
    scrub_state(ctx).active
}

/// Writes the scrub state back to egui's temp storage.
fn save_scrub_state(ctx: &egui::Context, state: ScrubState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(SCRUB_STATE_KEY), state));
}

/// Handles audible scrubbing on the ruler (and on the playhead handle, which
/// is just a ruler press).
///
/// A press on the ruler anywhere except a marker triangle starts a scrub at
/// the pointer's sample; every frame while the button is down the target
/// follows the pointer (no snapping); release ends the scrub with the playhead
/// at the last target. When the pointer comes within [`SCRUB_EDGE_PX`] of a
/// lane edge the view scrolls in that direction, faster the closer it gets.
/// Requests a repaint every frame while scrubbing.
pub fn handle_scrub(ui: &egui::Ui, editor: &mut Editor, ruler_rect: egui::Rect, markers_taken: bool) {
    let mut scrub = scrub_state(ui.ctx());
    let (pressed, down, released, pos) =
        ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.any_released(), i.pointer.latest_pos()));
    let g = Geom { lane_left: ruler_rect.left(), lane_width: ruler_rect.width(), spp: editor.view.samples_per_pixel, scroll_x: editor.view.scroll_x };

    if released {
        if scrub.active {
            editor.scrub_end();
            scrub = ScrubState::default();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        }
    } else if scrub.active && down {
        // Auto-scroll when the pointer nears a lane edge; the speed grows the
        // closer it gets (quadratic ramp from 0 at 20 px to full at the edge).
        if let Some(p) = pos {
            let dist_l = p.x - ruler_rect.left();
            let dist_r = ruler_rect.right() - p.x;
            let (dir, t) = if dist_l < SCRUB_EDGE_PX {
                (-1.0, 1.0 - (dist_l / SCRUB_EDGE_PX).max(0.0))
            } else if dist_r < SCRUB_EDGE_PX {
                (1.0, 1.0 - (dist_r / SCRUB_EDGE_PX).max(0.0))
            } else {
                (0.0, 0.0)
            };
            if dir != 0.0 {
                let dt = ui.input(|i| if i.stable_dt > 0.0 { i.stable_dt } else { 1.0 / 60.0 });
                let spp = editor.view.samples_per_pixel;
                let delta = (SCRUB_EDGE_SPEED_PX * t * t * dt * spp as f32) as u64;
                let new_x = if dir < 0.0 { editor.view.scroll_x.saturating_sub(delta) } else { editor.view.scroll_x.saturating_add(delta) };
                editor.set_view_x(spp, super::clamp_scroll_x(editor, spp, ruler_rect.width(), new_x));
            }
            // The target follows the pointer, so a scroll also scrubs on.
            let target = g.x_to_sample(p.x);
            if target != scrub.last {
                editor.scrub_to(target);
                scrub.last = target;
            }
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    } else if pressed
        && !markers_taken
        && !ui.ctx().egui_wants_keyboard_input()
        && let Some(p) = pos
        && ruler_rect.contains(p)
        && marker_hit(&editor.project.markers, &g, ruler_rect, p).is_none()
    {
        let pos_sample = g.x_to_sample(p.x);
        editor.scrub_start(pos_sample);
        scrub = ScrubState { active: true, last: pos_sample };
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        ui.ctx().request_repaint();
    } else if !down
        && let Some(p) = pos
        && ruler_rect.contains(p)
        && marker_hit(&editor.project.markers, &g, ruler_rect, p).is_none()
    {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    save_scrub_state(ui.ctx(), scrub);
}

/// Draws the marker context menu and runs its action.
///
/// Closes on Esc, on a click outside the menu, or after the action.
pub fn draw_marker_menu(ui: &egui::Ui, editor: &mut Editor) {
    let mut state = marker_state(ui.ctx());
    let Some(menu) = state.menu else {
        return;
    };
    let (clicked, pointer) = ui.input(|i| (i.pointer.any_click(), i.pointer.latest_pos()));
    if crate::shortcuts::peek(ui.ctx(), "close_menu") {
        state.menu = None;
        save_marker_state(ui.ctx(), &state);
        return;
    }
    // The marker may have been deleted another way while the menu was open.
    if !editor.project.markers.iter().any(|m| m.id == menu.id) {
        state.menu = None;
        save_marker_state(ui.ctx(), &state);
        return;
    }

    let mut close = false;
    let response = egui::Area::new(egui::Id::new("marker_context_menu")).order(egui::Order::Foreground).fixed_pos(menu.pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(metrics::CLIP_MENU_MIN_W);
            if ui.button("Delete Marker").clicked() {
                let _ = editor.apply(Action::DeleteMarkers { ids: vec![menu.id] });
                close = true;
            }
        });
    });
    let menu_rect = response.response.rect;
    if clicked && pointer.is_some_and(|p| !menu_rect.contains(p)) {
        close = true;
    }
    if close {
        state.menu = None;
    }
    save_marker_state(ui.ctx(), &state);
}

/// The ui-kit marker kind of every marker. The model keeps position-only
/// markers the user adds, so they are all plain user markers.
const MARKER_KIND: MarkerKind = MarkerKind::User;

/// Draws the ruler strip through ui-kit's `paint_ruler`: ticks and time
/// labels at the adaptive step, and the markers as dots in the strip under the
/// labels. Each marker also gets a faint vertical line down through the lane
/// area, and the selected one a full line from the ruler top.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    ruler_rect: egui::Rect,
    lane_rect: egui::Rect,
    g: &Geom,
    rate: u32,
    project_length: u64,
    markers: &[Marker],
    selected: Option<Uuid>,
) {
    // Major ticks with timecode labels, plus minor ticks between them.
    // The label step is the smallest step whose labels are at least 60 px
    // apart; the labels adapt their format to that step.
    let step = tick_step(g.spp, rate, metrics::RULER_MIN_TICK_PX);
    let (start, end) = g.visible_range();
    let first = start / step * step;
    let minor = step / 5;
    let minor_px = minor as f64 / g.spp;
    let show_minor = minor_px >= 8.0;
    let over_hour = project_length / u64::from(rate) > 3600;

    let mut ticks: Vec<(f32, Option<String>)> = Vec::new();
    let mut m = first;
    while m <= end {
        let x = g.sample_to_x(m);
        if x >= ruler_rect.left() - 1.0 {
            if x > ruler_rect.right() + 1.0 {
                break;
            }
            ticks.push((x, Some(ruler_label(step, m, rate, over_hour))));
            if show_minor {
                for k in 1..5u64 {
                    let sub = m.checked_add(minor * k);
                    let Some(s) = sub else { break };
                    let mx = g.sample_to_x(s);
                    if mx > ruler_rect.right() {
                        break;
                    }
                    ticks.push((mx, None));
                }
            }
        }
        m += step;
    }

    // Markers: the lines come first so the dots paint over them.
    let line_color = widgets::marker_color(MARKER_KIND);
    let mut dots: Vec<(f32, MarkerKind)> = Vec::new();
    for marker in markers {
        let x = g.sample_to_x(marker.position);
        if x < g.lane_left - 1.0 || x > ruler_rect.right() + 1.0 {
            continue;
        }
        if Some(marker.id) == selected {
            painter.line_segment([egui::pos2(x, ruler_rect.top()), egui::pos2(x, lane_rect.bottom())], egui::Stroke::new(metrics::HAIRLINE, line_color));
        } else {
            painter.line_segment(
                [egui::pos2(x, ruler_rect.bottom()), egui::pos2(x, lane_rect.bottom())],
                egui::Stroke::new(metrics::HAIRLINE, line_color.gamma_multiply(MARKER_LINE_ALPHA)),
            );
        }
        dots.push((x, MARKER_KIND));
    }
    ui_kit::paint_ruler(painter, ruler_rect, &ticks, &dots);
}

/// Adaptive ruler label for a tick at `samples`, formatted for `step_secs`.
///
/// Steps of 1 s or more read "m:ss" (or "h:mm:ss" when `over_hour`), 100 ms
/// steps read "m:ss.d" (or "h:mm:ss.d"), anything finer reads "m:ss.mmm"
/// (or "h:mm:ss.mmm"). The "h:" prefix appears only when `over_hour`.
fn ruler_label(step: u64, samples: u64, rate: u32, over_hour: bool) -> String {
    let step_secs = step as f64 / f64::from(rate);
    let total_ms = samples * 1000 / u64::from(rate);
    if over_hour {
        let hh = total_ms / 3_600_000;
        let rem = total_ms % 3_600_000;
        let mm = rem / 60_000;
        let ss = (rem / 1000) % 60;
        let ms = rem % 1000;
        if step_secs >= 1.0 {
            format!("{hh}:{mm:02}:{ss:02}")
        } else if (step_secs - 0.1).abs() < f64::EPSILON {
            format!("{hh}:{mm:02}:{ss:02}.{}", ms / 100)
        } else {
            format!("{hh}:{mm:02}:{ss:02}.{ms:03}")
        }
    } else {
        let mm = (total_ms / 60_000) % 60;
        let ss = (total_ms / 1000) % 60;
        let ms = total_ms % 1000;
        if step_secs >= 1.0 {
            format!("{mm}:{ss:02}")
        } else if (step_secs - 0.1).abs() < f64::EPSILON {
            format!("{mm}:{ss:02}.{}", ms / 100)
        } else {
            format!("{mm}:{ss:02}.{ms:03}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_labels_adapt_to_the_step() {
        // 1 s step at 48 kHz: m:ss.
        assert_eq!(ruler_label(48_000, 90_000, 48_000, false), "0:01");
        // 100 ms step: m:ss.d.
        assert_eq!(ruler_label(4_800, 90_240, 48_000, false), "0:01.8");
        // Sub-100 ms step: m:ss.mmm.
        assert_eq!(ruler_label(480, 90_240, 48_000, false), "0:01.880");
        // 1 h 2 min 3.5 s with the hour prefix: h:mm:ss.
        assert_eq!(ruler_label(48_000, 3_723_500, 1_000, true), "1:02:03");
    }

    #[test]
    fn ruler_labels_without_hour_prefix_wrap_minutes() {
        // 2 min 5 s, no hour prefix: minutes are not wrapped past 59.
        assert_eq!(ruler_label(48_000, 6_000_000, 48_000, false), "2:05");
    }
}
