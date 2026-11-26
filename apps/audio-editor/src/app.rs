//! app: the eframe::App; start screen vs editor.

use std::path::PathBuf;

use eframe::egui;
use eframe::egui::ViewportCommand;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::editor::message::{self, Message};
use crate::editor::transport::TransportState;
use crate::help;
use crate::ui::bin_panel::{self, BinPanel};
use crate::ui::dialogs::confirm::{CloseChoice, CloseConfirmDialog, ConfirmDialog, ExportCloseChoice, ExportCloseDialog, RecordCloseChoice, RecordCloseDialog};
use crate::ui::dialogs::export::{BANNER_SECONDS, ExportBanner};
use crate::ui::master_strip;
use crate::ui::menu::{HelpMenuAction, ProjectMenu, ProjectMenuAction, app_mark};
use crate::ui::start_screen::StartScreen;
use crate::ui::status_bar;
use crate::ui::timeline::{self, ViewMenuAction};
use crate::ui::widgets::meter::MeterState;
use ui_kit::help::{HelpBook, HelpState, KEYBOARD_PAGE_ID, handle_f1, help_window, open as help_open};
use ui_kit::tokens::{size, space};
use ui_kit::tour;

/// The user-visible application name, used for the window title, the start
/// screen heading and the installer.
pub const APP_NAME: &str = "CutMark Audio Editor";
/// The audio editor's own appearance file folder and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("audio", ui_kit::Preset::Blue);

/// The short app name shown in the start screen's toolbar title area and its
/// panel heading.
pub const APP_SHORT_NAME: &str = "Cutmark Audio";

/// Height of the 44px main toolbar (the window's title bar).
const TOP_BAR_H: f32 = size::TOOLBAR_MAIN_HEIGHT;
/// Height of the bottom status bar panel.
const STATUS_BAR_H: f32 = ui_kit::STATUS_BAR_GAP_HEIGHT;

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

/// The menu choices of one editor frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EditorActions {
    /// The Project menu's chosen action.
    project: ProjectMenuAction,
    /// The Help menu's chosen action.
    help: HelpMenuAction,
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
    /// The help window's open state and its Back and Forward history.
    help: HelpState,
    /// The app's help topics, embedded at build time.
    book: HelpBook,
    /// The first-run tour's running state.
    tour: tour::TourState,
    /// True while the About box is open.
    about_open: bool,
    /// True once the first editing frame checked whether to show the tour.
    tour_checked: bool,
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
            help: HelpState::default(),
            book: help::book(),
            tour: tour::TourState::default(),
            about_open: false,
            tour_checked: false,
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
        // The app's own appearance file feeds the runtime theme; toasts draw
        // on top of everything.
        ui_kit::appearance::poll(ui.ctx());
        ui_kit::show_toasts(ui.ctx());
        // Work on the screen outside self so menu, editor and screen can be
        // borrowed separately within one frame.
        let mut screen = std::mem::replace(&mut self.screen, Screen::Start(StartScreen::default()));
        let mut opened: Option<Editor> = None;
        let mut action = ProjectMenuAction::None;
        let mut help_action = HelpMenuAction::None;
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
                let chosen = Self::show_editor(
                    ui,
                    editor,
                    &mut self.menu,
                    &mut self.bin,
                    &mut self.messages,
                    &mut self.meters,
                    &mut self.export_banner,
                    self.pending_close.is_some(),
                );
                action = chosen.project;
                help_action = chosen.help;
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
        match help_action {
            HelpMenuAction::None => {}
            HelpMenuAction::Topics => help_open(&mut self.help, help::FALLBACK_TOPIC_ID),
            HelpMenuAction::Shortcuts => help_open(&mut self.help, KEYBOARD_PAGE_ID),
            HelpMenuAction::Tour => self.tour.start(),
            HelpMenuAction::About => self.about_open = true,
        }
        if let Some(pending) = resolved {
            if pending == PendingClose::Quit {
                self.close_confirmed = true;
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
            } else {
                self.apply_pending(&mut screen, pending);
            }
        }
        // Help draws over everything, after the screen registered its F1 areas
        // and tour anchors this frame.
        self.show_help(ui, matches!(screen, Screen::Editing(_)));
        self.screen = screen;
    }

    /// Handles F1 and Ctrl+/, starts the first-run tour on the first project
    /// and draws the help window, the About box and the tour over everything.
    /// Called after the screen drew, so the F1 areas and the tour anchors it
    /// reads were registered this frame.
    fn show_help(&mut self, ui: &egui::Ui, editing: bool) {
        let ctx = ui.ctx();
        // The tour shows once, the first time a project opens (spec: S1 Help
        // "First-run tour"); Help > Show the tour starts it again by hand.
        if editing && !self.tour_checked {
            self.tour_checked = true;
            if !tour::help_store::is_seen(help::TOUR.app, help::TOUR.id, help::TOUR.version) {
                self.tour.start();
            }
        }
        // F1 opens the topic for the area under the pointer or the focus.
        handle_f1(ctx, &mut self.help, &self.book);
        if crate::shortcuts::pressed(ctx, "keyboard_shortcuts") {
            help_open(&mut self.help, KEYBOARD_PAGE_ID);
        }
        self.show_help_window(ctx);
        ui_kit::about_dialog(ctx, &mut self.about_open, &help::ABOUT);
        if tour::show_tour(ctx, &mut self.tour, &help::TOUR).is_some() {
            let _ = tour::help_store::mark_seen(help::TOUR.app, help::TOUR.id, help::TOUR.version);
        }
    }

    /// Draws the help window, routing the generated Keyboard shortcuts page
    /// through the help window's `extra` callback (spec: S1 Help "Shortcut
    /// sheet").
    fn show_help_window(&mut self, ctx: &egui::Context) {
        let book = &self.book;
        let help_state = &mut self.help;
        let shortcuts = crate::shortcuts::table();
        let filter_id = egui::Id::new("audio-editor-shortcut-filter");
        help_window(ctx, help_state, book, &mut |id: &str, ui: &mut egui::Ui| {
            if id != KEYBOARD_PAGE_ID {
                return false;
            }
            let mut filter = ui.ctx().data_mut(|data| data.get_temp::<String>(filter_id).unwrap_or_default());
            ui_kit::shortcuts::shortcut_sheet(ui, shortcuts, &mut filter);
            ui.ctx().data_mut(|data| data.insert_temp(filter_id, filter));
            true
        });
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
        let neutral = crate::ui::neutrals(ui.ctx());
        let chosen = std::cell::Cell::new(ProjectMenuAction::None);
        egui::Panel::top("start_bar").exact_size(TOP_BAR_H).frame(egui::Frame::new().fill(neutral.bg_ground)).show(ui, |ui| {
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
    ) -> EditorActions {
        let neutral = crate::ui::neutrals(ui.ctx());
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
        let chosen = std::cell::Cell::new((ProjectMenuAction::None, ViewMenuAction::None, HelpMenuAction::None));
        egui::Panel::top("app_bar").exact_size(TOP_BAR_H).frame(egui::Frame::new().fill(neutral.bg_ground)).show(ui, |ui| {
            ui_kit::main_toolbar(
                ui,
                |ui| {
                    app_mark(ui);
                    ui.add_space(space::SCALE_8);
                    chosen.set(crate::ui::menu::menu_bar(ui, &menu_cell, &editor_cell));
                },
                |ui| {
                    let editor = editor_cell.borrow();
                    let caption = status_bar::format_caption(editor.project.sample_rate, editor.project.export_bit_depth);
                    ui_kit::title_block(ui, &ui_kit::Title::Document { name: &editor.project.name, unsaved: editor.dirty, caption: Some(&caption) });
                },
                |ui| {
                    if ui_kit::primary_button(ui, "Export").clicked() {
                        menu_cell.borrow_mut().open_export(&editor_cell.borrow());
                    }
                    ui.add_space(space::SCALE_4);
                },
            );
            // The whole title bar is the tour's "Menu bar" target.
            ui_kit::tour::anchor(ui, help::ANCHOR_MENU_BAR, ui.max_rect());
        });
        // The dialogs the menu items opened draw once per frame, outside the
        // toolbar's measuring passes.
        menu.show_dialogs(&ui.ctx().clone(), editor);
        let (mut action, view_action, help_action) = chosen.get();

        if action == ProjectMenuAction::Import {
            msgs.extend(media_messages(bin_panel::pick_and_import(editor)));
            action = ProjectMenuAction::None;
        }
        // Bottom: the status bar's strip. Its items are drawn after the
        // timeline, whose center they leave free.
        let status_area = egui::Panel::bottom("status_bar")
            .exact_size(STATUS_BAR_H)
            .frame(egui::Frame::new().fill(neutral.bg_ground).inner_margin(egui::Margin { left: 8, ..Default::default() }))
            .show(ui, |ui| ui.max_rect())
            .inner;

        // Right: Master panel on the ground, fixed width, with the window gap
        // on its right, top and bottom.
        let gap = space::WINDOW_GAP as i8;
        egui::Panel::right("master_strip")
            .resizable(false)
            .exact_size(136.0)
            .frame(egui::Frame::new().fill(neutral.bg_ground).inner_margin(egui::Margin { left: 0, top: gap, bottom: gap, right: gap }))
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
            .frame(egui::Frame::new().fill(neutral.bg_ground).inner_margin(egui::Margin { left: gap, top: gap, bottom: gap, right: 0 }))
            .show(ui, |ui| {
                ui.take_available_space();
                msgs.extend(media_messages(bin.show(ui, editor)));
            });

        // Center: the timeline panel on the ground, 8px window padding all
        // round, filling the remainder.
        let stage_center_x = egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(neutral.bg_ground).inner_margin(egui::Margin::same(space::WINDOW_GAP as i8)))
            .show(ui, |ui| {
                let center_x = ui.max_rect().center().x;
                msgs.extend(timeline::show(ui, editor, view_action, meters));
                center_x
            })
            .inner;
        let mut status_ui = ui.new_child(egui::UiBuilder::new().id_salt("status_bar_items").max_rect(status_area));
        status_bar::show(&mut status_ui, editor, banner.as_ref(), stage_center_x);

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
        // never while an export locks the project or a take is recorded. Every
        // key comes from the shortcut table (`crate::shortcuts`); entries that
        // share a logical key are checked most specific first, because egui's
        // matching ignores an extra Shift or Alt.
        if !ui.ctx().egui_wants_keyboard_input() && !editor.is_locked() && !recording {
            let ctx = ui.ctx();
            // A continuous gesture is active: undo/redo keys are ignored.
            let gesture_active = editor.gesture_base.is_some();
            // Ctrl+G, Ctrl+L and Ctrl+R ignore a held Shift, as before.
            let shift = ui.input(|i| i.modifiers.shift);
            // These read the table without consuming: the prompt dialogs
            // drawn after this handler still get their Esc and Space.
            if !gesture_active && crate::shortcuts::peek(ctx, "redo") {
                let _ = editor.redo();
            } else if !gesture_active && crate::shortcuts::peek(ctx, "undo") {
                let _ = editor.undo();
            } else if crate::shortcuts::peek(ctx, "select_all") {
                editor.select_all();
            } else if crate::shortcuts::peek(ctx, "process_normalize") {
                crate::ui::dialogs::normalize::request(ctx);
            } else if crate::shortcuts::peek(ctx, "fade_in") {
                if let Err(error) = editor.fade_in() {
                    *messages = Some(ConfirmDialog::new("Fade", Self::op_message(&error)));
                }
            } else if crate::shortcuts::peek(ctx, "fade_out") {
                if let Err(error) = editor.fade_out() {
                    *messages = Some(ConfirmDialog::new("Fade", Self::op_message(&error)));
                }
            } else if crate::shortcuts::peek(ctx, "save_as") {
                menu.open_save_as(editor);
            } else if crate::shortcuts::peek(ctx, "add_track") {
                let _ = editor.apply(Action::AddTrack { index: None, name: None });
            } else if crate::shortcuts::peek(ctx, "new_project") {
                action = ProjectMenuAction::New;
            } else if crate::shortcuts::peek(ctx, "open_project") {
                action = ProjectMenuAction::Open;
            } else if crate::shortcuts::peek(ctx, "import_media") {
                msgs.extend(media_messages(bin_panel::pick_and_import(editor)));
            } else if crate::shortcuts::peek(ctx, "export") {
                menu.open_export(editor);
            } else if !shift && crate::shortcuts::peek(ctx, "record") {
                // Record onto the armed track from the playhead; "Arm a track
                // first" and the like go to the messages dialog.
                if let Err(error) = editor.start_recording() {
                    msgs.push(Message::recording(error.to_string()));
                }
            } else if crate::shortcuts::peek(ctx, "save") {
                menu.save(editor);
            } else if crate::shortcuts::peek(ctx, "delete_selection") {
                Self::delete_selection(editor);
            } else if crate::shortcuts::peek(ctx, "clear_selection") {
                editor.clear_selection();
            } else if crate::shortcuts::peek(ctx, "toggle_ripple") {
                editor.toggle_ripple();
            } else if crate::shortcuts::peek(ctx, "toggle_snap") {
                editor.toggle_snap();
            } else if crate::shortcuts::peek(ctx, "split_at_playhead") {
                let _ = editor.split();
            } else if crate::shortcuts::peek(ctx, "copy") {
                editor.copy_selection();
            } else if !gesture_active && crate::shortcuts::peek(ctx, "cut") {
                let _ = editor.cut_selection();
            } else if crate::shortcuts::peek(ctx, "paste") {
                let _ = editor.paste();
            } else if crate::shortcuts::peek(ctx, "trim_to_selection") {
                let _ = editor.trim_to_selection();
            } else if crate::shortcuts::peek(ctx, "add_marker") {
                let _ = editor.add_marker_at_playhead();
            } else if crate::shortcuts::peek(ctx, "previous_marker") {
                editor.jump_prev_marker();
            } else if crate::shortcuts::peek(ctx, "next_marker") {
                editor.jump_next_marker();
            } else if !shift && crate::shortcuts::peek(ctx, "process_gain") {
                // Process keys. egui itself binds none of Ctrl+G / Ctrl+Shift+N
                // / Ctrl+Shift+I / Ctrl+Shift+O / Ctrl+L, so nothing else
                // opens on them.
                crate::ui::dialogs::gain::request_process(ctx);
            } else if !shift
                && crate::shortcuts::peek(ctx, "silence")
                && let Err(error) = editor.silence()
            {
                *messages = Some(ConfirmDialog::new("Silence", Self::op_message(&error)));
            }
        }

        // Transport keys (Space is consumed so a focused button does not also
        // activate). While a take is recorded Space belongs to
        // `handle_recording_keys`.
        if !ui.ctx().egui_wants_keyboard_input() && !recording {
            let keys = crate::shortcuts::table();
            let ctx = ui.ctx();
            if keys.pressed(ctx, "play_selection") {
                // Play selection: plays the range once, then returns to its
                // start. Without a range, Shift+Space does nothing.
                if let crate::editor::selection::Selection::Range { start, end, .. } = editor.selection().clone() {
                    editor.play_range(start, end);
                }
            } else if keys.pressed(ctx, "play_pause") {
                editor.toggle_play();
            } else if keys.pressed(ctx, "pause") {
                editor.toggle_pause();
            } else if keys.pressed(ctx, "jump_to_start") {
                editor.jump_start();
            } else if keys.pressed(ctx, "jump_to_end") {
                editor.jump_end();
            }
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
        EditorActions { project: action, help: help_action }
    }

    /// The keys that work while a take is recorded: Space ends the take and
    /// keeps it, Esc ends it and discards it (the press is consumed, so it
    /// never also clears the selection), M adds a marker at the record
    /// position and Ctrl+S saves the project (the take keeps recording). Ctrl+R
    /// and every other key, Ctrl+N and Ctrl+O included, do nothing.
    fn handle_recording_keys(ui: &egui::Ui, editor: &mut Editor, menu: &mut ProjectMenu) {
        let keys = crate::shortcuts::table();
        let ctx = ui.ctx();
        let ctrl_only = ui.input(|i| i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt);
        if ctrl_only && keys.pressed(ctx, "save") {
            menu.save(editor);
            return;
        }
        if ui.input(|i| i.modifiers.any()) {
            return;
        }
        if keys.pressed(ctx, "record_stop_keep") {
            editor.stop_recording();
        } else if keys.pressed(ctx, "record_discard") {
            editor.cancel_recording();
        } else if keys.pressed(ctx, "add_marker") {
            let _ = editor.add_marker_at_playhead();
        }
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
        let keys = crate::shortcuts::table();
        let ctx = ui.ctx();
        if keys.pressed(ctx, "new_project") {
            *start = StartScreen::with_new_dialog();
        } else if keys.pressed(ctx, "open_project") {
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
    use ui_kit::tokens::color;

    /// The app saves its accent in its own folder and resets to its own default.
    #[test]
    fn the_app_has_its_own_appearance_identity() {
        assert_eq!(APPEARANCE.id, "audio");
        assert_eq!(APPEARANCE.default_preset, ui_kit::Preset::Blue);
    }
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

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
            // The first-run tour would cover the panels; these tests skip it.
            Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), tour_checked: true, ..App::default() }, _dir: dir }
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
        Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), tour_checked: true, ..App::default() }, _dir: dir }
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
        out.rects().into_iter().find(|r| r.stroke.color == color::SEM_RECORD && r.stroke.width > 0.0).map(|r| r.rect)
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
        let mut rig =
            Rig { h: Headless::new(1280.0, 800.0), app: App { screen: Screen::Editing(Box::new(editor)), tour_checked: true, ..App::default() }, _dir: dir };
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
        // Bottom bar: the record position 1.5 s; status bar: elapsed 0.5 s.
        assert!(out.text_rect("00:00:01.500").is_some(), "the bar timecode");
        let rec = out.texts().into_iter().find(|t| t.text.starts_with("Rec ")).expect("the Rec readout");
        assert_eq!(rec.text, "Rec 00:00:00.500", "the elapsed time");
        assert_eq!(rec.color, ui_kit::tokens::color::TEXT_TERTIARY, "the component's caption color");
        assert!(out.text_rect("Playhead 00:00:01.000").is_none(), "Rec replaces Playhead");
    }

    /// The transport glyphs near the window bottom (the bottom bar and the
    /// Play button), left to right.
    fn toolbar_glyphs(out: &Out, glyph: &str) -> Vec<crate::ui::test_util::TextItem> {
        let mut found: Vec<_> = out.texts().into_iter().filter(|t| t.text == glyph && t.rect.center().y > 600.0).collect();
        found.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
        found
    }

    #[test]
    fn the_record_button_toggles_the_take_and_shows_stop_while_recording() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let glyphs = toolbar_glyphs(&out, egui_phosphor::fill::RECORD);
        assert_eq!(glyphs.len(), 1, "one Record button in the bar");
        assert_eq!(glyphs[0].color, color::SEM_RECORD, "the glyph keeps the record color");
        // After Stop in the bar, before the end button.
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
        assert!(toolbar_glyphs(&out, egui_phosphor::fill::RECORD).is_empty(), "the Record button shows Stop while recording");
        // The Stop button and the Record button both show Stop, the latter in
        // the record color; Play stays Play.
        let stops = toolbar_glyphs(&out, egui_phosphor::fill::STOP);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[1].color, color::SEM_RECORD);
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
        // Slots: Play, the Stop button, the Record button (showing Stop).
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
    fn the_bar_jumps_to_both_ends_and_selects_nothing() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        rig.editor_mut().view.playhead = 48_000;
        let out = rig.frame();
        // Back to start is left of the notch, forward right of it.
        let back = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_BACK)[0].rect.center();
        let forward = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_FORWARD)[0].rect.center();
        let app = &mut rig.app;
        let _ = rig.h.click(back, |ui| app.run(ui));
        assert_eq!(rig.editor().view.playhead, 0, "back to start");
        let app = &mut rig.app;
        let _ = rig.h.click(forward, |ui| app.run(ui));
        assert_eq!(rig.editor().view.playhead, 480_000, "forward to the end");
        assert_eq!(rig.editor().selection(), &crate::editor::selection::Selection::None, "a press on the bar selects nothing");
    }

    use crate::audio::player::Command;

    /// A recording rig with a stream-less audio engine, so transport commands
    /// can be read back. Returns the rig and the command queue.
    fn rig_with_engine() -> (Rig, crossbeam_channel::Receiver<Command>) {
        let mut rig = recording_rig();
        let (engine, commands) = crate::audio::engine::Engine::test_double(48_000);
        rig.editor_mut().audio = Some(engine);
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let _ = commands.try_iter().count();
        (rig, commands)
    }

    /// Publishes a transport state and position the way the audio callback
    /// would, then draws the frame that reads them.
    fn audio_reports(rig: &mut Rig, state: u8, playhead: u64) -> Out {
        use std::sync::atomic::Ordering;
        let shared = rig.editor().audio.as_ref().expect("the engine").shared();
        shared.playhead.store(playhead, Ordering::Relaxed);
        shared.state.store(state, Ordering::Relaxed);
        let _ = rig.frame();
        rig.frame()
    }

    /// `Shared::state` codes.
    const STOPPED: u8 = 0;
    /// Playing.
    const PLAYING: u8 = 1;
    /// Paused.
    const PAUSED: u8 = 2;

    /// Clicks the one bottom transport glyph `glyph`.
    fn click_transport(rig: &mut Rig, glyph: &str) {
        let out = rig.frame();
        let found = toolbar_glyphs(&out, glyph);
        assert_eq!(found.len(), 1, "one {glyph:?} button");
        let at = found[0].rect.center();
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
    }

    /// The Play button's 68 px circle.
    fn play_circle(out: &Out) -> egui::epaint::CircleShape {
        out.circles().into_iter().find(|c| c.radius == 34.0).expect("the Play button in the notch")
    }

    #[test]
    fn play_in_the_notch_toggles_playback() {
        let (mut rig, commands) = rig_with_engine();
        let sent = || commands.try_iter().collect::<Vec<_>>();
        rig.editor_mut().view.playhead = 24_000;
        let out = rig.frame();
        let glyph = toolbar_glyphs(&out, egui_phosphor::fill::PLAY);
        assert_eq!(glyph.len(), 1, "stopped shows Play");
        let circle = play_circle(&out);
        assert!((glyph[0].rect.center() - circle.center).length() < 1.5, "the glyph is on the button");
        assert_eq!(circle.fill, ui_kit::Theme::get(&rig.h.ctx).accent.accent, "accent fill");

        click_transport(&mut rig, egui_phosphor::fill::PLAY);
        assert_eq!(sent(), [Command::Play { from: 24_000 }], "stopped: play from the playhead");

        let out = audio_reports(&mut rig, PLAYING, 30_000);
        assert!(toolbar_glyphs(&out, egui_phosphor::fill::PLAY).is_empty(), "playing shows Pause");
        let _ = sent();
        click_transport(&mut rig, egui_phosphor::fill::PAUSE);
        assert_eq!(sent(), [Command::Pause], "playing: pause");

        let out = audio_reports(&mut rig, PAUSED, 31_000);
        assert_eq!(toolbar_glyphs(&out, egui_phosphor::fill::PLAY).len(), 1, "paused shows Play");
        let _ = sent();
        click_transport(&mut rig, egui_phosphor::fill::PLAY);
        assert_eq!(sent(), [Command::Play { from: 30_000 }], "paused: play on from the playhead");
    }

    #[test]
    fn every_transport_action_works_from_the_bar_and_its_shortcut() {
        let (mut rig, commands) = rig_with_engine();
        let sent = || commands.try_iter().collect::<Vec<_>>();

        // Back to start and go to end: the bar buttons, then Home and End.
        for by_key in [false, true] {
            rig.editor_mut().view.playhead = 48_000;
            if by_key {
                tap(&mut rig, egui::Key::End);
            } else {
                click_transport(&mut rig, egui_phosphor::fill::SKIP_FORWARD);
            }
            assert_eq!(sent(), [Command::Seek(480_000)], "go to end, key: {by_key}");
            if by_key {
                tap(&mut rig, egui::Key::Home);
            } else {
                click_transport(&mut rig, egui_phosphor::fill::SKIP_BACK);
            }
            assert_eq!(sent(), [Command::Seek(0)], "back to start, key: {by_key}");
        }

        // Play: Space (the button is covered by the test above).
        tap(&mut rig, egui::Key::Space);
        assert_eq!(sent(), [Command::Play { from: 0 }], "Space plays");
        // While playing, P pauses; paused, P resumes.
        let _ = audio_reports(&mut rig, PLAYING, 1_000);
        let _ = sent();
        tap(&mut rig, egui::Key::P);
        assert_eq!(sent(), [Command::Pause], "P pauses");
        let _ = audio_reports(&mut rig, PAUSED, 2_000);
        let _ = sent();
        tap(&mut rig, egui::Key::P);
        assert_eq!(sent(), [Command::Play { from: 2_000 }], "P resumes");

        // Stop: the bar button, and Space while playing.
        let _ = audio_reports(&mut rig, PLAYING, 3_000);
        let _ = sent();
        click_transport(&mut rig, egui_phosphor::fill::STOP);
        assert_eq!(sent(), [Command::Stop], "the Stop button");
        tap(&mut rig, egui::Key::Space);
        assert_eq!(sent(), [Command::Stop], "Space stops");
        let _ = audio_reports(&mut rig, STOPPED, 0);
        let _ = sent();

        // Record: the bar button starts a take, then Ctrl+R does.
        for by_key in [false, true] {
            if by_key {
                tap_with(&mut rig, egui::Key::R, egui::Modifiers::CTRL);
            } else {
                click_transport(&mut rig, egui_phosphor::fill::RECORD);
            }
            assert!(rig.editor().take_is_live(), "a take started, key: {by_key}");
            crate::ui::test_util::feed_take(rig.editor_mut(), 4_800, 0.5, 4_800);
            tap(&mut rig, egui::Key::Space);
            assert!(!rig.editor().take_is_live(), "Space kept the take");
            frames_until(&mut rig, "the take to be placed", |r| !r.editor().is_recording());
        }
    }

    #[test]
    fn the_status_bar_leaves_the_play_button_clear() {
        let mut rig = rig_with_audio();
        // A range selection and a finished export make the longest status line.
        rig.editor_mut().select_all();
        rig.app.export_banner = Some(ExportBanner { name: "episode_12_final_mix.wav".to_string(), path: PathBuf::from("episode_12_final_mix.wav"), at: 0.0 });
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let center = play_circle(&out).center.x;
        assert!((center - 640.0).abs() > 20.0, "the Media and Master panels differ, so the stage is off the window center: {center}");
        let items: Vec<egui::Rect> = out.texts().into_iter().filter(|t| t.rect.center().y > 770.0).map(|t| t.rect).collect();
        assert!(items.len() >= 5, "the status items are drawn: {}", items.len());
        let half = ui_kit::STATUS_BAR_CENTER_GAP / 2.0;
        for rect in &items {
            assert!(rect.right() <= center - half + 0.5 || rect.left() >= center + half - 0.5, "{rect:?} reaches into the gap around {center}");
        }
        assert!(items.iter().any(|r| r.left() >= center + half - 0.5), "items continue after the gap");
    }

    #[test]
    fn a_picked_accent_tints_the_whole_editor_and_the_notch() {
        let mut rig = Rig::new();
        let _ = rig.frame();
        let teal = ui_kit::Theme::new(ui_kit::accent::derive(ui_kit::tokens::accent::PRESET_TEAL), false);
        ui_kit::Theme::set(&rig.h.ctx, teal);
        ui_kit::refresh(&rig.h.ctx);
        let _ = rig.frame();
        let out = rig.frame();
        let fills: Vec<egui::Color32> = out.rects().into_iter().map(|r| r.fill).collect();
        for (name, fill) in [("ground", teal.neutrals.bg_ground), ("side panels", teal.neutrals.bg_side), ("stage", teal.neutrals.bg_panel)] {
            assert!(fills.contains(&fill), "the {name} follow the accent");
        }
        for (name, fixed) in [("ground", color::BG_GROUND), ("panel", color::BG_PANEL)] {
            assert!(!fills.contains(&fixed), "no untinted {name} is left");
        }
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

    #[test]
    fn the_editor_shows_no_dot_joined_strings() {
        let mut rig = rig_with_audio();
        rig.editor_mut().select_all();
        let _ = rig.frame();
        let out = rig.frame();
        for text in out.texts() {
            assert!(!text.text.contains('\u{b7}'), "{:?} joins facts with a dot", text.text);
        }
    }

    /// The title bar's height: nothing of the panel toolbar sits inside it.
    const TITLE_BAR_H: f32 = size::TOOLBAR_MAIN_HEIGHT;

    #[test]
    fn the_title_bar_centers_the_project_name_dot_and_caption() {
        let mut rig = Rig::new();
        let _ = rig.frame();
        let out = rig.frame();
        let caption = status_bar::format_caption(48_000, BitDepth::Int24);
        assert_eq!(caption, "48 kHz 24-bit", "the caption is one fact, the project format");
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
    // ---- Shortcut table: one kittest per group (S1 phase 14c) ----

    /// A kittest harness driving the real app frame over a fresh, dirty
    /// project in a real folder. The app stays behind an `Rc` so the test can
    /// read the editor after a key press; keep the returned folder alive.
    fn shortcut_harness() -> (egui_kittest::Harness<'static>, Rc<RefCell<App>>, tempfile::TempDir) {
        shortcut_harness_at(egui::vec2(1280.0, 800.0))
    }

    /// [`shortcut_harness`] in a window of `size`.
    fn shortcut_harness_at(size: egui::Vec2) -> (egui_kittest::Harness<'static>, Rc<RefCell<App>>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = create_layout(dir.path(), "Test 2").expect("layout");
        let mut editor = Editor::with_project(Project::new("Test 2".to_string(), 48_000, BitDepth::Int24), ViewState::default(), paths);
        editor.dirty = true;
        let app = Rc::new(RefCell::new(App { screen: Screen::Editing(Box::new(editor)), tour_checked: true, ..App::default() }));
        let draw = Rc::clone(&app);
        let first_pass = Rc::new(Cell::new(true));
        let mut harness = egui_kittest::Harness::builder().with_size(size).build_ui(move |ui| {
            // The first pass installs the ui-kit theme and fonts; egui applies
            // a font change at the start of the next pass.
            if first_pass.get() {
                first_pass.set(false);
                ui_kit::apply(ui.ctx());
                return;
            }
            draw.borrow_mut().run(ui);
        });
        harness.step();
        harness.step();
        (harness, app, dir)
    }

    /// In the smallest window with the Media panel dragged to its widest, the
    /// bottom bar leaves out what does not fit and no control leaves it.
    #[test]
    fn the_narrowest_bottom_bar_keeps_every_control_inside_it() {
        use egui_kittest::kittest::Queryable as _;
        let (mut harness, app, _dir) = shortcut_harness_at(egui::vec2(1100.0, 700.0));
        // A clip gives the bar a real total length to leave out.
        with_editor_mut(&app, |editor| {
            editor.project = crate::model::fixtures::project_with_clips(&[&[(0, 144_000)]]).into();
        });
        harness.step();
        harness.step();
        let panel = |harness: &egui_kittest::Harness<'_>, id: &str| {
            egui::containers::panel::PanelState::load(&harness.ctx, egui::Id::new(id)).expect("the panel").outer_rect
        };
        let media = panel(&harness, "media_bin");
        let edge = egui::pos2(media.right(), media.center().y);
        harness.hover_at(edge);
        harness.step();
        harness.drag_at(edge);
        harness.step();
        harness.hover_at(egui::pos2(900.0, edge.y));
        harness.step();
        harness.drop_at(egui::pos2(900.0, edge.y));
        for _ in 0..4 {
            harness.step();
        }
        let media = panel(&harness, "media_bin");
        assert!((media.width() - 400.0).abs() < 1.0, "the Media panel is at its widest: {media:?}");

        // The stage sits one window gap inside what the side panels leave; its
        // bar is its bottom 64, with the 112 wide notch column in the middle.
        let gap = space::WINDOW_GAP;
        let (left, right) = (media.right() + gap, panel(&harness, "master_strip").left() - gap);
        let play = harness.get_by_label("Play").rect();
        assert!((play.center().x - (left + right) / 2.0).abs() < 1.0, "the stage is where the test expects it: {play:?}");
        let bottom = play.center().y + 8.0;
        let bar = egui::Rect::from_min_max(egui::pos2(left, bottom - 64.0), egui::pos2(right, bottom));
        let notch = egui::Rect::from_center_size(bar.center(), egui::vec2(112.0, 64.0));

        let control = |label: &str| harness.get_by_label(label).rect();
        let left_side = [control("00:00:00.000"), control("Go to start (Home)")];
        let right_side = [control("Stop"), control("Record (Ctrl+R)")];
        for rect in left_side {
            assert!(bar.contains_rect(rect) && rect.right() <= notch.left(), "{rect:?} leaves the left side of {bar:?}");
        }
        for rect in right_side {
            assert!(bar.contains_rect(rect) && rect.left() >= notch.right(), "{rect:?} leaves the right side of {bar:?}");
        }
        assert!(left_side[0].right() < left_side[1].left(), "the timecode and back to start do not overlap");
        assert!(right_side[0].right() < right_side[1].left(), "Stop and Record do not overlap");
        assert!(left_side[0].height() < 18.0, "the timecode is the small one: {:?}", left_side[0]);
        assert!(harness.query_by_label("Go to end (End)").is_none(), "go to end is left out");
        assert!(harness.query_by_label("00:00:03.000").is_none(), "the total length is left out");
    }

    /// Presses `key` with `modifiers` in the harness: one frame per queued
    /// event, then a frame to settle.
    fn shortcut(harness: &mut egui_kittest::Harness<'static>, key: egui::Key, modifiers: egui::Modifiers) {
        harness.key_press_modifiers(modifiers, key);
        harness.step();
        harness.step();
    }

    /// Runs `read` on the editor behind `app` (panics on the start screen).
    fn with_editor<T>(app: &Rc<RefCell<App>>, read: impl FnOnce(&Editor) -> T) -> T {
        let app = app.borrow();
        match &app.screen {
            Screen::Editing(editor) => read(editor),
            Screen::Start(_) => panic!("expected the editor screen"),
        }
    }

    /// Runs `change` on the editor behind `app` (panics on the start screen).
    fn with_editor_mut<T>(app: &Rc<RefCell<App>>, change: impl FnOnce(&mut Editor) -> T) -> T {
        let mut app = app.borrow_mut();
        match &mut app.screen {
            Screen::Editing(editor) => change(editor),
            Screen::Start(_) => panic!("expected the editor screen"),
        }
    }
    /// File: Ctrl+S saves the project.
    #[test]
    fn file_group_ctrl_s_saves_the_project() {
        let (mut harness, app, _dir) = shortcut_harness();
        assert!(with_editor(&app, |editor| editor.dirty), "starts dirty");
        shortcut(&mut harness, egui::Key::S, egui::Modifiers::CTRL);
        assert!(!with_editor(&app, |editor| editor.dirty), "Ctrl+S saved (File)");
    }

    /// Edit: R toggles ripple.
    #[test]
    fn edit_group_r_toggles_ripple() {
        let (mut harness, app, _dir) = shortcut_harness();
        assert!(!with_editor(&app, |editor| editor.view.ripple), "ripple starts off");
        shortcut(&mut harness, egui::Key::R, egui::Modifiers::NONE);
        assert!(with_editor(&app, |editor| editor.view.ripple), "R turned ripple on (Edit)");
    }

    /// Playback: Home jumps the playhead to the start.
    #[test]
    fn playback_group_home_jumps_to_the_start() {
        let (mut harness, app, _dir) = shortcut_harness();
        with_editor_mut(&app, |editor| editor.view.playhead = 48_000);
        shortcut(&mut harness, egui::Key::Home, egui::Modifiers::NONE);
        assert_eq!(with_editor(&app, |editor| editor.view.playhead), 0, "Home jumped to the start (Playback)");
    }

    /// Timeline: M adds a marker at the playhead.
    #[test]
    fn timeline_group_m_adds_a_marker() {
        let (mut harness, app, _dir) = shortcut_harness();
        with_editor_mut(&app, |editor| editor.view.playhead = 4_800);
        assert_eq!(with_editor(&app, |editor| editor.project.markers.len()), 0, "no markers yet");
        shortcut(&mut harness, egui::Key::M, egui::Modifiers::NONE);
        assert_eq!(with_editor(&app, |editor| editor.project.markers.len()), 1, "M added a marker (Timeline)");
    }

    /// Tracks: Ctrl+Shift+T adds a track.
    #[test]
    fn tracks_group_ctrl_shift_t_adds_a_track() {
        let (mut harness, app, _dir) = shortcut_harness();
        assert_eq!(with_editor(&app, |editor| editor.project.tracks.len()), 0, "no tracks yet");
        shortcut(&mut harness, egui::Key::T, egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
        assert_eq!(with_editor(&app, |editor| editor.project.tracks.len()), 1, "Ctrl+Shift+T added a track (Tracks)");
    }

    /// View: backslash zooms back to fit.
    #[test]
    fn view_group_backslash_zooms_to_fit() {
        let (mut harness, app, _dir) = shortcut_harness();
        // Let the panel layout settle, then take the fit the layout gives now.
        harness.step();
        shortcut(&mut harness, egui::Key::Backslash, egui::Modifiers::NONE);
        let fit = with_editor(&app, |editor| editor.view.samples_per_pixel);
        with_editor_mut(&app, |editor| editor.view.samples_per_pixel = fit * 1.5);
        harness.step();
        shortcut(&mut harness, egui::Key::Backslash, egui::Modifiers::NONE);
        let zoomed = with_editor(&app, |editor| editor.view.samples_per_pixel);
        assert!((zoomed - fit).abs() <= fit * 0.001, "backslash zoomed back to fit (View): {zoomed} vs {fit}");
    }

    /// Help: the two entries are reserved, so F1 and Ctrl+/ do nothing yet.
    #[test]
    fn help_group_is_reserved_and_does_nothing_yet() {
        let table = crate::shortcuts::table();
        assert!(table.get("help_topics").is_some() && table.get("keyboard_shortcuts").is_some(), "both Help entries exist (Help)");
        let (mut harness, app, _dir) = shortcut_harness();
        shortcut(&mut harness, egui::Key::F1, egui::Modifiers::NONE);
        shortcut(&mut harness, egui::Key::Slash, egui::Modifiers::CTRL);
        assert_eq!(with_editor(&app, |editor| editor.undo_label()), None, "nothing ran");
        assert!(with_editor(&app, |editor| editor.dirty), "nothing was saved or closed");
    }

    /// Typing in a focused text field never triggers the single-key editor
    /// shortcuts (the handler skips them while a text field has focus).
    #[test]
    fn typing_in_a_text_field_does_not_trigger_single_key_shortcuts() {
        use egui_kittest::kittest::Queryable;
        let (mut harness, app, _dir) = shortcut_harness();
        // The Project Settings dialog has the one text field on screen.
        {
            let mut app_ref = app.borrow_mut();
            let app = &mut *app_ref;
            match &mut app.screen {
                Screen::Editing(editor) => app.menu.open_settings(editor),
                Screen::Start(_) => panic!("expected the editor screen"),
            }
        }
        harness.step();
        harness.step();
        harness.get_by_role(egui::accesskit::Role::TextInput).click();
        harness.step();
        harness.step();
        assert!(harness.ctx.egui_wants_keyboard_input(), "the name field has the keyboard");
        harness.get_by_role(egui::accesskit::Role::TextInput).type_text("sm");
        shortcut(&mut harness, egui::Key::S, egui::Modifiers::NONE);
        shortcut(&mut harness, egui::Key::M, egui::Modifiers::NONE);
        assert_eq!(with_editor(&app, |editor| editor.project.markers.len()), 0, "M in a text field added no marker");
        assert_eq!(with_editor(&app, |editor| editor.project.tracks.len()), 0, "no track was renamed or added");
        assert_eq!(with_editor(&app, |editor| editor.undo_label()), None, "S in a text field split nothing");
    }

    // ---- Help: F1, the tour and the About box ----

    /// One lock for the tests that set the process-wide help dir.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Runs `work` with `CUTMARK_HELP_DIR` pointing at a fresh empty folder, so
    /// the tour's seen flag starts unset. The tour tests share `ENV_LOCK`, so
    /// they never see each other's folder.
    fn with_help_dir<T>(work: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = std::env::temp_dir().join(format!("cutmark-audio-help-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the help dir");
        // SAFETY: ENV_LOCK serializes every test that touches this variable.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("CUTMARK_HELP_DIR", &dir)
        };
        let result = work();
        // SAFETY: as above; the lock is still held here.
        #[allow(unsafe_code)]
        unsafe {
            std::env::remove_var("CUTMARK_HELP_DIR")
        };
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// One key press and release pair with no modifiers, for the app frame.
    fn key_event(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn f1_over_the_transport_opens_the_transport_topic() {
        let mut rig = Rig::new();
        // Let the panels settle, then point at the bar timecode and at the
        // Play button.
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let timecode = out.text_rect("00:00:00.000").expect("the bar timecode").center();
        let play = play_circle(&out).center;
        for (what, at) in [("the bar", timecode), ("the Play button", play)] {
            rig.app.help = HelpState::default();
            let events = vec![egui::Event::PointerMoved(at), key_event(egui::Key::F1)];
            let app = &mut rig.app;
            let _ = rig.h.frame(events, |ui| app.run(ui));
            assert!(rig.app.help.open, "F1 opened the help window");
            assert_eq!(rig.app.help.current.as_deref(), Some("playback-and-transport"), "F1 over {what} opened the transport topic");
        }
    }

    #[test]
    fn the_tour_shows_once_on_a_fresh_profile() {
        with_help_dir(|| {
            let mut rig = Rig::new();
            // The test harness skips the tour; this test wants the real check.
            rig.app.tour_checked = false;
            let _ = rig.frame();
            assert!(rig.app.tour.open, "a fresh profile starts the tour on the first project frame");

            // Esc skips it, and skipping marks it seen.
            let app = &mut rig.app;
            let _ = rig.h.frame(vec![key_event(egui::Key::Escape)], |ui| app.run(ui));
            assert!(!rig.app.tour.open, "Esc skipped the tour");
            assert!(
                ui_kit::tour::help_store::is_seen(crate::help::TOUR.app, crate::help::TOUR.id, crate::help::TOUR.version),
                "skipping the tour marked it seen"
            );
        });
    }

    #[test]
    fn the_tour_does_not_show_after_it_is_marked_seen() {
        with_help_dir(|| {
            ui_kit::tour::help_store::mark_seen(crate::help::TOUR.app, crate::help::TOUR.id, crate::help::TOUR.version).expect("mark the tour seen");
            let mut rig = Rig::new();
            rig.app.tour_checked = false;
            let _ = rig.frame();
            assert!(!rig.app.tour.open, "a seen tour does not show again");
        });
    }

    #[test]
    fn about_opens_from_the_help_menu() {
        let mut rig = Rig::new();
        let _ = rig.frame();
        // Help > About Cutmark Audio.
        let _ = rig.click("Help");
        let out = rig.click("About Cutmark Audio");
        assert!(rig.app.about_open, "the Help menu opened the About box");
        assert!(out.text_rect("About Cutmark Audio").is_some(), "the About dialog is drawn");
        // The dialog's footer settles a frame after the modal appears.
        let app = &mut rig.app;
        let out = rig.h.settle(|ui| app.run(ui));
        assert!(out.text_rect("Close").is_some(), "the About dialog has its Close button");
    }
}
