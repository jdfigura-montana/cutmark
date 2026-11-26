//! ui::timeline::track_header: the per-track header column on the timeline's
//! left: number and name on one row with the M, S and R toggles right-aligned
//! (the name elided when too long), the volume and pan slider rows under it as
//! the height allows, and a meter strip along the bottom, plus reorder grip,
//! inline rename, context menu and bottom-edge resize.

use crate::ui::timeline::metrics;
use std::collections::HashMap;
use std::ops::RangeInclusive;

use eframe::egui;
use ui_kit::icons::regular;
use ui_kit::tokens::{color, size, space, typography};
use ui_kit::{SliderKind, ToggleKind};
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::widgets::fader;
use crate::ui::widgets::meter;

/// Key under which the interaction state is carried between frames.
const STATE_KEY: &str = "track_header_state";
/// Width of the reorder grip, between the name and the toggles.
const GRIP_W: f32 = 14.0;
/// Right padding of the whole header: nothing touches the lane edge.
const RIGHT_PAD: f32 = space::SCALE_12;
/// Width of the right-hand value column of the slider rows, in points.
const VALUE_COL_W: f32 = 56.0;
/// Height of the "+ Add track" row under the last track header.
pub const ADD_ROW_H: f32 = 32.0;
/// Side of a toggle square (spec "Track header": 24 x 24).
const TOGGLE_SIDE: f32 = 24.0;
/// Gap between neighbouring toggles.
const TOGGLE_GAP: f32 = 4.0;
/// Corner radius of a toggle square.
const TOGGLE_ROUNDING: f32 = 6.0;
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
/// Each header is drawn by hand from the tokens: row 1 holds the number and
/// the name (elided when too long) with the M, S and R toggles right-aligned,
/// the volume and pan slider rows sit under it as the height allows (pan goes
/// first), and a plain meter strip runs along the bottom. A reorder grip sits
/// between the name and the toggles (double-click the name to rename inline),
/// and there is a context menu and a 4 px resize strip at the bottom.
/// `lane_rect` is only used to span the reorder insertion line across the full
/// timeline width.
pub fn show(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_y: f32,
    meters: &mut HashMap<Uuid, [meter::ChannelBallistics; 2]>,
) {
    let neutral = crate::ui::neutrals(ui.ctx());
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
    if state.reorder.is_some() && crate::shortcuts::peek(ui.ctx(), "cancel_gesture") {
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
        let reading = {
            let channels = meters.entry(ht.id).or_default();
            (channels[0].peak_db.max(channels[1].peak_db), channels[0].clip || channels[1].clip)
        };
        let renaming_here = state.renaming.as_ref().is_some_and(|(rid, _)| *rid == ht.id);

        // The header fill and its bottom line; a selected track takes the
        // selected header token.
        paint_header_row(&painter, rect, selected);

        // The header itself is the host for the context menu; Ctrl+click
        // toggles the track in or out of the current range selection (the
        // toggles, grip, name, meter and sliders sit on top and take their own
        // clicks). A plain click leaves the clip lights alone.
        let host = ui.interact(rect, egui::Id::new(("track_host", ht.id)), egui::Sense::click());
        host.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &ht.name));

        // Row 1: the number and the name on the left, the toggles right.
        let pad = space::SCALE_2;
        let number_color = if selected { neutral.text_primary } else { neutral.text_tertiary };
        let number_galley = painter.layout_job(ui_kit::fonts::tracked(
            (index + 1).to_string(),
            ui_kit::fonts::mono(typography::VALUE_SIZE),
            number_color,
            ui_kit::fonts::MONO_LETTER_SPACING_EM,
        ));
        let geo = geometry(rect, number_galley.size().x);
        painter.galley(egui::pos2(rect.left() + pad, geo.row.center().y - number_galley.size().y / 2.0), number_galley, number_color);
        if !renaming_here {
            let name = name_galley(&painter, &ht.name, geo.name.width(), neutral.text_primary);
            painter.with_clip_rect(geo.name).galley(egui::pos2(geo.name.left(), geo.row.center().y - name.size().y / 2.0), name, neutral.text_primary);
        }

        // The M, S and R toggles, right-aligned at the end of row 1; the
        // toggles edit local copies, applied below as the same editor actions
        // the old buttons ran.
        let mut mute = ht.mute;
        let mut solo = ht.solo;
        let mut armed = ht.armed;
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
        let (grip_rect, name_rect) = (geo.grip, geo.name);

        // The meter strip along the bottom; clicking it clears its clip
        // lights, as before; nothing else does.
        paint_meter_strip(&painter, geo.meter, reading.0, reading.1);
        let meter_resp = ui.interact(geo.meter, egui::Id::new(("track_meter", ht.id)), egui::Sense::click());
        if meter_resp.clicked() {
            for channel in meters.entry(ht.id).or_default().iter_mut() {
                channel.clear_clip();
            }
        }

        // Resize strip at the bottom edge.
        let resize_rect = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - metrics::TRACK_RESIZE_H), egui::pos2(rect.right(), rect.bottom()));
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
                neutral.text_tertiary,
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
        // Name: inline rename on double-click; the name is drawn above
        // otherwise.
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
                let enter = crate::shortcuts::peek(ui.ctx(), "confirm_edit");
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

        // The volume and pan rows under row 1 (pan goes first when the height
        // runs out): the type.micro label on the left, the slider filling the
        // space, the value right-aligned in the fixed 56 px mono column. Each
        // drag is one gesture: begin, preview every frame, end on release.
        if let Some(top) = geo.vol_top {
            let slot = egui::Rect::from_min_max(egui::pos2(rect.left() + pad, top), egui::pos2(rect.right() - RIGHT_PAD, top + size::SEGMENT_HEIGHT));
            let vol_rect = slider_cell(&painter, slot, "Vol", format!("{:+.1}", ht.volume_db.max(-60.0)));
            let tip = format!("Volume {}", fader::db_text(ht.volume_db));
            let make = |db| Action::SetTrackVolume { id: ht.id, db };
            gesture_slider(ui, editor, locked, ("track_volume", ht.id), vol_rect, ht.volume_db, -60.0..=12.0, SliderKind::Accent, make, tip);
        }
        if let Some(top) = geo.pan_top {
            let slot = egui::Rect::from_min_max(egui::pos2(rect.left() + pad, top), egui::pos2(rect.right() - RIGHT_PAD, top + size::SEGMENT_HEIGHT));
            let pan_text = fader::pan_text(ht.pan);
            let pan_rect = slider_cell(&painter, slot, "Pan", pan_text.clone());
            let tip = format!("Pan {pan_text}");
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
        let text_color = if add.hovered() && !locked { neutral.text_secondary } else { neutral.text_tertiary };
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
                egui::Stroke::new(metrics::REORDER_LINE_W, ui_kit::Theme::get(ui.ctx()).accent.accent),
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
    let neutral = crate::ui::neutrals(painter.ctx());
    let line = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - metrics::HAIRLINE), rect.right_bottom());
    painter.rect_filled(line, 0.0, neutral.stroke_track);
}

/// Paints one header's fill (`bg.track.header`, or `bg.track.header.selected`
/// when the track is selected) and its `stroke.track` bottom line.
fn paint_header_row(painter: &egui::Painter, rect: egui::Rect, selected: bool) {
    let neutral = crate::ui::neutrals(painter.ctx());
    painter.rect_filled(rect, 0.0, if selected { neutral.bg_track_header_selected } else { neutral.bg_track_header });
    paint_row_line(painter, rect);
}

/// Right-click menu on one track header: Rename, Delete Track, Add Above/Below.
/// While a take is recorded (`locked`) every item is disabled.
fn context_menu(editor: &mut Editor, resp: &egui::Response, state: &mut State, id: Uuid, index: usize, track_name: &str, locked: bool) {
    egui::Popup::context_menu(resp).show(|ui| {
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
    if resp.dragged() && crate::shortcuts::peek(ui.ctx(), "cancel_gesture") {
        editor.cancel_gesture();
    }
}

/// Where the parts of one header sit.
struct Geometry {
    /// Row 1: the number, the name and the toggles share it.
    row: egui::Rect,
    /// The name, between the number and the reorder grip.
    name: egui::Rect,
    /// The reorder grip, between the name and the toggles.
    grip: egui::Rect,
    /// The M, S and R toggles, right-aligned at the end of row 1.
    toggles: [egui::Rect; 3],
    /// The meter strip along the bottom, inset 12 px each side.
    meter: egui::Rect,
    /// Top of the volume row, when the height holds it.
    vol_top: Option<f32>,
    /// Top of the pan row, when the height holds it.
    pan_top: Option<f32>,
}

/// Lays out one header in `rect`. `number_w` is the track number's width.
///
/// Row 1 and the meter strip always show. The pan slider row goes first when
/// the height runs out, then the volume row (the layout by height tiers); the
/// toggles are fully inside the header at every height from the model minimum.
fn geometry(rect: egui::Rect, number_w: f32) -> Geometry {
    let pad = space::SCALE_2;
    let content_bottom = rect.bottom() - metrics::TRACK_RESIZE_H;
    // The meter strip always shows along the bottom, inset 12 px each side.
    let meter = egui::Rect::from_min_max(
        egui::pos2(rect.left() + RIGHT_PAD, content_bottom - metrics::METER_STRIP_H),
        egui::pos2(rect.right() - RIGHT_PAD, content_bottom),
    );
    let row_top = rect.top() + pad;
    let row = egui::Rect::from_min_max(egui::pos2(rect.left(), row_top), egui::pos2(rect.right(), row_top + TOGGLE_SIDE));
    // The M, S and R toggles right-aligned: R ends at the right padding edge.
    let toggles_w = 3.0 * TOGGLE_SIDE + 2.0 * TOGGLE_GAP;
    let toggles = std::array::from_fn(|i| {
        let left = rect.right() - RIGHT_PAD - toggles_w + i as f32 * (TOGGLE_SIDE + TOGGLE_GAP);
        egui::Rect::from_min_size(egui::pos2(left, row_top), egui::vec2(TOGGLE_SIDE, TOGGLE_SIDE))
    });
    // The reorder grip sits between the name and the toggles.
    let grip = egui::Rect::from_min_max(egui::pos2(toggles[0].left() - TOGGLE_GAP - GRIP_W, row_top), egui::pos2(toggles[0].left() - TOGGLE_GAP, row.bottom()));
    let name_left = rect.left() + pad + number_w + space::SCALE_8;
    let name = egui::Rect::from_min_max(egui::pos2(name_left, row_top), egui::pos2(grip.left() - TOGGLE_GAP, row.bottom()));
    // The slider rows fill in from the top; each must end above the meter.
    let vol_top = row.bottom() + pad;
    let fits = |top: f32| top + size::SEGMENT_HEIGHT + pad <= meter.top();
    let pan_top = fits(vol_top + size::SEGMENT_HEIGHT + pad).then_some(vol_top + size::SEGMENT_HEIGHT + pad);
    let vol_top = fits(vol_top).then_some(vol_top);
    Geometry { row, name, grip, toggles, meter, vol_top, pan_top }
}

/// Paints one toggle square exactly as ui-kit's track header does.
fn paint_toggle(painter: &egui::Painter, rect: egui::Rect, label: &str, on: bool, kind: ToggleKind) {
    let neutral = crate::ui::neutrals(painter.ctx());
    let (fill, glyph_color) = match (kind, on) {
        (ToggleKind::RecordArm, true) => (color::SEM_RECORD, egui::Color32::WHITE),
        (_, true) => (neutral.bg_selected, neutral.text_primary),
        (_, false) => (neutral.bg_control, neutral.text_tertiary),
    };
    painter.rect_filled(rect, TOGGLE_ROUNDING, fill);
    let glyph = painter.layout_no_wrap(label.to_owned(), ui_kit::fonts::semibold(typography::CAPTION_SIZE), glyph_color);
    let pos = egui::pos2(rect.center().x - glyph.size().x / 2.0, rect.center().y - glyph.size().y / 2.0);
    painter.with_clip_rect(rect).galley(pos, glyph, glyph_color);
}

/// Paints one slider row's `type.micro` label on the left and its value
/// (`type.value`, `text.tertiary`) right-aligned in the fixed value column at
/// the right. Returns the rect left for the slider between them. The label
/// column is one fixed width for Vol and Pan, so both sliders start at the
/// same x.
fn slider_cell(painter: &egui::Painter, slot: egui::Rect, label: &str, value: String) -> egui::Rect {
    let neutral = crate::ui::neutrals(painter.ctx());
    let gap = space::SCALE_4;
    let label_font = ui_kit::fonts::medium(typography::MICRO_SIZE);
    let label_w =
        ["Vol", "Pan"].iter().map(|text| painter.layout_no_wrap((*text).to_owned(), label_font.clone(), neutral.text_tertiary).size().x).fold(0.0, f32::max);
    let label_galley = painter.layout_no_wrap(label.to_owned(), label_font, neutral.text_tertiary);
    painter.galley(egui::pos2(slot.left(), slot.center().y - label_galley.size().y / 2.0), label_galley, neutral.text_tertiary);
    let value_font = ui_kit::fonts::mono(typography::VALUE_SIZE);
    let value_galley = painter.layout_no_wrap(value, value_font, neutral.text_tertiary);
    painter.galley(egui::pos2(slot.right() - value_galley.size().x, slot.center().y - value_galley.size().y / 2.0), value_galley, neutral.text_tertiary);
    let left = slot.left() + label_w + gap;
    egui::Rect::from_min_max(egui::pos2(left, slot.top()), egui::pos2((slot.right() - VALUE_COL_W - gap).max(left), slot.bottom()))
}

/// The row-1 name galley: one line, elided with an ellipsis when wider than
/// `max_w` (the room between the number and the reorder grip).
fn name_galley(painter: &egui::Painter, name: &str, max_w: f32, text_color: egui::Color32) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::single_section(name.to_owned(), egui::TextFormat::simple(ui_kit::fonts::semibold(typography::HEADLINE_SIZE), text_color));
    job.wrap.max_width = max_w.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}

/// The meter strip: a plain 4 px bar from the meter floor up to the peak in
/// the ok, warn and clip zone colors, with the clip square at its right end,
/// lit only on clip. No knob or dot look: square ends, no separate light.
fn paint_meter_strip(painter: &egui::Painter, rect: egui::Rect, peak_db: f32, clipped: bool) {
    let neutral = crate::ui::neutrals(painter.ctx());
    painter.rect_filled(rect, 0.0, neutral.bg_control);
    let fraction = |db: f32| {
        if db.is_nan() { 0.0 } else { ((db - metrics::METER_FLOOR_DB) / -metrics::METER_FLOOR_DB).clamp(0.0, 1.0) }
    };
    let bands = [
        (metrics::METER_FLOOR_DB, metrics::METER_WARN_DB, color::SEM_METER_OK),
        (metrics::METER_WARN_DB, metrics::METER_CLIP_DB, color::SEM_METER_WARN),
        (metrics::METER_CLIP_DB, 0.0, color::SEM_METER_CLIP),
    ];
    for (from, to, band_color) in bands {
        let low = fraction(from);
        let high = fraction(peak_db.min(to)).max(low);
        if high > low {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + rect.width() * low, rect.top()),
                    egui::pos2(rect.left() + rect.width() * high, rect.bottom()),
                ),
                0.0,
                band_color,
            );
        }
    }
    let side = rect.height();
    let light = egui::Rect::from_min_max(egui::pos2(rect.right() - side, rect.top()), rect.right_bottom());
    painter.rect_filled(light, 0.0, if clipped { color::SEM_METER_CLIP } else { neutral.stroke_hairline });
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
        assert_eq!(editor.project.tracks[0].height, 40.0, "the model minimum: row 1 and the meter only");
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
        // (height, slider rows shown): the pan row goes first, then the volume
        // row; row 1 and the meter always show.
        for (height, sliders) in [(40.0, 0), (54.0, 0), (65.0, 0), (66.0, 1), (80.0, 1), (96.0, 2)] {
            editor.set_track_height(id, height);
            assert_eq!(editor.project.tracks[0].height, height);
            let out = h.frame(Vec::new(), |ui| {
                show(ui, &mut editor, header, lane, 0.0, &mut meters);
            });
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(header.width(), height));
            let inside = rect.with_max_y(rect.bottom() - metrics::TRACK_RESIZE_H);
            let squares = toggle_squares(&out);
            assert_eq!(squares.len(), 3, "M, S and R at {height} px");
            for square in &squares {
                assert!(inside.contains_rect(*square), "toggle {square:?} inside the header {inside:?} at {height} px");
            }
            for label in ["M", "S", "R"] {
                assert!(out.text_rect(label).is_some_and(|r| inside.contains_rect(r)), "the {label} label inside the header at {height} px");
            }
            assert_eq!(slider_knobs(&out), sliders, "slider rows at {height} px");
            let vol = out.text_rect("Vol").is_some();
            let pan = out.text_rect("Pan").is_some();
            assert_eq!((vol as usize) + (pan as usize), sliders, "slider labels at {height} px");
            assert_eq!(out.text_rect("+0.0").is_some(), vol, "the volume value shows with its row (in the fixed value column) at {height} px");
            assert_eq!(out.text_rect("C").is_some(), pan, "the pan value shows with its row at {height} px");
            // Row 1 always: the number, the name and the toggles share it.
            let number = out.text_rect("1").expect("track number");
            let name = out.text_rect("Track 1").expect("track name");
            let m = out.text_rect("M").expect("M toggle");
            assert!((number.center().y - m.center().y).abs() < TOL && (name.center().y - m.center().y).abs() < TOL, "row 1 at {height} px");
            assert!(name.right() <= m.left(), "the name ends before the toggles at {height} px");
        }
    }

    #[test]
    fn the_geometry_keeps_the_toggles_inside_at_the_three_key_heights() {
        for height in [40.0, 66.0, 96.0] {
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(208.0, height));
            let geo = geometry(rect, 8.0);
            let inside = rect.with_max_y(rect.bottom() - metrics::TRACK_RESIZE_H);
            for toggle in geo.toggles {
                assert!(inside.contains_rect(toggle), "{toggle:?} inside {inside:?} at {height} px");
            }
            assert!(inside.contains_rect(geo.grip), "the grip inside at {height} px");
            assert!(inside.contains_rect(geo.meter), "the meter strip inside at {height} px");
            assert!((geo.meter.bottom() - inside.bottom()).abs() < 0.01, "the meter strip is at the bottom at {height} px");
            assert_eq!(geo.meter.width(), rect.width() - 2.0 * space::SCALE_12, "the meter strip is inset 12 px each side at {height} px");
            assert_eq!(geo.vol_top.is_some(), height >= 66.0, "the volume row tier at {height} px");
            assert_eq!(geo.pan_top.is_some(), height >= 96.0, "the pan row tier at {height} px");
            for top in [geo.vol_top, geo.pan_top].into_iter().flatten() {
                assert!(top + size::SEGMENT_HEIGHT + space::SCALE_2 <= geo.meter.top() + 0.01, "the slider rows end above the meter at {height} px");
            }
            assert!(geo.toggles.windows(2).all(|pair| pair[0].right() <= pair[1].left()), "the toggles do not overlap at {height} px");
            assert!(geo.name.right() <= geo.grip.left() && geo.grip.right() <= geo.toggles[0].left(), "name, grip and toggles in order at {height} px");
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
        let meter_rect = geometry(rect, 8.0).meter;
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
    fn the_number_and_name_share_row_1_with_the_right_aligned_toggles() {
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
        assert!((name.center().y - m.center().y).abs() < TOL, "the name shares row 1 with the toggles: {name:?} vs {m:?}");
        assert!(name.right() < m.left(), "the name ends before the toggles");
        // The toggles end 12 px inside the lane edge (the header right padding).
        let squares = toggle_squares(&out);
        let rightmost = squares.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
        assert!((rightmost - (header.right() - space::SCALE_12)).abs() < TOL, "the toggles end at the right padding: {rightmost}");
    }

    #[test]
    fn the_slider_rows_share_the_label_column_and_the_value_column() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let vol = out.text_rect("Vol").expect("Vol label");
        let pan = out.text_rect("Pan").expect("Pan label");
        assert!((vol.left() - pan.left()).abs() < TOL, "one fixed label column: {vol:?} vs {pan:?}");
        let vol_value = out.text_rect("+0.0").expect("volume value");
        let pan_value = out.text_rect("C").expect("pan value");
        assert!((vol_value.right() - pan_value.right()).abs() < TOL, "one fixed value column, right-aligned");
        assert!((vol_value.right() - (header.right() - space::SCALE_12)).abs() < TOL, "the value column ends at the right padding");
        // Both sliders span the same x (the space between the two columns).
        let troughs: Vec<_> =
            out.rects().into_iter().filter(|r| (r.rect.height() - 4.0).abs() < 1e-3 && r.fill == color::BG_SELECTED).map(|r| r.rect).collect();
        assert_eq!(troughs.len(), 2, "one trough per slider: {troughs:?}");
        assert!(
            (troughs[0].left() - troughs[1].left()).abs() < TOL && (troughs[0].right() - troughs[1].right()).abs() < TOL,
            "both sliders start and end alike"
        );
    }

    #[test]
    fn the_meter_strip_is_a_plain_4px_line_at_the_bottom_inset_12px() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor(1);
        let (header, lane) = rects();
        let mut meters = HashMap::new();
        let out = h.frame(Vec::new(), |ui| {
            show(ui, &mut editor, header, lane, 0.0, &mut meters);
        });
        let strips: Vec<_> = out
            .rects()
            .into_iter()
            .filter(|r| (r.rect.height() - metrics::METER_STRIP_H).abs() < 1e-3 && r.fill == color::BG_CONTROL)
            .map(|r| r.rect)
            .collect();
        assert_eq!(strips.len(), 1, "one meter strip: {strips:?}");
        let strip = strips[0];
        assert!(
            (strip.left() - (header.left() + space::SCALE_12)).abs() < TOL && (strip.right() - (header.right() - space::SCALE_12)).abs() < TOL,
            "inset 12 px each side: {strip:?}"
        );
        // It runs along the bottom of the track, above the resize strip.
        let track = egui::Rect::from_min_size(header.left_top(), egui::vec2(header.width(), editor.project.tracks[0].height));
        assert!(track.contains_rect(strip), "the strip sits inside its track");
        assert!((strip.bottom() - (track.bottom() - metrics::TRACK_RESIZE_H)).abs() < TOL, "above the resize strip: {strip:?}");
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

    // ---- kittest at the minimum sizes ----

    /// Painted text runs of one frame: (text, screen rect, clip rect).
    fn text_runs(output: &egui::FullOutput) -> Vec<(String, egui::Rect, egui::Rect)> {
        let mut runs = Vec::new();
        for clipped in &output.shapes {
            let mut stack = vec![&clipped.shape];
            while let Some(shape) = stack.pop() {
                match shape {
                    egui::Shape::Vec(inner) => stack.extend(inner.iter()),
                    egui::Shape::Text(text) => {
                        runs.push((text.galley.text().to_string(), text.galley.rect.translate(text.pos.to_vec2()), clipped.clip_rect));
                    }
                    _ => {}
                }
            }
        }
        runs
    }

    /// At the minimum header width (208) and the minimum track height (40)
    /// the header shows row 1 and the meter strip only, nothing overlaps,
    /// nothing is clipped, and the name is the one thing that may be elided.
    #[test]
    fn the_min_header_kittest_keeps_every_part_inside_without_overlap() {
        use std::cell::Cell;
        use std::cell::RefCell;
        use std::rc::Rc;

        use egui_kittest::kittest::Queryable as _;

        let editor = Rc::new(RefCell::new(editor(1)));
        let long = "A track name far too long for the narrowest header";
        let track_id = editor.borrow().project.tracks[0].id;
        let _ = editor.borrow_mut().apply(Action::RenameTrack { id: track_id, name: long.to_string() });
        editor.borrow_mut().set_track_height(track_id, 40.0);
        let meters = Rc::new(RefCell::new(HashMap::new()));
        let first_pass = Rc::new(Cell::new(true));
        let draw_editor = Rc::clone(&editor);
        let draw_meters = Rc::clone(&meters);
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1108.0, 400.0)).build_ui(move |ui| {
            // The first pass installs the ui-kit theme and fonts; egui applies
            // a font change at the start of the next pass.
            if first_pass.get() {
                first_pass.set(false);
                ui_kit::apply(ui.ctx());
                return;
            }
            let header = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(metrics::HEADER_COL_W, 40.0));
            let lane = egui::Rect::from_min_max(egui::pos2(header.right(), 0.0), egui::pos2(header.right() + 900.0, 400.0));
            show(ui, &mut draw_editor.borrow_mut(), header, lane, 0.0, &mut draw_meters.borrow_mut());
        });
        harness.step();
        harness.step();

        let header = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(metrics::HEADER_COL_W, 40.0));
        // Row 1: the toggles sit right-aligned inside the header, no overlap.
        let toggles: Vec<egui::Rect> = ["M", "S", "R"].iter().map(|label| harness.get_by_label(label).rect()).collect();
        for rect in &toggles {
            assert!(header.contains_rect(*rect), "toggle {rect:?} inside the header {header:?}");
        }
        for pair in toggles.windows(2) {
            assert!(pair[0].right() <= pair[1].left(), "the toggles do not overlap: {pair:?}");
        }
        let rightmost = toggles.iter().map(|r| r.right()).fold(f32::MIN, f32::max);
        assert!((rightmost - (header.right() - space::SCALE_12)).abs() < TOL, "the toggles end 12 px inside the lane edge: {rightmost}");

        // The host spans the header and carries the full track name.
        let host = harness.get_by_label(long).rect();
        assert!((host.left() - header.left()).abs() < TOL && (host.right() - header.right()).abs() < TOL, "the host spans the header: {host:?}");

        // The slider rows are dropped at the minimum height.
        let runs = text_runs(harness.output());
        let texts: Vec<_> = runs.iter().map(|(text, _, _)| text.clone()).collect();
        assert!(runs.iter().all(|(text, _, _)| text != "Vol" && text != "Pan"), "no slider rows at the minimum height: {texts:?}");

        // Nothing is clipped except the name's ellipsis: every text run sits
        // inside its clip rect, every clip rect inside the header, and the
        // elided name ends before the grip and the toggles.
        let toggle_left = toggles[0].left();
        for (text, rect, clip) in &runs {
            assert!(header.contains_rect(*clip), "{text:?} paints with a clip outside the header: {clip:?}");
            assert!(clip.contains_rect(*rect), "{text:?} is cut by its clip rect: {rect:?} vs {clip:?}");
            if text.starts_with("A track name") {
                assert!(rect.right() <= toggle_left - TOGGLE_GAP - GRIP_W + TOL, "the name is elided before the grip: {rect:?}");
                assert!(rect.width() < 150.0, "the long name is truncated, not overflowing: {rect:?}");
            }
        }
    }
}
