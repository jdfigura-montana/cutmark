//! Minimal window to hand-check the Win32 chrome (S1 phase 7a): a 52px
//! caption bar with a placeholder maximize button, a label with the chrome
//! state. Run with: `cargo run -p ui-kit --example chrome`.
//!
//! Hand-check list (Windows 11): the bar drags, double-click on it maximizes
//! and restores, dragging to a screen edge snaps, every edge and corner
//! resizes, Win+Left/Right/Up work, hovering the maximize rect shows the Snap
//! Layouts flyout, corners are round.

use eframe::egui;
use ui_kit::tokens;

/// The caption bar height, from the design language.
const BAR_HEIGHT: f32 = 52.0;
/// The placeholder maximize button size, from the design language.
const BUTTON_SIZE: f32 = 46.0;

/// The demo app: publishes the chrome regions and shows the state.
#[derive(Default)]
struct ChromeDemo {}

impl eframe::App for ChromeDemo {
    /// Publishes the chrome regions and draws the bar and the status label.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let screen = ui.max_rect();
        let width = screen.width();
        let bar = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, BAR_HEIGHT));
        let maximize = egui::Rect::from_min_size(egui::pos2(width - BUTTON_SIZE - 8.0, 3.0), egui::vec2(BUTTON_SIZE, BUTTON_SIZE));
        ui_kit::chrome::publish(ui.ctx(), &ui_kit::chrome::Regions::new(vec![bar]).with_maximize_button(maximize));

        let hover = ui_kit::chrome::is_maximized_button_hovered(ui.ctx());
        let glyph = if hover { tokens::color::TEXT_PRIMARY } else { tokens::color::TEXT_SECONDARY };
        let painter = ui.painter();
        painter.rect_filled(screen, 0.0, tokens::color::BG_GROUND);
        // Placeholder maximize glyph: a 1px square outline.
        painter.rect_stroke(maximize.shrink(14.0), 0.0, egui::Stroke::new(1.0, glyph), egui::StrokeKind::Inside);
        let status = format!(
            "custom chrome: {} | maximize hovered: {} | drag the top bar, resize from any edge, hover the square for Snap Layouts",
            ui_kit::chrome::is_custom(),
            hover
        );
        ui.put(egui::Rect::from_min_size(egui::pos2(12.0, BAR_HEIGHT + 16.0), egui::vec2(width - 24.0, 24.0)), egui::Label::new(status));
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
            Ok(Box::new(ChromeDemo::default()))
        }),
    )
}
