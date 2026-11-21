//! ui::dialogs::normalize: the Normalize dialog.

use eframe::egui;

use super::NARROW_W;
use crate::editor::Editor;

/// Temp-storage key holding the open Normalize dialog.
const DIALOG_KEY: &str = "normalize_dialog";

/// The Normalize dialog: pick a target peak and apply it to the process
/// target. Lives in egui's temp storage; opened from the Process menu and
/// Ctrl+Shift+N.
#[derive(Clone, Default)]
pub struct NormalizeDialog {
    /// Whether the dialog is shown.
    pub open: bool,
    /// Target peak in dBFS.
    pub target_dbfs: f32,
    /// Whether each track normalizes to its own peak.
    pub per_track: bool,
    /// Red error text shown inside the dialog, when set.
    pub error: Option<String>,
}

/// Opens the Normalize dialog with the default target (-1.0 dBFS) and
/// per-track normalization off.
pub fn request(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_KEY), NormalizeDialog { open: true, target_dbfs: -1.0, per_track: false, error: None }));
}

/// Shows the Normalize dialog when open. OK calls `Editor::normalize` (one
/// undo step); errors show in red inside the dialog and keep it open.
/// Cancel, the backdrop or Esc dismisses it.
pub fn show(ctx: &egui::Context, editor: &mut Editor) -> Vec<String> {
    let Some(mut dialog) = ctx.data_mut(|d| d.remove_temp::<NormalizeDialog>(egui::Id::new(DIALOG_KEY))) else {
        return Vec::new();
    };
    let (mut clicked, mut enter, mut cancel) = (false, false, false);
    if dialog.open {
        let mut open = true;
        let error = dialog.error.clone();
        let (target, per_track) = (&mut dialog.target_dbfs, &mut dialog.per_track);
        super::dialog(
            ctx,
            "normalize_dialog",
            "Normalize",
            NARROW_W,
            &mut open,
            |ui| {
                super::description(ui, "Raise the loudest peak to the target level.");
                ui_kit::inspector_group(ui, |ui| {
                    ui_kit::inspector_row(ui, "Target peak", |ui| super::db_control(ui, target, (-60.0, 0.0)), None);
                    ui_kit::inspector_row(
                        ui,
                        "Normalize each track separately",
                        |ui| {
                            ui_kit::switch(ui, per_track);
                        },
                        None,
                    );
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
        dialog.error = None;
        match editor.normalize(dialog.target_dbfs, dialog.per_track) {
            Ok(()) => dialog.open = false,
            Err(error) => dialog.error = Some(error.to_string()),
        }
    }
    if dialog.open {
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(DIALOG_KEY), dialog));
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_target_and_per_track() {
        let ctx = egui::Context::default();
        request(&ctx);
        ctx.data_mut(|d| {
            let dialog = d.get_temp_mut_or_default::<NormalizeDialog>(egui::Id::new(DIALOG_KEY));
            assert!(dialog.open);
            assert_eq!(dialog.target_dbfs, -1.0);
            assert!(!dialog.per_track);
            assert!(dialog.error.is_none());
        });
    }
}
