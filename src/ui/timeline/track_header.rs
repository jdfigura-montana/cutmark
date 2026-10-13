//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: reorder grip, inline rename, context menu and bottom-edge resize.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::action::Action;

/// Width of the reorder grip at the header's left edge.
const GRIP_W: f32 = 10.0;
/// Height of the resize strip at the header's bottom edge.
const RESIZE_H: f32 = 4.0;
/// Key under which the interaction state is carried between frames.
const STATE_KEY: &str = "track_header_state";
/// Color of the reorder insertion line.
const INSERTION_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 200, 255);

/// Interaction state of the header column, carried between frames.
#[derive(Clone, Default)]
struct State {
    /// Track being renamed inline: id and current text.
    renaming: Option<(uuid::Uuid, String)>,
    /// Track being reordered by its grip: id and target insertion index.
    reorder: Option<(uuid::Uuid, usize)>,
}

/// Draws the header column: one header per track, scrolled with the lanes.
///
/// Each header has a reorder grip on the left, the track name (double-click
/// to rename inline), a context menu and a 4 px resize strip at the bottom.
/// `lane_rect` is only used to span the reorder insertion line across the
/// full timeline width.
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_y: f32,
) {
    let mut state = ui.ctx().data_mut(|d| {
        d.get_temp_mut_or_default::<State>(egui::Id::new(STATE_KEY))
            .clone()
    });

    // Escape cancels an in-progress reorder.
    if state.reorder.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.reorder = None;
    }

    // Snapshot the track list so the project can be mutated while drawing.
    let tracks: Vec<(uuid::Uuid, String, f32)> = editor
        .project
        .tracks
        .iter()
        .map(|t| (t.id, t.name.clone(), t.height))
        .collect();

    // State referring to deleted tracks goes away.
    if let Some((id, _)) = state.renaming
        && !tracks.iter().any(|(tid, _, _)| *tid == id)
    {
        state.renaming = None;
    }
    if let Some((id, _)) = state.reorder
        && !tracks.iter().any(|(tid, _, _)| *tid == id)
    {
        state.reorder = None;
    }

    let visuals = ui.visuals().clone();
    let painter = ui.painter_at(header_rect);
    painter.rect_filled(header_rect, 0.0, visuals.faint_bg_color);

    let mut y = header_rect.top() - scroll_y;
    for (index, (id, track_name, height)) in tracks.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(header_rect.left(), y),
            egui::vec2(header_rect.width(), *height),
        );
        y += *height;
        if rect.bottom() < header_rect.top() || rect.top() > header_rect.bottom() {
            continue;
        }

        // Reorder grip on the left edge.
        let grip_rect =
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(GRIP_W, rect.height()));
        let grip = ui.interact(
            grip_rect,
            egui::Id::new(("track_grip", *id)),
            egui::Sense::drag(),
        );
        let grip = grip.on_hover_cursor(egui::CursorIcon::Grab);
        let cx = rect.left() + GRIP_W * 0.5;
        let cy = rect.center().y;
        for dy in [-4.0, 0.0, 4.0] {
            painter.line_segment(
                [egui::pos2(cx - 3.0, cy + dy), egui::pos2(cx + 3.0, cy + dy)],
                visuals.widgets.noninteractive.fg_stroke,
            );
        }

        // Name area (everything but grip and resize strip).
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + GRIP_W, rect.top()),
            egui::pos2(rect.right(), rect.bottom() - RESIZE_H),
        );
        let name_resp = ui.interact(
            name_rect,
            egui::Id::new(("track_name", *id)),
            egui::Sense::click(),
        );
        // Resize strip at the bottom edge.
        let resize_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - RESIZE_H),
            egui::pos2(rect.right(), rect.bottom()),
        );
        let resize = ui.interact(
            resize_rect,
            egui::Id::new(("track_resize", *id)),
            egui::Sense::drag(),
        );
        let resize = resize.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resize.dragged() {
            let d = resize.drag_delta().y;
            if d != 0.0 {
                editor.set_track_height(*id, *height + d);
            }
        }

        // Right-click menu host for the whole header. Ctrl+click toggles the
        // track in or out of the current range selection.
        let host = ui.interact(
            rect,
            egui::Id::new(("track_host", *id)),
            egui::Sense::click(),
        );
        if host.clicked() && ui.input(|i| i.modifiers.ctrl) {
            editor.toggle_range_track(*id);
        }
        context_menu(editor, &host, &mut state, *id, index, track_name);
        context_menu(editor, &grip, &mut state, *id, index, track_name);
        context_menu(editor, &resize, &mut state, *id, index, track_name);

        // Reorder by dragging the grip.
        if let Some((drag_id, _)) = state.reorder
            && drag_id == *id
        {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let content_y = (p.y - header_rect.top() + scroll_y).max(0.0);
                let target = insert_index_at(&tracks, content_y);
                state.reorder = Some((*id, target));
            }
            if grip.drag_stopped() {
                let target = state.reorder.map(|(_, idx)| idx).unwrap_or(index);
                let new_index = if target > index { target - 1 } else { target };
                if new_index != index {
                    let _ = editor.apply(Action::MoveTrack { id: *id, new_index });
                }
                state.reorder = None;
            }
        } else if grip.drag_started() {
            state.reorder = Some((*id, index));
        }

        // Name: inline rename on double-click, plain label otherwise.
        if let Some((rid, text)) = state.renaming.as_mut()
            && *rid == *id
        {
            let edit = egui::TextEdit::singleline(text)
                .desired_rows(1)
                .desired_width(name_rect.width())
                .frame(egui::Frame::NONE);
            let resp = ui.put(name_rect.shrink2(egui::vec2(4.0, 2.0)), edit);
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && !text.trim().is_empty() && *text != *track_name {
                    let _ = editor.apply(Action::RenameTrack {
                        id: *id,
                        name: text.clone(),
                    });
                }
                state.renaming = None;
            }
        } else if name_resp.double_clicked() {
            state.renaming = Some((*id, track_name.clone()));
        } else {
            painter.text(
                egui::pos2(name_rect.left() + 8.0, name_rect.center().y),
                egui::Align2::LEFT_CENTER,
                track_name,
                egui::FontId::proportional(13.0),
                visuals.strong_text_color(),
            );
        }
    }

    // Insertion line while reordering, across headers and lanes.
    if let Some((_, target)) = state.reorder {
        let mut acc = 0.0;
        for (_, _, h) in tracks.iter().take(target.min(tracks.len())) {
            acc += *h;
        }
        let line_y = header_rect.top() - scroll_y + acc;
        if line_y >= header_rect.top() - 1.0 && line_y <= header_rect.bottom() + 1.0 {
            let line_painter = ui.painter_at(header_rect.union(lane_rect));
            line_painter.line_segment(
                [
                    egui::pos2(header_rect.left(), line_y),
                    egui::pos2(lane_rect.right(), line_y),
                ],
                egui::Stroke::new(2.0, INSERTION_COLOR),
            );
        }
    }

    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(STATE_KEY), state));
}

/// Right-click menu on one track header: Rename, Delete Track, Add Above/Below.
fn context_menu(
    editor: &mut Editor,
    resp: &egui::Response,
    state: &mut State,
    id: uuid::Uuid,
    index: usize,
    track_name: &str,
) {
    resp.context_menu(|menu| {
        if menu.button("Rename").clicked() {
            state.renaming = Some((id, track_name.to_string()));
            menu.close();
        }
        if menu.button("Delete Track").clicked() {
            let _ = editor.apply(Action::DeleteTrack { id });
            menu.close();
        }
        if menu.button("Add Track Above").clicked() {
            let _ = editor.apply(Action::AddTrack {
                index: Some(index),
                name: None,
            });
            menu.close();
        }
        if menu.button("Add Track Below").clicked() {
            let _ = editor.apply(Action::AddTrack {
                index: Some(index + 1),
                name: None,
            });
            menu.close();
        }
    });
}

/// Insertion boundary index (0..=count) for content-space y position.
fn insert_index_at(tracks: &[(uuid::Uuid, String, f32)], y_content: f32) -> usize {
    let mut acc = 0.0;
    let mut index = 0;
    for (_, _, h) in tracks {
        acc += *h;
        if y_content >= acc {
            index += 1;
        } else {
            break;
        }
    }
    index
}
