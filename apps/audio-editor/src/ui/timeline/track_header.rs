//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: the ui-kit track header (number, name, M, S and R toggles, compact
//! meter) plus reorder grip, inline rename, context menu, bottom-edge resize
//! and the volume and pan sliders.

use std::collections::HashMap;
use std::ops::RangeInclusive;

use eframe::egui;
use ui_kit::icons::regular;
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{SliderKind, Toggle, ToggleKind};
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::theme::THEME;
use crate::ui::widgets::fader;
use crate::ui::widgets::meter;

/// Key under which the interaction state is carried between frames.
const STATE_KEY: &str = "track_header_state";
/// Width of the reorder grip at the right end of the name row.
const GRIP_W: f32 = 14.0;
/// Height of the toggle row of ui-kit's track header (spec "Track header":
/// 24 x 24 toggles). The slider row sits below the header content, so this
/// mirrors that layout.
const TOGGLE_ROW_H: f32 = 24.0;
/// Height of the compact meter of ui-kit's track header (spec "Meter": 6 px).
const METER_H: f32 = 6.0;

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
/// Each header is a ui-kit track header (number, name, M, S and R toggles,
/// compact level meter) with a reorder grip at the right end of the name row
/// (double-click the name to rename inline), a volume and a pan slider side
/// by side below it when they fit, a context menu and a 4 px resize strip at
/// the bottom. `lane_rect` is only used to span the reorder insertion line
/// across the full timeline width.
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_y: f32,
    meters: &mut HashMap<Uuid, [meter::ChannelBallistics; 2]>,
) {
    let mut state = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<State>(egui::Id::new(STATE_KEY)).clone());
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
    if ui.input(|i| i.pointer.any_click()) && ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| header_rect.contains(p)) {
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
    let selected_tracks = super::range_tracks(editor);

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
    let ctrl = ui.input(|i| i.modifiers.ctrl);

    let mut y = header_rect.top() - scroll_y;
    for (index, ht) in tracks.iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(header_rect.left(), y), egui::vec2(header_rect.width(), ht.height));
        y += ht.height;
        if rect.bottom() < header_rect.top() || rect.top() > header_rect.bottom() {
            continue;
        }

        // Row background across the header column (the lanes paint their own).
        let selected = selected_tracks.contains(&ht.id);
        ui_kit::paint_track_row(&painter, rect, index, selected);

        // Geometry of the ui-kit header's content, mirrored for the controls
        // that sit on it: the first row (number and name), then the toggles,
        // the meter and, when they fit, the slider row.
        let pad = space::SCALE_2;
        let number_galley = painter.layout_no_wrap((index + 1).to_string(), ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY);
        let name_galley = painter.layout_no_wrap(ht.name.clone(), ui_kit::fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
        let row_h = name_galley.size().y.max(number_galley.size().y);
        let first_row = egui::Rect::from_min_size(egui::pos2(rect.left() + pad, rect.top() + pad), egui::vec2(rect.width() - 2.0 * pad, row_h));
        let grip_rect = egui::Rect::from_min_max(egui::pos2(first_row.right() - GRIP_W, first_row.top()), first_row.right_bottom());
        let name_rect =
            egui::Rect::from_min_max(egui::pos2(first_row.left() + number_galley.size().x + space::SCALE_8, first_row.top()), grip_rect.left_bottom());
        let slider_h = size::SEGMENT_HEIGHT;
        let slider_top = first_row.bottom() + pad + TOGGLE_ROW_H + pad + METER_H + pad;
        let show_sliders = rect.height() >= THEME.compact_track_h && slider_top + slider_h + THEME.track_resize_h <= rect.bottom();
        let renaming_here = state.renaming.as_ref().is_some_and(|(rid, _)| *rid == ht.id);

        // The ui-kit header: the toggles edit local copies, applied below as
        // the same editor actions the old buttons ran. The name is left to the
        // inline editor while it is open.
        let mut mute = ht.mute;
        let mut solo = ht.solo;
        let mut armed = ht.armed;
        let reading = {
            let channels = meters.entry(ht.id).or_default();
            (channels[0].peak_db.max(channels[1].peak_db), channels[0].clip || channels[1].clip)
        };
        let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("track_header_ui", ht.id)));
        header_ui.set_clip_rect(rect.intersect(header_rect));
        let host = {
            let mut toggles = [
                Toggle::visible("M", &mut mute),
                Toggle::visible("S", &mut solo),
                Toggle { label: "R", on: &mut armed, kind: ToggleKind::RecordArm, always_visible: true },
            ];
            ui_kit::track_header(&mut header_ui, index + 1, if renaming_here { "" } else { &ht.name }, selected, &mut toggles, Some(reading))
        };

        // Resize strip at the bottom edge.
        let resize_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - THEME.track_resize_h), egui::pos2(rect.right(), rect.bottom()));
        let resize = ui.interact(resize_rect, egui::Id::new(("track_resize", ht.id)), egui::Sense::drag());
        let resize = resize.on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resize.dragged() {
            let d = resize.drag_delta().y;
            if d != 0.0 {
                editor.set_track_height(ht.id, ht.height + d);
            }
        }

        // Reorder by dragging the grip; the phosphor dots show on hover.
        let grip = ui.interact(grip_rect, egui::Id::new(("track_grip", ht.id)), egui::Sense::drag());
        let grip = grip.on_hover_cursor(egui::CursorIcon::Grab);

        // The header itself is the host for the context menu; Ctrl+click
        // toggles the track in or out of the current range selection (the
        // toggles, sliders, name and grip sit on top and take their own
        // clicks). A plain click clears the track's meter clip lights.
        if host.clicked() {
            if ctrl {
                editor.toggle_range_track(ht.id);
            } else {
                for channel in meters.entry(ht.id).or_default().iter_mut() {
                    channel.clear_clip();
                }
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
                regular::DOTS_SIX_VERTICAL,
                ui_kit::fonts::icon_regular(typography::BODY_SIZE),
                color::TEXT_TERTIARY,
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
                    let _ = editor.apply(Action::MoveTrack { id: ht.id, new_index });
                }
                state.reorder = None;
            }
        } else if grip.drag_started() && !locked {
            state.reorder = Some((ht.id, index));
        }
        // Name: inline rename on double-click; the ui-kit header draws the
        // name otherwise.
        let name_resp = ui.interact(name_rect, egui::Id::new(("track_name", ht.id)), egui::Sense::click());
        if let Some((rid, text)) = state.renaming.as_mut()
            && *rid == ht.id
        {
            let edit = egui::TextEdit::singleline(text)
                .desired_rows(1)
                .desired_width(name_rect.width())
                .font(ui_kit::fonts::semibold(typography::HEADLINE_SIZE))
                .frame(egui::Frame::NONE);
            let resp = ui.put(name_rect, edit);
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter && !text.trim().is_empty() && *text != ht.name {
                    let _ = editor.apply(Action::RenameTrack { id: ht.id, name: text.clone() });
                }
                state.renaming = None;
            }
        } else if name_resp.double_clicked() && !locked {
            state.renaming = Some((ht.id, ht.name.clone()));
        }

        // Mute, solo and arm; Ctrl+click solo is exclusive.
        if !locked {
            if mute != ht.mute {
                let _ = editor.apply(Action::SetTrackMute { id: ht.id, mute });
            }
            if solo != ht.solo {
                if ctrl {
                    let _ = editor.apply(Action::SoloExclusive { id: ht.id });
                } else {
                    let _ = editor.apply(Action::SetTrackSolo { id: ht.id, solo });
                }
            }
            if armed != ht.armed {
                editor.toggle_arm(ht.id);
            }
        }

        // Volume and pan, side by side below the header content when they
        // fit. Each drag is one gesture: begin, preview every frame, end on
        // release.
        if show_sliders {
            let gap = space::SCALE_8;
            let slot_w = (rect.width() - 2.0 * pad - gap) / 2.0;
            let vol_slot = egui::Rect::from_min_size(egui::pos2(rect.left() + pad, slider_top), egui::vec2(slot_w, slider_h));
            let pan_slot = vol_slot.translate(egui::vec2(slot_w + gap, 0.0));
            let tip = format!("Volume {}", fader::db_text(ht.volume_db));
            gesture_slider(
                ui,
                editor,
                locked,
                ("track_volume", ht.id),
                vol_slot,
                ht.volume_db,
                -60.0..=12.0,
                |db| Action::SetTrackVolume { id: ht.id, db },
                tip,
            );
            let tip = format!("Pan {}", fader::pan_text(ht.pan));
            gesture_slider(ui, editor, locked, ("track_pan", ht.id), pan_slot, ht.pan, -1.0..=1.0, |pan| Action::SetTrackPan { id: ht.id, pan }, tip);
        }

        // 1 px border-colored separator under the header, across the lane.
        if rect.bottom() <= lane_rect.bottom() {
            ui.painter_at(header_rect.union(lane_rect)).line_segment(
                [egui::pos2(rect.left(), rect.bottom()), egui::pos2(lane_rect.right(), rect.bottom())],
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
                [egui::pos2(header_rect.left(), line_y), egui::pos2(lane_rect.right(), line_y)],
                egui::Stroke::new(THEME.reorder_line_w, THEME.reorder_line_color),
            );
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(STATE_KEY), state));
}

/// Right-click menu on one track header: Rename, Delete Track, Add Above/Below.
/// While a take is recorded (`locked`) every item is disabled.
fn context_menu(editor: &mut Editor, resp: &egui::Response, state: &mut State, id: Uuid, index: usize, track_name: &str, locked: bool) {
    egui::Popup::context_menu(resp).style(crate::ui::theme::menu_item_style as fn(&mut egui::Style)).show(|ui| {
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
            let _ = editor.apply(Action::AddTrack { index: Some(index), name: None });
            ui.close();
        }
        if ui.button("Add Track Below").clicked() {
            let _ = editor.apply(Action::AddTrack { index: Some(index + 1), name: None });
            ui.close();
        }
    });
}

/// One header slider through ui-kit's slider, run as an editor gesture on
/// `make(v)`.
///
/// Double-click resets to 0; a drag is one undo step; Esc cancels. A plain
/// click changes nothing: the slider shows the track's value again on the
/// next frame. While `locked` the slider draws but applies nothing.
#[allow(clippy::too_many_arguments)]
fn gesture_slider(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    locked: bool,
    id_salt: (&str, Uuid),
    slot: egui::Rect,
    value: f32,
    range: RangeInclusive<f32>,
    make: impl Fn(f32) -> Action,
    tip: String,
) {
    let mut shown = value;
    let mut slot_ui = ui.new_child(egui::UiBuilder::new().max_rect(slot).id_salt(id_salt));
    let resp = ui_kit::slider(&mut slot_ui, &mut shown, range, SliderKind::Accent).on_hover_text(tip);
    if locked {
        return;
    }
    if resp.double_clicked() {
        let _ = editor.apply(make(0.0));
        return;
    }
    if resp.drag_started() {
        editor.begin_gesture();
    }
    if resp.dragged() {
        let _ = editor.preview(make(shown));
    }
    if resp.drag_stopped() {
        editor.end_gesture();
    }
    if resp.dragged() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
    /// Side of a ui-kit toggle square.
    const TOGGLE_SIDE: f32 = 24.0;
    /// Gap between ui-kit toggles.
    const TOGGLE_GAP: f32 = 4.0;

    /// An editor over `count` empty tracks (no input device is ever opened).
    fn editor(count: usize) -> Editor {
        let tracks: Vec<&[(u64, u64)]> = vec![&[]; count];
        Editor::with_project(project_with_clips(&tracks), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") })
    }

    /// The header column as the app lays it out: 208 px wide, lanes beside it.
    fn rects() -> (egui::Rect, egui::Rect) {
        (egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(208.0, 600.0)), egui::Rect::from_min_size(egui::pos2(208.0, 0.0), egui::vec2(900.0, 600.0)))
    }

    /// Centers of the painted "R" labels: one record-arm toggle per track, top
    /// to bottom.
    fn arm_centers(out: &Out) -> Vec<egui::Pos2> {
        let mut found: Vec<_> = out.texts().into_iter().filter(|t| t.text == "R").map(|t| t.rect.center()).collect();
        found.sort_by(|a, b| a.y.total_cmp(&b.y));
        found
    }

    /// The painted armed toggles (24x24, `sem.record`).
    fn red_pills(out: &Out) -> Vec<egui::Rect> {
        out.rects().into_iter().filter(|r| r.fill == color::SEM_RECORD && r.rect.size() == egui::vec2(TOGGLE_SIDE, TOGGLE_SIDE)).map(|r| r.rect).collect()
    }

    /// The white knobs of the volume and pan sliders.
    fn slider_knobs(out: &Out) -> usize {
        out.circles().into_iter().filter(|c| c.fill == egui::Color32::WHITE).count()
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
        assert_eq!(centers.len(), 2, "one record-arm toggle per track");
        assert!(red_pills(&out).is_empty(), "nothing armed yet");
        assert_eq!(editor.armed_track(), None);

        // Arm the first track: it turns red.
        let _ = h.click(centers[0], |ui| draw(ui, &mut editor));
        assert_eq!(editor.armed_track(), Some(a));
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        assert_eq!(red_pills(&out).len(), 1, "one red arm toggle");
        let glyph = out.texts().into_iter().find(|t| t.text == "R" && t.rect.center() == centers[0]).expect("armed label");
        assert_eq!(glyph.color, egui::Color32::WHITE, "armed = white label");

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
        let m = out.text_rect("M").expect("M toggle").center();
        let s = out.text_rect("S").expect("S toggle").center();
        let arm = arm_centers(&out)[0];
        let pitch = TOGGLE_SIDE + TOGGLE_GAP;
        assert!(m.x < s.x && s.x < arm.x, "M, S, R left to right");
        assert!((s.x - m.x - pitch).abs() < TOL, "M to S {}", s.x - m.x);
        assert!((arm.x - s.x - pitch).abs() < TOL, "S to R {}", arm.x - s.x);
        assert!((m.y - arm.y).abs() < TOL && (m.y - s.y).abs() < TOL);
        // The toggle is the same 24x24 rounded square as M and S.
        let square =
            out.rects().into_iter().find(|r| r.rect.contains(arm) && r.rect.size() == egui::vec2(TOGGLE_SIDE, TOGGLE_SIDE)).expect("the arm toggle background");
        assert_eq!(square.fill, color::BG_CONTROL, "off = bg.control");
        assert_eq!(square.corner_radius.nw, 6);
        // The off label is text.tertiary.
        let glyph = out.texts().into_iter().find(|t| t.text == "R").expect("R label");
        assert_eq!(glyph.color, color::TEXT_TERTIARY);
    }

    #[test]
    fn the_compact_header_keeps_the_toggles_and_drops_the_sliders() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let id = editor.project.tracks[0].id;
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        assert_eq!(slider_knobs(&out), 2, "the default height shows the volume and pan sliders");

        editor.set_track_height(id, 40.0);
        assert!(editor.project.tracks[0].height < THEME.compact_track_h);
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        assert!(out.text_rect("M").is_some());
        assert!(out.text_rect("S").is_some());
        assert_eq!(arm_centers(&out).len(), 1);
        assert_eq!(slider_knobs(&out), 0, "compact has no slider row");
    }

    #[test]
    fn the_name_sits_on_the_first_row_above_the_toggles() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let number = out.text_rect("1").expect("track number");
        let name = out.text_rect("Track 1").expect("track name");
        let m = out.text_rect("M").expect("M toggle");
        assert!(number.right() < name.left(), "number {number:?} left of name {name:?}");
        assert!(name.bottom() <= m.top(), "name {name:?} above M {m:?}");
    }

    /// Pointer events for a left-button drag of `from` to `to` in four steps.
    fn drag_events(from: egui::Pos2, to: egui::Pos2) -> Vec<Vec<egui::Event>> {
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let middle = from + (to - from) * 0.5;
        vec![
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(middle)],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ]
    }

    #[test]
    fn dragging_the_volume_slider_is_one_undo_step_and_double_click_resets() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let mut draw = |ui: &mut egui::Ui, editor: &mut Editor| {
            show(ui, editor, header, lane, 0.0, &mut meters);
        };
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        let knob = out.circles().into_iter().filter(|c| c.fill == egui::Color32::WHITE).map(|c| c.center).next().expect("the volume knob");

        // The drag previews every frame and commits once on release.
        for events in drag_events(knob, knob - egui::vec2(40.0, 0.0)) {
            let _ = h.frame(events, |ui| draw(ui, &mut editor));
        }
        let dragged = editor.project.tracks[0].volume_db;
        assert!(dragged < -1.0, "the slider moved the volume down: {dragged}");
        assert!(editor.undo_label().is_some(), "the drag is an undo step");
        assert!(editor.undo());
        assert_eq!(editor.project.tracks[0].volume_db, 0.0, "one undo undoes the whole drag");
        assert!(editor.undo_label().is_none(), "and it was one step");

        // Double-click resets to 0 dB.
        editor.apply(Action::SetTrackVolume { id: editor.project.tracks[0].id, db: -12.0 }).expect("set the volume");
        let out = h.frame(Vec::new(), |ui| draw(ui, &mut editor));
        let knob = out.circles().into_iter().filter(|c| c.fill == egui::Color32::WHITE).map(|c| c.center).next().expect("the volume knob");
        let _ = h.click(knob, |ui| draw(ui, &mut editor));
        let _ = h.click(knob, |ui| draw(ui, &mut editor));
        assert_eq!(editor.project.tracks[0].volume_db, 0.0, "double-click resets the volume");
    }

    // ---- While a take is recorded ----

    /// Centers of the painted "M" toggles, top to bottom.
    fn m_centers(out: &Out) -> Vec<egui::Pos2> {
        let mut found: Vec<_> = out.texts().into_iter().filter(|t| t.text == "M").map(|t| t.rect.center()).collect();
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
