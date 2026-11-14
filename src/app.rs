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
    RecordCloseChoice, RecordCloseDialog,
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

/// The user-visible application name, used for the window title, the start
/// screen heading and the installer.
pub const APP_NAME: &str = "CutMark Audio Editor";

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
                if !self.start_output_queried {
                    self.start_output = crate::audio::engine::default_output_name();
                    self.start_output_queried = true;
                }
                if !ui.ctx().egui_wants_keyboard_input() && !start.dialog_is_open() {
                    Self::handle_start_shortcuts(ui, start);
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
                Self::intercept_window_close(
                    ui.ctx(),
                    editor,
                    &mut self.pending_close,
                    self.close_confirmed,
                );
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
    fn intercept_window_close(
        ctx: &egui::Context,
        editor: &Editor,
        pending_close: &mut Option<PendingClose>,
        confirmed: bool,
    ) {
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
        let transport_active = matches!(
            editor.transport_state(),
            TransportState::Playing | TransportState::Previewing | TransportState::Scrubbing
        );
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
                msgs.extend(toolbar::show(ui, editor));
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

        // While a take is recorded the keyboard does three things (Space, Esc,
        // M) and the timeline handles zoom and scroll; everything else is
        // ignored. An open prompt owns Esc and Space, so they do not reach the
        // take then.
        let recording = editor.is_recording();
        if recording
            && !ui.ctx().egui_wants_keyboard_input()
            && !close_pending
            && !messages.as_ref().is_some_and(|dialog| dialog.open)
            && !menu.modal_open()
        {
            Self::handle_recording_keys(ui, editor);
        }

        // Editor shortcuts, only when no text field has keyboard focus, and
        // never while an export locks the project or a take is recorded.
        if !ui.ctx().egui_wants_keyboard_input() && !editor.is_locked() && !recording {
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
            } else if ctrl && r && !shift {
                // Record onto the armed track from the playhead; "Arm a track
                // first" and the like go to the messages dialog.
                if let Err(error) = editor.start_recording() {
                    msgs.push(error.to_string());
                }
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

    /// The keys that work while a take is recorded: Space ends the take and
    /// keeps it, Esc ends it and discards it (the press is consumed, so it
    /// never also clears the selection) and M adds a marker at the record
    /// position. Ctrl+R and every other key do nothing.
    fn handle_recording_keys(ui: &egui::Ui, editor: &mut Editor) {
        ui.input_mut(|i| {
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
    /// "*<name> - CutMark Audio Editor" when dirty,
    /// "<name> - CutMark Audio Editor" otherwise.
    fn update_title(&mut self, ui: &egui::Ui) {
        let title = match &self.screen {
            Screen::Start(_) => APP_NAME.to_string(),
            Screen::Editing(editor) => format!(
                "{}{} - {}",
                if editor.dirty { "*" } else { "" },
                editor.project.name,
                APP_NAME
            ),
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

    const ACTIONS: [PendingClose; 4] = [
        PendingClose::Quit,
        PendingClose::New,
        PendingClose::Open,
        PendingClose::Close,
    ];

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
            let mut editor = Editor::with_project(
                Project::new("Test 2".to_string(), 48_000, BitDepth::Int24),
                ViewState::default(),
                paths,
            );
            editor.dirty = true;
            Rig {
                h: Headless::new(1280.0, 800.0),
                app: App {
                    screen: Screen::Editing(Box::new(editor)),
                    ..App::default()
                },
                _dir: dir,
            }
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
            let mut screen =
                std::mem::replace(&mut self.app.screen, Screen::Start(StartScreen::default()));
            self.app.request_close(&mut screen, pending);
            self.app.screen = screen;
            self.frame()
        }

        /// Clicks the button whose text is `label`; returns the release frame.
        fn click(&mut self, label: &str) -> Out {
            let _ = self.frame();
            let out = self.frame();
            let target = out
                .text_rect(label)
                .unwrap_or_else(|| panic!("no {label} button on screen"))
                .center();
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
        let project =
            crate::model::fixtures::project_with_clips(&[&[(0, 478_080)], &[(268_800, 297_600)]]);
        let media_id = project.media[0].id;
        let samples: Vec<f32> = (0..1_000_000usize)
            .flat_map(|i| {
                let v = ((i as f32) * 0.05).sin() * 0.5;
                [v, v]
            })
            .collect();
        let peaks = crate::io::peaks::build(&samples, 2);
        editor.media_store.insert(
            media_id,
            crate::io::import::Loaded {
                samples: samples.into(),
                peaks: peaks.into(),
            },
        );
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
            let event = egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat: false,
                modifiers,
            };
            let app = &mut rig.app;
            let _ = rig.h.frame(vec![event], |ui| app.run(ui));
        }
        rig.h.set_modifiers(egui::Modifiers::NONE);
    }

    /// A rig over a real project folder with a 10 s clip on armed track 1 and
    /// an empty track 2 (the test input is armed, nothing records yet).
    fn recording_rig() -> Rig {
        let (editor, dir) = crate::ui::test_util::armed_editor(&[&[(0, 480_000)], &[]], "Test 2");
        Rig {
            h: Headless::new(1280.0, 800.0),
            app: App {
                screen: Screen::Editing(Box::new(editor)),
                ..App::default()
            },
            _dir: dir,
        }
    }

    /// Runs frames of the real app until `done` holds; fails after 10 s.
    fn frames_until(rig: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(rig) {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
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
        rig.editor()
            .project
            .media
            .iter()
            .map(|item| item.display_name.clone())
            .collect()
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
            (Screen::Start(start), PendingClose::Open | PendingClose::Close) => {
                !start.dialog_is_open()
            }
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
        frames_until(&mut rig, "the take to be placed", |r| {
            !r.editor().is_recording()
        });

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
        frames_until(&mut rig, "the take to be discarded", |r| {
            !r.editor().is_recording()
        });

        assert_eq!(bin_names(&rig), before, "no bin item");
        assert!(!rig.editor_mut().undo(), "no undo step");
        assert!(!rig.editor().dirty);
        assert!(
            !matches!(
                rig.editor().selection(),
                crate::editor::selection::Selection::None
            ),
            "Esc was consumed by the take, it did not clear the selection"
        );
    }

    #[test]
    fn ctrl_r_without_an_armed_track_says_arm_a_track_first() {
        let mut rig = recording_rig();
        let armed = rig.editor().armed_track().expect("armed");
        rig.editor_mut().toggle_arm(armed);
        for _ in 0..3 {
            let _ = rig.frame();
        }
        tap_with(&mut rig, egui::Key::R, egui::Modifiers::CTRL);
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(!rig.editor().is_recording());
        assert!(
            out.text_rect("Arm a track first").is_some(),
            "the messages dialog says why"
        );
    }

    #[test]
    fn m_adds_a_marker_and_other_keys_do_nothing_while_recording() {
        let mut rig = recording_rig();
        rig.editor_mut().select_all();
        start_take(&mut rig, 2_400);
        let _ = rig.frame();
        let start = rig.editor().recording_status().expect("recording").start;
        assert_eq!(
            rig.editor().view.playhead,
            start + 2_400,
            "the record position"
        );

        tap(&mut rig, egui::Key::M);
        let markers: Vec<u64> = rig
            .editor()
            .project
            .markers
            .iter()
            .map(|marker| marker.position)
            .collect();
        assert_eq!(markers, [start + 2_400], "M marked the record position");

        let clips = rig.editor().project.tracks[0].clips.clone();
        let (ripple, snap) = (rig.editor().view.ripple, rig.editor().view.snap);
        let selection = rig.editor().selection().clone();
        for key in [
            egui::Key::Delete,
            egui::Key::Backspace,
            egui::Key::S,
            egui::Key::R,
            egui::Key::N,
            egui::Key::Home,
            egui::Key::End,
            egui::Key::P,
        ] {
            tap(&mut rig, key);
        }
        for key in [egui::Key::Z, egui::Key::Y, egui::Key::A, egui::Key::S] {
            tap_with(&mut rig, key, egui::Modifiers::CTRL);
        }
        assert_eq!(
            rig.editor().project.markers.len(),
            1,
            "Ctrl+Z undid nothing"
        );
        assert_eq!(rig.editor().project.tracks[0].clips, clips, "Delete and S");
        assert_eq!(
            (rig.editor().view.ripple, rig.editor().view.snap),
            (ripple, snap),
            "R and N toggled nothing"
        );
        assert_eq!(
            rig.editor().selection(),
            &selection,
            "Ctrl+A changed nothing"
        );

        // Ctrl+R again does not start a second take or complain.
        tap_with(&mut rig, egui::Key::R, egui::Modifiers::CTRL);
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(rig.editor().take_is_live());
        assert_eq!(
            rig.editor().recording_status().map(|s| s.start),
            Some(start),
            "still the same take"
        );
        assert!(
            out.text_rect("Recording in progress").is_none(),
            "no dialog"
        );
        assert!(out.text_rect("Arm a track first").is_none());
    }

    /// The painted live clip: the rect with the solid `record_red` border.
    fn live_clip_rect(out: &Out) -> Option<egui::Rect> {
        out.rects()
            .into_iter()
            .find(|r| r.stroke.color == THEME.record_red && r.stroke.width > 0.0)
            .map(|r| r.rect)
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

        let lane_left = out
            .rects()
            .into_iter()
            .filter(|r| r.fill == THEME.lane_a)
            .map(|r| r.rect.left())
            .fold(f32::INFINITY, f32::min);
        let live = live_clip_rect(&out).expect("the live clip is painted");
        assert!(
            (live.left() - (lane_left + 200.0)).abs() < 0.01,
            "starts at the record start: {} vs {}",
            live.left(),
            lane_left + 200.0
        );
        assert!(
            (live.width() - 240.0).abs() < 0.01,
            "24,000 frames at 100 spp: {}",
            live.width()
        );

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
        let mut rig = Rig {
            h: Headless::new(1280.0, 800.0),
            app: App {
                screen: Screen::Editing(Box::new(editor)),
                ..App::default()
            },
            _dir: dir,
        };
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
        assert!(
            editor.view.scroll_x > 20_000 && editor.view.scroll_x <= 90_000,
            "scroll_x {}",
            editor.view.scroll_x
        );
    }

    #[test]
    fn the_timecodes_and_status_bar_show_the_take() {
        let mut rig = recording_rig();
        rig.editor_mut().view.playhead = 48_000;
        let idle = {
            let _ = rig.frame();
            rig.frame()
        };
        assert!(idle.text_rect("PLAYHEAD").is_some());
        assert!(idle.text_rect("REC").is_none());

        start_take(&mut rig, 24_000);
        let out = rig.frame();
        // Toolbar: the record position 1.5 s; status bar: elapsed 0.5 s.
        assert!(out.text_rect("00:00:01.500").is_some(), "toolbar timecode");
        assert!(out.text_rect("00:00:00.500").is_some(), "elapsed time");
        let rec = out
            .texts()
            .into_iter()
            .find(|t| t.text == "REC")
            .expect("REC readout");
        assert_eq!(rec.color, THEME.record_red);
        assert!(out.text_rect("PLAYHEAD").is_none(), "REC replaces PLAYHEAD");
    }

    /// The toolbar's glyphs (above y = 80), left to right.
    fn toolbar_glyphs(out: &Out, glyph: &str) -> Vec<crate::ui::test_util::TextItem> {
        let mut found: Vec<_> = out
            .texts()
            .into_iter()
            .filter(|t| t.text == glyph && t.rect.center().y < 80.0)
            .collect();
        found.sort_by(|a, b| a.rect.center().x.total_cmp(&b.rect.center().x));
        found
    }

    /// The red disc of the Record button: a filled circle of its diameter.
    fn record_disc(out: &Out) -> Option<egui::epaint::CircleShape> {
        out.circles().into_iter().find(|c| {
            c.fill == THEME.record_red && (c.radius * 2.0 - THEME.record_btn_d).abs() < 0.01
        })
    }

    #[test]
    fn the_record_button_toggles_the_take_and_is_red_while_recording() {
        let mut rig = recording_rig();
        for _ in 0..3 {
            let _ = rig.frame();
        }
        let out = rig.frame();
        let glyphs = toolbar_glyphs(&out, egui_phosphor::fill::RECORD);
        assert_eq!(glyphs.len(), 1, "one Record button in the toolbar");
        assert_eq!(glyphs[0].color, THEME.record_red, "red glyph when idle");
        assert!(record_disc(&out).is_none(), "no red fill when idle");
        // After Stop in the transport group, before End.
        let stop = toolbar_glyphs(&out, egui_phosphor::fill::STOP);
        let end = toolbar_glyphs(&out, egui_phosphor::fill::SKIP_FORWARD);
        assert!(stop.last().unwrap().rect.center().x < glyphs[0].rect.center().x);
        assert!(glyphs[0].rect.center().x < end[0].rect.center().x);

        // Click: the take starts.
        let at = glyphs[0].rect.center();
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
        assert!(rig.editor().take_is_live(), "the button started the take");
        crate::ui::test_util::feed_take(rig.editor_mut(), 4_800, 0.5, 4_800);
        let out = rig.frame();
        let disc = record_disc(&out).expect("record_red fill while recording");
        let glyph = &toolbar_glyphs(&out, egui_phosphor::fill::RECORD)[0];
        assert_eq!(glyph.color, THEME.on_record_glyph, "white glyph");
        assert!((disc.center - glyph.rect.center()).length() < 1.5);
        // While recording the Play slot shows Stop.
        assert_eq!(toolbar_glyphs(&out, egui_phosphor::fill::STOP).len(), 2);
        assert!(toolbar_glyphs(&out, egui_phosphor::fill::PLAY).is_empty());

        // Click again: the take ends and is kept.
        let app = &mut rig.app;
        let _ = rig.h.click(at, |ui| app.run(ui));
        assert!(!rig.editor().take_is_live(), "the button stopped the take");
        frames_until(&mut rig, "the take to be placed", |r| {
            !r.editor().is_recording()
        });
        assert!(bin_names(&rig).contains(&"Take 1".to_string()));
        let out = rig.frame();
        assert!(record_disc(&out).is_none(), "red fill is gone");
    }

    #[test]
    fn the_play_and_stop_buttons_keep_the_take_while_recording() {
        for slot in [0, 1] {
            let mut rig = recording_rig();
            start_take(&mut rig, 4_800);
            let out = rig.frame();
            // Play (showing Stop) and Stop are the two STOP glyphs.
            let stops = toolbar_glyphs(&out, egui_phosphor::fill::STOP);
            assert_eq!(stops.len(), 2);
            let at = stops[slot].rect.center();
            let app = &mut rig.app;
            let _ = rig.h.click(at, |ui| app.run(ui));
            assert!(!rig.editor().take_is_live(), "slot {slot} ended the take");
            frames_until(&mut rig, "the take to be placed", |r| {
                !r.editor().is_recording()
            });
            assert!(
                bin_names(&rig).contains(&"Take 1".to_string()),
                "slot {slot}"
            );
        }
    }

    /// Drags the primary button from `from` to `to` over a few frames of the
    /// real app.
    fn drag(rig: &mut Rig, from: egui::Pos2, to: egui::Pos2) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
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
            // Track 2 is empty and has the alternate lane shade.
            let lane = out
                .rects()
                .into_iter()
                .filter(|rect| rect.fill == THEME.lane_b)
                .map(|rect| rect.rect)
                .next()
                .expect("a lane");
            let y = lane.center().y;
            drag(
                r,
                egui::pos2(lane.left() + 200.0, y),
                egui::pos2(lane.left() + 400.0, y),
            );
            let selected = !matches!(
                r.editor().selection(),
                crate::editor::selection::Selection::None
            );
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
        assert!(
            out.text_rect("A recording is in progress.").is_some(),
            "the new dialog"
        );
        for label in ["Keep Take", "Discard", "Cancel"] {
            assert!(out.text_rect(label).is_some(), "{label} button");
        }
        let (keep, discard, cancel) = (
            out.text_rect("Keep Take").unwrap().center().x,
            out.text_rect("Discard").unwrap().center().x,
            out.text_rect("Cancel").unwrap().center().x,
        );
        assert!(keep < discard && discard < cancel, "left to right");
        assert!(out.text_rect("Save changes to Test 2?").is_none());

        let out = rig.click("Keep Take");
        assert!(!out.sent_close());
        frames_until(&mut rig, "the take to be placed", |r| {
            !r.editor().is_recording()
        });
        assert!(bin_names(&rig).contains(&"Take 1".to_string()), "placed");
        let name = rig.editor().project.name.clone();
        let mut out = rig.frame();
        for _ in 0..3 {
            out = rig.frame();
        }
        assert!(
            out.text_rect(&format!("Save changes to {name}?")).is_some(),
            "the unsaved-changes prompt follows"
        );
        assert_eq!(rig.app.pending_close, Some(PendingClose::Quit));

        let out = rig.click("Don't Save");
        assert!(
            out.sent_close() || closes_within(&mut rig, 5),
            "then it closes"
        );
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
            frames_until(&mut rig, "the take to be placed", |r| {
                !r.editor().is_recording()
            });
            // The placed take made the project dirty: Save ends the flow.
            let _ = rig.click("Save");
            assert!(
                left_for_start(&rig, pending),
                "{pending:?} after Keep + Save"
            );
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
}
