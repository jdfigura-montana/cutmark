//! ui::master_strip: the Master panel (ui-kit panel, 44 tall header, the
//! stereo meter with its scale beside the fader, and the peak readout).

use eframe::egui::{self, Align, Layout, Sense};
use ui_kit::tokens::{color, size, space, typography};

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::widgets::meter::MeterState;

/// Left padding of the header title from the panel's inner edge; with the
/// panel's own 8px padding the title sits 14px from the panel edge (spec
/// "Panel header").
const HEADER_PAD: f32 = 6.0;
/// The 1 px hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// Shortest the meter and fader row may get in a very short window.
const MIN_STRIP_H: f32 = 80.0;

/// Draws the Master panel: header, the meter and the fader filling the panel
/// height, and the peak readout under them.
///
/// Clicking the meters clears the master clip lights and every track's;
/// clicking the peak readout resets it. The fader drags as one undo step,
/// double-click resets to 0 dB and Esc cancels a drag.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, meters: &mut MeterState) {
    // F1 over the panel opens Master and levels.
    ui_kit::help::context(ui, crate::help::CONTEXT_MASTER);
    ui_kit::panel(ui, |ui| {
        // The panel fills the area the app gives it.
        ui.set_min_size(ui.available_size());
        header(ui);
        ui.add_space(space::SCALE_8);
        body(ui, editor, meters);
    });
}

/// Header row (44 tall): the "Master" title and a hairline along the bottom.
fn header(ui: &mut egui::Ui) {
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.add(
            egui::Label::new(egui::RichText::new("Master").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY)).selectable(false),
        );
    });
    // The hairline runs edge to edge of the panel, through its padding.
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// The meter and fader row, then the peak readout group under it.
fn body(ui: &mut egui::Ui, editor: &mut Editor, meters: &mut MeterState) {
    let peak_h = size::INSPECTOR_ROW_HEIGHT + 2.0 * space::SCALE_4;
    let strip_h = (ui.available_height() - peak_h - space::SCALE_8).max(MIN_STRIP_H);
    let row = egui::vec2(ui.available_width(), strip_h);
    ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Min), |ui| {
        meter(ui, meters, strip_h);
        // The fader sits at the right edge, the meter at the left.
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| fader(ui, editor, strip_h));
    });
    ui.add_space(space::SCALE_8);
    peak_readout(ui, meters);
}

/// The stereo meter with its dB scale and the clip light above the bars.
fn meter(ui: &mut egui::Ui, meters: &mut MeterState, height: f32) {
    let clipped = meters.master[0].clip || meters.master[1].clip;
    let response = ui_kit::stereo_meter_vertical(ui, meters.master[0].rms_db(), meters.master[1].rms_db(), clipped, height, true);
    // Clicking the meters (or the clip light) clears every clip light.
    if ui.interact(response.rect, ui.id().with("master_meter"), Sense::click()).clicked() {
        clear_all_clips(meters);
    }
}

/// The master fader with its dB readout: one undo step per drag, double-click
/// resets to 0 dB, Esc cancels the drag.
fn fader(ui: &mut egui::Ui, editor: &mut Editor, height: f32) {
    let before = editor.project.master_volume_db;
    let mut db = before;
    let response = ui_kit::fader(ui, &mut db, height);
    if response.drag_started() {
        editor.begin_gesture();
    }
    if db != before {
        let action = Action::SetMasterVolume { db };
        // A drag previews inside the gesture; a click, the reset and the
        // arrow keys are single edits.
        let _ = if response.dragged() { editor.preview(action) } else { editor.apply(action) };
    }
    if response.drag_stopped() {
        // egui itself ends the drag when Esc is pressed, so Esc shows up as a
        // stopped drag on the frame of the key press.
        if crate::shortcuts::peek(ui.ctx(), "cancel_gesture") {
            editor.cancel_gesture();
        } else {
            editor.end_gesture();
        }
    }
}

/// The peak readout: the highest master peak since the last reset, as "-9.6",
/// in an inspector row. Clicking it resets it.
fn peak_readout(ui: &mut egui::Ui, meters: &mut MeterState) {
    let text = match meters.master_peak_db {
        Some(db) => format!("{db:.1}"),
        None => "-inf".to_string(),
    };
    let row = ui_kit::inspector_group(ui, |ui| ui_kit::inspector_row(ui, "Peak", |_| {}, Some(&text))).inner;
    let response = ui.interact(row.rect, ui.id().with("master_peak"), Sense::click()).on_hover_text("Click to reset");
    if response.clicked() {
        meters.master_peak_db = None;
    }
}

/// Clears the master clip lights and every track's.
fn clear_all_clips(meters: &mut MeterState) {
    meters.master[0].clear_clip();
    meters.master[1].clear_clip();
    for ch in meters.tracks.values_mut() {
        ch[0].clear_clip();
        ch[1].clear_clip();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::{BitDepth, Project, ViewState};
    use crate::ui::test_util::{Headless, Out};
    use eframe::egui::{Event, Modifiers, PointerButton, Pos2};

    /// Width the tests give the panel (the app's right panel inner width).
    const PANEL_W: f32 = 128.0;

    fn test_editor() -> Editor {
        Editor::with_project(
            Project::new("Test".to_string(), 48_000, BitDepth::Int24),
            ViewState::default(),
            crate::io::paths::ProjectPaths { root: std::path::PathBuf::from("unused") },
        )
    }

    /// Runs one frame of the panel with `events`.
    fn frame(h: &mut Headless, editor: &mut Editor, meters: &mut MeterState, events: Vec<Event>) -> Out {
        h.frame(events, |ui| {
            ui.set_max_width(PANEL_W);
            show(ui, editor, meters);
        })
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    /// Where the meter bars and the fader are on screen, from the painted text.
    struct Spots {
        /// A point inside the left meter bar, midway down.
        bar: Pos2,
        /// The fader's center x and the y of its top and bottom.
        fader_x: f32,
        top: f32,
        bottom: f32,
    }

    fn spots(out: &Out) -> Spots {
        let zero = out.text_rect("0").expect("the 0 scale label");
        let floor = out.text_rect("-60").expect("the -60 scale label");
        let readout = out.text_rect("+0.0 dB").expect("the fader readout");
        Spots {
            bar: Pos2::new(zero.right() + 4.0 + 3.0, (zero.center().y + floor.center().y) / 2.0),
            fader_x: readout.center().x,
            top: zero.center().y,
            bottom: floor.center().y,
        }
    }

    #[test]
    fn the_panel_has_the_title_the_meter_scale_the_fader_readout_and_the_peak() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        let out = frame(&mut h, &mut editor, &mut meters, Vec::new());
        let title = out.texts().into_iter().find(|t| t.text == "Master").expect("the title");
        assert_eq!(title.color, color::TEXT_PRIMARY);
        assert!(title.rect.center().y < 60.0, "the title sits in the 44 tall header row");
        for label in ["0", "-12", "-60", "+0.0 dB", "Peak", "-inf"] {
            assert!(out.text_rect(label).is_some(), "{label} is drawn");
        }
        let scale = out.text_rect("-60").expect("scale");
        let fader = out.text_rect("+0.0 dB").expect("readout");
        assert!(scale.right() < fader.left(), "the meter is left of the fader");
    }

    #[test]
    fn clicking_the_meters_clears_the_master_and_track_clip_lights() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        let id = uuid::Uuid::new_v4();
        meters.master[0].clip = true;
        meters.master[1].clip = true;
        meters.tracks.entry(id).or_default()[1].clip = true;
        let at = spots(&frame(&mut h, &mut editor, &mut meters, Vec::new())).bar;
        let _ = h.click(at, |ui| {
            ui.set_max_width(PANEL_W);
            show(ui, &mut editor, &mut meters);
        });
        assert!(!meters.master[0].clip && !meters.master[1].clip, "the master lights clear");
        assert!(!meters.tracks[&id][1].clip, "the track lights clear too");
    }

    #[test]
    fn clicking_the_peak_readout_resets_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        meters.master_peak_db = Some(-9.6);
        let out = frame(&mut h, &mut editor, &mut meters, Vec::new());
        let at = out.text_rect("-9.6").expect("the peak value").center();
        let _ = h.click(at, |ui| {
            ui.set_max_width(PANEL_W);
            show(ui, &mut editor, &mut meters);
        });
        assert_eq!(meters.master_peak_db, None);
    }

    /// Presses on the fader, drags it down and returns the last frame's output.
    fn drag_fader_down(h: &mut Headless, editor: &mut Editor, meters: &mut MeterState, s: &Spots) -> Pos2 {
        let start = Pos2::new(s.fader_x, s.top);
        let end = Pos2::new(s.fader_x, (s.top + s.bottom) / 2.0);
        let _ = frame(h, editor, meters, vec![Event::PointerMoved(start)]);
        let _ = frame(h, editor, meters, vec![button(start, true)]);
        let _ = frame(h, editor, meters, vec![Event::PointerMoved(end)]);
        end
    }

    #[test]
    fn a_fader_drag_is_one_undo_step() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        let s = spots(&frame(&mut h, &mut editor, &mut meters, Vec::new()));
        let end = drag_fader_down(&mut h, &mut editor, &mut meters, &s);
        let mid = editor.project.master_volume_db;
        assert!(mid < -10.0, "the drag moved the level, got {mid}");
        assert_eq!(editor.undo_label(), None, "no history entry while dragging");
        let _ = frame(&mut h, &mut editor, &mut meters, vec![button(end, false)]);
        assert_eq!(editor.undo_label(), Some("Master Volume"));
        assert!(editor.undo());
        assert_eq!(editor.project.master_volume_db, 0.0);
        assert_eq!(editor.undo_label(), None, "one drag is one step");
    }

    #[test]
    fn escape_cancels_a_fader_drag() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        let s = spots(&frame(&mut h, &mut editor, &mut meters, Vec::new()));
        let end = drag_fader_down(&mut h, &mut editor, &mut meters, &s);
        assert!(editor.project.master_volume_db < -10.0);
        let esc = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let _ = frame(&mut h, &mut editor, &mut meters, vec![esc]);
        assert_eq!(editor.project.master_volume_db, 0.0, "the level goes back");
        // Still holding: moving on does not bring the edit back.
        let _ = frame(&mut h, &mut editor, &mut meters, vec![Event::PointerMoved(Pos2::new(end.x, end.y + 20.0))]);
        assert_eq!(editor.project.master_volume_db, 0.0);
        let _ = frame(&mut h, &mut editor, &mut meters, vec![button(end, false)]);
        assert_eq!(editor.undo_label(), None, "a cancelled drag leaves no history");
    }

    #[test]
    fn double_click_resets_the_fader_to_zero() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut editor, mut meters) = (test_editor(), MeterState::default());
        let s = spots(&frame(&mut h, &mut editor, &mut meters, Vec::new()));
        editor.apply(Action::SetMasterVolume { db: -12.0 }).expect("set");
        let at = Pos2::new(s.fader_x, (s.top + s.bottom) / 2.0);
        let _ = frame(&mut h, &mut editor, &mut meters, vec![Event::PointerMoved(at)]);
        for _ in 0..2 {
            let _ = frame(&mut h, &mut editor, &mut meters, vec![button(at, true)]);
            let _ = frame(&mut h, &mut editor, &mut meters, vec![button(at, false)]);
        }
        assert_eq!(editor.project.master_volume_db, 0.0);
    }
}
