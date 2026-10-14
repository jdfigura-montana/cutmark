//! ui::timeline::ruler: the timecode ruler with markers.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::snap;
use crate::model::marker::Marker;
use crate::util::time::samples_to_timecode;

use crate::ui::theme::{THEME, Theme};

use super::{Geom, tick_step};

/// Key under which the marker interaction state is carried between frames.
const MARKER_STATE_KEY: &str = "timeline_marker_state";

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
    ctx.data(|d| {
        d.get_temp(egui::Id::new(MARKER_STATE_KEY))
            .unwrap_or_default()
    })
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
            (p.x - x).abs() <= THEME.marker_tri_w + MARKER_GRAB_PX
        })
        .map(|m| m.id)
}

/// Lightens a color towards white by `t` (0..1), for the selected marker.
fn brighten(color: egui::Color32, t: f32) -> egui::Color32 {
    let mix = |c: u8| c + ((255 - i32::from(c)) as f32 * t) as u8;
    egui::Color32::from_rgb(mix(color.r()), mix(color.g()), mix(color.b()))
}

/// Handles all marker mouse input on the ruler.
///
/// A click selects the marker; a drag previews `MoveMarker` at the snapped
/// pointer position (Alt bypasses snapping), keeping the last good position
/// when the preview fails; Esc cancels; a right-click opens the marker menu.
///
/// Returns true when a marker interaction occupies the pointer this frame, so
/// the plain ruler click (playhead) must not also fire.
pub fn handle_markers(
    ui: &egui::Ui,
    editor: &mut Editor,
    g: &Geom,
    ruler_rect: egui::Rect,
) -> bool {
    let mut state = marker_state(ui.ctx());
    let (pressed, down, released, secondary, pos, alt, esc) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_released(),
            i.pointer.secondary_pressed(),
            i.pointer.latest_pos(),
            i.modifiers.alt,
            i.key_pressed(egui::Key::Escape),
        )
    });
    let mut blocked = state.drag.is_some() || state.menu.is_some();

    // Esc during a drag cancels; the key is consumed so the global Esc
    // handler does not also clear the selection.
    if esc && state.drag.is_some() {
        editor.cancel_gesture();
        state.drag = None;
        state.press = None;
        save_marker_state(ui.ctx(), &state);
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
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
        if dist >= THEME.drag_start_px
            && state.drag.is_none()
            && let Some(base) = editor
                .project
                .markers
                .iter()
                .find(|m| m.id == press.id)
                .map(|m| m.position)
        {
            editor.begin_gesture();
            state.drag = Some(MarkerDrag {
                id: press.id,
                last_good: base,
            });
        }
        if let Some(drag) = state.drag {
            let raw = g.x_to_sample(p.x);
            let target = if editor.view.snap && !alt {
                let candidates = editor.snap_candidates(&[], Some(drag.id));
                snap::snap(raw, &candidates, g.spp, THEME.snap_px).unwrap_or(raw)
            } else {
                raw
            };
            if editor
                .preview(Action::MoveMarker {
                    id: drag.id,
                    pos: target,
                })
                .is_ok()
            {
                state.drag = Some(MarkerDrag {
                    id: drag.id,
                    last_good: target,
                });
            } else {
                // Another marker sits there: keep the last good position.
                let last = drag.last_good;
                let _ = editor.preview(Action::MoveMarker {
                    id: drag.id,
                    pos: last,
                });
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

/// Draws the marker context menu and runs its action.
///
/// Closes on Esc, on a click outside the menu, or after the action.
pub fn draw_marker_menu(ui: &egui::Ui, editor: &mut Editor) {
    let mut state = marker_state(ui.ctx());
    let Some(menu) = state.menu else {
        return;
    };
    let (esc, clicked, pointer) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.pointer.any_click(),
            i.pointer.latest_pos(),
        )
    });
    if esc {
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
    let response = egui::Area::new(egui::Id::new("marker_context_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(menu.pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(THEME.clip_menu_min_w);
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

/// Draws the ruler strip, its ticks and labels, marker triangles with faint
/// lines down through the lane area.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    painter: &egui::Painter,
    ruler_rect: egui::Rect,
    lane_rect: egui::Rect,
    g: &Geom,
    rate: u32,
    markers: &[Marker],
    selected: Option<Uuid>,
    text: egui::Color32,
    stroke: egui::Stroke,
) {
    // Major ticks with timecode labels, plus minor ticks between them.
    let step = tick_step(g.spp, rate, THEME.ruler_min_tick_px);
    let (start, end) = g.visible_range();
    let first = start / step * step;
    let minor = step / 5;
    let minor_px = minor as f64 / g.spp;
    let show_minor = minor_px >= 8.0;

    let mut m = first;
    while m <= end {
        let x = g.sample_to_x(m);
        if x >= ruler_rect.left() - 1.0 {
            if x > ruler_rect.right() + 1.0 {
                break;
            }
            painter.line_segment(
                [
                    egui::pos2(x, ruler_rect.bottom() - THEME.ruler_tick_h),
                    egui::pos2(x, ruler_rect.bottom()),
                ],
                stroke,
            );
            let tc = samples_to_timecode(m, rate);
            painter.text(
                egui::pos2(
                    x + THEME.ruler_label_pad_x,
                    ruler_rect.top() + THEME.ruler_label_pad_y,
                ),
                egui::Align2::LEFT_TOP,
                tc,
                Theme::mono(THEME.ruler_label_size),
                text,
            );
            if show_minor {
                for k in 1..5u64 {
                    let sub = m.checked_add(minor * k);
                    let Some(s) = sub else { break };
                    let mx = g.sample_to_x(s);
                    if mx > ruler_rect.right() {
                        break;
                    }
                    painter.line_segment(
                        [
                            egui::pos2(mx, ruler_rect.bottom() - THEME.ruler_minor_tick_h),
                            egui::pos2(mx, ruler_rect.bottom()),
                        ],
                        stroke,
                    );
                }
            }
        }
        m += step;
    }

    // Markers: a small downward triangle on the ruler and a faint vertical
    // line down through the lane area. The selected marker draws brighter.
    for marker in markers {
        let x = g.sample_to_x(marker.position);
        if x < g.lane_left - 1.0 || x > ruler_rect.right() + 1.0 {
            continue;
        }
        let is_selected = Some(marker.id) == selected;
        let marker_color = if is_selected {
            brighten(THEME.marker_color, 0.55)
        } else {
            THEME.marker_color
        };
        let line_color = if is_selected {
            brighten(THEME.marker_color, 0.45)
        } else {
            THEME.marker_color
        };
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(
                    x - THEME.marker_tri_w,
                    ruler_rect.bottom() - THEME.marker_tri_h,
                ),
                egui::pos2(
                    x + THEME.marker_tri_w,
                    ruler_rect.bottom() - THEME.marker_tri_h,
                ),
                egui::pos2(x, ruler_rect.bottom()),
            ],
            marker_color,
            egui::Stroke::NONE,
        ));
        painter.line_segment(
            [
                egui::pos2(x, ruler_rect.bottom()),
                egui::pos2(x, lane_rect.bottom()),
            ],
            egui::Stroke::new(
                THEME.hairline,
                egui::Color32::from_rgba_premultiplied(
                    line_color.r() / 4,
                    line_color.g() / 4,
                    line_color.b() / 4,
                    line_color.a() / 4,
                ),
            ),
        );
    }
}
