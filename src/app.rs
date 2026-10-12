//! app: the eframe::App; start screen vs editor (phase 1 shows empty panels).

use eframe::egui;

/// Root application state shown in the main window.
#[derive(Default)]
pub struct App;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Top: menu bar with an empty "Project" menu, then the toolbar row.
        egui::Panel::top("menu_bar").show(ui, |ui| {
            ui.menu_button("Project", |_menu| {});
            grey_label(ui, "Menu Bar");
        });
        egui::Panel::top("toolbar").show(ui, |ui| {
            grey_label(ui, "Toolbar");
        });

        // Bottom: status bar.
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            grey_label(ui, "Status");
        });

        // Right: Master strip, fixed width.
        egui::Panel::right("master_strip")
            .resizable(false)
            .exact_size(90.0)
            .show(ui, |ui| {
                grey_label(ui, "Master");
            });

        // Left: resizable Media Bin panel.
        egui::Panel::left("media_bin")
            .resizable(true)
            .default_size(220.0)
            .show(ui, |ui| {
                ui.take_available_space();
                grey_label(ui, "Media Bin");
            });

        // Center: timeline, filling the remainder.
        egui::CentralPanel::default().show(ui, |ui| {
            grey_label(ui, "Timeline");
        });
    }
}

/// Draws a label in grey, naming a panel region.
fn grey_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(egui::Color32::GRAY));
}
