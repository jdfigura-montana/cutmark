//! The custom window chrome with the real main toolbar (S1 phase 7b): the
//! 52px toolbar is the title bar, the caption buttons work, and
//! `main_toolbar` publishes the chrome regions itself, so no manual
//! `chrome::publish` call is needed. Run with:
//! `cargo run -p ui-kit --example chrome`.
//!
//! Hand-check list (Windows 11): the toolbar drags, double-click on it
//! maximizes and restores, dragging to a screen edge snaps, every edge and
//! corner resizes, Win+Left/Right/Up work, hovering the maximize button
//! shows the Snap Layouts flyout, corners are round; minimize, maximize and
//! close work; clicking the segmented control or the primary button never
//! drags the window; another window on top dims the toolbar text.

use eframe::egui;
use egui_phosphor::variants::regular;
use ui_kit::{Theme, fonts, tokens};

/// The demo app: draws the main toolbar and a status line.
struct ChromeDemo {
    /// The selected mode of the segmented control.
    mode: std::rc::Rc<std::cell::Cell<usize>>,
}

/// Draws the app mark placeholder: an accent circle with a notch cut from its
/// top right (spec "Open questions" item 1: placeholder until a real logo).
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    painter.circle_filled(c, 9.0, Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x, c.y - 9.0), egui::vec2(9.0, 9.0)), 0.0, tokens::color::BG_GROUND);
}

/// Draws the left zone: app mark, document name with its caption line, Undo
/// and Redo.
fn left_zone(ui: &mut egui::Ui) {
    app_mark(ui);
    ui.add_space(tokens::space::SCALE_4);
    ui.vertical(|ui| {
        ui.label(egui::RichText::new("Interview take 3").font(fonts::semibold(tokens::typography::BODY_SIZE)));
        ui.label(egui::RichText::new("Saved \u{b7} WAV").font(fonts::medium(tokens::typography::CAPTION_SIZE)));
    });
    ui.add_space(tokens::space::SCALE_6);
    ui_kit::icon_button(ui, regular::ARROW_COUNTER_CLOCKWISE, "Undo");
    ui_kit::icon_button(ui, regular::ARROW_CLOCKWISE, "Redo");
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
            move |ui| {
                let mut m = mode.get();
                ui_kit::segmented(ui, &mut m, &["Edit", "Mix"]);
                mode.set(m);
            },
            |ui| {
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
