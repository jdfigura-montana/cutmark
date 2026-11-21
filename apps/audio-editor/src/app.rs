//! app: the eframe::App; start screen vs editor.

use std::path::PathBuf;

use eframe::egui;
use eframe::egui::ViewportCommand;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::message::{self, Message};
use crate::editor::transport::TransportState;
use crate::ui::bin_panel::{self, BinPanel};
use crate::ui::dialogs::confirm::{CloseChoice, CloseConfirmDialog, ConfirmDialog, ExportCloseChoice, ExportCloseDialog, RecordCloseChoice, RecordCloseDialog};
use crate::ui::dialogs::export::{BANNER_SECONDS, ExportBanner};
use crate::ui::master_strip;
use crate::ui::menu::{ProjectMenu, ProjectMenuAction, app_mark};
use crate::ui::start_screen::StartScreen;
use crate::ui::status_bar;
use crate::ui::theme::THEME;
use crate::ui::timeline::{self, ViewMenuAction};
use crate::ui::widgets::meter::MeterState;
use ui_kit::tokens::{color, size, space};

/// The user-visible application name, used for the window title, the start
/// screen heading and the installer.
pub const APP_NAME: &str = "CutMark Audio Editor";

/// The short app name shown in the start screen's toolbar title area and its
/// panel heading.
pub const APP_SHORT_NAME: &str = "Cutmark Audio";

/// Height of the 44px main toolbar (the window's title bar).
const TOP_BAR_H: f32 = size::TOOLBAR_MAIN_HEIGHT;
/// Height of the bottom status bar panel.
const STATUS_BAR_H: f32 = size::STATUS_BAR_HEIGHT;

/// Wraps plain panel messages (import, relink, bin and timeline edits) as
/// "Media" messages for the messages dialog.
fn media_messages(texts: Vec<String>) -> impl Iterator<Item = Message> {
    texts.into_iter().map(Message::media)
}

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
    /// True once a quit survived the prompts. `ViewportCommand::Close` comes
    /// back as a close request on the next frame; this lets that one through
    /// instead of prompting again (Don't Save leaves the project dirty).
    close_confirmed: bool,
    /// Default output device name for the start screen status bar, queried
    /// from the audio engine once per start-screen session.
    start_output: Option<String>,
    /// True once `start_output` has been queried (None then means no device).
    start_output_queried: bool,
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
            title: crate::app::APP_NAME.to_string(),
            pending_close: None,
            close_confirmed: false,
            start_output: None,
            start_output_queried: false,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.run(ui);
    }
}

impl App {
    /// One frame of the app; `eframe::App::ui` forwards here so tests can
    /// drive the real frame without an `eframe::Frame`.
    fn run(&mut self, ui: &mut egui::Ui) {
        self.update_title(ui);
        // The appearance file feeds the runtime theme (shared with the whole
        // suite); toasts draw on top of everything.
        ui_kit::appearance::poll(ui.ctx());
        ui_kit::show_toasts(ui.ctx());
        // Work on the screen outside self so menu, editor and screen can be
        // borrowed separately within one frame.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Start(StartScreen::default()));
        let mut opened: Option<Editor> = None;
        let mut action = ProjectMenuAction::None;
        // A close that survived the prompts this frame, applied below.
        let mut resolved: Option<PendingClose> = None;
        match &mut screen {
            Screen::Start(start) => {
                if !self.start_output_queried {
                    self.start_output = crate::audio::engine::default_output_name();
                    self.start_output_queried = true;
                }
                if !ui.ctx().egui_wants_keyboard_input() && !start.dialog_is_open() {
                    Self::handle_start_shortcuts(ui, start);
                }
                match Self::show_start_bar(ui) {
                    ProjectMenuAction::New => *start = StartScreen::with_new_dialog(),
                    ProjectMenuAction::Open => *start = StartScreen::with_open_request(),
                    _ => {}
                }
                opened = start.show(ui, self.start_output.as_deref());
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
                    self.pending_close.is_some(),
                );
                Self::intercept_window_close(ui.ctx(), editor, &mut self.pending_close, self.close_confirmed);
                resolved = Self::run_close_prompts(&ui.ctx().clone(), editor, &mut self.messages, &mut self.pending_close);
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
                self.close_confirmed = true;
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            } else {
                self.apply_pending(&mut screen, pending);
            }
        }
        self.screen = screen;
    }
}

impl App {
    /// True when leaving the project needs a prompt: an export runs, a take
    /// is being recorded or there are unsaved changes.
    fn needs_prompt(editor: &Editor) -> bool {
        editor.is_locked() || editor.is_recording() || editor.dirty
    }

    /// The window X and Alt+F4 go through the same prompts as the Project
    /// menu while an export runs, a take is recorded or changes are unsaved.
    /// A quit the prompts already approved (`confirmed`) is not intercepted a
    /// second time.
    fn intercept_window_close(ctx: &egui::Context, editor: &Editor, pending_close: &mut Option<PendingClose>, confirmed: bool) {
        let wants_close = ctx.input(|i| i.viewport().close_requested());
        if wants_close && !confirmed && Self::needs_prompt(editor) {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if pending_close.is_none() {
                *pending_close = Some(PendingClose::Quit);
            }
        }
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
    /// first, then the recording question, then the unsaved-changes prompt.
    /// Returns the close to apply, or None while a prompt is up or was
    /// cancelled.
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
        if editor.is_recording() {
            // A take is running: keep it or throw it away, then wait for the
            // take job to finish (the clip lands, or the file is deleted) so
            // the unsaved-changes prompt below sees the final state. Once the
            // take was stopped there is nothing left to ask, only to wait.
            if editor.take_is_live() {
                match RecordCloseDialog.show(ctx) {
                    Some(RecordCloseChoice::KeepTake) => editor.stop_recording(),
                    Some(RecordCloseChoice::Discard) => editor.cancel_recording(),
                    Some(RecordCloseChoice::Cancel) => *pending_close = None,
                    None => {}
                }
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

    /// The 44px title bar on the start screen: the app mark and the File menu
    /// on the left, the app name in the center, nothing on the right (no
    /// Export while no project is open), and it is still the window's drag
    /// area and the caption buttons' home. Returns the File menu action.
    fn show_start_bar(ui: &mut egui::Ui) -> ProjectMenuAction {
        let chosen = std::cell::Cell::new(ProjectMenuAction::None);
        egui::Panel::top("start_bar").exact_size(TOP_BAR_H).frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            ui_kit::main_toolbar(
                ui,
                |ui| {
                    app_mark(ui);
                    ui.add_space(space::SCALE_8);
                    chosen.set(crate::ui::menu::start_menu_bar(ui));
                },
                |ui| {
                    ui_kit::title_block(ui, &ui_kit::Title::App(APP_SHORT_NAME));
                },
                |_| {},
            );
        });
        chosen.get()
    }

    /// Draws the editor layout, handles the menus and the editor shortcuts.
    /// `close_pending` is true while a close prompt is up.
    #[allow(clippy::too_many_arguments)]
    fn show_editor(
        ui: &mut egui::Ui,
        editor: &mut Editor,
        menu: &mut ProjectMenu,
        bin: &mut BinPanel,
        messages: &mut Option<ConfirmDialog>,
        meters: &mut MeterState,
        banner: &mut Option<ExportBanner>,
        close_pending: bool,
    ) -> ProjectMenuAction {
        // Pump background jobs and keep repainting while any are running.
        editor.poll_jobs();
        // Retire old mix plans, keep the audio status fresh and mirror the
        // audio playhead into the view (while a take runs, the record
        // position instead).
        editor.tick();
        editor.poll_transport();
        let transport_active = matches!(editor.transport_state(), TransportState::Playing | TransportState::Previewing | TransportState::Scrubbing);
        // A take being recorded repaints every frame: the live clip grows,
        // the timecodes run and the input meter moves.
        if editor.has_pending_jobs() || transport_active || editor.is_recording() {
            ui.ctx().request_repaint();
        }

        // Meter ballistics advance with egui's frame time; keep repainting
        // for one more second after playback stops so meters fall to empty.
        // An armed track's meter shows the live input, so it keeps repainting
        // while armed even when nothing plays.
        if transport_active || editor.armed_track().is_some() {
            meters.mark_active();
        }
        let (now, dt) = ui.input(|i| (i.time, if i.stable_dt > 0.0 { i.stable_dt } else { 1.0 / 60.0 }));
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
                        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string()),
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

        let mut msgs: Vec<Message> = Vec::new();
        // Top: the 44px main toolbar (the title bar): app mark and the menu
        // bar on the left, the project title in the center, Export on the
        // right, then the separator and the caption buttons. It is also the
        // window's drag area.
        let menu_cell = std::cell::RefCell::new(&mut *menu);
        let editor_cell = std::cell::RefCell::new(&mut *editor);
        let chosen = std::cell::Cell::new((ProjectMenuAction::None, ViewMenuAction::None));
        egui::Panel::top("app_bar").exact_size(TOP_BAR_H).frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            ui_kit::main_toolbar(
                ui,
                |ui| {
                    app_mark(ui);
                    ui.add_space(space::SCALE_8);
                    chosen.set(crate::ui::menu::menu_bar(ui, &menu_cell, &editor_cell));
                },
                |ui| {
                    let editor = editor_cell.borrow();
                    let caption = status_bar::format_caption(editor.project.sample_rate, editor.project.export_bit_depth, editor.project.tracks.len());
                    ui_kit::title_block(ui, &ui_kit::Title::Document { name: &editor.project.name, unsaved: editor.dirty, caption: Some(&caption) });
                },
                |ui| {
                    if ui_kit::primary_button(ui, "Export").clicked() {
                        menu_cell.borrow_mut().open_export(&editor_cell.borrow());
                    }
                    ui.add_space(space::SCALE_4);
                },
            );
        });
        // The dialogs the menu items opened draw once per frame, outside the
        // toolbar's measuring passes.
        menu.show_dialogs(&ui.ctx().clone(), editor);
        let (mut action, view_action) = chosen.get();

        if action == ProjectMenuAction::Import {
            msgs.extend(media_messages(bin_panel::pick_and_import(editor)));
            action = ProjectMenuAction::None;
        }
        // Bottom: status bar.
        egui::Panel::bottom("status_bar")
            .exact_size(STATUS_BAR_H)
            .frame(egui::Frame::new().fill(THEME.bar_bg).inner_margin(egui::Margin { left: 8, ..Default::default() }))
            .show(ui, |ui| {
                status_bar::show(ui, editor, banner.as_ref());
            });

        // Right: Master panel on the ground, fixed width, with the window gap
        // on its right, top and bottom.
        let gap = space::WINDOW_GAP as i8;
        egui::Panel::right("master_strip")
            .resizable(false)
            .exact_size(136.0)
            .frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin { left: 0, top: gap, bottom: gap, right: gap }))
            .show(ui, |ui| {
                ui.take_available_space();
                master_strip::show(ui, editor, meters);
            });

        // Left: Media panel on the ground, resizable between 200 and 400 px,
        // 236 default, with the window gap on its left, top and bottom.
        let gap = space::WINDOW_GAP as i8;
        egui::Panel::left("media_bin")
            .resizable(true)
            .default_size(236.0)
            .size_range(200.0..=400.0)
            .frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin { left: gap, top: gap, bottom: gap, right: 0 }))
            .show(ui, |ui| {
                ui.take_available_space();
                msgs.extend(media_messages(bin.show(ui, editor)));
            });

        // Center: the timeline panel on the ground, 8px window padding all
        // round, filling the remainder.
        egui::CentralPanel::default().frame(egui::Frame::new().fill(color::BG_GROUND).inner_margin(egui::Margin::same(space::WINDOW_GAP as i8))).show(
            ui,
            |ui| {
                msgs.extend(timeline::show(ui, editor, view_action, meters));
            },
        );

        // Files dropped anywhere on the window go to the importer.
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            msgs.extend(media_messages(editor.import_files(dropped)));
        }

        // While an export job runs: click blocker, progress window and clip
        // warning. Drawn last so the blocker sits above every panel.
        crate::ui::dialogs::export::show_progress(&ui.ctx().clone(), editor);

        // While a take is recorded the keyboard does four things (Space, Esc,
        // M and Ctrl+S) and the timeline handles zoom and scroll; everything
        // else is ignored. An open prompt owns Esc and Space, so they do not
        // reach the take then.
        let recording = editor.is_recording();
        if recording && !ui.ctx().egui_wants_keyboard_input() && !close_pending && !messages.as_ref().is_some_and(|dialog| dialog.open) && !menu.modal_open() {
            Self::handle_recording_keys(ui, editor, menu);
        }

        // Editor shortcuts, only when no text field has keyboard focus, and
        // never while an export locks the project or a take is recorded.
        if !ui.ctx().egui_wants_keyboard_input() && !editor.is_locked() && !recording {
            let (ctrl, shift, n, o, s, e, i, z, y, t, r, a, c, x, v, escape, del, back, g, l, m, comma, period) = ui.input(|i| {
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
                msgs.extend(media_messages(bin_panel::pick_and_import(editor)));
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
            } else if ctrl && r && !shift {
                // Record onto the armed track from the playhead; "Arm a track
                // first" and the like go to the messages dialog.
                if let Err(error) = editor.start_recording() {
                    msgs.push(Message::recording(error.to_string()));
                }
            } else if ctrl && shift && t {
                let _ = editor.apply(Action::AddTrack { index: None, name: None });
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
        // activate). While a take is recorded Space belongs to
        // `handle_recording_keys`.
        if !ui.ctx().egui_wants_keyboard_input() && !recording {
            ui.input_mut(|i| {
                if i.key_pressed(egui::Key::Space) && i.modifiers.is_none() {
                    editor.toggle_play();
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
                } else if i.key_pressed(egui::Key::Space)
                    && i.modifiers.shift
                    && !i.modifiers.ctrl
                    && let crate::editor::selection::Selection::Range { start, end, .. } = editor.selection().clone()
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
            let text: Vec<&str> = msgs.iter().map(|m| m.text.as_str()).collect();
            *messages = Some(ConfirmDialog::new(message::dialog_title(&msgs), text.join("\n")));
        }
        if let Some(dialog) = messages {
            dialog.show(&ui.ctx().clone());
        }
        action
    }

    /// The keys that work while a take is recorded: Space ends the take and
    /// keeps it, Esc ends it and discards it (the press is consumed, so it
    /// never also clears the selection), M adds a marker at the record
    /// position and Ctrl+S saves the project (the take keeps recording). Ctrl+R
    /// and every other key, Ctrl+N and Ctrl+O included, do nothing.
    fn handle_recording_keys(ui: &egui::Ui, editor: &mut Editor, menu: &mut ProjectMenu) {
        ui.input_mut(|i| {
            let ctrl_only = i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt;
            if ctrl_only && i.key_pressed(egui::Key::S) {
                menu.save(editor);
                return;
            }
            if i.modifiers.any() {
                return;
            }
            if i.key_pressed(egui::Key::Space) {
                editor.stop_recording();
                i.consume_key(egui::Modifiers::NONE, egui::Key::Space);
            } else if i.key_pressed(egui::Key::Escape) {
                editor.cancel_recording();
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            } else if i.key_pressed(egui::Key::M) {
                let _ = editor.add_marker_at_playhead();
                i.consume_key(egui::Modifiers::NONE, egui::Key::M);
            }
        });
    }

    /// Renders an ops error for a modal: the ops prefix is stripped so the
    /// SPEC's message shows verbatim.
    fn op_message(error: &crate::editor::editor::EditorError) -> String {
        let text = error.to_string();
        text.strip_prefix("invalid operation: ").unwrap_or(&text).to_string()
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
                let _ = editor.apply(Action::DeleteClips { clip_ids: ids, ripple: editor.view.ripple });
            }
            crate::editor::selection::Selection::Range { start, end, tracks } => {
                let _ = editor.apply(Action::DeleteRange { s: start, e: end, track_ids: tracks, ripple: editor.view.ripple });
                editor.clear_selection();
                editor.set_playhead(start);
            }
            crate::editor::selection::Selection::None => {}
        }
    }

    /// Handles Ctrl+N (New Project dialog) and Ctrl+O (Open picker) on the start screen.
    fn handle_start_shortcuts(ui: &egui::Ui, start: &mut StartScreen) {
        let (ctrl, n, o) = ui.input(|i| (i.modifiers.ctrl, i.key_pressed(egui::Key::N), i.key_pressed(egui::Key::O)));
        if ctrl && n {
            *start = StartScreen::with_new_dialog();
        } else if ctrl && o {
            *start = StartScreen::with_open_request();
        }
    }

    /// Sends the window title to the OS when it changed:
    /// "*<name> - CutMark Audio Editor" when dirty,
    /// "<name> - CutMark Audio Editor" otherwise.
    fn update_title(&mut self, ui: &egui::Ui) {
        let title = match &self.screen {
            Screen::Start(_) => APP_NAME.to_string(),
            Screen::Editing(editor) => format!("{}{} - {}", if editor.dirty { "*" } else { "" }, editor.project.name, APP_NAME),
        };
        if title != self.title {
            self.title = title.clone();
            ui.ctx().send_viewport_cmd(ViewportCommand::Title(title));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::paths::create_layout;
    use crate::model::project::{BitDepth, Project, ViewState};
    use crate::ui::test_util::{Headless, Out};

    const ACTIONS: [PendingClose; 4] = [PendingClose::Quit, PendingClose::New, PendingClose::Open, PendingClose::Close];

    /// An app editing a dirty project in a real folder (so Save can work),
    /// driven frame by frame through the real `App::run`.
    struct Rig {
        h: Headless,
        app: App,
        _dir: tempfile::TempDir,
    }

    impl Rig {
        fn new() -> Rig {
            let dir = tempfile::tempdir().expect("tempdir");
            let paths = create_layout(dir.path(), "Test 2").expect("layout");
            let mut editor = Editor::with_project(Project::new("Test 2".to_string(), 48_000, BitDepth::Int24), ViewState::default(), paths);
            editor.dirty = true;
            Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), ..App::default() }, _dir: dir }
        }

        fn frame(&mut self) -> Out {
            let app = &mut self.app;
            self.h.frame(Vec::new(), |ui| app.run(ui))
        }

        /// A frame in which the window reports that its X was pressed.
        fn window_x(&mut self) -> Out {
            let app = &mut self.app;
            self.h.frame_close_requested(|ui| app.run(ui))
        }

        /// Starts the close the way the user does: the window X for Quit,
        /// the Project menu action otherwise. Returns the first frame.
        fn request(&mut self, pending: PendingClose) -> Out {
            if pending == PendingClose::Quit {
                return self.window_x();
            }
            let mut screen = std::mem::replace(&mut self.app.screen, Screen::Start(StartScreen::default()));
            self.app.request_close(&mut screen, pending);
            self.app.screen = screen;
            self.frame()
        }

        /// Clicks the button whose text is `label`; returns the release frame.
        fn click(&mut self, label: &str) -> Out {
            let _ = self.frame();
            let out = self.frame();
            let target = out.text_rect(label).unwrap_or_else(|| panic!("no {label} button on screen")).center();
            let app = &mut self.app;
            self.h.click(target, |ui| app.run(ui))
        }

        fn editor(&self) -> &Editor {
            match &self.app.screen {
                Screen::Editing(editor) => editor,
                Screen::Start(_) => panic!("expected the editor screen"),
            }
        }

        fn editor_mut(&mut self) -> &mut Editor {
            match &mut self.app.screen {
                Screen::Editing(editor) => editor,
                Screen::Start(_) => panic!("expected the editor screen"),
            }
        }
    }

    /// A rig whose editor has two tracks with loaded media (samples and
    /// peaks), the playhead inside the first clip, like a real session.
    fn rig_with_audio() -> Rig {
        let mut rig = Rig::new();
        let Screen::Editing(editor) = &mut rig.app.screen else {
            panic!("editor screen");
        };
        let project = crate::model::fixtures::project_with_clips(&[&[(0, 478_080)], &[(268_800, 297_600)]]);
        let media_id = project.media[0].id;
        let samples: Vec<f32> = (0..1_000_000usize)
            .flat_map(|i| {
                let v = ((i as f32) * 0.05).sin() * 0.5;
                [v, v]
            })
            .collect();
        let peaks = crate::io::peaks::build(&samples, 2);
        editor.media_store.insert(media_id, crate::io::import::Loaded { samples: samples.into(), peaks: peaks.into() });
        editor.project = std::sync::Arc::new(project);
        editor.view.playhead = 100_000;
        rig
    }

    /// Presses and releases `key` in two frames of the real app.
    fn tap(rig: &mut Rig, key: egui::Key) {
        tap_with(rig, key, egui::Modifiers::NONE);
    }

    /// Presses and releases `key` with `modifiers` held, in two frames of the
    /// real app.
    fn tap_with(rig: &mut Rig, key: egui::Key, modifiers: egui::Modifiers) {
        rig.h.set_modifiers(modifiers);
        for pressed in [true, false] {
            let event = egui::Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
            let app = &mut rig.app;
            let _ = rig.h.frame(vec![event], |ui| app.run(ui));
        }
        rig.h.set_modifiers(egui::Modifiers::NONE);
    }

    /// A rig over a real project folder with a 10 s clip on armed track 1 and
    /// an empty track 2 (the test input is armed, nothing records yet).
    fn recording_rig() -> Rig {
        let (editor, dir) = crate::ui::test_util::armed_editor(&[&[(0, 480_000)], &[]], "Test 2");
        Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), ..App::default() }, _dir: dir }
    }

    /// Runs frames of the real app until `done` holds; fails after 10 s.
    fn frames_until(rig: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(rig) {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            let _ = rig.frame();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Ctrl+R and `frames` mono frames of audio: a take that is running.
    fn start_take(rig: &mut Rig, frames: usize) {
        for _ in 0..3 {
            let _ = rig.frame();
        }
        tap_with(rig, egui::Key::R, egui::Modifiers::CTRL);
        assert!(rig.editor().take_is_live(), "Ctrl+R started a take");
        crate::ui::test_util::feed_take(rig.editor_mut(), frames, 0.5, frames as u64);
    }

    /// Names of the bin items of the editor.
    fn bin_names(rig: &Rig) -> Vec<String> {
        rig.editor().project.media.iter().map(|item| item.display_name.clone()).collect()
    }

    #[test]
    fn pressing_s_splits_the_clip_and_the_next_frames_still_draw() {
        // Regression: the frame after a split never returned (the clip name
        // truncation looped forever), so the whole app froze on Split.
        let clips = crate::ui::test_util::within_secs(30, || {
            let mut rig = rig_with_audio();
            for _ in 0..3 {
                let _ = rig.frame();
            }
            tap(&mut rig, egui::Key::S);
            for _ in 0..3 {
                let _ = rig.frame();
            }
            rig.editor().project.tracks[0].clips.len()
        });
        assert_eq!(clips, 2, "Split cut the clip under the playhead in two");
    }

    /// True when the app moved on to the start screen for `pending`.
    fn left_for_start(rig: &Rig, pending: PendingClose) -> bool {
        match (&rig.app.screen, pending) {
            (Screen::Start(start), PendingClose::New) => start.dialog_is_open(),
            (Screen::Start(start), PendingClose::Open | PendingClose::Close) => !start.dialog_is_open(),
            _ => false,
        }
    }

    #[test]
    fn save_continues_every_pending_close() {
        for pending in ACTIONS {
            let mut rig = Rig::new();
            let first = rig.request(pending);
            assert_eq!(first.cancelled_close(), pending == PendingClose::Quit);
            let out = rig.click("Save");
            if pending == PendingClose::Quit {
                assert!(out.sent_close(), "Quit + Save closes the window");
                assert!(!rig.editor().dirty, "Save clears the dirty flag");
            } else {
                assert!(left_for_start(&rig, pending), "{pending:?} + Save");
            }
            assert!(rig.app.pending_close.is_none(), "{pending:?} + Save");
        }
    }

    #[test]
    fn dont_save_discards_and_continues_every_pending_close() {
        for pending in ACTIONS {
            let mut rig = Rig::new();
            let first = rig.request(pending);
            assert_eq!(first.cancelled_close(), pending == PendingClose::Quit);
            let out = rig.click("Don't Save");
            assert!(rig.app.pending_close.is_none(), "{pending:?} + Don't Save");
            if pending == PendingClose::Quit {
                assert!(out.sent_close(), "Quit + Don't Save closes the window");
                // The window then reports its own close request; the unsaved
                // changes were discarded on purpose, so it must go through.
                let again = rig.window_x();
                assert!(!again.cancelled_close(), "Quit + Don't Save is not blocked");
                assert!(rig.app.pending_close.is_none(), "no second prompt");
            } else {
                assert!(left_for_start(&rig, pending), "{pending:?} + Don't Save");
            }
        }
    }

    #[test]
    fn cancel_stays_in_the_project_for_every_pending_close() {
        for pending in ACTIONS {
            let mut rig = Rig::new();
            let _ = rig.request(pending);
            let out = rig.click("Cancel");
            assert!(!out.sent_close(), "{pending:?} + Cancel must not close");
            assert!(rig.app.pending_close.is_none(), "{pending:?} + Cancel");
            assert!(rig.editor().dirty, "{pending:?} + Cancel keeps the changes");
        }
    }

    // ---- Recording UI ----

    #[test]
    fn ctrl_r_starts_space_stops_and_the_take_is_one_undo_step() {
        let mut rig = recording_rig();
        start_take(&mut rig, 4_800);
        assert!(rig.editor().is_recording());

        tap(&mut rig, egui::Key::Space);
        assert!(!rig.editor().take_is_live(), "Space ended the take");
        frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());

        assert!(bin_names(&rig).contains(&"Take 1".to_string()));
        assert_eq!(rig.editor().undo_label(), Some("Record"));
        let editor = rig.editor_mut();
        assert!(editor.undo(), "one undo step takes the take back");
        assert!(!bin_names(&rig).contains(&"Take 1".to_string()));
        assert!(!rig.editor_mut().undo(), "and it was the only step");
    }

    #[test]
    fn esc_cancels_the_take_adds_nothing_and_keeps_the_selection() {
        let mut rig = recording_rig();
        rig.editor_mut().select_all();
        let before = bin_names(&rig);
        start_take(&mut rig, 4_800);

        tap(&mut rig, egui::Key::Escape);
        frames_until(&mut rig, "the take to be discarded", |r| !r.editor().is_recording());

        assert_eq!(bin_names(&rig), before, "no bin item");
        assert!(!rig.editor_mut().undo(), "no undo step");
        assert!(!rig.editor().dirty);
        assert!(!matches!(rig.editor().selection(), crate::editor::selection::Selection::None), "Esc was consumed by the take, it did not clear the selection");
    }

    #[test]
    fn ctrl_r_without_an_armed_track_says_arm_a_track_first() {
        let mut rig = recording_rig();
        let armed = rig.editor().armed_track().expect("armed");
        rig.editor_mut().toggle_arm(armed);
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let idle = rig.frame();
        assert!(idle.text_rect("Recording").is_none(), "no title painted before the key");
        tap_with(&mut rig, egui::Key::R, egui::Modifiers::CTRL);
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(!rig.editor().is_recording());
        assert!(out.text_rect("Arm a track first").is_some(), "the messages dialog says why");
        assert!(out.text_rect("Recording").is_some(), "the dialog is titled Recording");
        let media = out.texts().into_iter().filter(|t| t.text == "Media").count();
        assert_eq!(media, 1, "only the Media panel header says Media, not the dialog");
    }

    #[test]
    fn m_adds_a_marker_and_other_keys_do_nothing_while_recording() {
        let mut rig = recording_rig();
        rig.editor_mut().select_all();
        start_take(&mut rig, 2_400);
        let _ = rig.frame();
        let start = rig.editor().recording_status().expect("recording").start;
        assert_eq!(rig.editor().view.playhead, start + 2_400, "the record position");

        tap(&mut rig, egui::Key::M);
        let markers: Vec<u64> = rig.editor().project.markers.iter().map(|marker| marker.position).collect();
        assert_eq!(markers, [start + 2_400], "M marked the record position");

        let clips = rig.editor().project.tracks[0].clips.clone();
        let (ripple, snap) = (rig.editor().view.ripple, rig.editor().view.snap);
        let selection = rig.editor().selection().clone();
        for key in [egui::Key::Delete, egui::Key::Backspace, egui::Key::S, egui::Key::R, egui::Key::N, egui::Key::Home, egui::Key::End, egui::Key::P] {
            tap(&mut rig, key);
        }
        for key in [egui::Key::Z, egui::Key::Y, egui::Key::A] {
            tap_with(&mut rig, key, egui::Modifiers::CTRL);
        }
        assert_eq!(rig.editor().project.markers.len(), 1, "Ctrl+Z undid nothing");
        assert_eq!(rig.editor().project.tracks[0].clips, clips, "Delete and S");
        assert_eq!((rig.editor().view.ripple, rig.editor().view.snap), (ripple, snap), "R and N toggled nothing");
        assert_eq!(rig.editor().selection(), &selection, "Ctrl+A changed nothing");

        // Ctrl+R again does not start a second take or complain.
        tap_with(&mut rig, egui::Key::R, egui::Modifiers::CTRL);
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(rig.editor().take_is_live());
        assert_eq!(rig.editor().recording_status().map(|s| s.start), Some(start), "still the same take");
        assert!(out.text_rect("Recording in progress").is_none(), "no dialog");
        assert!(out.text_rect("Arm a track first").is_none());
    }

    #[test]
    fn ctrl_s_saves_while_recording_and_the_take_keeps_going() {
        let mut rig = recording_rig();
        start_take(&mut rig, 2_400);
        let _ = rig.frame();
        let start = rig.editor().recording_status().expect("recording").start;

        // M is the one edit a take allows: it makes the project dirty.
        tap(&mut rig, egui::Key::M);
        assert!(rig.editor().dirty, "the marker is an unsaved change");
        let json = rig.editor().paths.project_json();
        let saved_markers =
            |json: &std::path::Path| crate::io::project_file::load(json).map(|(_, project, _)| project.markers.len()).expect("project.json loads");
        assert!(!json.exists(), "nothing saved yet");

        tap_with(&mut rig, egui::Key::S, egui::Modifiers::CTRL);
        assert!(!rig.editor().dirty, "Ctrl+S saved the project");
        assert_eq!(saved_markers(&json), 1, "the marker reached project.json");
        assert!(rig.editor().take_is_live(), "the take is still recording");
        assert!(!rig.app.menu.modal_open(), "no save error");
        assert!(rig.app.messages.is_none(), "no dialog");

        // The take goes on: more audio arrives, then Space keeps it.
        crate::ui::test_util::feed_take(rig.editor_mut(), 2_400, 0.5, 4_800);
        let _ = rig.frame();
        assert_eq!(rig.editor().recording_status().map(|s| (s.start, s.frames)), Some((start, 4_800)), "same take, still growing");
        tap(&mut rig, egui::Key::Space);
        frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
        assert!(bin_names(&rig).contains(&"Take 1".to_string()));
        assert_eq!(rig.editor().undo_label(), Some("Record"));
    }

    #[test]
    fn the_other_file_shortcuts_stay_ignored_while_recording() {
        let mut rig = recording_rig();
        start_take(&mut rig, 2_400);
        let _ = rig.frame();
        // A marker makes the project dirty, so a stray save would show.
        tap(&mut rig, egui::Key::M);
        assert!(rig.editor().dirty);
        for (key, modifiers) in [
            (egui::Key::N, egui::Modifiers::CTRL),
            (egui::Key::O, egui::Modifiers::CTRL),
            (egui::Key::S, egui::Modifiers::CTRL | egui::Modifiers::SHIFT),
            (egui::Key::S, egui::Modifiers::ALT | egui::Modifiers::CTRL),
        ] {
            tap_with(&mut rig, key, modifiers);
            for _ in 0..3 {
                let _ = rig.frame();
            }
            assert!(rig.app.pending_close.is_none(), "{key:?} + {modifiers:?} opened a close prompt");
            assert!(!rig.app.menu.modal_open(), "{key:?} + {modifiers:?} opened a dialog");
        }
        assert!(rig.editor().take_is_live(), "the take was not disturbed");
        assert!(rig.editor().dirty, "nothing was saved");
    }

    /// The painted live clip: the rect with the solid `record_red` border.
    fn live_clip_rect(out: &Out) -> Option<egui::Rect> {
        out.rects().into_iter().find(|r| r.stroke.color == THEME.record_red && r.stroke.width > 0.0).map(|r| r.rect)
    }

    #[test]
    fn the_live_clip_starts_at_the_record_start_and_is_as_wide_as_the_frames() {
        let mut rig = recording_rig();
        // 100 samples per pixel, the view scrolled to 50,000; the take starts at
        // 70,000 (200 px in) and 24,000 frames are 240 px wide.
        {
            let editor = rig.editor_mut();
            editor.view.samples_per_pixel = 100.0;
            editor.view.scroll_x = 50_000;
            editor.view.playhead = 70_000;
        }
        start_take(&mut rig, 24_000);
        let out = rig.frame();

        // The shaded rows of the header column and the lanes: the lanes sit to the right.
        let lane_left =
            out.rects().into_iter().filter(|r| r.fill == ui_kit::tokens::color::BG_TRACK_ALT).map(|r| r.rect.left()).fold(f32::NEG_INFINITY, f32::max);
        let live = live_clip_rect(&out).expect("the live clip is painted");
        assert!((live.left() - (lane_left + 200.0)).abs() < 0.01, "starts at the record start: {} vs {}", live.left(), lane_left + 200.0);
        assert!((live.width() - 240.0).abs() < 0.01, "24,000 frames at 100 spp: {}", live.width());

        // It grows with the frames and keeps its left edge.
        crate::ui::test_util::feed_take(rig.editor_mut(), 12_000, 0.5, 36_000);
        let out = rig.frame();
        let live = live_clip_rect(&out).expect("still painted");
        assert!((live.left() - (lane_left + 200.0)).abs() < 0.01);
        assert!((live.width() - 360.0).abs() < 0.01, "{}", live.width());
        assert!(out.text_rect("Recording").is_some(), "the header says so");
    }

    #[test]
    fn the_view_pages_to_follow_the_take_past_the_project_end() {
        let (editor, dir) = crate::ui::test_util::armed_editor(&[&[(0, 10_000)], &[]], "Test 2");
        let mut rig = Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), ..App::default() }, _dir: dir };
        {
            let editor = rig.editor_mut();
            editor.view.samples_per_pixel = 100.0;
            editor.view.scroll_x = 0;
        }
        start_take(&mut rig, 90_000);
        let _ = rig.frame();
        let _ = rig.frame();
        let editor = rig.editor();
        assert_eq!(editor.view.playhead, 90_000, "the record position");
        // The project is 10,000 samples long: without counting the take the
        // view could not scroll past about 7,200.
        assert!(editor.view.scroll_x > 20_000 && editor.view.scroll_x <= 90_000, "scroll_x {}", editor.view.scroll_x);
    }

    #[test]
    fn the_timecodes_and_status_bar_show_the_take() {
        let mut rig = recording_rig();
        rig.editor_mut().view.playhead = 48_000;
        let idle = {
            let _ = rig.frame();
            rig.frame()
        };
        assert!(idle.text_rect("Playhead 00:00:01.000").is_some());
        assert!(idle.texts().iter().all(|t| !t.text.starts_with("Rec ")));

        start_take(&mut rig, 24_000);
        let out = rig.frame();
        // Deck: the record position 1.5 s (fraction in its own run); status
        // bar: elapsed 0.5 s.
        assert!(out.text_rect("00:00:01").is_some() && out.text_rect(".500").is_some(), "deck timecode");
        let rec = out.texts().into_iter().find(|t| t.text.starts_with("Rec ")).expect("the Rec readout");
        assert_eq!(rec.text, "Rec 00:00:00.500", "the elapsed time");
        assert_eq!(rec.color, ui_kit::tokens::color::TEXT_TERTIARY, "the component's caption color");
        assert!(out.text_rect("Playhead 00:00:01.000").is_none(), "Rec replaces Playhead");
    }

    /// The transport deck's glyphs (it floats near the window bottom), left to
    /// right.
    fn toolbar_glyphs(out: &Out, glyph: &str) -> Vec<crate::ui::test_util::TextItem> {
        let mut found: Vec<_> = out.texts().into_iter().filter(|t| t.text == glyph && t.rect.center().y > 600.0).collect();
        found.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
        found
    }

    /// The Record capsule: the 56 x 44 `sem.record` pill of the deck.
    fn record_capsule(out: &Out) -> Option<egui::epaint::RectShape> {
        out.rects().into_iter().find(|r| r.fill == color::SEM_RECORD && r.rect.size() == egui::vec2(56.0, 44.0) && r.rect.center().y > 600.0)
    }

    #[test]
    fn the_record_button_toggles_the_take_and_shows_stop_while_recording() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let glyphs = toolbar_glyphs(&out, egui_phosphor::fill::RECORD);
        assert_eq!(glyphs.len(), 1, "one Record button in the deck");
        assert_eq!(glyphs[0].color, egui::Color32::WHITE, "white glyph on the red capsule");
        assert!(record_capsule(&out).is_some(), "the capsule is always sem.record");
        // After Stop in the deck, before the end button.
        let stop = toolbar_glyphs(&out, egui_phosphor::fill::STOP);
        let end = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_FORWARD);
        assert_eq!(stop.len(), 1, "only the Stop button shows Stop when idle");
        assert!(stop[0].rect.center().x < glyphs[0].rect.center().x);
        assert!(glyphs[0].rect.center().x < end[0].rect.center().x);

        // Click: the take starts.
        let at = glyphs[0].rect.center();
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
        assert!(rig.editor().take_is_live(), "the button started the take");
        crate::ui::test_util::feed_take(rig.editor_mut(), 4_800, 0.5, 4_800);
        let out = rig.frame();
        assert!(toolbar_glyphs(&out, egui_phosphor::fill::RECORD).is_empty(), "the capsule shows Stop while recording");
        // The Stop button and the capsule both show Stop; Play stays Play.
        assert_eq!(toolbar_glyphs(&out, egui_phosphor::fill::STOP).len(), 2);
        assert_eq!(toolbar_glyphs(&out, egui_phosphor::fill::PLAY).len(), 1);

        // Click again: the take ends and is kept.
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
        assert!(!rig.editor().take_is_live(), "the button stopped the take");
        frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
        assert!(bin_names(&rig).contains(&"Take 1".to_string()));
        let out = rig.frame();
        assert_eq!(toolbar_glyphs(&out, egui_phosphor::fill::RECORD).len(), 1, "Record again");
    }

    #[test]
    fn the_play_stop_and_record_buttons_keep_the_take_while_recording() {
        // Slots: Play, the Stop button, the Record capsule (showing Stop).
        for slot in [0, 1, 2] {
            let mut rig = recording_rig();
            start_take(&mut rig, 4_800);
            let out = rig.frame();
            let at = match slot {
                0 => toolbar_glyphs(&out, egui_phosphor::fill::PLAY)[0].rect.center(),
                n => toolbar_glyphs(&out, egui_phosphor::fill::STOP)[n - 1].rect.center(),
            };
            let app = &mut rig.app;
            let _ = rig.h.click(at, |ui| app.run(ui));
            assert!(!rig.editor().take_is_live(), "slot {slot} ended the take");
            frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
            assert!(bin_names(&rig).contains(&"Take 1".to_string()), "slot {slot}");
        }
    }

    #[test]
    fn the_deck_jumps_and_plays_and_leaves_the_lanes_beneath_alone() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        rig.editor_mut().view.playhead = 48_000;
        let out = rig.frame();
        // Back to start is the first deck glyph, forward the last.
        let back = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_BACK)[0].rect.center();
        let forward = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_FORWARD)[0].rect.center();
        let app = &mut rig.app;
        let _ = rig.h.click(back, |ui| app.run(ui));
        assert_eq!(rig.editor().view.playhead, 0, "back to start");
        let app = &mut rig.app;
        let _ = rig.h.click(forward, |ui| app.run(ui));
        assert_eq!(rig.editor().view.playhead, 480_000, "forward to the end");
        assert_eq!(rig.editor().selection(), &crate::editor::selection::Selection::None, "a press on the deck selects nothing under it");
    }

    #[test]
    fn the_add_track_row_adds_a_track_and_the_wave_slider_scales_the_waveforms() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let add = out.text_rect("+ Add track").expect("the add row under the headers");
        let tracks = rig.editor().project.tracks.len();
        let app = &mut rig.app;
        let _ = rig.h.click(add.center(), |ui| app.run(ui));
        assert_eq!(rig.editor().project.tracks.len(), tracks + 1, "Add track");

        // The waveform icon sits left of its 90 px slider; the slider's right
        // end is the 16x scale.
        let icon = out
            .texts()
            .into_iter()
            .find(|t| t.text == egui_phosphor::regular::WAVEFORM && t.rect.center().x > 600.0)
            .map(|t| t.rect)
            .expect("the waveform icon in the panel toolbar");
        assert_eq!(rig.editor().wave_scale(), 1.0);
        let app = &mut rig.app;
        let _ = rig.h.click(egui::pos2(icon.right() + 8.0 + 88.0, icon.center().y), |ui| app.run(ui));
        assert!(rig.editor().wave_scale() > 8.0, "the slider end raises the scale: {}", rig.editor().wave_scale());
    }

    /// Drags the primary button from `from` to `to` over a few frames of the
    /// real app.
    fn drag(rig: &mut Rig, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let middle = from + (to - from) * 0.5;
        let steps = [
            vec![egui::Event::PointerMoved(from)],
            vec![button(from, true)],
            vec![egui::Event::PointerMoved(middle)],
            vec![egui::Event::PointerMoved(to)],
            vec![button(to, false)],
        ];
        for events in steps {
            let app = &mut rig.app;
            let _ = rig.h.frame(events, |ui| app.run(ui));
        }
    }

    #[test]
    fn dragging_on_the_lanes_selects_nothing_while_recording() {
        // Control: the same drag on an idle project selects a range.
        let mut idle = recording_rig();
        // While recording: the drag is ignored.
        let mut rig = recording_rig();
        for r in [&mut idle, &mut rig] {
            let editor = r.editor_mut();
            editor.view.samples_per_pixel = 100.0;
            editor.view.scroll_x = 0;
        }
        start_take(&mut rig, 4_800);
        for _ in 0..3 {
            let _ = idle.frame();
        }
        for (r, expect_range) in [(&mut idle, true), (&mut rig, false)] {
            let out = r.frame();
            // Track 2 is empty and has the alternate row shade: the widest of the
            // shaded rects is its lane (the header column paints the same shade).
            let lane = out
                .rects()
                .into_iter()
                .filter(|rect| rect.fill == ui_kit::tokens::color::BG_TRACK_ALT)
                .map(|rect| rect.rect)
                .max_by(|a, b| a.width().total_cmp(&b.width()))
                .expect("a lane");
            let y = lane.center().y;
            drag(r, egui::pos2(lane.left() + 200.0, y), egui::pos2(lane.left() + 400.0, y));
            let selected = !matches!(r.editor().selection(), crate::editor::selection::Selection::None);
            assert_eq!(selected, expect_range, "range selected: {expect_range}");
        }
        assert!(rig.editor().take_is_live());
    }

    /// Frames of the app until it asks the window to close, if it does within
    /// `limit` frames.
    fn closes_within(rig: &mut Rig, limit: usize) -> bool {
        (0..limit).any(|_| {
            let closed = rig.frame().sent_close();
            std::thread::sleep(std::time::Duration::from_millis(2));
            closed
        })
    }

    #[test]
    fn closing_while_recording_asks_and_keep_take_places_it_then_asks_about_unsaved_changes() {
        let mut rig = recording_rig();
        start_take(&mut rig, 4_800);
        let first = rig.window_x();
        assert!(first.cancelled_close(), "the window stays open");
        let _ = rig.frame();
        let out = rig.frame();
        assert!(out.text_rect("A recording is in progress.").is_some(), "the new dialog");
        for label in ["Keep Take", "Discard", "Cancel"] {
            assert!(out.text_rect(label).is_some(), "{label} button");
        }
        let (keep, discard, cancel) =
            (out.text_rect("Keep Take").unwrap().center().x, out.text_rect("Discard").unwrap().center().x, out.text_rect("Cancel").unwrap().center().x);
        assert!(cancel < discard && discard < keep, "left to right, the primary action last");
        assert!(out.text_rect("Save changes to Test 2?").is_none());

        let out = rig.click("Keep Take");
        assert!(!out.sent_close());
        frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
        assert!(bin_names(&rig).contains(&"Take 1".to_string()), "placed");
        let name = rig.editor().project.name.clone();
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(out.text_rect(&format!("Save changes to {name}?")).is_some(), "the unsaved-changes prompt follows");
        assert_eq!(rig.app.pending_close, Some(PendingClose::Quit));

        let out = rig.click("Don't Save");
        assert!(out.sent_close() || closes_within(&mut rig, 5), "then it closes");
    }

    #[test]
    fn discard_while_closing_adds_nothing_and_a_clean_project_just_closes() {
        let mut rig = recording_rig();
        let before = bin_names(&rig);
        start_take(&mut rig, 4_800);
        let _ = rig.window_x();
        let out = rig.click("Discard");
        let closed = out.sent_close() || closes_within(&mut rig, 3_000);
        assert!(closed, "nothing to save, so the quit goes through");
        assert_eq!(bin_names(&rig), before, "no bin item");
        assert!(!rig.editor_mut().undo(), "no undo step");
        assert!(!rig.editor().dirty);
    }

    #[test]
    fn cancel_while_closing_keeps_recording_for_every_close_action() {
        for pending in ACTIONS {
            let mut rig = recording_rig();
            start_take(&mut rig, 4_800);
            let first = rig.request(pending);
            assert_eq!(first.cancelled_close(), pending == PendingClose::Quit);
            let out = rig.click("Cancel");
            assert!(!out.sent_close(), "{pending:?} + Cancel must not close");
            assert!(rig.app.pending_close.is_none(), "{pending:?} + Cancel");
            assert!(rig.editor().take_is_live(), "{pending:?} keeps recording");
            // The take is still fed.
            crate::ui::test_util::feed_take(rig.editor_mut(), 2_400, 0.5, 7_200);
            let _ = rig.frame();
            assert_eq!(rig.editor().view.playhead, 7_200, "{pending:?}");
        }
    }

    #[test]
    fn a_new_open_or_close_action_keeps_the_take_and_continues_after_the_prompts() {
        for pending in [PendingClose::New, PendingClose::Close] {
            let mut rig = recording_rig();
            start_take(&mut rig, 4_800);
            let _ = rig.request(pending);
            let _ = rig.click("Keep Take");
            frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
            // The placed take made the project dirty: Save ends the flow.
            let _ = rig.click("Save");
            assert!(left_for_start(&rig, pending), "{pending:?} after Keep + Save");
        }
    }

    #[test]
    fn esc_and_space_go_to_an_open_prompt_not_to_the_take() {
        let mut rig = recording_rig();
        start_take(&mut rig, 4_800);
        let _ = rig.window_x();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        // Esc closes the prompt (Cancel) and must not discard the take.
        tap(&mut rig, egui::Key::Escape);
        for _ in 0..3 {
            let _ = rig.frame();
        }
        assert!(rig.editor().take_is_live(), "Esc cancelled the prompt only");
        assert!(rig.app.pending_close.is_none());
    }

    /// Clicks the toolbar icon button whose glyph is `icon`.
    fn click_icon(rig: &mut Rig, icon: &str) -> Out {
        let out = rig.frame();
        let at = out.text_rect(icon).unwrap_or_else(|| panic!("no {icon} button in the toolbar")).center();
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
        rig.frame()
    }

    #[test]
    fn undo_and_redo_work_from_the_toolbar_and_are_disabled_when_there_is_nothing() {
        let mut rig = Rig::new();
        // Nothing to undo: the Undo button is dead.
        let _ = click_icon(&mut rig, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE);
        assert_eq!(rig.editor().project.markers.len(), 0, "a disabled Undo does nothing");

        // M adds a marker (one undo step); Undo removes it, Redo brings it back.
        tap(&mut rig, egui::Key::M);
        assert_eq!(rig.editor().project.markers.len(), 1, "the marker was added");
        let _ = click_icon(&mut rig, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE);
        assert_eq!(rig.editor().project.markers.len(), 0, "Undo removed the marker");
        assert_eq!(rig.editor().undo_label(), None, "nothing left to undo");
        let _ = click_icon(&mut rig, ui_kit::icons::regular::ARROW_CLOCKWISE);
        assert_eq!(rig.editor().project.markers.len(), 1, "Redo restored the marker");
        assert_eq!(rig.editor().redo_label(), None, "nothing left to redo");
    }

    #[test]
    fn the_export_button_opens_the_export_dialog() {
        let mut rig = Rig::new();
        assert!(!rig.app.menu.export_dialog_open(), "closed to begin with");
        let _ = rig.click("Export");
        assert!(rig.app.menu.export_dialog_open(), "the toolbar Export opened the dialog");
    }

    #[test]
    fn the_status_bar_reads_ripple_and_snap_in_sentence_case() {
        let mut rig = Rig::new();
        let out = rig.frame();
        assert!(out.text_rect("48 kHz").is_some(), "the sample rate");
        assert!(out.text_rect("24-bit").is_some(), "the bit depth");
        assert!(out.text_rect("Snap on").is_some(), "snap defaults to on");
        assert!(out.text_rect("Ripple on").is_none(), "ripple defaults to off");
        assert!(out.text_rect("SNAP ON").is_none() && out.text_rect("RIPPLE OFF").is_none(), "the old all-caps readouts are gone");

        tap(&mut rig, egui::Key::R);
        let out = rig.frame();
        assert!(out.text_rect("Ripple on").is_some(), "R turned ripple on");
        assert!(out.text_rect("Snap on").is_some());
        tap(&mut rig, egui::Key::N);
        let out = rig.frame();
        assert!(out.text_rect("Snap on").is_none(), "N turned snap off");
        assert!(out.text_rect("Ripple on").is_some());
    }

    /// The title bar's height: nothing of the panel toolbar sits inside it.
    const TITLE_BAR_H: f32 = size::TOOLBAR_MAIN_HEIGHT;

    #[test]
    fn the_title_bar_centers_the_project_name_dot_and_caption() {
        let mut rig = Rig::new();
        let _ = rig.frame();
        let out = rig.frame();
        let caption = status_bar::format_caption(48_000, BitDepth::Int24, rig.editor().project.tracks.len());
        assert!(caption.starts_with("48 kHz \u{b7} 24-bit \u{b7} "), "the caption reads rate, depth, tracks: {caption}");
        assert!(caption.ends_with(" tracks"), "{caption}");
        let name = out.text_rect("Test 2").expect("the project name");
        let caption = out.text_rect(&caption).expect("the format caption");
        let dot = out.circles().into_iter().find(|c| c.fill == ui_kit::tokens::color::TEXT_SECONDARY && c.radius == 3.0).expect("the unsaved dot");
        assert!(name.right() <= dot.center.x && dot.center.x <= caption.left(), "name, then the dot, then the caption, on one line");
        for y in [name.center().y, dot.center.y, caption.center().y] {
            assert!(y < TITLE_BAR_H, "the title block sits in the title bar");
        }
        let block = name.union(caption);
        assert!((block.center().x - 640.0).abs() <= 1.0, "the title block is centered on the window, off by {}", block.center().x - 640.0);
    }

    #[test]
    fn no_project_centers_the_app_name_and_the_start_screen_has_only_the_file_menu() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut app = App::default();
        let _ = h.frame(Vec::new(), |ui| app.run(ui));
        let out = h.frame(Vec::new(), |ui| app.run(ui));
        let title = out.texts().into_iter().find(|t| t.text == APP_SHORT_NAME && t.rect.center().y < TITLE_BAR_H).expect("the app name in the title bar");
        assert!((title.rect.center().x - 640.0).abs() <= 1.0, "the app name is centered, off by {}", title.rect.center().x - 640.0);
        let file = out.text_rect("File").expect("the File menu title");
        assert!(file.center().y < TITLE_BAR_H && file.right() < title.rect.left(), "File sits in the left zone, before the centered name");
        assert!(out.text_rect("Edit").is_none() && out.text_rect("Export").is_none(), "only File, and no Export, without a project");
    }

    #[test]
    fn the_start_file_menu_has_new_and_open_with_their_shortcuts() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut app = App::default();
        let _ = h.frame(Vec::new(), |ui| app.run(ui));
        let out = h.frame(Vec::new(), |ui| app.run(ui));
        let at = out.text_rect("File").expect("the File menu title").center();
        let _ = h.click(at, |ui| app.run(ui));
        let out = h.settle(|ui| app.run(ui));
        // The hero cards repeat "New project", "Ctrl+N" and "Ctrl+O" far below
        // the title bar: only the popover's copies, just under it, count.
        let in_popover = |out: &Out, label: &str| out.texts().into_iter().find(|t| t.text == label && t.rect.center().y < 160.0).map(|t| t.rect);
        for label in ["New project", "Open project...", "Ctrl+N", "Ctrl+O"] {
            assert!(in_popover(&out, label).is_some(), "the File menu shows {label}");
        }
        let item = in_popover(&out, "New project").expect("the New project item").center();
        let _ = h.click(item, |ui| app.run(ui));
        let _ = h.settle(|ui| app.run(ui));
        assert!(matches!(&app.screen, Screen::Start(start) if start.dialog_is_open()), "New project opens the New Project dialog");
    }

    #[test]
    fn undo_and_redo_sit_left_of_the_panel_toolbar_chips_not_in_the_title_bar() {
        let mut rig = Rig::new();
        let _ = rig.frame();
        let out = rig.frame();
        let undo = out.text_rect(ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE).expect("Undo button");
        let redo = out.text_rect(ui_kit::icons::regular::ARROW_CLOCKWISE).expect("Redo button");
        let ripple = out.text_rect("Ripple").expect("Ripple chip");
        let snap = out.text_rect("Snap").expect("Snap chip");
        assert!(undo.center().y > TITLE_BAR_H && redo.center().y > TITLE_BAR_H, "Undo and Redo left the title bar");
        assert!(undo.right() < redo.left() && redo.right() < ripple.left() && ripple.right() < snap.left(), "Undo, Redo, then the chips, left to right");
        assert!((undo.center().y - ripple.center().y).abs() <= 1.0, "one row");
    }
}
