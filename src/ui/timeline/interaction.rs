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
