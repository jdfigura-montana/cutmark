//! app: the eframe::App; start screen vs editor.

use std::path::PathBuf;

use eframe::egui;
use eframe::egui::ViewportCommand;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::transport::TransportState;
use crate::ui::bin_panel::{self, BinPanel};
use crate::ui::dialogs::confirm::{
    CloseChoice, CloseConfirmDialog, ConfirmDialog, ExportCloseChoice, ExportCloseDialog,
};
use crate::ui::dialogs::export::{BANNER_SECONDS, ExportBanner};
use crate::ui::master_strip;
use crate::ui::menu::{ProjectMenu, ProjectMenuAction};
use crate::ui::start_screen::StartScreen;
use crate::ui::status_bar;
use crate::ui::theme::THEME;
use crate::ui::timeline::{self, ViewMenuAction};
use crate::ui::toolbar;
use crate::ui::widgets::meter::MeterState;

/// Height of the top menu bar panel.
const MENU_BAR_H: f32 = 30.0;
/// Height of the toolbar panel.
const TOOLBAR_H: f32 = 48.0;
/// Height of the bottom status bar panel.
const STATUS_BAR_H: f32 = 26.0;

/// The currently shown screen.
enum Screen {
    /// Start screen with New/Open/Recent.
    Start(StartScreen),
    /// An open project being edited.
    Editing(Box<Editor>),
}

/// What to do once the close prompts are answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingClose {
    /// Quit the application.
    Quit,
    /// Return to the start screen with the New Project dialog.
    New,
    /// Return to the start screen with the project picker.
    Open,
    /// Return to the start screen.
    Close,
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
    /// Status-bar banner for the last finished export, when shown.
    export_banner: Option<ExportBanner>,
    /// UI-side meter ballistics: tracks, master and peak readout.
    meters: MeterState,
    /// Last window title sent to the OS, to avoid resending every frame.
    title: String,
    /// A close (quit / New / Open / Close) waiting for the export and
    /// unsaved-changes prompts, when shown.
    pending_close: Option<PendingClose>,
}

impl Default for App {
    fn default() -> Self {
        App {
            screen: Screen::Start(StartScreen::default()),
            menu: ProjectMenu::default(),
            bin: BinPanel::default(),
            messages: None,
            export_banner: None,
            meters: MeterState::default(),
            title: "Audio Editor".to_string(),
            pending_close: None,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.update_title(ui);
        // Debug-only F12 window showing the egui style.
        #[cfg(debug_assertions)]
        crate::ui::theme::debug_window(ui.ctx());
        // Work on the screen outside self so menu, editor and screen can be
        // borrowed separately within one frame.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Start(StartScreen::default()));
        let mut opened: Option<Editor> = None;
        let mut action = ProjectMenuAction::None;
        // A close that survived the prompts this frame, applied below.
        let mut resolved: Option<PendingClose> = None;
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
                    &mut self.export_banner,
                );
                // The window X and Alt+F4 go through the same prompts as the
                // Project menu while an export runs or changes are unsaved.
                let wants_close = ui.ctx().input(|i| i.viewport().close_requested());
                if wants_close && Self::needs_prompt(editor) {
                    ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
                    if self.pending_close.is_none() {
                        self.pending_close = Some(PendingClose::Quit);
                    }
                }
                resolved = Self::run_close_prompts(
                    &ui.ctx().clone(),
                    editor,
                    &mut self.messages,
                    &mut self.pending_close,
                );
            }
        }
        if let Some(mut editor) = opened {
            editor.start_audio();
            screen = Screen::Editing(Box::new(editor));
        }
        match action {
            ProjectMenuAction::None => {}
            ProjectMenuAction::New => self.request_close(&mut screen, PendingClose::New),
            ProjectMenuAction::Open => self.request_close(&mut screen, PendingClose::Open),
            ProjectMenuAction::Close => self.request_close(&mut screen, PendingClose::Close),
            ProjectMenuAction::Import => {}
        }
        if let Some(pending) = resolved {
            if pending == PendingClose::Quit {
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            } else {
                self.apply_pending(&mut screen, pending);
            }
        }
        self.screen = screen;
    }
}

impl App {
    /// True when leaving the project needs a prompt: an export runs or
    /// there are unsaved changes.
    fn needs_prompt(editor: &Editor) -> bool {
        editor.is_locked() || editor.dirty
    }

    /// Handles a Project menu action that leaves the project; a prompt is
    /// queued first when an export runs or there are unsaved changes.
    fn request_close(&mut self, screen: &mut Screen, pending: PendingClose) {
        if let Screen::Editing(editor) = screen
            && Self::needs_prompt(editor)
        {
            if self.pending_close.is_none() {
                self.pending_close = Some(pending);
            }
            return;
        }
        self.apply_pending(screen, pending);
    }

    /// Applies an answered close to the screen.
    fn apply_pending(&mut self, screen: &mut Screen, pending: PendingClose) {
        match pending {
            PendingClose::Quit => {}
            PendingClose::New => *screen = Screen::Start(StartScreen::with_new_dialog()),
            PendingClose::Open => *screen = Screen::Start(StartScreen::with_open_request()),
            PendingClose::Close => *screen = Screen::Start(StartScreen::default()),
        }
    }

    /// Runs the close-prompt flow for a pending close: the export question
    /// first, then the unsaved-changes prompt. Returns the close to apply,
    /// or None while a prompt is up or was cancelled.
    fn run_close_prompts(
        ctx: &egui::Context,
        editor: &mut Editor,
        messages: &mut Option<ConfirmDialog>,
        pending_close: &mut Option<PendingClose>,
    ) -> Option<PendingClose> {
        let pending = (*pending_close)?;
        if editor.is_locked() {
            let dialog = ExportCloseDialog;
            match dialog.show(ctx) {
                Some(ExportCloseChoice::CancelExport) => editor.export_cancel(),
                Some(ExportCloseChoice::KeepExporting) => *pending_close = None,
                None => {}
            }
            return None;
        }
        if editor.dirty {
            let dialog = CloseConfirmDialog::new(&editor.project.name);
            match dialog.show(ctx) {
                Some(CloseChoice::Save) => {
                    if let Err(error) = editor.save() {
                        // The save failed: show the error and stay put.
                        *messages = Some(ConfirmDialog::new("Save Error", error.to_string()));
                        *pending_close = None;
                        return None;
                    }
                }
                Some(CloseChoice::DontSave) => {}
                Some(CloseChoice::Cancel) => {
                    *pending_close = None;
                    return None;
                }
                None => return None,
            }
        }
        *pending_close = None;
        Some(pending)
    }

    /// Draws the editor layout, handles the menus and the editor shortcuts.
    fn show_editor(
        ui: &mut egui::Ui,
        editor: &mut Editor,
        menu: &mut ProjectMenu,
        bin: &mut BinPanel,
        messages: &mut Option<ConfirmDialog>,
        meters: &mut MeterState,
        banner: &mut Option<ExportBanner>,
    ) -> ProjectMenuAction {
        // Pump background jobs and keep repainting while any are running.
        editor.poll_jobs();
        // Retire old mix plans, keep the audio status fresh and mirror the
        // audio playhead into the view.
        editor.tick();
        editor.poll_transport();
        let transport_active = matches!(
            editor.transport_state(),
            TransportState::Playing | TransportState::Previewing | TransportState::Scrubbing
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

        // Finished export: success starts the status-bar banner, failure
        // shows in a modal; a plain cancel shows nothing.
        if let Some(result) = editor.take_export_result() {
            match result {
                Ok(path) => {
                    *banner = Some(ExportBanner {
                        name: path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string()),
                        path,
                        at: now,
                    });
                }
                Err(message) if message != "Cancelled" => {
                    *messages = Some(ConfirmDialog::new("Export", message));
                }
                Err(_) => {}
            }
        }
        // Expire the banner after ten seconds; keep repainting while visible.
        if let Some(export) = banner
            && now - export.at > BANNER_SECONDS
        {
            *banner = None;
        }
        if banner.is_some() {
            ui.ctx().request_repaint();
        }

        let mut msgs: Vec<String> = Vec::new();
        // Top: menu bar (one 30 px panel) with the Project, Edit, Markers,
        // Process and View menus in a single row, then the toolbar row.
        let mut action = ProjectMenuAction::None;
        let mut view_action = ViewMenuAction::None;
        egui::Panel::top("menu_bar")
            .exact_size(MENU_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(THEME.bar_bg)
                    .inner_margin(egui::Margin {
                        left: 8,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                (action, view_action) = menu.show(ui, editor);
            });

        if action == ProjectMenuAction::Import {
            msgs.extend(bin_panel::pick_and_import(editor));
            action = ProjectMenuAction::None;
        }
        egui::Panel::top("toolbar")
            .exact_size(TOOLBAR_H)
            .frame(
                egui::Frame::new()
                    .fill(THEME.panel_bg)
                    .inner_margin(egui::Margin {
                        left: 8,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                toolbar::show(ui, editor);
            });

        // Bottom: status bar.
        egui::Panel::bottom("status_bar")
            .exact_size(STATUS_BAR_H)
            .frame(
                egui::Frame::new()
                    .fill(THEME.bar_bg)
                    .inner_margin(egui::Margin {
                        left: 8,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                status_bar::show(ui, editor, banner.as_ref());
            });

        // Right: Master strip, fixed width.
        egui::Panel::right("master_strip")
            .resizable(false)
            .exact_size(116.0)
            .frame(egui::Frame::new().fill(THEME.panel_bg))
            .show(ui, |ui| {
                master_strip::show(ui, editor, meters);
            });

        // Left: Media Bin panel, resizable between 200 and 400 px, 236 default.
        egui::Panel::left("media_bin")
            .resizable(true)
            .default_size(236.0)
            .size_range(200.0..=400.0)
            .frame(egui::Frame::new().fill(THEME.panel_bg))
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

        // While an export job runs: click blocker, progress window and clip
        // warning. Drawn last so the blocker sits above every panel.
        crate::ui::dialogs::export::show_progress(&ui.ctx().clone(), editor);

        // Editor shortcuts, only when no text field has keyboard focus, and
        // never while an export locks the project.
        if !ui.ctx().egui_wants_keyboard_input() && !editor.is_locked() {
            let (
                ctrl,
                shift,
                n,
                o,
                s,
                e,
                i,
                z,
                y,
                t,
                r,
                a,
                c,
                x,
                v,
                escape,
                del,
                back,
                g,
                l,
                m,
                comma,
                period,
            ) = ui.input(|i| {
                (
                    i.modifiers.ctrl,
                    i.modifiers.shift,
                    i.key_pressed(egui::Key::N),
                    i.key_pressed(egui::Key::O),
                    i.key_pressed(egui::Key::S),
                    i.key_pressed(egui::Key::E),
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
                    i.key_pressed(egui::Key::M),
                    i.key_pressed(egui::Key::Comma),
                    i.key_pressed(egui::Key::Period),
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
            } else if ctrl && e {
                menu.open_export(editor);
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
            } else if m && !ctrl {
                let _ = editor.add_marker_at_playhead();
            } else if comma && !ctrl {
                editor.jump_prev_marker();
            } else if period && !ctrl {
                editor.jump_next_marker();
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

    /// Delete / Backspace: removes a selected marker, else the clip selection
    /// (with the ripple setting), or the range selection and moves the
    /// playhead to its start.
    fn delete_selection(editor: &mut Editor) {
        if let Some(id) = editor.selected_marker() {
            let _ = editor.apply(Action::DeleteMarkers { ids: vec![id] });
            editor.select_marker(None);
            return;
        }
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
