//! ui::master_strip: the fixed-width Master fader and meter strip.

use eframe::egui::{self, Pos2, Rect};

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::ui::theme::{THEME, Theme};
use crate::ui::widgets::fader;
use crate::ui::widgets::meter::{self, MeterState};

/// Draws the master strip: vertical fader, labeled meter with clip lights and
/// the clickable peak readout.
///
/// Clicking the meter's clip lights clears the master lights and every
/// track's; clicking the peak readout resets it.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, meters: &mut MeterState) {
    ui.label(egui::RichText::new("Master").color(ui.visuals().strong_text_color()));
    ui.add_space(THEME.space_sm);

    let avail = ui.available_rect_before_wrap();
    let pad = THEME.master_pad;
    let meter_rect = Rect::from_min_max(
        Pos2::new(avail.left() + pad, avail.top() + pad),
        Pos2::new(
            avail.left() + pad + THEME.master_meter_w,
            avail.bottom() - THEME.master_readout_h - pad,
        ),
    );
    let fader_rect = Rect::from_min_max(
        Pos2::new(meter_rect.right() + pad, meter_rect.top()),
        Pos2::new(
            meter_rect.right() + pad + THEME.master_fader_w,
            meter_rect.bottom(),
        ),
    );

    // Tall labeled meter; a click clears every clip light (master + tracks).
    let clicked = meter::stereo_meter(
        ui,
        egui::Id::new("master_meter"),
        meter_rect,
        &mut meters.master,
        true,
    );
    if clicked {
        for ch in meters.tracks.values_mut() {
            ch[0].clear_clip();
            ch[1].clear_clip();
        }
    }

    // Vertical master fader, -60 to +12 dB, one undo step per drag.
    let out = fader::slider(
        ui,
        egui::Id::new("master_fader"),
        fader_rect,
        editor.project.master_volume_db,
        (-60.0, 12.0),
        true,
        fader::db_text,
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

    // Peak readout: highest master peak since the last reset.
    let readout_rect = Rect::from_min_max(
        Pos2::new(avail.left() + pad, avail.bottom() - THEME.master_readout_h),
        Pos2::new(avail.right() - pad, avail.bottom()),
    );
    let text = match meters.master_peak_db {
        Some(db) => format!("{db:.1} dBFS"),
        None => "-inf dBFS".to_string(),
    };
    let color = if meters.master_peak_db.is_some_and(|db| db >= 0.0) {
        THEME.meter_red
    } else {
        THEME.fader_text
    };
    let painter = ui.painter_at(readout_rect);
    // The peak readout is a recessed timecode-style box.
    painter.rect_filled(readout_rect, THEME.corner_xs, THEME.recessed_b);
    painter.text(
        Pos2::new(readout_rect.center().x, readout_rect.center().y),
        egui::Align2::CENTER_CENTER,
        text,
        Theme::mono(THEME.fader_text_size),
        color,
    );
    let resp = ui.interact(
        readout_rect,
        egui::Id::new("master_peak"),
        egui::Sense::click(),
    );
    // Hover highlight for this clickable readout.
    if resp.hovered() {
        painter.rect_filled(readout_rect, THEME.corner_r, THEME.hover_overlay);
    }
    if resp.clicked() {
        meters.master_peak_db = None;
    }
}
