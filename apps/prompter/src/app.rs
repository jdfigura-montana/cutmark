//! The Prompter's root window.

use prompter_platform::os::OsError;

/// The Prompter's appearance identity: its settings folder name and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("prompter", ui_kit::Preset::Violet);

/// The root window's state.
pub struct PrompterApp {
    /// False when the Windows 11 check failed: only the dialog is shown, then the app exits.
    supported: bool,
}

impl PrompterApp {
    /// Takes the result of the startup OS check.
    pub fn new(os_check: Result<(), OsError>) -> Self {
        if let Err(err) = &os_check {
            log::error!("unsupported system: {err}");
        }
        Self { supported: os_check.is_ok() }
    }
}

impl eframe::App for PrompterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            if self.supported {
                ui_kit::panel(ui, |ui| {
                    ui.label("Library comes in V4");
                });
            } else {
                unsupported_dialog(ui.ctx());
            }
        });
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
