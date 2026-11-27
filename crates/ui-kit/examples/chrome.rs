//! The custom window chrome with the real main toolbar (S1 phase 7b, rev 3
//! layout): the 44px toolbar is the title bar, with a menu bar on the left,
//! the document title in the center, and the actions, a separator and the
//! caption buttons on the right. The caption buttons work, and `main_toolbar`
//! publishes the chrome regions itself, so no manual `chrome::publish` call is
//! needed. Run with: `cargo run -p ui-kit --example chrome`.
//!
//! Hand-check list (Windows 11): the toolbar drags, double-click on it
//! maximizes and restores, dragging to a screen edge snaps, every edge and
//! corner resizes, Win+Left/Right/Up work, hovering the maximize button
//! shows the Snap Layouts flyout, corners are round; minimize, maximize and
//! close work; clicking a menu title, the segmented control or the primary
//! button never drags the window; clicking a menu title opens its menu and
//! hovering the other title switches menus; another window on top dims the
//! toolbar text.

use eframe::egui;
use ui_kit::{Menu, Title, brand, tokens};

/// The brand art in the title bar. The demo is no app of its own, so it
/// borrows the blue one.
const MARK: brand::App = brand::App::Prompter;

/// The demo app: draws the main toolbar and a status line.
struct ChromeDemo {
    /// The selected mode of the segmented control.
    mode: std::rc::Rc<std::cell::Cell<usize>>,
}

/// Draws the left zone: the app mark, then a File and an Edit menu.
fn left_zone(ui: &mut egui::Ui) {
    brand::mark(ui, MARK, brand::TITLE_BAR_MARK);
    ui.add_space(tokens::space::SCALE_8);
    let mut menus = [
        Menu::new("File", |ui| {
            for item in ["New", "Open...", "Save"] {
                if ui.button(item).clicked() {
                    ui.close();
                }
            }
        }),
        Menu::new("Edit", |ui| {
            for item in ["Undo", "Redo"] {
                if ui.button(item).clicked() {
                    ui.close();
                }
            }
        }),
    ];
    ui_kit::menu_bar(ui, &mut menus);
}

impl eframe::App for ChromeDemo {
    /// Draws the main toolbar, which publishes the chrome regions, and a
    /// status line under it.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, tokens::color::BG_GROUND);
        let mode = self.mode.clone();
        // The toolbar publishes the chrome regions (caption drag areas plus
        // the maximize button) every frame; nothing else is needed.
        ui_kit::main_toolbar(
            ui,
            left_zone,
            |ui| {
                ui_kit::title_block(ui, &Title::Document { name: "Interview take 3", unsaved: true, caption: Some("48 kHz \u{b7} 24-bit \u{b7} 4 tracks") });
            },
            move |ui| {
                let mut m = mode.get();
                ui_kit::segmented(ui, "chrome-modes", &mut m, &["Edit", "Mix"]);
                mode.set(m);
                ui.add_space(tokens::space::SCALE_6);
                ui_kit::primary_button(ui, "Export");
            },
        );
        let status = format!(
            "custom chrome: {} \u{b7} drag the toolbar, double-click to maximize, snap from the edges, hover the square for Snap Layouts",
            ui_kit::chrome::is_custom(),
        );
        ui.put(
            egui::Rect::from_min_size(egui::pos2(12.0, tokens::size::TOOLBAR_MAIN_HEIGHT + 16.0), egui::vec2(ui.available_width() - 24.0, 24.0)),
            egui::Label::new(status),
        );
    }
}

/// Starts the demo window with the chrome viewport and install hooks.
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: ui_kit::chrome::viewport(egui::ViewportBuilder::default().with_inner_size([1024.0, 640.0])),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Cutmark chrome check",
        options,
        Box::new(|cc| {
            ui_kit::apply(&cc.egui_ctx);
            ui_kit::chrome::install(cc);
            Ok(Box::new(ChromeDemo { mode: std::rc::Rc::new(std::cell::Cell::new(0)) }))
        }),
    )
}
