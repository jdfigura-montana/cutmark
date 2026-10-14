//! ui::dialogs::normalize: the Normalize dialog.

use eframe::egui;

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
    ctx.data_mut(|d| {
        d.insert_temp(
            egui::Id::new(DIALOG_KEY),
            NormalizeDialog {
                open: true,
                target_dbfs: -1.0,
                per_track: false,
                error: None,
            },
        )
    });
}

/// Shows the Normalize dialog when open. OK calls `Editor::normalize` (one
/// undo step); errors show in red inside the dialog and keep it open.
/// Cancel, the close box or Esc dismisses it.
pub fn show(ctx: &egui::Context, editor: &mut Editor) -> Vec<String> {
    let Some(mut dialog) =
        ctx.data_mut(|d| d.remove_temp::<NormalizeDialog>(egui::Id::new(DIALOG_KEY)))
    else {
        return Vec::new();
    };
    let mut ok = false;
    if dialog.open {
        let mut open = true;
        let mut closed = false;
        egui::Window::new("Normalize")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Target peak (dBFS):");
                let resp = ui.add(
                    egui::DragValue::new(&mut dialog.target_dbfs)
                        .range(-60.0..=0.0)
                        .speed(0.1)
                        .suffix(" dBFS"),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    ok = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    closed = true;
                }
                ui.checkbox(&mut dialog.per_track, "Normalize each track separately");
                if let Some(error) = &dialog.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            ok = true;
                        }
                        if ui.button("Cancel").clicked() {
                            closed = true;
                        }
                    });
                });
            });
        if closed {
            open = false;
        }
        dialog.open = open;
    }
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
