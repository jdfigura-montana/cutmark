//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: reorder grip, inline rename, context menu and bottom-edge resize,
//! plus volume, pan, mute and solo controls and a stereo meter.

use std::collections::HashMap;

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::widgets::fader;
use crate::ui::widgets::meter;

/// Width of the reorder grip at the header's left edge.
const GRIP_W: f32 = 10.0;
/// Height of the resize strip at the header's bottom edge.
const RESIZE_H: f32 = 4.0;
/// Width of the stereo meter at the header's right edge.
const METER_W: f32 = 14.0;
/// Height of the name row.
const NAME_H: f32 = 18.0;
/// Height of one control row (fader or pan).
const CTRL_H: f32 = 14.0;
/// Width of an M / S toggle button.
const MS_W: f32 = 16.0;
/// Key under which the interaction state is carried between frames.
const STATE_KEY: &str = "track_header_state";
/// Color of the reorder insertion line.
const INSERTION_COLOR: egui::Color32 = egui::Color32::from_rgb(120, 200, 255);

/// Snapshot of one track's header-relevant data for one frame.
struct HeaderTrack {
    /// Track id.
    id: Uuid,
    /// Track name.
    name: String,
    /// Header height in points.
    height: f32,
    /// Volume in dB.
    volume_db: f32,
    /// Pan, -1 to +1.
    pan: f32,
    /// Mute flag.
    mute: bool,
    /// Solo flag.
    solo: bool,
}

/// Interaction state of the header column, carried between frames.
#[derive(Clone, Default)]
struct State {
    /// Track being renamed inline: id and current text.
    renaming: Option<(Uuid, String)>,
    /// Track being reordered by its grip: id and target insertion index.
    reorder: Option<(Uuid, usize)>,
}

/// Draws the header column: one header per track, scrolled with the lanes.
///
/// Each header has a reorder grip on the left, the track name (double-click
/// to rename inline), M and S buttons, a volume fader and pan slider (from
/// 64 px of height up), a stereo meter on the right edge, a context menu and
/// a 4 px resize strip at the bottom. `lane_rect` is only used to span the
/// reorder insertion line across the full timeline width.
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_y: f32,
    meters: &mut HashMap<Uuid, [meter::ChannelBallistics; 2]>,
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
    let tracks: Vec<HeaderTrack> = editor
        .project
        .tracks
        .iter()
        .map(|t| HeaderTrack {
            id: t.id,
            name: t.name.clone(),
            height: t.height,
            volume_db: t.volume_db,
            pan: t.pan,
            mute: t.mute,
            solo: t.solo,
        })
        .collect();

    // State referring to deleted tracks goes away.
    if let Some((id, _)) = state.renaming
        && !tracks.iter().any(|t| t.id == id)
    {
        state.renaming = None;
    }
    if let Some((id, _)) = state.reorder
        && !tracks.iter().any(|t| t.id == id)
    {
        state.reorder = None;
    }

    let visuals = ui.visuals().clone();
    let painter = ui.painter_at(header_rect);
    painter.rect_filled(header_rect, 0.0, visuals.faint_bg_color);

    let mut y = header_rect.top() - scroll_y;
    for (index, ht) in tracks.iter().enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(header_rect.left(), y),
            egui::vec2(header_rect.width(), ht.height),
        );
        y += ht.height;
        if rect.bottom() < header_rect.top() || rect.top() > header_rect.bottom() {
            continue;
        }

        // Layout: meter on the right edge, controls to its left.
        let meter_rect = egui::Rect::from_min_max(
            egui::pos2(rect.right() - METER_W, rect.top()),
            egui::pos2(rect.right(), rect.bottom() - RESIZE_H),
        );
        let compact = rect.height() < 64.0;
        let ctrl_left = rect.left() + GRIP_W + 4.0;
        let ctrl_right = meter_rect.left() - 4.0;
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(ctrl_left, rect.top() + 2.0),
            egui::pos2(ctrl_right, rect.top() + 2.0 + NAME_H),
        );
        let (m_rect, s_rect, vol_rect, pan_rect) = if compact {
            let row_y = name_rect.bottom() + 2.0;
            (
                egui::Rect::from_min_size(egui::pos2(ctrl_left, row_y), egui::vec2(MS_W, CTRL_H)),
                egui::Rect::from_min_size(
                    egui::pos2(ctrl_left + MS_W + 4.0, row_y),
                    egui::vec2(MS_W, CTRL_H),
                ),
                name_rect,
                name_rect,
            )
        } else {
            let top = name_rect.top() + 2.0;
            (
                egui::Rect::from_min_size(
                    egui::pos2(ctrl_right - 2.0 * MS_W - 4.0, top),
                    egui::vec2(MS_W, NAME_H - 4.0),
                ),
                egui::Rect::from_min_size(
                    egui::pos2(ctrl_right - MS_W, top),
                    egui::vec2(MS_W, NAME_H - 4.0),
                ),
                egui::Rect::from_min_max(
                    egui::pos2(ctrl_left, name_rect.bottom() + 4.0),
                    egui::pos2(ctrl_right, name_rect.bottom() + 4.0 + CTRL_H),
                ),
                egui::Rect::from_min_max(
                    egui::pos2(ctrl_left, name_rect.bottom() + 8.0 + CTRL_H),
                    egui::pos2(ctrl_right, name_rect.bottom() + 8.0 + 2.0 * CTRL_H),
                ),
            )
        };

        // Reorder grip on the left edge.
        let grip_rect =
            egui::Rect::from_min_size(rect.left_top(), egui::vec2(GRIP_W, rect.height()));
        let grip = ui.interact(
            grip_rect,
            egui::Id::new(("track_grip", ht.id)),
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
        // Resize strip at the bottom edge.
        let resize_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - RESIZE_H),
            egui::pos2(rect.right(), rect.bottom()),
        );
        let resize = ui.interact(
            resize_rect,
            egui::Id::new(("track_resize", ht.id)),
            egui::Sense::drag(),
        );
        let resize = resize.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resize.dragged() {
            let d = resize.drag_delta().y;
            if d != 0.0 {
                editor.set_track_height(ht.id, ht.height + d);
            }
        }

        // Host for the context menu; Ctrl+click toggles the track in or out
        // of the current range selection, unless the click hit a control.
        let host = ui.interact(
            rect,
            egui::Id::new(("track_host", ht.id)),
            egui::Sense::click(),
        );
        if host.clicked() && ui.input(|i| i.modifiers.ctrl) {
            let over_controls = host.interact_pointer_pos().is_some_and(|p| {
                meter_rect.contains(p)
                    || m_rect.contains(p)
                    || s_rect.contains(p)
                    || (!compact && (vol_rect.contains(p) || pan_rect.contains(p)))
            });
            if !over_controls {
                editor.toggle_range_track(ht.id);
            }
        }
        context_menu(editor, &host, &mut state, ht.id, index, &ht.name);
        context_menu(editor, &grip, &mut state, ht.id, index, &ht.name);
        context_menu(editor, &resize, &mut state, ht.id, index, &ht.name);

        // Reorder by dragging the grip.
        if let Some((drag_id, _)) = state.reorder
            && drag_id == ht.id
        {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
                let content_y = (p.y - header_rect.top() + scroll_y).max(0.0);
                let target = insert_index_at(&tracks, content_y);
                state.reorder = Some((ht.id, target));
            }
            if grip.drag_stopped() {
                let target = state.reorder.map(|(_, idx)| idx).unwrap_or(index);
                let new_index = if target > index { target - 1 } else { target };
                if new_index != index {
                    let _ = editor.apply(Action::MoveTrack {
                        id: ht.id,
                        new_index,
                    });
                }
                state.reorder = None;
            }
        } else if grip.drag_started() {
            state.reorder = Some((ht.id, index));
        }
        // Name: inline rename on double-click, plain label otherwise.
        let name_resp = ui.interact(
            name_rect,
            egui::Id::new(("track_name", ht.id)),
            egui::Sense::click(),
        );
        if let Some((rid, text)) = state.renaming.as_mut()
            && *rid == ht.id
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
                if enter && !text.trim().is_empty() && *text != ht.name {
                    let _ = editor.apply(Action::RenameTrack {
                        id: ht.id,
                        name: text.clone(),
                    });
                }
                state.renaming = None;
            }
        } else if name_resp.double_clicked() {
            state.renaming = Some((ht.id, ht.name.clone()));
        } else {
            painter.text(
                egui::pos2(name_rect.left() + 4.0, name_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &ht.name,
                egui::FontId::proportional(13.0),
                visuals.strong_text_color(),
            );
        }

        // Mute and solo toggles; Ctrl+click solo is exclusive.
        if toggle_button(
            ui,
            m_rect,
            egui::Id::new(("track_mute", ht.id)),
            "M",
            ht.mute,
        ) {
            let _ = editor.apply(Action::SetTrackMute {
                id: ht.id,
                mute: !ht.mute,
            });
        }
        if toggle_button(
            ui,
            s_rect,
            egui::Id::new(("track_solo", ht.id)),
            "S",
            ht.solo,
        ) {
            if ui.input(|i| i.modifiers.ctrl) {
                let _ = editor.apply(Action::SoloExclusive { id: ht.id });
            } else {
                let _ = editor.apply(Action::SetTrackSolo {
                    id: ht.id,
                    solo: !ht.solo,
                });
            }
        }

        // Volume fader and pan slider, 64 px of header height and up. Each
        // drag is one gesture: begin, preview every frame, end on release.
        if !compact {
            let out = fader::slider(
                ui,
                egui::Id::new(("track_volume", ht.id)),
                vol_rect,
                ht.volume_db,
                (-60.0, 12.0),
                false,
                fader::db_text,
            );
            apply_outcome(
                editor,
                &out,
                |db| Action::SetTrackVolume { id: ht.id, db },
                0.0,
            );
            let out = fader::slider(
                ui,
                egui::Id::new(("track_pan", ht.id)),
                pan_rect,
                ht.pan,
                (-1.0, 1.0),
                false,
                fader::pan_text,
            );
            apply_outcome(
                editor,
                &out,
                |p| Action::SetTrackPan { id: ht.id, pan: p },
                0.0,
            );
        }

        // Stereo meter down the right edge.
        let channels = meters.entry(ht.id).or_default();
        meter::stereo_meter(
            ui,
            egui::Id::new(("track_meter", ht.id)),
            meter_rect,
            channels,
            false,
        );
    }

    // Insertion line while reordering, across headers and lanes.
    if let Some((_, target)) = state.reorder {
        let mut acc = 0.0;
        for t in tracks.iter().take(target.min(tracks.len())) {
            acc += t.height;
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
    id: Uuid,
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

/// Draws a small M / S toggle button; returns true when clicked this frame.
fn toggle_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    label: &str,
    active: bool,
) -> bool {
    let painter = ui.painter_at(rect);
    let resp = ui.interact(rect, id, egui::Sense::click());
    let visuals = ui.visuals().clone();
    let (bg, fg) = if active {
        let bg = if label == "S" {
            egui::Color32::from_rgb(70, 130, 220)
        } else {
            egui::Color32::from_rgb(200, 140, 50)
        };
        (bg, egui::Color32::from_rgb(20, 22, 26))
    } else {
        (
            visuals.widgets.inactive.bg_fill,
            visuals.strong_text_color(),
        )
    };
    painter.rect_filled(rect, 3.0, bg);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        fg,
    );
    resp.clicked()
}

/// Applies one slider outcome to the editor as a gesture on `make(v)`.
///
/// Double-click resets to `default`; a drag is one undo step; Esc cancels.
fn apply_outcome(
    editor: &mut Editor,
    out: &fader::SliderOutcome,
    make: impl Fn(f32) -> Action,
    default: f32,
) {
    if out.reset {
        let _ = editor.apply(make(default));
        return;
    }
    if out.started {
        editor.begin_gesture();
    }
    if let Some(v) = out.value {
        let _ = editor.preview(make(v));
    }
    if out.ended {
        editor.end_gesture();
    }
    if out.cancelled {
        editor.cancel_gesture();
    }
}

/// Insertion boundary index (0..=count) for content-space y position.
fn insert_index_at(tracks: &[HeaderTrack], y_content: f32) -> usize {
    let mut acc = 0.0;
    let mut index = 0;
    for t in tracks {
        acc += t.height;
        if y_content >= acc {
            index += 1;
        } else {
            break;
        }
    }
    index
}
