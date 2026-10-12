//! app: the eframe::App; start screen vs editor.

use eframe::egui;

use crate::editor::Editor;
use crate::ui::start_screen::StartScreen;

/// The currently shown screen.
enum Screen {
    /// Start screen with New/Open/Recent.
    Start(StartScreen),
    /// An open project being edited.
    Editing(Editor),
}

/// Root application state shown in the main window.
pub struct App {
    /// Which screen is currently shown.
    screen: Screen,
}

impl Default for App {
    fn default() -> Self {
        App {
            screen: Screen::Start(StartScreen::default()),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        match &mut self.screen {
            Screen::Start(start) => {
                if let Some(editor) = start.show(ui) {
                    self.screen = Screen::Editing(editor);
                }
            }
            Screen::Editing(_editor) => draw_editor_panels(ui),
        }
    }
}

/// Draws the panel layout from task 1.1 (placeholder content for now).
fn draw_editor_panels(ui: &mut egui::Ui) {
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

/// Draws a label in grey, naming a panel region.
fn grey_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(egui::Color32::GRAY));
}
