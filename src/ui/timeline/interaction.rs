//! ui::timeline::interaction: mouse actions over the timeline, including bin
//! drags and range selection.
//! ui::timeline::interaction: mouse actions over the timeline, including bin drags.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;

use super::Geom;

/// The drag payload set by the media bin: the dragged media item's id.
#[derive(Clone, Copy)]
pub struct MediaDrag {
    /// Id of the media item being dragged from the bin.
    pub media_id: Uuid,
}

/// Height of the dashed "new track" row shown while hovering below the last track.
const NEW_TRACK_H: f32 = 80.0;
/// Translucent fill of the ghost clip.
fn ghost_fill() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(120, 160, 220, 90)
}
/// Border of the ghost clip.
fn ghost_stroke() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(140, 180, 240, 220)
}
/// Bright vertical insert line shown on Ctrl while hovering.
fn insert_line_color() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(255, 210, 90, 230)
}

/// Where a bin drag would land over the lanes.
enum DropTarget {
    /// Onto the track at this index.
    Track(usize),
    /// Below the last track: a new track would be created.
    NewTrack,
    /// Nowhere droppable.
    None,
}

/// Handles a bin media drag each frame: Esc cancels, release over the lanes drops.
///
/// Returns human-readable error messages for failed drops.
pub fn handle_bin_drag(
    ui: &egui::Ui,
    editor: &mut Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
) -> Vec<String> {
    let Some(payload) = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()) else {
        return Vec::new();
    };
    let media_id = payload.media_id;
    let (pointer, released, cancelled, ctrl) = ui.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.any_released(),
            i.key_pressed(egui::Key::Escape),
            i.modifiers.ctrl,
        )
    });
    if cancelled {
        egui::DragAndDrop::clear_payload(ui.ctx());
        return Vec::new();
    }
    if !released {
        return Vec::new();
    }
    egui::DragAndDrop::clear_payload(ui.ctx());
    let Some(p) = pointer else {
        return Vec::new();
    };
    // Released anywhere else (ruler, headers, outside the lanes): nothing.
    if p.x < g.lane_left || p.x > lane_rect.right() || p.y < lane_rect.top() {
        return Vec::new();
    }
    let start = g.x_to_sample(p.x);
    match drop_target(editor, lane_rect, scroll_y, p.y) {
        DropTarget::Track(index) => match editor.drop_media(media_id, Some(index), start, ctrl) {
            Ok(()) => Vec::new(),
            Err(error) => vec![error.to_string()],
        },
        DropTarget::NewTrack => match editor.drop_media(media_id, None, start, false) {
            Ok(()) => Vec::new(),
            Err(error) => vec![error.to_string()],
        },
        DropTarget::None => Vec::new(),
    }
}
/// Draws the translucent ghost clip while a bin drag hovers the lanes.
///
/// Over a track: a ghost rect at `x_to_sample(pointer)` as wide as the media.
/// Below the last track: the same ghost in a dashed "new track" row. With Ctrl
/// held a bright vertical insert line is also drawn at the start.
pub fn draw_drop_ghost(
    ui: &egui::Ui,
    editor: &Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
) {
    let Some(payload) = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()) else {
        return;
    };
    let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    if p.x < g.lane_left || p.x > lane_rect.right() || p.y < lane_rect.top() {
        return;
    }
    let Some(media) = editor
        .project
        .media
        .iter()
        .find(|media| media.id == payload.media_id)
    else {
        return;
    };
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    let painter = ui.painter_at(lane_rect);
    let start = g.x_to_sample(p.x);
    let left = g.sample_to_x(start);
    let right = g.sample_to_x(start + media.length);

    match drop_target(editor, lane_rect, scroll_y, p.y) {
        DropTarget::Track(index) => {
            let top = lane_rect.top() - scroll_y + heights_before(editor, index);
            let rect = egui::Rect::from_min_max(
                egui::pos2(left, top + 2.0),
                egui::pos2(right, top + editor.project.tracks[index].height - 2.0),
            );
            ghost_rect(&painter, rect);
        }
        DropTarget::NewTrack => {
            let count = editor.project.tracks.len();
            let top = lane_rect.top() - scroll_y + heights_before(editor, count);
            let row = egui::Rect::from_min_max(
                egui::pos2(g.lane_left, top),
                egui::pos2(lane_rect.right(), top + NEW_TRACK_H),
            );
            painter.rect_filled(row, 2.0, ghost_fill());
            dashed_rect(&painter, row, egui::Stroke::new(1.0, ghost_stroke()));
            let ghost = egui::Rect::from_min_max(
                egui::pos2(left, top + 2.0),
                egui::pos2(right, top + NEW_TRACK_H - 2.0),
            );
            ghost_rect(&painter, ghost);
        }
        DropTarget::None => {}
    }

    if ctrl {
        painter.line_segment(
            [
                egui::pos2(left, lane_rect.top()),
                egui::pos2(left, lane_rect.bottom()),
            ],
            egui::Stroke::new(2.0, insert_line_color()),
        );
    }
}

/// Fills and strokes one ghost clip rect.
fn ghost_rect(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 2.0, ghost_fill());
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, ghost_stroke()),
        egui::StrokeKind::Inside,
    );
}

/// Where a drop at window y would land, given the lane rect and vertical scroll.
fn drop_target(editor: &Editor, lane_rect: egui::Rect, scroll_y: f32, y: f32) -> DropTarget {
    if y > lane_rect.bottom() {
        return DropTarget::None;
    }
    let content_y = y - lane_rect.top() + scroll_y;
    let mut offset = 0.0;
    for (index, track) in editor.project.tracks.iter().enumerate() {
        let bottom = offset + track.height;
        if content_y < bottom {
            return DropTarget::Track(index);
        }
        offset = bottom;
    }
    DropTarget::NewTrack
}

/// Total height of the tracks before the given index, in points.
fn heights_before(editor: &Editor, index: usize) -> f32 {
    editor.project.tracks[..index]
        .iter()
        .map(|t| t.height)
        .sum()
}

/// Draws a dashed rectangle border with fixed dash and gap lengths.
fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    dashed_line(
        painter,
        egui::pos2(rect.left(), rect.top()),
        egui::pos2(rect.right(), rect.top()),
        stroke,
    );
    dashed_line(
        painter,
        egui::pos2(rect.left(), rect.bottom()),
        egui::pos2(rect.right(), rect.bottom()),
        stroke,
    );
    dashed_line(
        painter,
        egui::pos2(rect.left(), rect.top()),
        egui::pos2(rect.left(), rect.bottom()),
        stroke,
    );
    dashed_line(
        painter,
        egui::pos2(rect.right(), rect.top()),
        egui::pos2(rect.right(), rect.bottom()),
        stroke,
    );
}

/// Draws one dashed line between two points.
fn dashed_line(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke) {
    const DASH: f32 = 6.0;
    const GAP: f32 = 4.0;
    let vec = b - a;
    let len = vec.length();
    if len <= 0.0 {
        return;
    }
    let dir = vec / len;
    let mut d = 0.0;
    while d < len {
        let seg_end = (d + DASH).min(len);
        painter.line_segment([a + dir * d, a + dir * seg_end], stroke);
        d = seg_end + GAP;
    }
}

/// Dragging starts once the pointer moves this many points from the press.
const DRAG_START_PX: f32 = 3.0;
/// A range boundary line is grabbable within this many points of its x.
const BOUNDARY_GRAB_PX: f32 = 4.0;
/// Key under which the lane interaction state is carried between frames.
const LANE_STATE_KEY: &str = "timeline_lane_state";

/// Tint drawn over the selected part of each selected track.
fn range_tint() -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(120, 170, 255, 40)
}
/// Solid vertical boundary lines at the range start and end.
fn range_line() -> egui::Color32 {
    egui::Color32::from_rgb(90, 160, 255)
}
/// Bright vertical line drawn at the snap point while a drag is snapping.
fn snap_line_color() -> egui::Color32 {
    egui::Color32::from_rgb(255, 230, 80)
}

/// A press on the lanes, kept until release to tell click from drag.
#[derive(Clone, Copy)]
struct Press {
    /// Pointer position at press time.
    pos: egui::Pos2,
    /// Timeline sample under the press point.
    sample: u64,
    /// Index of the track that was pressed.
    track: usize,
}

/// An in-progress drag of a range boundary line.
#[derive(Clone)]
struct BoundaryDrag {
    /// True for the start line, false for the end line.
    start_edge: bool,
    /// The opposite edge, kept fixed while this edge moves.
    other: u64,
    /// Range tracks, kept so the range survives the edges crossing.
    tracks: Vec<uuid::Uuid>,
}

/// Lane interaction state carried between frames.
#[derive(Clone, Default)]
pub struct LaneState {
    /// Press on the lanes, when the primary button is down.
    press: Option<Press>,
    /// Boundary line being dragged, when any.
    boundary: Option<BoundaryDrag>,
    /// True while a range drag (moved 3 px or more) is in progress.
    range_drag: bool,
    /// Snap point to highlight while a drag is snapping, if any.
    snap_line: Option<u64>,
}

/// Handles all mouse interaction on the lane area: range selection drags,
/// boundary line drags, playhead clicks and Shift+click extension.
pub fn handle_lane_input(
    ui: &egui::Ui,
    editor: &mut Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
) {
    let mut state: LaneState = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<LaneState>(egui::Id::new(LANE_STATE_KEY))
            .clone()
    });
    let (pressed, down, released, pos, shift, ctrl, alt) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_released(),
            i.pointer.latest_pos(),
            i.modifiers.shift,
            i.modifiers.ctrl,
            i.modifiers.alt,
        )
    });

    if released {
        state.boundary = None;
        state.range_drag = false;
        state.snap_line = None;
        if let (Some(press), Some(p)) = (state.press.take(), pos) {
            finish_click(editor, g, press, p, shift, alt);
        }
    } else if let (true, Some(p), Some(press)) = (down, pos, state.press) {
        let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
        if dist >= DRAG_START_PX {
            state.range_drag = true;
            drag_range(editor, g, lane_rect, scroll_y, press, p, alt, &mut state);
        }
    } else if down
        && let Some(drag) = state.boundary.clone()
        && let Some(p) = pos
    {
        let (snapped, snap_point) = editor.snap_pos(g.x_to_sample(p.x), &[], alt);
        let (a, b) = if drag.start_edge {
            (snapped, drag.other)
        } else {
            (drag.other, snapped)
        };
        editor.set_range(a, b, drag.tracks);
        state.snap_line = snap_point;
    } else if pressed
        && let Some(p) = pos
        && lane_rect.contains(p)
    {
        press_started(editor, g, lane_rect, scroll_y, p, ctrl, &mut state);
    }

    // Resize cursor near a boundary line (and while dragging one).
    if state.boundary.is_some() || boundary_hover(editor, g, lane_rect, pos) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }

    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(LANE_STATE_KEY), state));
}
/// A fresh press on the lanes: grab a boundary line, or remember the press.
fn press_started(
    editor: &mut Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
    p: egui::Pos2,
    ctrl: bool,
    state: &mut LaneState,
) {
    // Ctrl+click on the lanes is left for clip selection (task 6.3).
    if ctrl {
        return;
    }
    // A track under the pointer is remembered for later edit targets.
    if let Some(index) = track_index_at(editor, lane_rect, scroll_y, p.y, false) {
        editor.set_last_clicked_track(editor.project.tracks[index].id);
    }
    // Within 4 px of a boundary line: drag that line instead of selecting.
    if let Some((start_edge, other, tracks)) = boundary_press(editor, g, lane_rect, p) {
        state.boundary = Some(BoundaryDrag {
            start_edge,
            other,
            tracks,
        });
        return;
    }
    if let Some(track) = track_index_at(editor, lane_rect, scroll_y, p.y, false) {
        state.press = Some(Press {
            pos: p,
            sample: g.x_to_sample(p.x),
            track,
        });
    }
}

/// Finishes a press that never became a drag: a click on the lanes.
fn finish_click(
    editor: &mut Editor,
    g: &Geom,
    press: Press,
    p: egui::Pos2,
    shift: bool,
    alt: bool,
) {
    // Clicks that moved 3 px or more, or with no track under the press,
    // are not lane clicks.
    let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
    if editor.project.tracks.get(press.track).is_none() || dist >= DRAG_START_PX {
        return;
    }
    let (snapped, _) = editor.snap_pos(g.x_to_sample(p.x), &[], alt);
    if shift {
        // Shift+click: extend the existing range, or start one at the playhead.
        if let crate::editor::selection::Selection::Range { start, end, tracks } =
            editor.selection().clone()
        {
            let (a, b) = if (snapped as i128 - start as i128).abs()
                <= (snapped as i128 - end as i128).abs()
            {
                (snapped, end)
            } else {
                (start, snapped)
            };
            editor.set_range(a, b, tracks);
        } else {
            let track = editor.project.tracks[press.track].id;
            editor.set_range(editor.view.playhead, snapped, vec![track]);
        }
    } else {
        editor.clear_selection();
        if editor.transport_state() == crate::editor::transport::TransportState::Playing {
            editor.seek(snapped);
        } else {
            editor.set_playhead(snapped);
        }
    }
}
/// One frame of range dragging: the range follows the pointer, both edges snap.
#[allow(clippy::too_many_arguments)]
fn drag_range(
    editor: &mut Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
    press: Press,
    p: egui::Pos2,
    alt: bool,
    state: &mut LaneState,
) {
    let (press_sample, snap_a) = editor.snap_pos(press.sample, &[], alt);
    let (cur_sample, snap_b) = editor.snap_pos(g.x_to_sample(p.x), &[], alt);
    let tracks = match track_index_at(editor, lane_rect, scroll_y, p.y, true) {
        Some(index) => {
            let (lo, hi) = if index < press.track {
                (index, press.track)
            } else {
                (press.track, index)
            };
            editor.project.tracks[lo..=hi]
                .iter()
                .map(|t| t.id)
                .collect()
        }
        None => {
            // No tracks in the project: keep whatever the selection was.
            match editor.selection() {
                crate::editor::selection::Selection::Range { tracks, .. } => tracks.clone(),
                _ => return,
            }
        }
    };
    editor.set_range(press_sample, cur_sample, tracks);
    state.snap_line = snap_b.or(snap_a);
}

/// Index of the track under window `y`, or the nearest track when `clamp`
/// (used while dragging, so crossing the lane edges keeps the range alive).
fn track_index_at(
    editor: &Editor,
    lane_rect: egui::Rect,
    scroll_y: f32,
    y: f32,
    clamp: bool,
) -> Option<usize> {
    let count = editor.project.tracks.len();
    if count == 0 {
        return None;
    }
    if y > lane_rect.bottom() {
        return if clamp { Some(count - 1) } else { None };
    }
    let content_y = (y - lane_rect.top() + scroll_y).max(0.0);
    let mut acc = 0.0;
    for (index, track) in editor.project.tracks.iter().enumerate() {
        acc += track.height;
        if content_y < acc {
            return Some(index);
        }
    }
    if clamp { Some(count - 1) } else { None }
}

/// If the press is within 4 px of a range boundary line: that line's edge,
/// the fixed opposite edge and the range tracks (for the swap-across case).
fn boundary_press(
    editor: &Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    p: egui::Pos2,
) -> Option<(bool, u64, Vec<uuid::Uuid>)> {
    if !lane_rect.contains(p) {
        return None;
    }
    if let crate::editor::selection::Selection::Range { start, end, tracks } = editor.selection() {
        let start_x = g.sample_to_x(*start);
        let end_x = g.sample_to_x(*end);
        if (p.x - start_x).abs() <= BOUNDARY_GRAB_PX {
            return Some((true, *end, tracks.clone()));
        }
        if (p.x - end_x).abs() <= BOUNDARY_GRAB_PX {
            return Some((false, *start, tracks.clone()));
        }
    }
    None
}

/// True when the pointer hovers within 4 px of a range boundary line.
fn boundary_hover(
    editor: &Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    pos: Option<egui::Pos2>,
) -> bool {
    pos.is_some_and(|p| boundary_press(editor, g, lane_rect, p).is_some())
}

/// Draws the range selection: tinted overlay on the selected tracks only,
/// solid vertical boundary lines over the full lane height, and the snap line
/// while a drag is snapping.
pub fn draw_range(ui: &egui::Ui, editor: &Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) {
    let state: LaneState = ui.ctx().data(|d| {
        d.get_temp(egui::Id::new(LANE_STATE_KEY))
            .unwrap_or_default()
    });
    let crate::editor::selection::Selection::Range { start, end, tracks } = editor.selection()
    else {
        return;
    };
    let painter = ui.painter_at(lane_rect);
    let x0 = g.sample_to_x(*start).max(g.lane_left);
    let x1 = g.sample_to_x(*end).min(lane_rect.right());
    if x1 >= g.lane_left && x0 <= lane_rect.right() {
        let mut y = lane_rect.top() - scroll_y;
        for track in &editor.project.tracks {
            let bottom = y + track.height;
            if tracks.contains(&track.id) && bottom >= lane_rect.top() && y <= lane_rect.bottom() {
                let rect = egui::Rect::from_min_max(
                    egui::pos2(x0, y.max(lane_rect.top())),
                    egui::pos2(x1, bottom.min(lane_rect.bottom())),
                );
                painter.rect_filled(rect, 0.0, range_tint());
            }
            y = bottom;
        }
        let stroke = egui::Stroke::new(1.5, range_line());
        for x in [x0, x1] {
            painter.line_segment(
                [
                    egui::pos2(x, lane_rect.top()),
                    egui::pos2(x, lane_rect.bottom()),
                ],
                stroke,
            );
        }
    }
    if let Some(snap) = state.snap_line {
        let x = g.sample_to_x(snap);
        painter.line_segment(
            [
                egui::pos2(x, lane_rect.top()),
                egui::pos2(x, lane_rect.bottom()),
            ],
            egui::Stroke::new(2.0, snap_line_color()),
        );
    }
}
