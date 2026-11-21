//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: number, name, M, S and R toggles and a compact meter (the ui-kit
//! track header when the height allows two rows, one row of the same parts
//! below that), plus reorder grip, inline rename, context menu, bottom-edge
//! resize and the volume and pan sliders.

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
/// Height of the "+ Add track" row under the last track header.
pub const ADD_ROW_H: f32 = 32.0;
/// Side of a toggle square, as in ui-kit's track header.
const TOGGLE_SIDE: f32 = 24.0;
/// Gap between neighbouring toggles, as in ui-kit's track header.
const TOGGLE_GAP: f32 = 4.0;
/// Corner radius of a toggle square, as in ui-kit's track header.
const TOGGLE_ROUNDING: f32 = 6.0;
/// Header height from which the slider values show next to the sliders; below
/// it the value is only in the hover tooltip.
const VALUE_MIN_H: f32 = 86.0;
/// Tooltips of the M, S and R toggles, in toggle order.
const TOGGLE_TIPS: [&str; 3] = ["Mute", "Solo", "Arm for recording"];

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

        let selected = selected_tracks.contains(&ht.id);

        // Geometry of the header's parts. Two rows (ui-kit's track header, its
        // layout mirrored) when the height holds the sliders under it, one row
        // of the same parts otherwise.
        let pad = space::SCALE_2;
        let number_galley = painter.layout_no_wrap((index + 1).to_string(), ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY);
        let name_galley = painter.layout_no_wrap(ht.name.clone(), ui_kit::fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
        let geo = geometry(rect, number_galley.size().x, name_galley.size().y.max(number_galley.size().y));
        let renaming_here = state.renaming.as_ref().is_some_and(|(rid, _)| *rid == ht.id);

        // The header: the toggles edit local copies, applied below as the same
        // editor actions the old buttons ran. The name is left to the inline
        // editor while it is open.
        let mut mute = ht.mute;
        let mut solo = ht.solo;
        let mut armed = ht.armed;
        let reading = {
            let channels = meters.entry(ht.id).or_default();
            (channels[0].peak_db.max(channels[1].peak_db), channels[0].clip || channels[1].clip)
        };
        let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).id_salt(("track_header_ui", ht.id)));
        header_ui.set_clip_rect(rect.intersect(header_rect));
        let host = if geo.two_row {
            let host = {
                let mut toggles = [
                    Toggle::visible("M", &mut mute),
                    Toggle::visible("S", &mut solo),
                    Toggle { label: "R", on: &mut armed, kind: ToggleKind::RecordArm, always_visible: true },
                ];
                ui_kit::track_header(&mut header_ui, index + 1, if renaming_here { "" } else { &ht.name }, selected, &mut toggles, Some(reading))
            };
            // ui-kit's toggles carry no tooltip: hover-only areas on top add it.
            for (toggle_rect, tip) in geo.toggles.iter().zip(TOGGLE_TIPS) {
                let _ = ui.interact(*toggle_rect, egui::Id::new(("track_toggle_tip", ht.id, tip)), egui::Sense::hover()).on_hover_text(tip);
            }
            host
        } else {
            // The one-row header has no ui-kit widget to paint its selected
            // fill and bottom line (the two-row one does).
            paint_header_row(&painter, rect, selected);
            let host = ui.interact(rect, egui::Id::new(("track_host", ht.id)), egui::Sense::click());
            let row_center_y = geo.toggles[0].center().y;
            let number_color = if selected { color::TEXT_PRIMARY } else { color::TEXT_TERTIARY };
            let number_pos = egui::pos2(rect.left() + pad, row_center_y - number_galley.size().y / 2.0);
            painter.galley(number_pos, number_galley, number_color);
            if !renaming_here {
                let name_pos = egui::pos2(geo.name.left(), row_center_y - name_galley.size().y / 2.0);
                painter.with_clip_rect(geo.name).galley(name_pos, name_galley, color::TEXT_PRIMARY);
            }
            let flags = [&mut mute, &mut solo, &mut armed];
            let kinds = [ToggleKind::Normal, ToggleKind::Normal, ToggleKind::RecordArm];
            for ((((toggle_rect, tip), label), on), kind) in geo.toggles.iter().zip(TOGGLE_TIPS).zip(["M", "S", "R"]).zip(flags).zip(kinds) {
                let resp = ui.interact(*toggle_rect, egui::Id::new(("track_toggle", ht.id, label)), egui::Sense::click()).on_hover_text(tip);
                resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, *on, label));
                if resp.clicked() {
                    *on = !*on;
                }
                paint_toggle(&painter, *toggle_rect, label, *on, kind);
            }
            if let Some(meter_rect) = geo.meter {
                let mut meter_ui = header_ui.new_child(egui::UiBuilder::new().max_rect(meter_rect));
                ui_kit::level_meter(&mut meter_ui, reading.0, reading.1, meter_rect.width());
            }
            host
        };
        let (grip_rect, name_rect) = (geo.grip, geo.name);

        // Clicking the meter clears its clip lights, as before; nothing else does.
        if let Some(meter_rect) = geo.meter {
            let meter_resp = ui.interact(meter_rect, egui::Id::new(("track_meter", ht.id)), egui::Sense::click());
            if meter_resp.clicked() {
                for channel in meters.entry(ht.id).or_default().iter_mut() {
                    channel.clear_clip();
                }
            }
        }

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
        let grip = grip.on_hover_cursor(egui::CursorIcon::Grab).on_hover_text("Drag to reorder");

        // The header itself is the host for the context menu; Ctrl+click
        // toggles the track in or out of the current range selection (the
        // toggles, meter, sliders, name and grip sit on top and take their own
        // clicks). A plain click leaves the clip lights alone.
        if host.clicked() && ctrl {
            editor.toggle_range_track(ht.id);
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

        // Volume and pan, side by side below the header content from 64 px
        // up, each with its label on the left and, from 86 px up, its value on
        // the right (below that the value is the hover tooltip). Each drag is
        // one gesture: begin, preview every frame, end on release.
        if let Some(slider_top) = geo.slider_top {
            let gap = space::SCALE_8;
            let slot_w = (rect.width() - 2.0 * pad - gap) / 2.0;
            let vol_slot = egui::Rect::from_min_size(egui::pos2(rect.left() + pad, slider_top), egui::vec2(slot_w, size::SEGMENT_HEIGHT));
            let pan_slot = vol_slot.translate(egui::vec2(slot_w + gap, 0.0));
            let show_values = rect.height() >= VALUE_MIN_H;

            let value = show_values.then(|| (format!("{:+.1}", ht.volume_db.max(-60.0)), "-60.0"));
            let vol_rect = slider_cell(&painter, vol_slot, "Vol", value);
            let tip = format!("Volume {}", fader::db_text(ht.volume_db));
            let make = |db| Action::SetTrackVolume { id: ht.id, db };
            gesture_slider(ui, editor, locked, ("track_volume", ht.id), vol_rect, ht.volume_db, -60.0..=12.0, SliderKind::Accent, make, tip);

            let value = show_values.then(|| (fader::pan_text(ht.pan), "R100"));
            let pan_rect = slider_cell(&painter, pan_slot, "Pan", value);
            let tip = format!("Pan {}", fader::pan_text(ht.pan));
            let make = |pan| Action::SetTrackPan { id: ht.id, pan };
            gesture_slider(ui, editor, locked, ("track_pan", ht.id), pan_rect, ht.pan, -1.0..=1.0, SliderKind::Bipolar, make, tip);
        }
    }

    // "+ Add track" row under the last header (same action and shortcut as
    // before: Ctrl+Shift+T); off while a take is recorded. It ends in a
    // `stroke.track` line across the header column and the lanes, like a
    // track row.
    let tracks_h = tracks.iter().map(|t| t.height).sum();
    let add_full = add_row_rect(header_rect, scroll_y, tracks_h);
    let add_rect = add_full.intersect(header_rect);
    if add_full.bottom() <= lane_rect.bottom() {
        paint_row_line(
            &ui.painter_at(header_rect.union(lane_rect)),
            egui::Rect::from_min_max(add_full.left_top(), egui::pos2(lane_rect.right(), add_full.bottom())),
        );
    }
    if add_rect.height() > 0.0 {
        let add = ui.interact(add_rect, egui::Id::new("track_add_row"), egui::Sense::click()).on_hover_text("Add a new track (Ctrl+Shift+T)");
        let text_color = if add.hovered() && !locked { color::TEXT_SECONDARY } else { color::TEXT_TERTIARY };
        painter.text(
            egui::pos2(add_rect.left() + space::SCALE_8, add_rect.center().y),
            egui::Align2::LEFT_CENTER,
            "+ Add track",
            ui_kit::fonts::medium(typography::BODY_SIZE),
            text_color,
        );
        if add.clicked() && !locked {
            let _ = editor.apply(Action::AddTrack { index: None, name: None });
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

/// The "+ Add track" row's rect, unclipped: it starts where the last track
/// header ends (`tracks_h` is the sum of the track heights, `scroll_y` the
/// vertical scroll) and spans the header column.
pub fn add_row_rect(header_rect: egui::Rect, scroll_y: f32, tracks_h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(header_rect.left(), header_rect.top() - scroll_y + tracks_h), egui::vec2(header_rect.width(), ADD_ROW_H))
}

/// The 1 px `stroke.track` line along the bottom edge of `rect`.
fn paint_row_line(painter: &egui::Painter, rect: egui::Rect) {
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - THEME.hairline), rect.right_bottom());
    painter.rect_filled(line, 0.0, color::STROKE_TRACK);
}

/// Paints a one-row header's selected fill (`bg.track.header.selected`; the
/// unselected header is the column strip itself) and its bottom line.
fn paint_header_row(painter: &egui::Painter, rect: egui::Rect, selected: bool) {
    if selected {
        painter.rect_filled(rect, 0.0, color::BG_TRACK_HEADER_SELECTED);
    }
    paint_row_line(painter, rect);
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
    kind: SliderKind,
    make: impl Fn(f32) -> Action,
    tip: String,
) {
    let mut shown = value;
    let mut slot_ui = ui.new_child(egui::UiBuilder::new().max_rect(slot).id_salt(id_salt));
    let resp = ui_kit::slider(&mut slot_ui, &mut shown, range, kind).on_hover_text(tip);
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

/// Where the parts of one header sit.
struct Geometry {
    /// True when ui-kit's two-row header draws the number, name, toggles and
    /// meter; false for the one-row layout drawn here.
    two_row: bool,
    /// The reorder grip at the right end of the first row.
    grip: egui::Rect,
    /// The name, between the number and the toggles (two rows: the grip).
    name: egui::Rect,
    /// The M, S and R toggles, in that order.
    toggles: [egui::Rect; 3],
    /// The compact level meter, when it fits.
    meter: Option<egui::Rect>,
    /// Top of the slider row (28 px high), from 64 px of header height up.
    slider_top: Option<f32>,
}

/// Lays out one header in `rect`. `number_w` and `line_h` are the track
/// number's width and the height of the number and name line.
///
/// Two rows (number and name, then toggles, meter and sliders) once the height
/// holds the sliders under the meter; below that one row: number, name,
/// toggles and grip, with the meter as a thin line under it if it fits. The
/// toggles are fully inside the header at every height from the model minimum.
fn geometry(rect: egui::Rect, number_w: f32, line_h: f32) -> Geometry {
    let pad = space::SCALE_2;
    let content_bottom = rect.bottom() - THEME.track_resize_h;
    let two_row_h = pad + line_h + pad + TOGGLE_ROW_H + pad + METER_H + pad + size::SEGMENT_HEIGHT + THEME.track_resize_h;
    let toggles_at = |left: f32, top: f32| -> [egui::Rect; 3] {
        std::array::from_fn(|i| egui::Rect::from_min_size(egui::pos2(left + i as f32 * (TOGGLE_SIDE + TOGGLE_GAP), top), egui::vec2(TOGGLE_SIDE, TOGGLE_SIDE)))
    };
    let grip_left = rect.right() - pad - GRIP_W;
    if rect.height() >= two_row_h.max(VALUE_MIN_H) {
        let first_row_top = rect.top() + pad;
        let grip = egui::Rect::from_min_max(egui::pos2(grip_left, first_row_top), egui::pos2(rect.right() - pad, first_row_top + line_h));
        let name = egui::Rect::from_min_max(egui::pos2(rect.left() + pad + number_w + space::SCALE_8, first_row_top), grip.left_bottom());
        let toggles = toggles_at(rect.left() + pad, grip.bottom() + pad);
        let meter = egui::Rect::from_min_size(egui::pos2(rect.left() + pad, toggles[0].bottom() + pad), egui::vec2(rect.width() - 2.0 * pad, METER_H));
        return Geometry { two_row: true, grip, name, toggles, meter: Some(meter), slider_top: Some(meter.bottom() + pad) };
    }
    let row_top = rect.top() + pad;
    let row_bottom = row_top + TOGGLE_ROW_H;
    let grip = egui::Rect::from_min_max(egui::pos2(grip_left, row_top), egui::pos2(rect.right() - pad, row_bottom));
    let toggles_w = 3.0 * TOGGLE_SIDE + 2.0 * TOGGLE_GAP;
    let toggles = toggles_at(grip.left() - TOGGLE_GAP - toggles_w, row_top);
    let name_left = rect.left() + pad + number_w + space::SCALE_8;
    let name = egui::Rect::from_min_max(egui::pos2(name_left, row_top), egui::pos2((toggles[0].left() - TOGGLE_GAP).max(name_left), row_bottom));
    let meter_top = row_bottom + pad;
    let meter = (meter_top + METER_H <= content_bottom)
        .then(|| egui::Rect::from_min_size(egui::pos2(rect.left() + pad, meter_top), egui::vec2(rect.width() - 2.0 * pad, METER_H)));
    // The slider row sits centered between the meter and the resize strip; the
    // 28 px hit area may touch the meter, the visible 14 px knob never does.
    let slider_top = (rect.height() >= THEME.compact_track_h).then(|| {
        let above = meter.map_or(row_bottom + pad, |m| m.bottom());
        let slack = content_bottom - above - size::SEGMENT_HEIGHT;
        (above + slack / 2.0).min(content_bottom - size::SEGMENT_HEIGHT)
    });
    Geometry { two_row: false, grip, name, toggles, meter, slider_top }
}

/// Paints one toggle square exactly as ui-kit's track header does.
fn paint_toggle(painter: &egui::Painter, rect: egui::Rect, label: &str, on: bool, kind: ToggleKind) {
    let (fill, glyph_color) = match (kind, on) {
        (ToggleKind::RecordArm, true) => (color::SEM_RECORD, egui::Color32::WHITE),
        (_, true) => (color::BG_SELECTED, color::TEXT_PRIMARY),
        (_, false) => (color::BG_CONTROL, color::TEXT_TERTIARY),
    };
    painter.rect_filled(rect, TOGGLE_ROUNDING, fill);
    let glyph = painter.layout_no_wrap(label.to_owned(), ui_kit::fonts::semibold(typography::CAPTION_SIZE), glyph_color);
    let pos = egui::pos2(rect.center().x - glyph.size().x / 2.0, rect.center().y - glyph.size().y / 2.0);
    painter.with_clip_rect(rect).galley(pos, glyph, glyph_color);
}

/// Paints one slider cell's label (`type.micro`) on the left and, when
/// `value` is given, its text (`type.value`, `text.tertiary`) right-aligned on
/// the right, in a column as wide as the `sample` text. Returns the rect left
/// for the slider between them.
fn slider_cell(painter: &egui::Painter, slot: egui::Rect, label: &str, value: Option<(String, &str)>) -> egui::Rect {
    let gap = space::SCALE_4;
    let label_font = ui_kit::fonts::medium(typography::MICRO_SIZE);
    // One label column for Vol and Pan, so both sliders start alike.
    let label_w =
        ["Vol", "Pan"].iter().map(|text| painter.layout_no_wrap((*text).to_owned(), label_font.clone(), color::TEXT_TERTIARY).size().x).fold(0.0, f32::max);
    let label_galley = painter.layout_no_wrap(label.to_owned(), label_font, color::TEXT_TERTIARY);
    painter.galley(egui::pos2(slot.left(), slot.center().y - label_galley.size().y / 2.0), label_galley, color::TEXT_TERTIARY);
    let mut right = slot.right();
    if let Some((text, sample)) = value {
        let value_font = ui_kit::fonts::mono(typography::VALUE_SIZE);
        let column_w = painter.layout_no_wrap(sample.to_owned(), value_font.clone(), color::TEXT_TERTIARY).size().x;
        let galley = painter.layout_no_wrap(text, value_font, color::TEXT_TERTIARY);
        painter.galley(egui::pos2(slot.right() - galley.size().x, slot.center().y - galley.size().y / 2.0), galley, color::TEXT_TERTIARY);
        right -= column_w + gap;
    }
    egui::Rect::from_min_max(egui::pos2(slot.left() + label_w + gap, slot.top()), egui::pos2(right.max(slot.left() + label_w + gap), slot.bottom()))
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

    /// The off toggles painted in one frame (24x24, `bg.control`).
    fn toggle_squares(out: &Out) -> Vec<egui::Rect> {
        out.rects().into_iter().filter(|r| r.fill == color::BG_CONTROL && r.rect.size() == egui::vec2(TOGGLE_SIDE, TOGGLE_SIDE)).map(|r| r.rect).collect()
    }

    #[test]
    fn every_height_from_the_minimum_shows_all_toggles_inside_the_header() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let id = editor.project.tracks[0].id;
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        // (height, sliders shown, values shown)
        for (height, sliders, values) in
            [(40.0, false, false), (54.0, false, false), (63.0, false, false), (64.0, true, false), (80.0, true, false), (86.0, true, true), (96.0, true, true)]
        {
            editor.set_track_height(id, height);
            assert_eq!(editor.project.tracks[0].height, height);
            let out = h.frame(Vec::new(), |ui| {
                show(ui, &mut editor, header, lane, 0.0, &mut meters);
            });
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(header.width(), height));
            let inside = rect.with_max_y(rect.bottom() - THEME.track_resize_h);
            let squares = toggle_squares(&out);
            assert_eq!(squares.len(), 3, "M, S and R at {height} px");
            for square in &squares {
                assert!(inside.contains_rect(*square), "toggle {square:?} inside the header {inside:?} at {height} px");
            }
            for label in ["M", "S", "R"] {
                assert!(out.text_rect(label).is_some_and(|r| inside.contains_rect(r)), "the {label} label inside the header at {height} px");
            }
            assert_eq!(slider_knobs(&out), if sliders { 2 } else { 0 }, "sliders at {height} px");
            assert_eq!(out.text_rect("Vol").is_some() && out.text_rect("Pan").is_some(), sliders, "slider labels at {height} px");
            assert_eq!(out.text_rect("+0.0").is_some() && out.text_rect("C").is_some(), values, "slider values at {height} px");
            if height < 86.0 {
                // One row: the number, the name and the toggles share it.
                let number = out.text_rect("1").expect("track number");
                let name = out.text_rect("Track 1").expect("track name");
                let m = out.text_rect("M").expect("M toggle");
                assert!((number.center().y - m.center().y).abs() < TOL && (name.center().y - m.center().y).abs() < TOL, "one row at {height} px");
                assert!(name.right() <= m.left(), "the name ends before the toggles at {height} px");
            }
        }
    }

    #[test]
    fn the_geometry_keeps_the_toggles_inside_at_the_three_key_heights() {
        for height in [40.0, 64.0, 86.0] {
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(208.0, height));
            let geo = geometry(rect, 8.0, 17.0);
            let inside = rect.with_max_y(rect.bottom() - THEME.track_resize_h);
            for toggle in geo.toggles {
                assert!(inside.contains_rect(toggle), "{toggle:?} inside {inside:?} at {height} px");
            }
            assert!(inside.contains_rect(geo.grip), "the grip inside at {height} px");
            assert!(geo.meter.is_none_or(|meter| inside.contains_rect(meter)), "the meter inside at {height} px");
            assert_eq!(geo.slider_top.is_some(), height >= THEME.compact_track_h);
            if let Some(top) = geo.slider_top {
                assert!(top + size::SEGMENT_HEIGHT <= inside.bottom() + 0.01, "the sliders end above the resize strip at {height} px");
            }
        }
    }

    #[test]
    fn pan_is_a_bipolar_slider_filling_from_the_center() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let id = editor.project.tracks[0].id;
        editor.apply(Action::SetTrackPan { id, pan: -0.5 }).expect("pan");
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let accent = ui_kit::Theme::get(&h.ctx).accent.accent;
        let thin: Vec<_> = out.rects().into_iter().filter(|r| (r.rect.height() - 4.0).abs() < 1e-3).collect();
        let fills: Vec<_> = thin.iter().filter(|r| r.fill == accent).map(|r| r.rect).collect();
        assert_eq!(fills.len(), 2, "a volume fill and a pan fill: {thin:?}");
        // The pan fill ends at the middle of its track and starts left of it.
        let pan_fill = fills.iter().find(|fill| {
            fill.left() > header.left() + 1.0
                && thin.iter().any(|t| t.fill == color::BG_SELECTED && t.rect.left() < fill.left() - 1.0 && (t.rect.center().x - fill.right()).abs() < 0.5)
        });
        assert!(pan_fill.is_some(), "the pan fill runs from the center to the value: {fills:?}");
    }

    #[test]
    fn a_plain_click_on_the_header_keeps_a_latched_clip_light_and_the_meter_click_clears_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let id = editor.project.tracks[0].id;
        let (header, lane) = rects();
        let mut meters: HashMap<Uuid, [meter::ChannelBallistics; 2]> = HashMap::new();
        meters.entry(id).or_default()[0].clip = true;
        let mut draw = |ui: &mut egui::Ui, editor: &mut Editor| {
            show(ui, editor, header, lane, 0.0, &mut meters);
        };
        // A plain click on the bare header (left of the name, above the toggles).
        let _ = h.click(egui::pos2(3.0, 6.0), |ui| draw(ui, &mut editor));
        // A click on the number row's name area is a plain click too.
        let _ = h.click(egui::pos2(60.0, 10.0), |ui| draw(ui, &mut editor));
        drop(draw);
        assert!(meters[&id][0].clip, "plain clicks leave the clip light on");

        // Clicking the meter clears it.
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(header.width(), editor.project.tracks[0].height));
        let meter_rect = geometry(rect, 8.0, 17.0).meter.expect("the meter");
        let mut draw = |ui: &mut egui::Ui, editor: &mut Editor| {
            show(ui, editor, header, lane, 0.0, &mut meters);
        };
        let _ = h.click(meter_rect.center(), |ui| draw(ui, &mut editor));
        drop(draw);
        assert!(!meters[&id][0].clip, "clicking the meter clears the clip light");
    }

    #[test]
    fn a_selected_header_fills_with_the_selected_header_token_at_every_height() {
        let mut h = Headless::new(1280.0, 800.0);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        for height in [40.0, 96.0] {
            let mut editor = editor(2);
            let id = editor.project.tracks[0].id;
            editor.set_track_height(id, height);
            editor.set_range(0, 1_000, vec![id]);
            let out = h.frame(Vec::new(), |ui| {
                show(ui, &mut editor, header, lane, 0.0, &mut meters);
            });
            let selected: Vec<_> = out.rects().into_iter().filter(|r| r.fill == color::BG_TRACK_HEADER_SELECTED).collect();
            assert_eq!(selected.len(), 1, "one selected header at {height} px");
            assert_eq!(selected[0].rect.top(), 0.0);
            assert_eq!(selected[0].rect.height(), height);
            assert!(out.rects().iter().all(|r| r.fill != color::BG_TRACK_SELECTED && r.fill != color::BG_TRACK_ALT), "the headers never take a lane fill");
            let lines = out.rects().into_iter().filter(|r| r.fill == color::STROKE_TRACK && r.rect.width() >= header.width()).count();
            assert_eq!(lines, 3, "a line under each header and one under the Add track row at {height} px");
        }
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
