//! The Prompter's root window.

use prompter_platform::os::OsError;
use prompter_platform::view::{NativeWindow, ViewStatus, ViewWindow};

use crate::renderer::AlphaInUse;
use crate::view::{self, FloatingView, log_failure};

/// The Prompter's appearance identity: its settings folder name and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("prompter", ui_kit::Preset::Violet);

/// The root window's title, which is also how its window handle is found.
pub const ROOT_TITLE: &str = "Cutmark Prompter";

/// How many frames to look for the root window before giving up.
const ROOT_FIND_FRAMES: u8 = 30;

/// The root window's state.
pub struct PrompterApp {
    /// False when the Windows 11 check failed: only the dialog is shown, then the app exits.
    supported: bool,
    alpha: AlphaInUse,
    view: Option<FloatingView>,
    /// The number of the last view opened. Numbers only go up, so no two windows share a title.
    last_view_number: u32,
    click_through: bool,
    /// The size of the monitor the root window opened on, taken to be the primary one.
    monitor: Option<egui::Vec2>,
    /// Frames left to find and protect the root window; 0 once done.
    root_find_frames: u8,
}

impl PrompterApp {
    /// Takes the result of the startup OS check.
    pub fn new(os_check: Result<(), OsError>, alpha: AlphaInUse) -> Self {
        if let Err(err) = &os_check {
            log::error!("unsupported system: {err}");
        }
        Self { supported: os_check.is_ok(), alpha, view: None, last_view_number: 0, click_through: false, monitor: None, root_find_frames: ROOT_FIND_FRAMES }
    }

    /// Keeps the Library out of screen captures.
    fn protect_root(&mut self, ctx: &egui::Context) {
        if self.root_find_frames == 0 {
            return;
        }
        self.root_find_frames -= 1;
        if let Some(window) = NativeWindow::find_own_window(ROOT_TITLE) {
            log_failure("exclude_from_capture on the Library", window.exclude_from_capture());
            log::info!("Library excluded from capture: {}", window.status().excluded);
            self.root_find_frames = 0;
        } else if self.root_find_frames == 0 {
            log::warn!("the Library window was not found; it is not excluded from capture");
        } else {
            ctx.request_repaint();
        }
    }

    /// The temporary view controls and diagnostics line, replaced in V4.
    fn view_controls(&mut self, ui: &mut egui::Ui) {
        // egui-winit reports Ctrl with C as a copy even when Alt is held, so no C key arrives.
        let hotkey = ui.input(|i| i.modifiers.ctrl && i.modifiers.alt && i.events.iter().any(|event| matches!(event, egui::Event::Copy)));
        if hotkey {
            self.click_through = !self.click_through;
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.view.is_none(), egui::Button::new("Show view")).clicked() {
                self.last_view_number += 1;
                self.view = Some(FloatingView::new(self.last_view_number, self.monitor.map(view::default_position)));
            }
            if ui.add_enabled(self.view.is_some(), egui::Button::new("Hide view")).clicked() {
                self.view = None;
            }
            let label = if self.click_through { "Click-through off" } else { "Click-through on" };
            if ui.button(label).on_hover_text("Ctrl+Alt+C").clicked() {
                self.click_through = !self.click_through;
            }
        });
        let status = self.view.as_ref().map(|view| view.status());
        ui.label(diagnostics(self.alpha.get().map(|mode| format!("{mode:?}")), status));
    }
}

/// The diagnostics line. `view` is `None` with no view open, and `Some(None)` while the open
/// view's window has not been found.
fn diagnostics(alpha: Option<String>, view: Option<Option<ViewStatus>>) -> String {
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    let alpha = alpha.unwrap_or_else(|| "unknown".to_owned());
    let view = match view {
        None => "hidden".to_owned(),
        Some(None) => "window not found".to_owned(),
        Some(Some(s)) => format!("excluded {}, no-activate {}, topmost {}", yes_no(s.excluded), yes_no(s.no_activate), yes_no(s.topmost)),
    };
    format!("Alpha mode: {alpha} | View: {view}")
}

impl eframe::App for PrompterApp {
    /// The root viewport is transparent only so that eframe picks a transparent alpha mode for
    /// the views; the Library itself stays opaque.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui_kit::tokens::color::BG_GROUND.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.monitor.is_none() {
            self.monitor = ui.input(|i| i.viewport().monitor_size);
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if self.supported {
                ui_kit::panel(ui, |ui| self.view_controls(ui));
            } else {
                unsupported_dialog(ui.ctx());
            }
        });
        if !self.supported {
            return;
        }
        self.protect_root(ui.ctx());
        if let Some(view) = &mut self.view
            && !view.show(ui.ctx(), self.click_through)
        {
            self.view = None;
        }
    }
}

/// The one dialog an unsupported system sees. OK, Esc and a backdrop click all exit.
fn unsupported_dialog(ctx: &egui::Context) {
    let mut open = true;
    let mut accepted = false;
    ui_kit::dialog(
        ctx,
        egui::Id::new("unsupported-os"),
        "Cutmark Prompter",
        &mut open,
        |ui| {
            ui.label("Cutmark Prompter needs Windows 11.");
        },
        |ui| accepted = ui_kit::primary_button(ui, "OK").clicked(),
    );
    if accepted || !open {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
