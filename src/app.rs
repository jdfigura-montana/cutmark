//! app: the eframe::App; start screen vs editor.

use std::path::PathBuf;

use eframe::egui;
use eframe::egui::ViewportCommand;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::transport::TransportState;
use crate::ui::bin_panel::{self, BinPanel};
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::master_strip;
use crate::ui::menu::{ProjectMenu, ProjectMenuAction};
use crate::ui::start_screen::StartScreen;
use crate::ui::status_bar;
use crate::ui::timeline;
use crate::ui::toolbar;
use crate::ui::widgets::meter::MeterState;

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
    /// UI-side meter ballistics: tracks, master and peak readout.
    meters: MeterState,
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
            meters: MeterState::default(),
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
                    &mut self.meters,
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
        meters: &mut MeterState,
    ) -> ProjectMenuAction {
        // Pump background jobs and keep repainting while any are running.
        editor.poll_jobs();
        // Retire old mix plans, keep the audio status fresh and mirror the
        // audio playhead into the view.
        editor.tick();
        editor.poll_transport();
        let transport_active = matches!(
            editor.transport_state(),
            TransportState::Playing | TransportState::Previewing
        );
        if editor.has_pending_jobs() || transport_active {
            ui.ctx().request_repaint();
        }

        // Meter ballistics advance with egui's frame time; keep repainting
        // for one more second after playback stops so meters fall to empty.
        if transport_active {
            meters.mark_active();
        }
        let (now, dt) = ui.input(|i| {
            (
                i.time,
                if i.stable_dt > 0.0 {
                    i.stable_dt
                } else {
                    1.0 / 60.0
                },
            )
        });
        let (track_readings, master_readings) = editor.read_meters();
        meters.update(&track_readings, &master_readings, now, dt);
        if meters.meter_tail() {
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
                master_strip::show(ui, editor, meters);
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
            msgs.extend(timeline::show(ui, editor, view_action, meters));
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
            let (ctrl, shift, n, o, s, i, z, y, t, r, a, c, x, v, escape, del, back, g, l) = ui
                .input(|i| {
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
                        i.key_pressed(egui::Key::C),
                        i.key_pressed(egui::Key::X),
                        i.key_pressed(egui::Key::V),
                        i.key_pressed(egui::Key::Escape),
                        i.key_pressed(egui::Key::Delete),
                        i.key_pressed(egui::Key::Backspace),
                        i.key_pressed(egui::Key::G),
                        i.key_pressed(egui::Key::L),
                    )
                });
            // A continuous gesture is active: undo/redo keys are ignored.
            let gesture_active = editor.gesture_base.is_some();
            if ctrl && a {
                editor.select_all();
            } else if ctrl && i && !shift {
                msgs.extend(bin_panel::pick_and_import(editor));
            } else if ctrl && z && !gesture_active {
                if shift {
                    let _ = editor.redo();
                } else {
                    let _ = editor.undo();
                }
            } else if ctrl && y && !gesture_active {
                let _ = editor.redo();
            } else if ctrl && n && !shift {
                action = ProjectMenuAction::New;
            } else if ctrl && o && !shift {
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
            } else if del || back {
                Self::delete_selection(editor);
            } else if escape {
                editor.clear_selection();
            } else if r && !ctrl {
                editor.toggle_ripple();
            } else if n && !ctrl {
                editor.toggle_snap();
            } else if s && !ctrl {
                let _ = editor.split();
            } else if ctrl && c {
                editor.copy_selection();
            } else if ctrl && x && !gesture_active {
                let _ = editor.cut_selection();
            } else if ctrl && v {
                let _ = editor.paste();
            } else if ctrl && t {
                let _ = editor.trim_to_selection();
            } else if ctrl && g && !shift {
                // Process keys. egui itself binds none of Ctrl+G / Ctrl+Shift+N
                // / Ctrl+Shift+I / Ctrl+Shift+O / Ctrl+L, so nothing else
                // opens on them.
                crate::ui::dialogs::gain::request_process(ui.ctx());
            } else if ctrl && shift && n {
                crate::ui::dialogs::normalize::request(ui.ctx());
            } else if ctrl && shift && i {
                if let Err(error) = editor.fade_in() {
                    *messages = Some(ConfirmDialog::new("Fade", Self::op_message(&error)));
                }
            } else if ctrl && shift && o {
                if let Err(error) = editor.fade_out() {
                    *messages = Some(ConfirmDialog::new("Fade", Self::op_message(&error)));
                }
            } else if ctrl
                && l
                && !shift
                && let Err(error) = editor.silence()
            {
                *messages = Some(ConfirmDialog::new("Silence", Self::op_message(&error)));
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

    /// Renders an ops error for a modal: the ops prefix is stripped so the
    /// SPEC's message shows verbatim.
    fn op_message(error: &crate::editor::editor::EditorError) -> String {
        let text = error.to_string();
        text.strip_prefix("invalid operation: ")
            .unwrap_or(&text)
            .to_string()
    }

    /// Delete / Backspace: removes the clip selection (with the ripple
    /// setting), or the range selection and moves the playhead to its start.
    fn delete_selection(editor: &mut Editor) {
        match editor.selection().clone() {
            crate::editor::selection::Selection::Clips(ids) => {
                let _ = editor.apply(Action::DeleteClips {
                    clip_ids: ids,
                    ripple: editor.view.ripple,
                });
            }
            crate::editor::selection::Selection::Range { start, end, tracks } => {
                let _ = editor.apply(Action::DeleteRange {
                    s: start,
                    e: end,
                    track_ids: tracks,
                    ripple: editor.view.ripple,
                });
                editor.clear_selection();
                editor.set_playhead(start);
            }
            crate::editor::selection::Selection::None => {}
        }
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
