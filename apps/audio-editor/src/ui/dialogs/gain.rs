//! ui::dialogs::gain: the Clip Gain and range Gain dialogs.

use std::sync::Arc;

use eframe::egui;
use uuid::Uuid;

use super::NARROW_W;
use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::selection::Selection;

/// Temp-storage key holding the open Clip Gain dialog.
const DIALOG_KEY: &str = "clip_gain_dialog";

/// Which behavior the gain dialog shows.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum GainMode {
    /// Clip Gain: sets absolute clip gain on the targeted clips.
    #[default]
    ClipGain,
    /// Gain: adds a delta to the selected range or clips (Process menu).
    Gain,
}

/// The gain dialog: type a dB value and apply it to one or more clips.
///
/// Lives in egui's temp storage; opened from the clip context menu, or in
/// [`GainMode::Gain`] mode from the Process menu and Ctrl+G.
#[derive(Clone, Default)]
pub struct GainDialog {
    /// Whether the dialog is shown.
    pub open: bool,
    /// Which dialog behavior is shown.
    pub mode: GainMode,
    /// The clips the value applies to (clip gain mode only).
    clip_ids: Vec<Uuid>,
    /// The dB value being edited.
    pub db: f32,
    /// Red error text shown inside the dialog, when set.
    pub error: Option<String>,
}

/// True when the clip is in the current clip selection.
fn clip_selected(editor: &Editor, id: Uuid) -> bool {
    matches!(
        editor.selection(),
        Selection::Clips(ids) if ids.contains(&id)
    )
}

/// Opens the Clip Gain dialog for a clip; when the clip is in the clip
/// selection the value applies to all selected clips. Called from the clip
/// context menu's "Clip Gain..." entry.
pub fn request(ctx: &egui::Context, clip_id: Uuid, editor: &Editor) {
    let clip_ids: Vec<Uuid> = if clip_selected(editor, clip_id) {
        match editor.selection() {
            Selection::Clips(ids) => ids.clone(),
            _ => vec![clip_id],
        }
    } else {
        vec![clip_id]
    };
    let db = editor.project.tracks.iter().flat_map(|track| track.clips.iter()).find(|clip| clip.id == clip_id).map(|clip| clip.gain_db).unwrap_or(0.0);
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_KEY), GainDialog { open: true, mode: GainMode::ClipGain, clip_ids, db, error: None }));
}

/// Opens the range Gain dialog: OK calls `Editor::apply_gain` on the current
/// process target with the given delta. Called from the Process menu and
/// Ctrl+G.
pub fn request_process(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_KEY), GainDialog { open: true, mode: GainMode::Gain, clip_ids: Vec::new(), db: 0.0, error: None }));
}

/// Shows the gain dialog when open. OK applies SetClipGain to every
/// target clip as one undo step; Cancel, the backdrop or Esc dismiss it.
/// Returns human-readable error messages from failed applies.
pub fn show(ctx: &egui::Context, editor: &mut Editor) -> Vec<String> {
    let Some(mut dialog) = ctx.data_mut(|d| d.remove_temp::<GainDialog>(egui::Id::new(DIALOG_KEY))) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    let (mut clicked, mut enter, mut cancel) = (false, false, false);
    if dialog.open {
        let (title, (min, max)) = if dialog.mode == GainMode::Gain { ("Gain", (-24.0, 24.0)) } else { ("Clip Gain", (-60.0, 24.0)) };
        let mut open = true;
        let error = dialog.error.clone();
        let db = &mut dialog.db;
        super::dialog(
            ctx,
            "gain_dialog",
            title,
            NARROW_W,
            &mut open,
            |ui| {
                ui_kit::help::context(ui, crate::help::CONTEXT_DIALOG_GAIN);
                super::description(ui, &format!("Gain change ({min:.0} to {max:+.0} dB)"));
                ui_kit::inspector_group(ui, |ui| {
                    ui_kit::inspector_row(ui, "Amount", |ui| super::db_control(ui, db, (min, max)), None);
                });
                if let Some(error) = &error {
                    super::error_text(ui, error);
                }
                // Enter confirms.
                enter = super::enter_pressed(ui);
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if ui_kit::primary_button(ui, "OK").clicked() {
                    clicked = true;
                }
            },
        );
        // Cancel, Esc or the backdrop dismisses the dialog.
        if cancel || !open {
            dialog.open = false;
        }
    }
    let ok = clicked || enter;
    if ok && dialog.open {
        match dialog.mode {
            GainMode::Gain => {
                dialog.error = None;
                match editor.apply_gain(dialog.db) {
                    Ok(()) => dialog.open = false,
                    Err(error) => dialog.error = Some(error.to_string()),
                }
            }
            GainMode::ClipGain => {
                messages.extend(apply_gain(editor, &dialog.clip_ids, dialog.db));
                dialog.open = false;
            }
        }
    }
    if dialog.open {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_KEY), dialog));
    }
    messages
}

/// Applies the gain to every target clip, recording one undo step in total.
fn apply_gain(editor: &mut Editor, clip_ids: &[Uuid], db: f32) -> Vec<String> {
    editor.begin_gesture();
    let mut messages = Vec::new();
    for id in clip_ids {
        if editor.preview(Action::SetClipGain { clip_id: *id, db }).is_ok() {
            // Rebase so the next clip's preview stacks on this change.
            editor.gesture_base = Some(Arc::clone(&editor.project));
        } else {
            messages.push(format!("Could not set gain on clip {id}"));
        }
    }
    editor.end_gesture();
    messages
}
