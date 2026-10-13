//! ui::toolbar: the toolbar with transport buttons, toggles and Add Track.

use eframe::egui;

use crate::editor::Editor;
use crate::editor::action::Action;

/// Draws the toolbar row under the menu.
///
/// The left part is reserved for the transport buttons (phase 5); for now it
/// only holds an Add Track button, appending one track (Ctrl+Shift+T).
pub fn show(ui: &mut egui::Ui, editor: &mut Editor) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // Reserved space for the transport buttons (phase 5).
        ui.add_space(140.0);
        if ui
            .button("Add Track")
            .on_hover_text("Add a new track (Ctrl+Shift+T)")
            .clicked()
        {
            let _ = editor.apply(Action::AddTrack {
                index: None,
                name: None,
            });
        }
    });
    ui.add_space(2.0);
}
