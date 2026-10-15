//! ui::master_strip: the fixed-width Master fader and meter strip
//! (docs/ui-target.png look): title, clip light, scale + meters + fader row,
//! peak readout and fader value.

use eframe::egui::{self, Align2, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::theme::{THEME, Theme};
use crate::ui::widgets::fader;
use crate::ui::widgets::meter::{self, FLOOR, MeterState, TICKS};

/// Horizontal and vertical padding inside the strip.
const PAD: f32 = 10.0;
/// Width of the dB scale column.
const SCALE_W: f32 = 22.0;
/// Width of one meter bar.
const BAR_W: f32 = 9.0;
/// Gap between the two meter bars.
const BAR_GAP: f32 = 2.0;
/// Width of the fader.
const FADER_W: f32 = 26.0;
/// Height of the clip light.
const LIGHT_H: f32 = 10.0;
/// Width of the clip light.
const LIGHT_W: f32 = 30.0;
/// Height of the peak readout button.
const READOUT_H: f32 = 24.0;
/// Height of the fader value line under the readout.
const VALUE_H: f32 = 16.0;
/// Gap between the stacked blocks.
const BLOCK_GAP: f32 = 6.0;

/// Draws the master strip: MASTER title, clip light, the scale/meters/fader
/// row, the peak readout and the fader value.
///
/// Clicking the clip light or the meters clears the master lights and every
/// track's; clicking the peak readout resets it.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, meters: &mut MeterState) {
    let avail = ui.available_rect_before_wrap();
    let painter = ui.painter_at(avail);
    let content = Rect::from_min_max(
        Pos2::new(avail.left() + PAD, avail.top() + PAD),
        Pos2::new(avail.right() - PAD, avail.bottom() - PAD),
    );

    // Title: MASTER, small caps, centered.
    painter.text(
        Pos2::new(content.center().x, content.top() + 7.0),
        Align2::CENTER_CENTER,
        "MASTER",
        egui::FontId::proportional(11.0),
        THEME.text_muted,
    );

    // Clip light: one 30x10 light for the master, below the title.
    let light = Rect::from_min_size(
        Pos2::new(content.center().x - LIGHT_W * 0.5, content.top() + 16.0),
        Vec2::new(LIGHT_W, LIGHT_H),
    );
    let lit = meters.master[0].clip || meters.master[1].clip;
    painter.rect_filled(
        light,
        3.0,
        if lit {
            THEME.clip_light_on
        } else {
            THEME.clip_light_off
        },
    );
    if ui
        .interact(
            light,
            egui::Id::new("master_clip_light"),
            egui::Sense::click(),
        )
        .clicked()
    {
        clear_all_clips(meters);
    }

    // Middle: the scale / meters / fader row, centered as one block.
    let row_w = SCALE_W + BLOCK_GAP + 2.0 * BAR_W + BAR_GAP + BLOCK_GAP + FADER_W;
    let row_left = content.left() + ((content.width() - row_w) * 0.5).max(0.0);
    let mid = Rect::from_min_max(
        Pos2::new(content.left(), light.bottom() + BLOCK_GAP),
        Pos2::new(
            content.right(),
            content.bottom() - READOUT_H - VALUE_H - BLOCK_GAP,
        ),
    );

    // Meter bars: unlit background, lit part in dB zones, 2 px hold line.
    let bars_left = row_left + SCALE_W + BLOCK_GAP;
    let bar_area = Rect::from_min_max(
        Pos2::new(bars_left, mid.top()),
        Pos2::new(bars_left + 2.0 * BAR_W + BAR_GAP, mid.bottom()),
    );
    let l_bar = Rect::from_min_max(
        bar_area.left_top(),
        Pos2::new(bar_area.left() + BAR_W, bar_area.bottom()),
    );
    let r_bar = Rect::from_min_max(
        Pos2::new(bar_area.right() - BAR_W, bar_area.top()),
        Pos2::new(bar_area.right(), bar_area.bottom()),
    );
    for (bar, ch) in [(&l_bar, &meters.master[0]), (&r_bar, &meters.master[1])] {
        painter.rect_filled(*bar, 2.0, THEME.meter_bar_bg);
        let zones = painter.with_clip_rect(*bar);
        let rms = ch.rms_db();
        meter::fill_zone(&zones, *bar, FLOOR, rms.min(-18.0), THEME.meter_green);
        meter::fill_zone(&zones, *bar, -18.0, rms.min(-6.0), THEME.meter_yellow);
        meter::fill_zone(&zones, *bar, -6.0, rms, THEME.meter_red);
        if ch.hold_db > FLOOR {
            let y = meter::db_y(*bar, ch.hold_db);
            zones.line_segment(
                [Pos2::new(bar.left(), y), Pos2::new(bar.right(), y)],
                Stroke::new(2.0, THEME.meter_hold),
            );
        }
    }
    // Clicking the meters also clears every clip light.
    if ui
        .interact(
            bar_area,
            egui::Id::new("master_meter"),
            egui::Sense::click(),
        )
        .clicked()
    {
        clear_all_clips(meters);
    }

    // Scale column: dB labels right-aligned on the same scale as the bars.
    for &t in &TICKS {
        let y = meter::db_y(bar_area, t);
        painter.text(
            Pos2::new(row_left + SCALE_W, y),
            Align2::RIGHT_CENTER,
            format!("{}", t as i32),
            Theme::mono(9.0),
            THEME.text_faint,
        );
    }

    // Fader: -60 to +12 dB, one undo step per drag, double-click resets.
    let fader_rect = Rect::from_min_max(
        Pos2::new(bars_left + 2.0 * BAR_W + BAR_GAP + BLOCK_GAP, mid.top()),
        Pos2::new(
            bars_left + 2.0 * BAR_W + BAR_GAP + BLOCK_GAP + FADER_W,
            mid.bottom(),
        ),
    );
    let out = fader::fader_bare(
        ui,
        egui::Id::new("master_fader"),
        fader_rect,
        editor.project.master_volume_db,
        (-60.0, 12.0),
    );
    if out.reset {
        let _ = editor.apply(Action::SetMasterVolume { db: 0.0 });
        return;
    }
    if out.started {
        editor.begin_gesture();
    }
    if let Some(db) = out.value {
        let _ = editor.preview(Action::SetMasterVolume { db });
    }
    if out.ended {
        editor.end_gesture();
    }
    if out.cancelled {
        editor.cancel_gesture();
    }

    // Peak readout: highest master peak since the last reset, as "-9.6".
    let readout = Rect::from_min_max(
        Pos2::new(content.left(), content.bottom() - READOUT_H - VALUE_H),
        Pos2::new(content.right(), content.bottom() - VALUE_H),
    );
    painter.rect_filled(readout, 5.0, THEME.recessed_a);
    painter.rect_stroke(
        readout,
        5.0,
        Stroke::new(THEME.hairline, THEME.control_border),
        StrokeKind::Inside,
    );
    let text = match meters.master_peak_db {
        Some(db) => format!("{db:.1}"),
        None => "-inf".to_string(),
    };
    let color = if meters.master_peak_db.is_some_and(|db| db >= 0.0) {
        THEME.meter_red
    } else {
        THEME.meter_yellow
    };
    painter.text(
        readout.center(),
        Align2::CENTER_CENTER,
        text,
        Theme::mono(11.5),
        color,
    );
    let resp = ui.interact(readout, egui::Id::new("master_peak"), egui::Sense::click());
    if resp.hovered() {
        painter.rect_filled(readout, 5.0, THEME.hover_overlay);
    }
    if resp.clicked() {
        meters.master_peak_db = None;
    }

    // Fader value under the readout, centered, never clipped.
    painter.text(
        Pos2::new(content.center().x, content.bottom() - VALUE_H * 0.5),
        Align2::CENTER_CENTER,
        fader::db_text(editor.project.master_volume_db),
        Theme::mono(10.5),
        THEME.text_normal,
    );
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
