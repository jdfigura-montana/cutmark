//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: reorder grip, inline rename, context menu and bottom-edge resize,
//! plus volume, pan, mute and solo controls and a stereo meter.

use std::collections::HashMap;

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::theme::{THEME, Theme};
use crate::ui::widgets::fader;
use crate::ui::widgets::meter;

/// Key under which the interaction state is carried between frames.
const STATE_KEY: &str = "track_header_state";

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
    /// True while this track is armed for recording.
    armed: bool,
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
/// to rename inline), M, S and Arm buttons, a volume fader and pan slider
/// (from 64 px of height up), a stereo meter on the right edge, a context menu
/// and a 4 px resize strip at the bottom. `lane_rect` is only used to span the
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
    // While a take is recorded the headers ignore input (Arm, M, S, the
    // sliders, rename, reorder and the right-click menu); the meters keep
    // running and the resize strip still works.
    let locked = editor.is_recording();
    if locked {
        state.renaming = None;
        state.reorder = None;
    }

    // Escape cancels an in-progress reorder.
    if state.reorder.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.reorder = None;
    }

    // Any click on a track header clears a marker selection.
    if ui.input(|i| i.pointer.any_click())
        && ui
            .input(|i| i.pointer.latest_pos())
            .is_some_and(|p| header_rect.contains(p))
    {
        editor.select_marker(None);
    }

    // Snapshot the track list so the project can be mutated while drawing.
    let armed_track = editor.armed_track();
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
            armed: armed_track == Some(t.id),
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

    let painter = ui.painter_at(header_rect);
    painter.rect_filled(header_rect, 0.0, THEME.panel_bg);

    // A 4 px left stripe in each track's color identifies the track row.
    let stripe_w = THEME.space_sm;

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

        // 4 px left stripe in the track color.
        painter.rect_filled(
            egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + stripe_w, rect.bottom()),
            ),
            0.0,
            THEME.track_color(index),
        );

        // Layout: 9 px vertical padding, 10 px after the stripe, 8 px right;
        // meter bars hug the inner right edge, controls fill the rest.
        let compact = rect.height() < THEME.compact_track_h;
        let inner = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + stripe_w + THEME.track_pad_l,
                rect.top() + THEME.track_pad_v,
            ),
            egui::pos2(
                rect.right() - THEME.track_pad_r,
                rect.bottom() - THEME.track_pad_v,
            ),
        );

        // Meter bars: two 4 px bars, 2 px gap, taking the rightmost 10 px of
        // the inner rect. The 4 px clip lights sit above them, inside the top
        // padding band.
        let bars_w = 2.0 * THEME.track_meter_bar_w + THEME.track_meter_gap;
        let bars_top = inner.top() + THEME.track_clip_light + THEME.track_meter_gap;
        let meter_rect = egui::Rect::from_min_max(
            egui::pos2(inner.right() - bars_w, bars_top),
            egui::pos2(inner.right(), inner.bottom()),
        );
        // Everything to the left of the meters stops a 6 px gap before them,
        // so no control or value text can ever reach the meter bars.
        let ctrl_right = inner.right() - bars_w - 6.0;

        // Row 1: drag handle, name, then M, S and Arm buttons, left to right.
        let row1 = egui::Rect::from_min_max(
            egui::pos2(inner.left(), inner.top()),
            egui::pos2(ctrl_right, inner.top() + THEME.track_ms_h),
        );
        let arm_rect = egui::Rect::from_min_size(
            egui::pos2(row1.right() - THEME.track_ms_w, row1.top()),
            egui::vec2(THEME.track_ms_w, THEME.track_ms_h),
        );
        let s_rect = egui::Rect::from_min_size(
            egui::pos2(
                arm_rect.left() - THEME.track_ms_gap - THEME.track_ms_w,
                row1.top(),
            ),
            egui::vec2(THEME.track_ms_w, THEME.track_ms_h),
        );
        let m_rect = egui::Rect::from_min_size(
            egui::pos2(
                s_rect.left() - THEME.track_ms_gap - THEME.track_ms_w,
                row1.top(),
            ),
            egui::vec2(THEME.track_ms_w, THEME.track_ms_h),
        );
        let grip_w = 14.0_f32.min(THEME.track_pad_l + stripe_w);
        let grip_rect = egui::Rect::from_min_size(
            egui::pos2(inner.left(), row1.top()),
            egui::vec2(grip_w, row1.height()),
        );
        let name_rect = egui::Rect::from_min_max(
            egui::pos2(grip_rect.right() + 2.0, row1.top()),
            egui::pos2(m_rect.left() - 4.0, row1.bottom()),
        );

        // Rows 2 and 3: VOL and PAN with fixed label and value columns. The
        // two slider rows share the space under row 1 evenly, 20 px at the
        // default 96 px header height.
        let row_h = (((inner.height() - row1.height() - 2.0 * THEME.track_row_gap) * 0.5).floor())
            .clamp(10.0, THEME.track_ms_h);
        let vol_row = egui::Rect::from_min_max(
            egui::pos2(inner.left(), row1.bottom() + THEME.track_row_gap),
            egui::pos2(ctrl_right, row1.bottom() + THEME.track_row_gap + row_h),
        );
        let pan_row = egui::Rect::from_min_max(
            egui::pos2(inner.left(), vol_row.bottom() + THEME.track_row_gap),
            egui::pos2(ctrl_right, vol_row.bottom() + THEME.track_row_gap + row_h),
        );
        let label_col = THEME.track_label_col_w;
        let value_col = THEME.track_value_col_w;
        // The slider track starts 6 px after the label column and ends 6 px
        // before the value column, so the thumb never overlaps either text.
        let vol_rect = egui::Rect::from_min_max(
            egui::pos2(vol_row.left() + label_col + 6.0, vol_row.top()),
            egui::pos2(vol_row.right() - value_col - 6.0, vol_row.bottom()),
        );
        let pan_rect = egui::Rect::from_min_max(
            egui::pos2(pan_row.left() + label_col + 6.0, pan_row.top()),
            egui::pos2(pan_row.right() - value_col - 6.0, pan_row.bottom()),
        );

        // Reorder by dragging the grip; the phosphor dots show on hover.
        let grip = ui.interact(
            grip_rect,
            egui::Id::new(("track_grip", ht.id)),
            egui::Sense::drag(),
        );
        let grip = grip.on_hover_cursor(egui::CursorIcon::Grab);
        // Resize strip at the bottom edge.
        let resize_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - THEME.track_resize_h),
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
                    || arm_rect.contains(p)
                    || (!compact && (vol_rect.contains(p) || pan_rect.contains(p)))
            });
            if !over_controls {
                editor.toggle_range_track(ht.id);
            }
        }
        context_menu(editor, &host, &mut state, ht.id, index, &ht.name, locked);
        context_menu(editor, &grip, &mut state, ht.id, index, &ht.name, locked);
        context_menu(editor, &resize, &mut state, ht.id, index, &ht.name, locked);

        // Phosphor drag handle, visible only while the header is hovered.
        if ui.rect_contains_pointer(rect) {
            painter.text(
                grip_rect.center(),
                egui::Align2::CENTER_CENTER,
                egui_phosphor::fill::DOTS_SIX_VERTICAL,
                egui::FontId::proportional(14.0),
                THEME.text_faint,
            );
        }

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
        } else if grip.drag_started() && !locked {
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
            let resp = ui.put(
                name_rect.shrink2(egui::vec2(THEME.space_sm, THEME.space_xs)),
                edit,
            );
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
        } else if name_resp.double_clicked() && !locked {
            state.renaming = Some((ht.id, ht.name.clone()));
        } else {
            // Name in the track color text; faint while the track is muted.
            // Clipped so long names truncate instead of overlapping the
            // M and S buttons.
            let name_color = if ht.mute {
                THEME.text_faint
            } else {
                THEME.text_primary
            };
            painter.with_clip_rect(name_rect).text(
                egui::pos2(name_rect.left(), name_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &ht.name,
                Theme::semibold(THEME.track_name_size),
                name_color,
            );
        }

        // Mute and solo toggles; Ctrl+click solo is exclusive.
        if toggle_button(
            ui,
            m_rect,
            egui::Id::new(("track_mute", ht.id)),
            "M",
            ht.mute,
        ) && !locked
        {
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
        ) && !locked
        {
            if ui.input(|i| i.modifiers.ctrl) {
                let _ = editor.apply(Action::SoloExclusive { id: ht.id });
            } else {
                let _ = editor.apply(Action::SetTrackSolo {
                    id: ht.id,
                    solo: !ht.solo,
                });
            }
        }
        // Arm for recording; Ctrl+click is a plain click here.
        if arm_button(ui, arm_rect, egui::Id::new(("track_arm", ht.id)), ht.armed) && !locked {
            editor.toggle_arm(ht.id);
        }

        // Volume and pan rows, 64 px of header height and up. Each drag is
        // one gesture: begin, preview every frame, end on release. The value
        // sits in its own fixed column, so nothing overlaps the thumb.
        if !compact {
            painter.text(
                egui::pos2(vol_row.left(), vol_row.center().y),
                egui::Align2::LEFT_CENTER,
                "VOL",
                egui::FontId::proportional(THEME.track_label_size),
                THEME.text_faint,
            );
            painter.text(
                egui::pos2(pan_row.left(), pan_row.center().y),
                egui::Align2::LEFT_CENTER,
                "PAN",
                egui::FontId::proportional(THEME.track_label_size),
                THEME.text_faint,
            );
            let out = fader::slider_bare(
                ui,
                egui::Id::new(("track_volume", ht.id)),
                vol_rect,
                ht.volume_db,
                (-60.0, 12.0),
                false,
            );
            if !locked {
                apply_outcome(
                    editor,
                    &out,
                    |db| Action::SetTrackVolume { id: ht.id, db },
                    0.0,
                );
            }
            painter.text(
                egui::pos2(vol_row.right(), vol_row.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{:+.1} dB", ht.volume_db.max(-60.0)),
                Theme::mono(THEME.track_value_size),
                THEME.text_normal,
            );
            let out = fader::slider_bare(
                ui,
                egui::Id::new(("track_pan", ht.id)),
                pan_rect,
                ht.pan,
                (-1.0, 1.0),
                true,
            );
            if !locked {
                apply_outcome(
                    editor,
                    &out,
                    |p| Action::SetTrackPan { id: ht.id, pan: p },
                    0.0,
                );
            }
            painter.text(
                egui::pos2(pan_row.right(), pan_row.center().y),
                egui::Align2::RIGHT_CENTER,
                fader::pan_text(ht.pan),
                Theme::mono(THEME.track_value_size),
                THEME.text_normal,
            );
        }

        // Compact meter bars down the inner right edge; click clears clips.
        let channels = meters.entry(ht.id).or_default();
        meter::header_meter(
            ui,
            egui::Id::new(("track_meter", ht.id)),
            meter_rect,
            channels,
        );

        // 1 px border-colored separator under the header, across the lane.
        if rect.bottom() <= lane_rect.bottom() {
            ui.painter_at(header_rect.union(lane_rect)).line_segment(
                [
                    egui::pos2(rect.left(), rect.bottom()),
                    egui::pos2(lane_rect.right(), rect.bottom()),
                ],
                egui::Stroke::new(THEME.hairline, THEME.border),
            );
        }
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
                egui::Stroke::new(THEME.reorder_line_w, THEME.reorder_line_color),
            );
        }
    }

    ui.ctx()
        .data_mut(|d| d.insert_temp(egui::Id::new(STATE_KEY), state));
}

/// Right-click menu on one track header: Rename, Delete Track, Add Above/Below.
/// While a take is recorded (`locked`) every item is disabled.
fn context_menu(
    editor: &mut Editor,
    resp: &egui::Response,
    state: &mut State,
    id: Uuid,
    index: usize,
    track_name: &str,
    locked: bool,
) {
    egui::Popup::context_menu(resp)
        .style(crate::ui::theme::menu_item_style as fn(&mut egui::Style))
        .show(|ui| {
            if locked {
                ui.disable();
            }
            if ui.button("Rename").clicked() {
                state.renaming = Some((id, track_name.to_string()));
                ui.close();
            }
            if ui.button("Delete Track").clicked() {
                let _ = editor.apply(Action::DeleteTrack { id });
                ui.close();
            }
            if ui.button("Add Track Above").clicked() {
                let _ = editor.apply(Action::AddTrack {
                    index: Some(index),
                    name: None,
                });
                ui.close();
            }
            if ui.button("Add Track Below").clicked() {
                let _ = editor.apply(Action::AddTrack {
                    index: Some(index + 1),
                    name: None,
                });
                ui.close();
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
            THEME.solo_color
        } else {
            THEME.mute_color
        };
        (bg, THEME.app_bg)
    } else if resp.hovered() {
        // Hover highlight on the toggle.
        (visuals.widgets.hovered.bg_fill, visuals.strong_text_color())
    } else {
        (THEME.control_fill, THEME.text_muted)
    };
    painter.rect_filled(rect, THEME.track_ms_radius, bg);
    painter.rect_stroke(
        rect,
        THEME.track_ms_radius,
        egui::Stroke::new(THEME.hairline, THEME.control_border),
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(THEME.toggle_size),
        fg,
    );
    resp.clicked()
}

/// Draws the round Arm button (a record glyph); returns true when clicked this
/// frame. Off it looks like M / S off; armed it is `record_red` with a white
/// glyph.
fn arm_button(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, armed: bool) -> bool {
    let painter = ui.painter_at(rect);
    let resp = ui.interact(rect, id, egui::Sense::click());
    let (bg, fg) = if armed {
        (THEME.record_red, THEME.on_record_glyph)
    } else if resp.hovered() {
        (
            ui.visuals().widgets.hovered.bg_fill,
            ui.visuals().strong_text_color(),
        )
    } else {
        (THEME.control_fill, THEME.text_muted)
    };
    // A pill as tall as the button: round, and the same 22x20 slot as M / S.
    let radius = rect.height() * 0.5;
    painter.rect_filled(rect, radius, bg);
    if !armed {
        painter.rect_stroke(
            rect,
            radius,
            egui::Stroke::new(THEME.hairline, THEME.control_border),
            egui::StrokeKind::Inside,
        );
    }
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        egui_phosphor::fill::RECORD,
        egui::FontId::proportional(THEME.arm_glyph_size),
        fg,
    );
    let clicked = resp.clicked();
    resp.on_hover_text("Arm for recording");
    clicked
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::fixtures::project_with_clips;
    use crate::model::project::ViewState;
    use crate::ui::test_util::{Headless, Out};

    /// Slack for float rounding when comparing painted positions.
    const TOL: f32 = 0.6;

    /// An editor over `count` empty tracks (no input device is ever opened).
    fn editor(count: usize) -> Editor {
        let tracks: Vec<&[(u64, u64)]> = vec![&[]; count];
        Editor::with_project(
            project_with_clips(&tracks),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        )
    }

    /// The header column as the app lays it out: 208 px wide, lanes beside it.
    fn rects() -> (egui::Rect, egui::Rect) {
        (
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(208.0, 600.0)),
            egui::Rect::from_min_size(egui::pos2(208.0, 0.0), egui::vec2(900.0, 600.0)),
        )
    }

    /// Centers of the painted record glyphs: one Arm button per track, top to
    /// bottom.
    fn arm_centers(out: &Out) -> Vec<egui::Pos2> {
        out.texts()
            .into_iter()
            .filter(|t| t.text == egui_phosphor::fill::RECORD)
            .map(|t| t.rect.center())
            .collect()
    }

    /// The painted red arm pills (22x20, `record_red`).
    fn red_pills(out: &Out) -> Vec<egui::Rect> {
        out.rects()
            .into_iter()
            .filter(|r| r.fill == THEME.record_red && r.rect.size() == egui::vec2(22.0, 20.0))
            .map(|r| r.rect)
            .collect()
    }

    #[test]
    fn clicking_arm_toggles_the_armed_track_and_moves_it_between_tracks() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(2);
        let (a, b) = (editor.project.tracks[0].id, editor.project.tracks[1].id);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let mut draw = |ui: &mut egui::Ui, editor: &mut Editor| {
            show(ui, editor, header, lane, 0.0, &mut meters);
        };

        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        let centers = arm_centers(&out);
        assert_eq!(centers.len(), 2, "one Arm button per track");
        assert!(red_pills(&out).is_empty(), "nothing armed yet");
        assert_eq!(editor.armed_track(), None);

        // Arm the first track: it turns red.
        let _ = h.click(centers[0], |ui| draw(ui, &mut editor));
        assert_eq!(editor.armed_track(), Some(a));
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        assert_eq!(red_pills(&out).len(), 1, "one red Arm button");
        let glyph = out
            .texts()
            .into_iter()
            .find(|t| t.text == egui_phosphor::fill::RECORD && t.rect.center() == centers[0])
            .expect("armed glyph");
        assert_eq!(glyph.color, egui::Color32::WHITE, "armed = white glyph");

        // Arming the second moves the arm.
        let _ = h.click(centers[1], |ui| draw(ui, &mut editor));
        assert_eq!(editor.armed_track(), Some(b));
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        let pills = red_pills(&out);
        assert_eq!(pills.len(), 1);
        assert!(pills[0].contains(centers[1]), "the second track is red");

        // Clicking the armed one disarms it.
        let _ = h.click(centers[1], |ui| draw(ui, &mut editor));
        assert_eq!(editor.armed_track(), None);
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        assert!(red_pills(&out).is_empty());
        assert!(!editor.dirty, "arming is never an edit");
        assert_eq!(editor.undo_label(), None);
    }

    #[test]
    fn arm_sits_after_m_and_s_with_the_same_gap_and_size() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let m = out.text_rect("M").expect("M button").center();
        let s = out.text_rect("S").expect("S button").center();
        let arm = arm_centers(&out)[0];
        let pitch = THEME.track_ms_w + THEME.track_ms_gap;
        assert!(m.x < s.x && s.x < arm.x, "M, S, Arm left to right");
        assert!((s.x - m.x - pitch).abs() < TOL, "M to S {}", s.x - m.x);
        assert!(
            (arm.x - s.x - pitch).abs() < TOL,
            "S to Arm {}",
            arm.x - s.x
        );
        assert!((m.y - arm.y).abs() < TOL && (m.y - s.y).abs() < TOL);
        // The button is the same 22x20 as M and S, and round (a full-height pill).
        let pill = out
            .rects()
            .into_iter()
            .find(|r| r.rect.contains(arm) && r.rect.size() == egui::vec2(22.0, 20.0))
            .expect("the Arm button background");
        assert_eq!(pill.fill, THEME.control_fill, "off = control fill");
        assert_eq!(pill.corner_radius.nw, (THEME.track_ms_h * 0.5) as u8);
        // The off glyph is text_muted.
        let glyph = out
            .texts()
            .into_iter()
            .find(|t| t.text == egui_phosphor::fill::RECORD)
            .expect("record glyph");
        assert_eq!(glyph.color, THEME.text_muted);
    }

    #[test]
    fn the_compact_header_keeps_m_s_and_arm() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let id = editor.project.tracks[0].id;
        editor.set_track_height(id, 40.0);
        assert!(editor.project.tracks[0].height < THEME.compact_track_h);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        assert!(out.text_rect("M").is_some());
        assert!(out.text_rect("S").is_some());
        assert_eq!(arm_centers(&out).len(), 1);
        assert!(out.text_rect("VOL").is_none(), "compact has no VOL row");
    }

    #[test]
    fn the_name_stays_left_of_the_buttons() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let name = out.text_rect("Track 1").expect("track name");
        let m = out.text_rect("M").expect("M button");
        assert!(name.right() < m.left(), "name {name:?} left of M {m:?}");
    }

    // ---- While a take is recorded ----

    /// Centers of the painted "M" buttons, top to bottom.
    fn m_centers(out: &Out) -> Vec<egui::Pos2> {
        let mut found: Vec<_> = out
            .texts()
            .into_iter()
            .filter(|t| t.text == "M")
            .map(|t| t.rect.center())
            .collect();
        found.sort_by(|a, b| a.y.total_cmp(&b.y));
        found
    }

    #[test]
    fn arm_and_mute_ignore_clicks_while_recording_and_work_otherwise() {
        for recording in [false, true] {
            let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[], &[]], "Rec");
            let (a, b) = (editor.project.tracks[0].id, editor.project.tracks[1].id);
            if recording {
                editor.start_recording().expect("a take");
            }
            let mut h = Headless::new(1280.0, 800.0);
            let (header, lane) = rects();
            let mut meters = HashMap::new();
            let mut draw = |ui: &mut egui::Ui, editor: &mut Editor| {
                show(ui, editor, header, lane, 0.0, &mut meters);
            };
            let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
            let arms = arm_centers(&out);
            let ms = m_centers(&out);

            let _ = h.click(arms[1], |ui| draw(ui, &mut editor));
            let _ = h.click(ms[1], |ui| draw(ui, &mut editor));
            let moved = editor.armed_track() == Some(b);
            let muted = editor.project.tracks[1].mute;
            if recording {
                assert_eq!(editor.armed_track(), Some(a), "Arm ignored");
                assert!(!muted, "M ignored");
                assert!(editor.take_is_live());
            } else {
                assert!(moved, "Arm moved");
                assert!(muted, "M muted the track");
            }
        }
    }

    #[test]
    fn the_meters_keep_drawing_while_recording() {
        let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[]], "Rec");
        editor.start_recording().expect("a take");
        let mut h = Headless::new(1280.0, 800.0);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let id = editor.project.tracks[0].id;
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        assert!(meters.contains_key(&id), "the header meter is alive");
        assert!(out.text_rect("Track 1").is_some());
    }
}
