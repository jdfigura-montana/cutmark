//! app: the eframe::App; start screen vs editor.

use std::path::PathBuf;

use eframe::egui;
use eframe::egui::ViewportCommand;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::transport::TransportState;
use crate::ui::bin_panel::{self, BinPanel};
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::menu::{ProjectMenu, ProjectMenuAction};
use crate::ui::start_screen::StartScreen;
use crate::ui::status_bar;
use crate::ui::timeline;
use crate::ui::toolbar;

/// The currently shown screen.
enum Screen {
    /// Start screen with New/Open/Recent.
    Start(StartScreen),
    /// An open project being edited.
    Editing(Box<Editor>),
}

/// Root application state shown in the main window.
pub struct App {
    /// Which screen is currently shown.
    screen: Screen,
    /// The Project menu with its save-error modal.
    menu: ProjectMenu,
    /// The media bin panel state.
    bin: BinPanel,
    /// Modal message dialog (import rejections, job errors), when shown.
    messages: Option<ConfirmDialog>,
    /// Last window title sent to the OS, to avoid resending every frame.
    title: String,
}

impl Default for App {
    fn default() -> Self {
        App {
            screen: Screen::Start(StartScreen::default()),
            menu: ProjectMenu::default(),
            bin: BinPanel::default(),
            messages: None,
            title: "Audio Editor".to_string(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_title(ui);
        // Work on the screen outside self so menu, editor and screen can be
        // borrowed separately within one frame.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Start(StartScreen::default()));
        let mut opened: Option<Editor> = None;
        let mut action = ProjectMenuAction::None;
        match &mut screen {
            Screen::Start(start) => {
                if !ui.ctx().egui_wants_keyboard_input() && !start.dialog_is_open() {
                    Self::handle_start_shortcuts(ui, start);
                }
                opened = start.show(ui);
            }
            Screen::Editing(editor) => {
                action = Self::show_editor(
                    ui,
                    editor,
                    &mut self.menu,
                    &mut self.bin,
                    &mut self.messages,
                );
            }
        }
        if let Some(mut editor) = opened {
            editor.start_audio();
            screen = Screen::Editing(Box::new(editor));
        }
        match action {
            ProjectMenuAction::None => {}
            ProjectMenuAction::New => screen = Screen::Start(StartScreen::with_new_dialog()),
            ProjectMenuAction::Open => screen = Screen::Start(StartScreen::with_open_request()),
            ProjectMenuAction::Close => screen = Screen::Start(StartScreen::default()),
            ProjectMenuAction::Import => {}
        }
        self.screen = screen;
    }
}

impl App {
    /// Draws the editor layout, handles the menus and the editor shortcuts.
    fn show_editor(
        ui: &mut egui::Ui,
        editor: &mut Editor,
        menu: &mut ProjectMenu,
        bin: &mut BinPanel,
        messages: &mut Option<ConfirmDialog>,
    ) -> ProjectMenuAction {
        // Pump background jobs and keep repainting while any are running.
        editor.poll_jobs();
        // Retire old mix plans, keep the audio status fresh and mirror the
        // audio playhead into the view.
        editor.tick();
        editor.poll_transport();
        if editor.has_pending_jobs()
            || matches!(
                editor.transport_state(),
                TransportState::Playing | TransportState::Previewing
            )
        {
            ui.ctx().request_repaint();
        }

        let mut msgs: Vec<String> = Vec::new();
        // Top: menu bar with the Project, Edit and View menus, then the toolbar row.
        let (mut action, view_action) = menu.show(ui, editor);

        if action == ProjectMenuAction::Import {
            msgs.extend(bin_panel::pick_and_import(editor));
            action = ProjectMenuAction::None;
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            toolbar::show(ui, editor);
        });

        // Bottom: status bar.
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            status_bar::show(ui, editor);
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
                msgs.extend(bin.show(ui, editor));
            });

        // Center: timeline, filling the remainder.
        egui::CentralPanel::default().show(ui, |ui| {
            msgs.extend(timeline::show(ui, editor, view_action));
        });

        // Files dropped anywhere on the window go to the importer.
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            msgs.extend(editor.import_files(dropped));
        }

        // Editor shortcuts, only when no text field has keyboard focus.
        if !ui.ctx().egui_wants_keyboard_input() {
            let (ctrl, shift, n, o, s, i, z, y, t, r, a, escape) = ui.input(|i| {
                (
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                    i.key_pressed(egui::Key::N),
                    i.key_pressed(egui::Key::O),
                    i.key_pressed(egui::Key::S),
                    i.key_pressed(egui::Key::I),
                    i.key_pressed(egui::Key::Z),
                    i.key_pressed(egui::Key::Y),
                    i.key_pressed(egui::Key::T),
                    i.key_pressed(egui::Key::R),
                    i.key_pressed(egui::Key::A),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if ctrl && a {
                editor.select_all();
            } else if ctrl && i {
                msgs.extend(bin_panel::pick_and_import(editor));
            } else if ctrl && z {
                if shift {
                    let _ = editor.redo();
                } else {
                    let _ = editor.undo();
                }
            } else if ctrl && y {
                let _ = editor.redo();
            } else if ctrl && n {
                action = ProjectMenuAction::New;
            } else if ctrl && o {
                action = ProjectMenuAction::Open;
            } else if ctrl && shift && s {
                menu.open_save_as(editor);
            } else if ctrl && shift && t {
                let _ = editor.apply(Action::AddTrack {
                    index: None,
                    name: None,
                });
            } else if ctrl && s {
                menu.save(editor);
            } else if escape {
                editor.clear_selection();
            } else if r && !ctrl {
                editor.toggle_ripple();
            } else if n && !ctrl {
                editor.toggle_snap();
            }
        }

        // Transport keys (Space is consumed so a focused button does not also
        // activate).
        if !ui.ctx().egui_wants_keyboard_input() {
            ui.input_mut(|i| {
                if i.key_pressed(egui::Key::Space) && i.modifiers.is_none() {
                    editor.toggle_play();
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
                } else if i.key_pressed(egui::Key::Space)
                    && i.modifiers.shift
                    && !i.modifiers.ctrl
                    && let crate::editor::selection::Selection::Range { start, end, .. } =
                        editor.selection().clone()
                {
                    // Play selection: plays the range once, then returns to its
                    // start. Without a range, Shift+Space does nothing.
                    editor.play_range(start, end);
                    i.consume_key(egui::Modifiers::SHIFT, egui::Key::Space);
                } else if i.key_pressed(egui::Key::P) && i.modifiers.is_none() {
                    editor.toggle_pause();
                    i.consume_key(egui::Modifiers::NONE, egui::Key::P);
                } else if i.key_pressed(egui::Key::Home) && i.modifiers.is_none() {
                    editor.jump_start();
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Home);
                } else if i.key_pressed(egui::Key::End) && i.modifiers.is_none() {
                    editor.jump_end();
                    i.consume_key(egui::Modifiers::NONE, egui::Key::End);
                }
            });
        }

        // Background job errors from this frame.
        msgs.extend(editor.take_job_errors());
        if !msgs.is_empty() {
            *messages = Some(ConfirmDialog::new("Media", msgs.join("\n")));
        }
        if let Some(dialog) = messages {
            dialog.show(&ui.ctx().clone());
        }
        action
    }

    /// Handles Ctrl+N (New Project dialog) and Ctrl+O (Open picker) on the start screen.
    fn handle_start_shortcuts(ui: &egui::Ui, start: &mut StartScreen) {
        let (ctrl, n, o) = ui.input(|i| {
            (
                i.modifiers.ctrl,
                i.key_pressed(egui::Key::N),
                i.key_pressed(egui::Key::O),
            )
        });
        if ctrl && n {
            *start = StartScreen::with_new_dialog();
        } else if ctrl && o {
            *start = StartScreen::with_open_request();
        }
    }

    /// Sends the window title to the OS when it changed:
    /// "*<name> - Audio Editor" when dirty, "<name> - Audio Editor" otherwise.
    fn update_title(&mut self, ui: &egui::Ui) {
        let title = match &self.screen {
            Screen::Start(_) => "Audio Editor".to_string(),
            Screen::Editing(editor) => format!(
                "{}{} - Audio Editor",
                if editor.dirty { "*" } else { "" },
                editor.project.name
            ),
        };
        if title != self.title {
            self.title = title.clone();
            ui.ctx().send_viewport_cmd(ViewportCommand::Title(title));
        }
    }
}

/// Draws a label in grey, naming a panel region.
fn grey_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(egui::Color32::GRAY));
}
