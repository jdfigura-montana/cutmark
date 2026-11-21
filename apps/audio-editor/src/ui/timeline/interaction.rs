//! ui::timeline::interaction: mouse actions over the timeline, including bin
//! drags and range selection.
//! ui::timeline::interaction: mouse actions over the timeline, including bin drags.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::ops::clip::Edge as TrimEdge;
use crate::model::clip::Clip;
use crate::ui::bin_panel;
use crate::ui::dialogs;
use crate::ui::theme::THEME;

use super::Geom;
use super::clip_view::{self, Zone};

/// Opacity of the ghost clip shown while a bin item is dragged over the lanes
/// (S1 spec, "timeline/interaction.rs visuals": the clip at 50%).
const GHOST_OPACITY: f32 = 0.5;

/// The drag payload set by the media bin: the dragged media item's id.
#[derive(Clone, Copy)]
pub struct MediaDrag {
    /// Id of the media item being dragged from the bin.
    pub media_id: Uuid,
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
pub fn handle_bin_drag(ui: &egui::Ui, editor: &mut Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) -> Vec<String> {
    let Some(payload) = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()) else {
        return Vec::new();
    };
    let media_id = payload.media_id;
    let (pointer, released, cancelled, ctrl) =
        ui.input(|i| (i.pointer.latest_pos(), i.pointer.any_released(), i.key_pressed(egui::Key::Escape), i.modifiers.ctrl));
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
pub fn draw_drop_ghost(ui: &egui::Ui, editor: &Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) {
    let Some(payload) = egui::DragAndDrop::payload::<MediaDrag>(ui.ctx()) else {
        return;
    };
    let Some(p) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    if p.x < g.lane_left || p.x > lane_rect.right() || p.y < lane_rect.top() {
        return;
    }
    let Some(media) = editor.project.media.iter().find(|media| media.id == payload.media_id) else {
        return;
    };
    let ctrl = ui.input(|i| i.modifiers.ctrl);
    let painter = ui.painter_at(lane_rect);
    let theme = ui_kit::Theme::get(ui.ctx());
    let start = g.x_to_sample(p.x);
    let left = g.sample_to_x(start);
    let right = g.sample_to_x(start + media.length);

    match drop_target(editor, lane_rect, scroll_y, p.y) {
        DropTarget::Track(index) => {
            let top = lane_rect.top() - scroll_y + heights_before(editor, index);
            let rect = egui::Rect::from_min_max(
                egui::pos2(left, top + THEME.clip_margin),
                egui::pos2(right, top + editor.project.tracks[index].height - THEME.clip_margin),
            );
            ghost_clip(&painter, rect, &media.display_name, &theme);
        }
        DropTarget::NewTrack => {
            let count = editor.project.tracks.len();
            let top = lane_rect.top() - scroll_y + heights_before(editor, count);
            let row = egui::Rect::from_min_max(egui::pos2(g.lane_left, top), egui::pos2(lane_rect.right(), top + THEME.new_track_h));
            painter.rect_filled(row, THEME.corner_r, THEME.ghost_fill);
            dashed_rect(&painter, row, egui::Stroke::new(THEME.hairline, THEME.ghost_stroke));
            let ghost = egui::Rect::from_min_max(egui::pos2(left, top + THEME.clip_margin), egui::pos2(right, top + THEME.new_track_h - THEME.clip_margin));
            ghost_clip(&painter, ghost, &media.display_name, &theme);
        }
        DropTarget::None => {}
    }

    if ctrl {
        painter.line_segment(
            [egui::pos2(left, lane_rect.top()), egui::pos2(left, lane_rect.bottom())],
            egui::Stroke::new(THEME.insert_line_w, THEME.insert_line_color),
        );
    }
}

/// Paints the dragged ghost clip: the ui-kit audio clip at 50% opacity,
/// labeled with the media name.
fn ghost_clip(painter: &egui::Painter, rect: egui::Rect, name: &str, theme: &ui_kit::Theme) {
    let look = ui_kit::ClipLook { opacity: GHOST_OPACITY, ..ui_kit::ClipLook::new(name, ui_kit::ClipKind::Audio) };
    ui_kit::paint_clip(painter, rect, &look, theme);
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
    editor.project.tracks[..index].iter().map(|t| t.height).sum()
}

/// Draws a dashed rectangle border with fixed dash and gap lengths.
fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    dashed_line(painter, egui::pos2(rect.left(), rect.top()), egui::pos2(rect.right(), rect.top()), stroke);
    dashed_line(painter, egui::pos2(rect.left(), rect.bottom()), egui::pos2(rect.right(), rect.bottom()), stroke);
    dashed_line(painter, egui::pos2(rect.left(), rect.top()), egui::pos2(rect.left(), rect.bottom()), stroke);
    dashed_line(painter, egui::pos2(rect.right(), rect.top()), egui::pos2(rect.right(), rect.bottom()), stroke);
}

/// Draws one dashed line between two points.
fn dashed_line(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke) {
    let vec = b - a;
    let len = vec.length();
    if len <= 0.0 {
        return;
    }
    let dir = vec / len;
    let mut d = 0.0;
    while d < len {
        let seg_end = (d + THEME.ghost_dash).min(len);
        painter.line_segment([a + dir * d, a + dir * seg_end], stroke);
        d = seg_end + THEME.ghost_gap;
    }
}

/// Key under which the lane interaction state is carried between frames.
const LANE_STATE_KEY: &str = "timeline_lane_state";

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
pub fn handle_lane_input(ui: &egui::Ui, editor: &mut Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) {
    let mut state: LaneState = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<LaneState>(egui::Id::new(LANE_STATE_KEY)).clone());
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
        if dist >= THEME.drag_start_px {
            state.range_drag = true;
            drag_range(editor, g, lane_rect, scroll_y, press, p, alt, &mut state);
        }
    } else if down
        && let Some(drag) = state.boundary.clone()
        && let Some(p) = pos
    {
        let (snapped, snap_point) = editor.snap_pos(g.x_to_sample(p.x), &[], None, alt, THEME.snap_px);
        let (a, b) = if drag.start_edge { (snapped, drag.other) } else { (drag.other, snapped) };
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

    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(LANE_STATE_KEY), state));
}
/// A fresh press on the lanes: grab a boundary line, or remember the press.
fn press_started(editor: &mut Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32, p: egui::Pos2, ctrl: bool, state: &mut LaneState) {
    // Any click on a lane clears a marker selection.
    editor.select_marker(None);
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
        state.boundary = Some(BoundaryDrag { start_edge, other, tracks });
        return;
    }
    if let Some(track) = track_index_at(editor, lane_rect, scroll_y, p.y, false) {
        state.press = Some(Press { pos: p, sample: g.x_to_sample(p.x), track });
    }
}

/// Finishes a press that never became a drag: a click on the lanes.
fn finish_click(editor: &mut Editor, g: &Geom, press: Press, p: egui::Pos2, shift: bool, alt: bool) {
    // Clicks that moved 3 px or more, or with no track under the press,
    // are not lane clicks.
    let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
    if editor.project.tracks.get(press.track).is_none() || dist >= THEME.drag_start_px {
        return;
    }
    let (snapped, _) = editor.snap_pos(g.x_to_sample(p.x), &[], None, alt, THEME.snap_px);
    if shift {
        // Shift+click: extend the existing range, or start one at the playhead.
        if let crate::editor::selection::Selection::Range { start, end, tracks } = editor.selection().clone() {
            let (a, b) = if (snapped as i128 - start as i128).abs() <= (snapped as i128 - end as i128).abs() { (snapped, end) } else { (start, snapped) };
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
fn drag_range(editor: &mut Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32, press: Press, p: egui::Pos2, alt: bool, state: &mut LaneState) {
    let (press_sample, snap_a) = editor.snap_pos(press.sample, &[], None, alt, THEME.snap_px);
    let (cur_sample, snap_b) = editor.snap_pos(g.x_to_sample(p.x), &[], None, alt, THEME.snap_px);
    let tracks = match track_index_at(editor, lane_rect, scroll_y, p.y, true) {
        Some(index) => {
            let (lo, hi) = if index < press.track { (index, press.track) } else { (press.track, index) };
            editor.project.tracks[lo..=hi].iter().map(|t| t.id).collect()
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
fn track_index_at(editor: &Editor, lane_rect: egui::Rect, scroll_y: f32, y: f32, clamp: bool) -> Option<usize> {
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
fn boundary_press(editor: &Editor, g: &Geom, lane_rect: egui::Rect, p: egui::Pos2) -> Option<(bool, u64, Vec<uuid::Uuid>)> {
    if !lane_rect.contains(p) {
        return None;
    }
    if let crate::editor::selection::Selection::Range { start, end, tracks } = editor.selection() {
        let start_x = g.sample_to_x(*start);
        let end_x = g.sample_to_x(*end);
        if (p.x - start_x).abs() <= THEME.boundary_grab_px {
            return Some((true, *end, tracks.clone()));
        }
        if (p.x - end_x).abs() <= THEME.boundary_grab_px {
            return Some((false, *start, tracks.clone()));
        }
    }
    None
}

/// True when the pointer hovers within 4 px of a range boundary line.
fn boundary_hover(editor: &Editor, g: &Geom, lane_rect: egui::Rect, pos: Option<egui::Pos2>) -> bool {
    pos.is_some_and(|p| boundary_press(editor, g, lane_rect, p).is_some())
}

/// Draws the range selection: tinted overlay on the selected tracks only,
/// solid vertical boundary lines over the full lane height, and the snap line
/// while a drag is snapping.
pub fn draw_range(ui: &egui::Ui, editor: &Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) {
    let state: LaneState = ui.ctx().data(|d| d.get_temp(egui::Id::new(LANE_STATE_KEY)).unwrap_or_default());
    let crate::editor::selection::Selection::Range { start, end, tracks } = editor.selection() else {
        return;
    };
    let painter = ui.painter_at(lane_rect);
    let theme = ui_kit::Theme::get(ui.ctx());
    let x0 = g.sample_to_x(*start).max(g.lane_left);
    let x1 = g.sample_to_x(*end).min(lane_rect.right());
    if x1 >= g.lane_left && x0 <= lane_rect.right() {
        let mut y = lane_rect.top() - scroll_y;
        for track in &editor.project.tracks {
            let bottom = y + track.height;
            if tracks.contains(&track.id) && bottom >= lane_rect.top() && y <= lane_rect.bottom() {
                let rect = egui::Rect::from_min_max(egui::pos2(x0, y.max(lane_rect.top())), egui::pos2(x1, bottom.min(lane_rect.bottom())));
                painter.rect_filled(rect, 0.0, theme.accent.tint);
            }
            y = bottom;
        }
        let stroke = egui::Stroke::new(THEME.range_line_w, THEME.range_line);
        for x in [x0, x1] {
            painter.line_segment([egui::pos2(x, lane_rect.top()), egui::pos2(x, lane_rect.bottom())], stroke);
        }
    }
    if let Some(snap) = state.snap_line {
        ui_kit::paint_snap_line(&painter, g.sample_to_x(snap), lane_rect, &theme);
    }
}

// ---- Clip interaction: selection, moving, trimming, context menu (6.3) ----

/// Key under which the clip interaction state is carried between frames.
const CLIP_STATE_KEY: &str = "timeline_clip_state";

/// A primary press on a clip's header or edge, kept until release.
#[derive(Clone, Copy)]
struct ClipPress {
    /// The pressed clip.
    clip_id: Uuid,
    /// Index of the clip's track at press time.
    track_index: usize,
    /// Pointer position at press time.
    pos: egui::Pos2,
    /// The zone that was pressed.
    zone: Zone,
}

/// An active clip drag: moving by the header or trimming an edge.
#[derive(Clone)]
enum ClipDrag {
    /// Moving the selected clips by the header.
    Move {
        /// All clips that move together.
        ids: Vec<Uuid>,
        /// Timeline start of the pressed clip in the gesture base.
        base_start: u64,
        /// Timeline end of the pressed clip in the gesture base.
        base_end: u64,
        /// Index of the pressed clip's track at press time.
        press_track: usize,
    },
    /// Trimming one edge of the pressed clip.
    Trim {
        /// The clip being trimmed.
        clip_id: Uuid,
        /// Which edge is being dragged.
        edge: TrimEdge,
    },
    /// Dragging a fade handle of the pressed clip.
    Fade {
        /// The clip whose fade is dragged.
        clip_id: Uuid,
        /// True for the fade-out (right) handle.
        out: bool,
    },
    /// Dragging the gain line of the pressed clip.
    Gain {
        /// The clip whose gain line is dragged.
        clip_id: Uuid,
        /// Clip gain at press time, in dB.
        base_db: f32,
        /// Pointer y at press time.
        press_y: f32,
    },
}

/// A clip context menu shown at a fixed screen position.
#[derive(Clone, Copy)]
struct ClipMenu {
    /// The right-clicked clip.
    clip_id: Uuid,
    /// Screen position of the menu's top left corner.
    pos: egui::Pos2,
}

/// Inline gain value editor opened by double-clicking the header gain text.
#[derive(Clone)]
struct GainEdit {
    /// The clip whose gain is edited.
    clip_id: Uuid,
    /// Current field text.
    text: String,
    /// Screen position of the edit field.
    pos: egui::Pos2,
}

/// Clip interaction state carried between frames.
#[derive(Clone, Default)]
struct ClipState {
    /// Press on a clip header or edge, when the primary button is down.
    press: Option<ClipPress>,
    /// Active move or trim drag, when any.
    drag: Option<ClipDrag>,
    /// Snap point to highlight while a clip drag is snapping, if any.
    snap_line: Option<u64>,
    /// Open clip context menu, if any.
    menu: Option<ClipMenu>,
    /// Inline gain value editor, if any.
    gain_edit: Option<GainEdit>,
}

/// Reads the clip interaction state from egui's temp storage.
fn clip_state(ctx: &egui::Context) -> ClipState {
    ctx.data(|d| d.get_temp(egui::Id::new(CLIP_STATE_KEY)).unwrap_or_default())
}

/// Writes the clip interaction state back to egui's temp storage.
fn save_clip_state(ctx: &egui::Context, state: &ClipState) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(CLIP_STATE_KEY), state.clone()));
}

/// True while a clip gain line is being dragged (for its brighter stroke).
pub fn gain_line_active(ctx: &egui::Context) -> bool {
    clip_state(ctx).drag.is_some_and(|drag| matches!(drag, ClipDrag::Gain { .. }))
}

/// True when the clip is in the current clip selection.
fn clip_selected(editor: &Editor, id: Uuid) -> bool {
    matches!(
        editor.selection(),
        crate::editor::selection::Selection::Clips(ids) if ids.contains(&id)
    )
}

/// The topmost clip under the pointer: `(clip id, track index, zone)`.
///
/// Tracks and clips are scanned in draw order, so the clip painted last wins.
fn clip_hit(editor: &Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32, p: egui::Pos2) -> Option<(Uuid, usize, Zone)> {
    if !lane_rect.contains(p) {
        return None;
    }
    let mut found = None;
    let mut y = lane_rect.top() - scroll_y;
    for (index, track) in editor.project.tracks.iter().enumerate() {
        let bottom = y + track.height;
        let track_rect = egui::Rect::from_min_max(egui::pos2(g.lane_left, y), egui::pos2(lane_rect.right(), bottom));
        for clip in &track.clips {
            let rect = clip_view::clip_rect(clip, g, track_rect);
            if let Some(zone) = clip_view::hit_test(clip, rect, g.spp, p) {
                found = Some((clip.id, index, zone));
            }
        }
        y = bottom;
    }
    found
}

/// Finds a clip and its on-screen rect by id anywhere in the project.
fn clip_by_id(editor: &Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32, clip_id: Uuid) -> Option<(Clip, egui::Rect)> {
    let mut y = lane_rect.top() - scroll_y;
    for track in &editor.project.tracks {
        let bottom = y + track.height;
        let track_rect = egui::Rect::from_min_max(egui::pos2(g.lane_left, y), egui::pos2(lane_rect.right(), bottom));
        if let Some(clip) = track.clips.iter().find(|clip| clip.id == clip_id) {
            return Some((clip.clone(), clip_view::clip_rect(clip, g, track_rect)));
        }
        y = bottom;
    }
    None
}

/// Starts a move or trim gesture for the pressed clip, selecting it alone
/// first when it is not in the clip selection. `None` when the clip is gone.
fn begin_clip_drag(editor: &mut Editor, press: &ClipPress) -> Option<ClipDrag> {
    match press.zone {
        Zone::LeftEdge | Zone::RightEdge => {
            editor.begin_gesture();
            Some(ClipDrag::Trim { clip_id: press.clip_id, edge: if press.zone == Zone::LeftEdge { TrimEdge::Start } else { TrimEdge::End } })
        }
        Zone::Header => {
            if !clip_selected(editor, press.clip_id) {
                editor.select_clips(vec![press.clip_id]);
            }
            let ids = match editor.selection() {
                crate::editor::selection::Selection::Clips(ids) => ids.clone(),
                _ => vec![press.clip_id],
            };
            let (base_start, base_end) =
                editor.project.tracks.iter().find_map(|track| track.clips.iter().find(|clip| clip.id == press.clip_id).map(|clip| (clip.start, clip.end())))?;
            editor.begin_gesture();
            Some(ClipDrag::Move { ids, base_start, base_end, press_track: press.track_index })
        }
        Zone::FadeInHandle => {
            editor.begin_gesture();
            Some(ClipDrag::Fade { clip_id: press.clip_id, out: false })
        }
        Zone::FadeOutHandle => {
            editor.begin_gesture();
            Some(ClipDrag::Fade { clip_id: press.clip_id, out: true })
        }
        Zone::GainLine => {
            let base_db =
                editor.project.tracks.iter().flat_map(|track| track.clips.iter()).find(|clip| clip.id == press.clip_id).map(|clip| clip.gain_db).unwrap_or(0.0);
            editor.begin_gesture();
            Some(ClipDrag::Gain { clip_id: press.clip_id, base_db, press_y: press.pos.y })
        }
        Zone::Body => None,
    }
}

/// One frame of an active clip drag: previews the move or trim.
#[allow(clippy::too_many_arguments)]
fn run_clip_drag(
    editor: &mut Editor,
    g: &Geom,
    lane_rect: egui::Rect,
    scroll_y: f32,
    press: &ClipPress,
    p: egui::Pos2,
    alt: bool,
    shift: bool,
    state: &mut ClipState,
) {
    let Some(drag) = state.drag.clone() else {
        return;
    };
    match drag {
        ClipDrag::Move { ids, base_start, base_end, press_track } => {
            let mut delta = ((p.x - press.pos.x) as f64 * g.spp).round() as i64;
            // Snap the pressed clip's start or end, whichever adjusts less.
            let mut snap_point = None;
            let mut options: Vec<(i64, u64)> = Vec::new();
            let cand_start = base_start as i64 + delta;
            let cand_end = base_end as i64 + delta;
            if cand_start >= 0 {
                let (snapped, _) = editor.snap_pos(cand_start as u64, &ids, None, alt, THEME.snap_px);
                if snapped != cand_start as u64 {
                    options.push((snapped as i64 - cand_start, snapped));
                }
            }
            if cand_end >= 0 {
                let (snapped, _) = editor.snap_pos(cand_end as u64, &ids, None, alt, THEME.snap_px);
                if snapped != cand_end as u64 {
                    options.push((snapped as i64 - cand_end, snapped));
                }
            }
            if let Some((adjust, point)) = options.into_iter().min_by_key(|(adjust, _)| adjust.abs()) {
                delta += adjust;
                snap_point = Some(point);
            }
            if let Some(track) = track_index_at(editor, lane_rect, scroll_y, p.y, false) {
                let delta_tracks = track as i64 - press_track as i64;
                if editor.preview(Action::MoveClips { clip_ids: ids.clone(), delta_time: delta, delta_tracks: delta_tracks as i32 }).is_ok() {
                    state.snap_line = snap_point;
                } else {
                    // Off the top or bottom track, or before 0: keep the last good preview.
                    state.snap_line = None;
                }
            } else {
                state.snap_line = None;
            }
        }
        ClipDrag::Trim { clip_id, edge } => {
            let (new_pos, snap_point) = editor.snap_pos(g.x_to_sample(p.x), &[clip_id], None, alt, THEME.snap_px);
            if editor.preview(Action::TrimClip { clip_id, edge, new_pos }).is_ok() {
                state.snap_line = snap_point;
            } else {
                state.snap_line = None;
            }
        }
        ClipDrag::Fade { clip_id, out } => {
            let Some((clip, _rect)) = clip_by_id(editor, g, lane_rect, scroll_y, clip_id) else {
                return;
            };
            let (snapped, snap_point) = editor.snap_pos(g.x_to_sample(p.x), &[clip_id], None, alt, THEME.snap_px);
            let value = if out { clip.end().saturating_sub(snapped) } else { snapped.saturating_sub(clip.start) }.min(clip.len());
            let fade_in = if out { clip.fade_in } else { value };
            let fade_out = if out { value } else { clip.fade_out };
            if editor.preview(Action::SetClipFades { clip_id, fade_in, fade_out }).is_ok() {
                state.snap_line = snap_point;
            } else {
                state.snap_line = None;
            }
        }
        ClipDrag::Gain { clip_id, base_db, press_y } => {
            let Some((_clip, rect)) = clip_by_id(editor, g, lane_rect, scroll_y, clip_id) else {
                return;
            };
            let body = clip_view::body_rect(rect);
            if body.height() <= 0.0 {
                return;
            }
            // Same px-to-dB mapping as the gain line drawing, inverted.
            let delta_db = -(p.y - press_y) * clip_view::GAIN_SPAN_DB / body.height();
            let mut db = base_db + delta_db;
            if shift {
                db = (db * 10.0).round() / 10.0;
            }
            let _ = editor.preview(Action::SetClipGain { clip_id, db: db.clamp(clip_view::GAIN_MIN_DB, clip_view::GAIN_MAX_DB) });
        }
    }
}

/// Handles clip mouse interaction: header selection and drags, edge trims,
/// right-click menus and hover cursors.
///
/// Returns true when the press (or an active clip drag or menu) is handled
/// here, so the lane interaction must not react to it. Clip body presses fall
/// through to the lane interaction as before.
pub fn handle_clip_input(ui: &egui::Ui, editor: &mut Editor, g: &Geom, lane_rect: egui::Rect, scroll_y: f32) -> bool {
    let mut state = clip_state(ui.ctx());
    let (pressed, down, released, secondary, pos, ctrl, alt, shift, esc) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.any_released(),
            i.pointer.secondary_pressed(),
            i.pointer.latest_pos(),
            i.modifiers.ctrl,
            i.modifiers.alt,
            i.modifiers.shift,
            i.key_pressed(egui::Key::Escape),
        )
    });

    // Esc while the inline gain editor is open closes it; consumed so the
    // global Esc handler does not also clear the selection.
    if esc && state.drag.is_none() && state.gain_edit.take().is_some() {
        save_clip_state(ui.ctx(), &state);
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        return true;
    }

    // Esc during a drag cancels the gesture; the key is consumed so the
    // global Esc handler does not also clear the selection.
    if esc && state.drag.is_some() {
        editor.cancel_gesture();
        state.drag = None;
        state.snap_line = None;
        state.press = None;
        state.gain_edit = None;
        save_clip_state(ui.ctx(), &state);
        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        return true;
    }

    let mut handled = false;
    if released {
        if let Some(press) = state.press.take() {
            handled = true;
            if state.drag.take().is_some() {
                editor.end_gesture();
                state.snap_line = None;
            } else if let Some(p) = pos {
                // A click on the header: select, or toggle with Ctrl.
                let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
                if dist < THEME.drag_start_px {
                    if ctrl {
                        editor.toggle_clip(press.clip_id);
                    } else {
                        editor.select_clips(vec![press.clip_id]);
                    }
                }
            }
        }
    } else if down && let (Some(press), Some(p)) = (state.press, pos) {
        handled = true;
        let dist = ((p.x - press.pos.x).powi(2) + (p.y - press.pos.y).powi(2)).sqrt();
        if dist >= THEME.drag_start_px {
            if state.drag.is_none()
                && let Some(drag) = begin_clip_drag(editor, &press)
            {
                state.drag = Some(drag);
            }
            if state.drag.is_some() {
                run_clip_drag(editor, g, lane_rect, scroll_y, &press, p, alt, shift, &mut state);
            }
        }
        match &state.drag {
            Some(ClipDrag::Move { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            Some(ClipDrag::Trim { .. } | ClipDrag::Fade { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            Some(ClipDrag::Gain { .. }) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            None => {}
        }
    } else if pressed && let Some(p) = pos {
        // Any click on a clip clears a marker selection.
        editor.select_marker(None);
        if secondary {
            if let Some((clip_id, _index, _zone)) = clip_hit(editor, g, lane_rect, scroll_y, p) {
                state.menu = Some(ClipMenu { clip_id, pos: p });
                handled = true;
            }
        } else if let Some((clip_id, index, zone)) = clip_hit(editor, g, lane_rect, scroll_y, p) {
            let double_clicked = ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));
            if double_clicked
                && zone == Zone::Header
                && let Some((clip, rect)) = clip_by_id(editor, g, lane_rect, scroll_y, clip_id)
                && clip_view::gain_text_rect(rect).contains(p)
            {
                // Double-click on the header gain value: inline edit.
                state.gain_edit = Some(GainEdit { clip_id, text: format!("{:.1}", clip.gain_db), pos: clip_view::gain_text_rect(rect).min });
                handled = true;
            } else {
                match zone {
                    Zone::Body => {}
                    _ => {
                        state.press = Some(ClipPress { clip_id, track_index: index, pos: p, zone });
                        handled = true;
                    }
                }
            }
        }
    } else if !down && let Some(p) = pos {
        // Hover cursors: grab over the header, horizontal resize over edges.
        if let Some((_id, _index, zone)) = clip_hit(editor, g, lane_rect, scroll_y, p) {
            let icon = match zone {
                Zone::Header => Some(egui::CursorIcon::Grab),
                Zone::LeftEdge | Zone::RightEdge | Zone::FadeInHandle | Zone::FadeOutHandle => Some(egui::CursorIcon::ResizeHorizontal),
                Zone::GainLine => Some(egui::CursorIcon::ResizeVertical),
                Zone::Body => None,
            };
            if let Some(icon) = icon {
                ui.ctx().set_cursor_icon(icon);
            }
        }
    }
    save_clip_state(ui.ctx(), &state);
    handled
}

/// Draws the bright snap line while a clip drag is snapping.
pub fn draw_clip_snap_line(ui: &egui::Ui, g: &Geom, lane_rect: egui::Rect) {
    let state = clip_state(ui.ctx());
    let Some(snap) = state.snap_line else {
        return;
    };
    let painter = ui.painter_at(lane_rect);
    ui_kit::paint_snap_line(&painter, g.sample_to_x(snap), lane_rect, &ui_kit::Theme::get(ui.ctx()));
}

/// Draws the clip context menu and runs its actions.
///
/// Closes on Esc, on a click outside the menu, or after an action. Returns
/// human-readable error messages from failed actions.
pub fn draw_clip_menu(ui: &egui::Ui, editor: &mut Editor) -> Vec<String> {
    let mut state = clip_state(ui.ctx());
    let mut messages = Vec::new();
    let Some(menu) = state.menu else {
        return messages;
    };
    let (esc, clicked, pointer) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.pointer.any_click(), i.pointer.latest_pos()));
    if esc {
        state.menu = None;
        save_clip_state(ui.ctx(), &state);
        return messages;
    }

    // Gather everything the menu entries need before any action mutates the project.
    let info = editor.project.tracks.iter().find_map(|track| track.clips.iter().find(|clip| clip.id == menu.clip_id).map(|clip| (clip.clone(), track.id)));
    let Some((clip, track_id)) = info else {
        state.menu = None;
        save_clip_state(ui.ctx(), &state);
        return messages;
    };
    let playhead = editor.view.playhead;
    let splittable = clip.start < playhead && playhead < clip.end();
    let delete_ids: Vec<Uuid> = if clip_selected(editor, clip.id) {
        match editor.selection() {
            crate::editor::selection::Selection::Clips(ids) => ids.clone(),
            _ => vec![clip.id],
        }
    } else {
        vec![clip.id]
    };
    let media_id = clip.media_id;

    let mut close = false;
    let response = egui::Area::new(egui::Id::new("clip_context_menu")).order(egui::Order::Foreground).fixed_pos(menu.pos).show(ui.ctx(), |ui| {
        // The glass menu: egui's menu frame reads the glass fill, rim radius
        // and shadow `ui_kit::apply` put into the visuals.
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            crate::ui::theme::menu_item_style(ui.style_mut());
            ui.set_min_width(THEME.clip_menu_min_w);
            if ui.add_enabled(splittable, egui::Button::new("Split")).clicked() {
                if let Err(error) = editor.apply(Action::SplitAt { pos: playhead, track_ids: Some(vec![track_id]) }) {
                    messages.push(error.to_string());
                }
                close = true;
            }
            if ui.button("Delete").clicked() {
                if let Err(error) = editor.apply(Action::DeleteClips { clip_ids: delete_ids, ripple: editor.view.ripple }) {
                    messages.push(error.to_string());
                }
                close = true;
            }
            if ui.button("Clip Gain...").clicked() {
                dialogs::gain::request(ui.ctx(), clip.id, editor);
                close = true;
            }
            if ui.button("Reset Fades").clicked() {
                if let Err(error) = editor.apply(Action::SetClipFades { clip_id: clip.id, fade_in: 0, fade_out: 0 }) {
                    messages.push(error.to_string());
                }
                close = true;
            }
            if ui.button("Reveal in Bin").clicked() {
                bin_panel::request_reveal(ui.ctx(), media_id);
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
    save_clip_state(ui.ctx(), &state);
    messages
}

/// Draws the inline gain editor opened by double-clicking the header gain
/// value. Enter applies SetClipGain, Esc or a click elsewhere cancels; text
/// that does not parse does nothing. Returns action error messages.
pub fn draw_gain_edit(ui: &egui::Ui, editor: &mut Editor) -> Vec<String> {
    let mut state = clip_state(ui.ctx());
    let mut messages = Vec::new();
    let Some(mut edit) = state.gain_edit.take() else {
        return messages;
    };
    if !editor.project.tracks.iter().any(|track| track.clips.iter().any(|clip| clip.id == edit.clip_id)) {
        save_clip_state(ui.ctx(), &state);
        return messages;
    }
    let mut apply = None;
    let mut cancel = false;
    egui::Area::new(egui::Id::new("clip_gain_edit")).order(egui::Order::Foreground).fixed_pos(edit.pos).show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_width(THEME.gain_edit_w);
            let resp = ui_kit::text_field(ui, &mut edit.text, "dB");
            if !resp.has_focus() {
                resp.request_focus();
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter {
                if let Ok(db) = edit.text.trim().parse::<f32>() {
                    apply = Some(db);
                }
            } else if resp.lost_focus() && !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                // Focus lost to anything else: close without a change.
                cancel = true;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancel = true;
                ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
            }
        });
    });
    if let Some(db) = apply
        && let Err(error) = editor.apply(Action::SetClipGain { clip_id: edit.clip_id, db })
    {
        messages.push(error.to_string());
    }
    if apply.is_none() && !cancel {
        state.gain_edit = Some(edit);
    }
    save_clip_state(ui.ctx(), &state);
    messages
}
