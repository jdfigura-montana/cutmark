//! The Prompter's root window: the Library (spec "Library window").

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use prompter_core::store::{Store, StoreError};
use prompter_platform::instance::{FOCUS_MESSAGE, SingleInstance};
use prompter_platform::os::OsError;
use prompter_platform::view::{NativeWindow, ViewStatus, ViewWindow};
use ui_kit::tokens::{color, radius, size, space, typography};

use crate::clipboard::{Clipboard, MenuClipboard, SystemClipboard};
use crate::dialogs::{Dialog, Naming};
use crate::editor::{self, Editor, PasteKeys};
use crate::file_drop;
use crate::find::FindBar;
use crate::formatting::{Action as FormatAction, BlockStyle, ListKind};
use crate::markup::Inline;
use crate::menu::{EDITOR_MENU, Item, MENUS, MenuState, SCRIPTS_MENU};
use crate::prefs::EditorPrefs;
use crate::preview::Preview;
use crate::renderer::AlphaInUse;
use crate::scripts_panel::{Action, ScriptsPanel};
use crate::snapshots::Snapshots;
use crate::stats;
use crate::tabs::{self, TabBar};
use crate::view::{self, FloatingView, log_failure};
use crate::workspace::{Autosave, Workspace};

/// The Prompter's appearance identity: its settings folder name and default accent.
pub const APPEARANCE: ui_kit::AppAppearance = ui_kit::AppAppearance::new("prompter", ui_kit::Preset::Violet);

/// The root window's title, which is also how its window handle is found.
pub const ROOT_TITLE: &str = "Cutmark Prompter";

/// How many frames to look for the root window before giving up.
const ROOT_FIND_FRAMES: u8 = 30;

/// Side of the app mark in the title bar (rev 3 "Title bar").
const APP_MARK: f32 = 18.0;
/// Width of the Scripts panel.
const SCRIPTS_WIDTH: f32 = 260.0;
/// Width of the Run panel.
const RUN_WIDTH: f32 = 320.0;
/// A panel title's inset past the panel's own padding, for 14 in all (rev 3 "Panel header").
const HEADER_PAD: f32 = 6.0;
/// The width of one column of the Format menu: its longest row and the row's padding.
const MENU_COLUMN_WIDTH: f32 = 224.0;
/// The hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// How often to repaint while a toast shows, so that it goes away on time.
const TOAST_TICK: Duration = Duration::from_millis(250);
/// How often to look for a message from a second copy of the app. A message does not wake egui.
const INSTANCE_POLL: Duration = Duration::from_millis(250);
/// How often to repaint while files are dragged over the window. The pointer's moves send no
/// events then.
const DRAG_TICK: Duration = Duration::from_millis(30);
/// How strongly the accent color fills what files would be dropped on, and its outline.
const DROP_TINT: f32 = 0.12;
const DROP_STROKE: f32 = 2.0;

const ABOUT: ui_kit::widgets::AboutInfo = ui_kit::widgets::AboutInfo {
    app_name: "Cutmark Prompter",
    version: env!("CARGO_PKG_VERSION"),
    copyright: "Copyright 2026 Cutmark. All rights reserved.",
    notices: "Inter and JetBrains Mono are licensed under the SIL Open Font License 1.1.\n\
              Phosphor Icons and the egui-phosphor crate are licensed under MIT or Apache-2.0.\n\
              egui, eframe, epaint and wgpu are licensed under MIT or Apache-2.0.",
};

mod toolbar;

/// The root window's state.
pub struct PrompterApp {
    /// The developer switch [`view::SHOW_IN_CAPTURE_VAR`]: nothing is kept out of captures.
    show_in_capture: bool,
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
    appearance_open: bool,
    about_open: bool,
    /// The script library, or why it could not be opened.
    workspace: Result<Workspace, String>,
    scripts: ScriptsPanel,
    editor: Editor,
    prefs: EditorPrefs,
    tab_bar: TabBar,
    find: FindBar,
    preview: Preview,
    clipboard: Box<dyn Clipboard>,
    menu_clipboard: MenuClipboard,
    paste_keys: PasteKeys,
    /// The modifier keys held at the end of the last frame.
    held: egui::Modifiers,
    /// This copy's hold on being the only one, through which later copies are heard.
    instance: Option<SingleInstance>,
    /// The root window, once found.
    root_window: Option<NativeWindow>,
    /// Stands in for the pointer's place during a file drag, which tests have no window to ask.
    #[cfg(test)]
    drag_pointer_at: Option<egui::Pos2>,
    /// The editor's right-click menu is open.
    editor_menu_open: bool,
    dialog: Option<Dialog>,
    /// An Edit or Format menu item picked this frame, to be done in the next one: the click
    /// that picked it would take the keyboard back from the field it is for.
    queued: Option<Item>,
    /// A save failed while closing, so closing again quits without saving.
    close_refused: bool,
}

impl PrompterApp {
    /// Takes the result of the startup OS check and the claim to be the only running copy.
    pub fn new(os_check: Result<(), OsError>, alpha: AlphaInUse, instance: SingleInstance) -> Self {
        if let Err(err) = &os_check {
            log::error!("unsupported system: {err}");
        }
        // An unsupported system only ever sees its dialog, so no library is made on it.
        let folder = if os_check.is_ok() { Some(data_folder()) } else { None };
        let prefs = folder.as_ref().and_then(|folder| folder.as_deref().ok()).map(EditorPrefs::load).unwrap_or_default();
        let workspace = match folder {
            Some(folder) => folder.and_then(|folder| open_library(&folder)).map_err(|err| {
                log::error!("the script library could not be opened: {err:#}");
                format!("The script library could not be opened.\n{err:#}")
            }),
            None => Err(String::new()),
        };
        let show_in_capture = view::show_in_capture(std::env::var_os(view::SHOW_IN_CAPTURE_VAR).as_deref());
        if show_in_capture {
            log::warn!("developer switch {} is on: no window is excluded from capture", view::SHOW_IN_CAPTURE_VAR);
        }
        Self { show_in_capture, instance: Some(instance), prefs, ..Self::with_library(os_check.is_ok(), alpha, workspace) }
    }

    fn with_library(supported: bool, alpha: AlphaInUse, workspace: Result<Workspace, String>) -> Self {
        Self {
            show_in_capture: false,
            supported,
            alpha,
            view: None,
            last_view_number: 0,
            click_through: false,
            monitor: None,
            root_find_frames: ROOT_FIND_FRAMES,
            appearance_open: false,
            about_open: false,
            workspace,
            scripts: ScriptsPanel::default(),
            editor: Editor::default(),
            prefs: EditorPrefs::default(),
            tab_bar: TabBar::default(),
            find: FindBar::default(),
            preview: Preview::default(),
            clipboard: Box::new(SystemClipboard),
            menu_clipboard: MenuClipboard::default(),
            paste_keys: PasteKeys::default(),
            held: egui::Modifiers::default(),
            instance: None,
            root_window: None,
            #[cfg(test)]
            drag_pointer_at: None,
            editor_menu_open: false,
            dialog: None,
            queued: None,
            close_refused: false,
        }
    }

    fn menu_state(&self) -> MenuState {
        let workspace = self.workspace.as_ref().ok();
        let script_open = workspace.is_some_and(|workspace| workspace.open_script().is_some());
        MenuState {
            view_open: self.view.is_some(),
            library_open: workspace.is_some(),
            script_open,
            several_tabs: workspace.is_some_and(|workspace| workspace.tabs().len() > 1),
            show_markdown: self.prefs.show_markdown(),
            editor: if script_open { self.editor.status() } else { Default::default() },
            clipboard_has_text: self.menu_clipboard.has_text(),
            clipboard_has_html: self.menu_clipboard.has_html(),
        }
    }

    /// True while typing goes to the shown script's editor.
    fn editor_has_keyboard(&self, ctx: &egui::Context) -> bool {
        let open = self.workspace.as_ref().ok().and_then(|workspace| workspace.open_script());
        open.is_some_and(|open| ctx.memory(|memory| memory.has_focus(editor::id(open.serial))))
    }

    /// Keeps the Library out of screen captures.
    fn protect_root(&mut self, ctx: &egui::Context) {
        if self.root_find_frames == 0 {
            return;
        }
        self.root_find_frames -= 1;
        if let Some(window) = NativeWindow::find_own_window(ROOT_TITLE) {
            if !self.show_in_capture {
                log_failure("exclude_from_capture on the Library", window.exclude_from_capture());
            }
            log::info!("Library excluded from capture: {}", window.status().excluded);
            self.root_window = Some(window);
            self.root_find_frames = 0;
        } else if self.root_find_frames == 0 {
            log::warn!("the Library window was not found; it is not excluded from capture");
        } else {
            ctx.request_repaint();
        }
    }

    /// Does what a menu item or a Run panel button asks. Items not wired yet are disabled, so
    /// they never get here.
    fn run(&mut self, ctx: &egui::Context, frame: &eframe::Frame, item: Item) {
        match item {
            Item::NewScript => self.dialog = Some(Dialog::name(Naming::NewScript, "")),
            Item::NewFolder => self.dialog = Some(Dialog::name(Naming::NewFolder, "")),
            Item::Import | Item::Export | Item::Duplicate | Item::Delete | Item::Snapshots => self.file_command(ctx, frame, item),
            Item::Undo
            | Item::Redo
            | Item::Cut
            | Item::Copy
            | Item::Paste
            | Item::PastePlain
            | Item::SelectAll
            | Item::FindInScript
            | Item::SearchAll
            | Item::Format(_) => {
                self.queued = Some(item);
                ctx.request_repaint();
            }
            Item::CloseTab => self.change_tabs(ctx, TabChange::CloseActive),
            Item::NextTab => self.change_tabs(ctx, TabChange::Cycle { forward: true }),
            Item::PreviousTab => self.change_tabs(ctx, TabChange::Cycle { forward: false }),
            Item::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Item::ShowView if self.view.is_none() => {
                self.last_view_number += 1;
                self.view = Some(FloatingView::new(self.last_view_number, self.monitor.map(view::default_position), self.show_in_capture));
            }
            Item::HideView => self.view = None,
            Item::ClickThrough => self.click_through = !self.click_through,
            Item::ShowMarkdown => self.toggle_show_markdown(ctx),
            Item::Appearance => self.appearance_open = true,
            Item::About => self.about_open = true,
            _ => {}
        }
    }

    fn toggle_show_markdown(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.prefs.set_show_markdown(!self.prefs.show_markdown()) {
            log::error!("the editor settings were not saved: {error}");
            toast(ctx, format!("Show Markdown could not be saved for next time: {error}"));
        }
        ctx.request_repaint();
    }

    /// The File menu's script items. All but Import act on the open script.
    fn file_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, item: Item) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let open = workspace.open_script().map(|open| open.id.clone());
        let result = match (item, open) {
            (Item::Import, _) => {
                let picker = rfd::FileDialog::new().set_parent(frame).add_filter("Scripts", &file_drop::EXTENSIONS);
                match picker.pick_file() {
                    Some(source) => workspace.import(&source, None).map(|split| report_split(ctx, split)),
                    None => Ok(()),
                }
            }
            (Item::Export, Some(id)) => {
                let name = export_name(workspace.title(&id));
                let picker = rfd::FileDialog::new().set_parent(frame).add_filter("Markdown", &["md"]).set_file_name(name);
                match picker.save_file() {
                    Some(dest) => workspace.export(&id, &dest).map(|()| toast(ctx, "Script exported.")),
                    None => Ok(()),
                }
            }
            (Item::Duplicate, Some(id)) => workspace.duplicate(&id),
            (Item::Delete, Some(id)) => {
                self.dialog = Some(Dialog::Delete { title: workspace.title(&id).to_owned(), id });
                Ok(())
            }
            (Item::Snapshots, Some(id)) => Snapshots::load(workspace, id).map(|snapshots| self.dialog = Some(Dialog::Snapshots(snapshots))),
            _ => Ok(()),
        };
        report(ctx, result);
        ctx.request_repaint();
    }

    /// What a click in the Scripts panel asks for.
    fn script_action(&mut self, ctx: &egui::Context, action: Action) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let result = match action {
            Action::Open(id) => workspace.open(&id),
            Action::Duplicate(id) => workspace.duplicate(&id),
            // One at a time. A failure stops there, with the ones before it moved.
            Action::Move(ids, folder) => ids.iter().try_for_each(|id| workspace.move_to_folder(id, folder.as_deref())),
            Action::MoveFolder(folder, to_index) => workspace.move_folder(&folder, to_index),
            Action::SetFavorite(id, favorite) => workspace.set_favorite(&id, favorite),
            Action::SetSort(sort) => workspace.set_sort(sort),
            Action::SetCollapsed(folder, collapsed) => workspace.set_folder_collapsed(&folder, collapsed),
            Action::DeleteFolder(folder) => workspace.delete_folder(&folder),
            Action::Rename(id) => {
                let title = workspace.title(&id).to_owned();
                self.dialog = Some(Dialog::name(Naming::Script(id), &title));
                Ok(())
            }
            Action::RenameFolder(folder) => {
                self.dialog = Some(Dialog::name(Naming::Folder(folder.clone()), &folder));
                Ok(())
            }
            Action::Delete(id) => {
                self.dialog = Some(Dialog::Delete { title: workspace.title(&id).to_owned(), id });
                Ok(())
            }
        };
        report(ctx, result);
        // What is already drawn this frame shows the old library.
        ctx.request_repaint();
    }

    /// The Scripts panel with its header.
    fn scripts_column(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, focus_search: bool) {
        // Made before the panel's widgets, so that it gets only the clicks none of them take.
        let background = ui.interact(ui.max_rect(), ui.id().with("scripts-background"), egui::Sense::click());
        let state = self.menu_state();
        let mut new_folder = false;
        panel_header(ui, "Scripts", |ui| {
            new_folder = new_folder_button(ui, state);
            ui.add_enabled_ui(state.library_open, |ui| ui_kit::widgets::chip(ui, &mut self.scripts.favorites_only, "Favorites"));
        });
        if new_folder {
            self.run(ui.ctx(), frame, Item::NewFolder);
        }
        let action = match &self.workspace {
            Ok(workspace) => self.scripts.show(ui, workspace, focus_search),
            Err(_) => None,
        };
        if let Some(action) = action {
            self.script_action(ui.ctx(), action);
        }
        if let (Some(item), _) = context_menu(&background, SCRIPTS_MENU, self.menu_state()) {
            self.run(ui.ctx(), frame, item);
        }
    }

    /// Shows, closes or steps through tabs. If the editor had the keyboard, the editor of
    /// the tab now shown takes it.
    fn change_tabs(&mut self, ctx: &egui::Context, change: TabChange) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let editor_of = |workspace: &Workspace| workspace.open_script().map(|open| editor::id(open.serial));
        let typing = editor_of(workspace).is_some_and(|id| ctx.memory(|memory| memory.has_focus(id)));
        let result = match change {
            TabChange::Click(tabs::Action::Activate(id)) => {
                workspace.activate(&id);
                Ok(())
            }
            TabChange::Click(tabs::Action::Close(id)) => workspace.close(&id),
            TabChange::CloseActive => match workspace.open_script().map(|open| open.id.clone()) {
                Some(id) => workspace.close(&id),
                None => Ok(()),
            },
            TabChange::Cycle { forward } => {
                workspace.cycle_tab(forward);
                Ok(())
            }
        };
        report(ctx, result);
        if typing && let Some(id) = editor_of(workspace) {
            ctx.memory_mut(|memory| memory.request_focus(id));
        }
        // The title bar, already drawn, shows the old tab.
        ctx.request_repaint();
    }

    /// The editor panel: its tab bar, toolbar and find bar, then the open script or what to do
    /// without one. `command` is an Edit or Format menu item for the editor to do now.
    fn editor_column(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, command: Option<Item>) {
        let clicked = match &self.workspace {
            Ok(workspace) => self.tab_bar.show(ui, workspace),
            Err(_) => None,
        };
        if let Some(action) = clicked {
            self.change_tabs(ui.ctx(), TabChange::Click(action));
        }
        let state = self.menu_state();
        if let Some(item) = toolbar::show(ui, state, &mut self.preview.open) {
            self.run(ui.ctx(), frame, item);
        }
        let workspace = match &mut self.workspace {
            Ok(workspace) => workspace,
            Err(error) => {
                empty_state(ui, error);
                return;
            }
        };
        let open_serials: Vec<u64> = workspace.tabs().iter().map(|tab| tab.serial).collect();
        self.editor.forget_closed(ui.ctx(), &open_serials);
        // Before the menu's pastes, which are ready as they are.
        let shown_editor = workspace.open_script().map(|open| editor::id(open.serial));
        editor::rich_paste(ui, shown_editor, self.clipboard.as_ref(), &mut self.paste_keys, self.held);
        let no_scripts = workspace.store().library().scripts.is_empty();
        let Some(open) = workspace.open_script_mut() else {
            self.find.close();
            self.editor_menu_open = false;
            if !no_scripts {
                empty_state(ui, "Pick a script or create one");
            } else if first_script(ui) {
                self.run(ui.ctx(), frame, Item::NewScript);
            }
            return;
        };
        let serial = open.serial;
        let id = editor::id(serial);
        let mut formatted = false;
        match command {
            Some(Item::FindInScript) => self.find.open(),
            Some(Item::Format(action)) => formatted = self.editor.format(ui.ctx(), serial, &mut open.text, action),
            Some(item) => editor::command(ui, id, item, self.clipboard.as_ref()),
            None => {}
        }
        if self.find.is_open() {
            let menu_open = self.editor.menu_open();
            panel_row(ui, |ui| self.find.show(ui, &open.text, id, menu_open));
        }
        let (left, right) = split(ui.available_rect_before_wrap(), self.preview.open);
        // Both layouts go through this scope, so that the editor keeps its scroll position.
        let left = egui::UiBuilder::new().id_salt("editor-half").max_rect(left);
        self.editor.show_markdown(self.prefs.show_markdown());
        let shown = ui.scope_builder(left, |ui| self.editor.show(ui, serial, &mut open.text, self.find.highlight())).inner;
        self.find.note_keyboard(ui, id);
        match right {
            Some(right) => {
                let right = egui::UiBuilder::new().id_salt("preview-half").max_rect(right);
                ui.scope_builder(right, |ui| self.preview.show(ui, serial, &open.text, self.editor.cursor_line()));
            }
            None => self.preview.forget(),
        }
        if shown.changed || formatted {
            workspace.text_changed(Instant::now());
            // The title bar and the find bar, already drawn, have to catch up.
            ui.ctx().request_repaint();
        }
        let (picked, menu_open) = context_menu(&shown.response, EDITOR_MENU, self.menu_state());
        self.editor_menu_open = menu_open;
        if let Some(item) = picked {
            self.run(ui.ctx(), frame, item);
        } else if menu_open && !ui.input(|i| i.pointer.any_down() || i.pointer.any_released()) {
            // The selection shows only while the editor has the keyboard.
            ui.memory_mut(|memory| memory.request_focus(id));
        }
    }

    /// Saves each open script when due, keeps the shown one's word count up to date, and
    /// saves them all before the window closes.
    fn keep_saved(&mut self, ctx: &egui::Context) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let now = Instant::now();
        match workspace.autosave(now) {
            Autosave::Done => {}
            Autosave::Waiting(wait) => ctx.request_repaint_after(wait),
            Autosave::Failed { error, first } => {
                if first {
                    log::error!("autosave failed: {error}");
                    toast(ctx, format!("The script could not be saved: {error}"));
                }
                ctx.request_repaint_after(crate::autosave::DELAY);
            }
        }
        if let Some(open) = workspace.open_script_mut() {
            open.count.tick(ctx, &open.text, now);
        }
        // With everything saved, the next failure on close is refused once again.
        if !workspace.tabs().iter().any(|tab| tab.unsaved()) {
            self.close_refused = false;
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.close_refused
            && let Err(error) = workspace.flush()
        {
            log::error!("the script was not saved on close: {error}");
            toast(ctx, "The script could not be saved. Close again to quit without saving it.");
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_refused = true;
        }
    }

    /// The title bar, the three panels and the status bar.
    fn library(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        // Before the menu bar, which has to see the hotkey's key press.
        if click_through_hotkey(ui) {
            self.click_through = !self.click_through;
        }
        let mut command = self.queued.take();
        let modal = self.dialog.is_some() || self.appearance_open || self.about_open;
        if modal {
            self.scripts.cancel_drag();
        }
        // The formatting keys are the editor's, and are done this frame, ahead of what is
        // typed next. Before the editor, which would delete to the line's start on Ctrl+U.
        if !modal && self.editor_has_keyboard(ui.ctx()) {
            let state = self.menu_state();
            if let Some(item) = format_shortcut(ui, state.editor.format.list.is_some()).filter(|item| item.enabled(state)) {
                command = Some(item);
            }
        }
        // Before the editor, which would take Tab and Ctrl+W for itself.
        if let Some(change) = (!modal).then(|| tab_shortcut(ui)).flatten() {
            self.change_tabs(ui.ctx(), change);
        }
        if let Some(item) = (!modal).then(|| shortcut(ui)).flatten()
            && item.enabled(self.menu_state())
        {
            self.run(ui.ctx(), frame, item);
        }
        let picked = Cell::new(None);
        let edit_menu_open = Cell::new(false);
        let bar = title_bar(ui, self.menu_state(), self.click_through, self.document().as_ref(), &picked, &edit_menu_open);
        if let Some(item) = picked.get() {
            self.run(ui.ctx(), frame, item);
        }

        let mut rest = ui.max_rect();
        rest.min.y = bar.bottom();
        let (body, status) = rest.split_top_bottom_at_y(rest.bottom() - size::STATUS_BAR_HEIGHT);
        let [scripts, editor, run] = columns(body.shrink(space::SCALE_8));
        column(ui, scripts, |ui| self.scripts_column(ui, frame, command == Some(Item::SearchAll)));
        column(ui, editor, |ui| self.editor_column(ui, frame, command));
        column(ui, run, |ui| {
            panel_header(ui, "Run", |_| {});
            self.view_controls(ui, frame);
        });
        self.menu_clipboard.watch(ui.ctx(), self.clipboard.as_ref(), edit_menu_open.get() || self.editor_menu_open);

        let capture = capture_label(self.show_in_capture, self.view.as_ref().map(|view| view.status()));
        let mut items = vec!["Ready"];
        items.extend(capture);
        ui.scope_builder(egui::UiBuilder::new().max_rect(status.shrink2(egui::vec2(space::SCALE_8, 0.0))), |ui| {
            ui_kit::status_bar(ui, &items);
        });

        ui_kit::appearance_dialog(ui.ctx(), &mut self.appearance_open);
        ui_kit::about_dialog(ui.ctx(), &mut self.about_open, &ABOUT);
        self.show_dialog(ui.ctx());
        // After everything that can copy, and before the toasts, which report a failure.
        self.write_copies(ui.ctx());
        self.file_drag(ui, scripts, modal);
        if ui_kit::widgets::show_toasts(ui.ctx()).is_some() {
            ui.ctx().request_repaint_after(TOAST_TICK);
        }
        if let Some(cursor) = self.scripts.drag_cursor() {
            ui.ctx().set_cursor_icon(cursor);
        }
        self.held = ui.input(|i| i.modifiers);
    }

    /// Puts what a widget copied or cut on the clipboard, in place of egui's own write.
    fn write_copies(&self, ctx: &egui::Context) {
        let mut copied = Vec::new();
        ctx.output_mut(|output| {
            output.commands.retain_mut(|command| match command {
                egui::OutputCommand::CopyText(text) => {
                    copied.push(std::mem::take(text));
                    false
                }
                _ => true,
            });
        });
        for text in copied {
            if let Err(error) = self.clipboard.write_text(&text) {
                log::error!("the clipboard could not be written: {error}");
                toast(ctx, "The text could not be copied to the clipboard.");
            }
        }
    }

    /// Files dragged in from another program. While they are over the window, the row under
    /// the pointer is marked if it is a folder's or one of its scripts', or else the whole
    /// Scripts panel, which is `panel`. A drop imports them into that folder, or into no
    /// folder. `blocked` is a dialog being open.
    fn file_drag(&mut self, ui: &egui::Ui, panel: egui::Rect, blocked: bool) {
        let (hovering, dropped) = ui.input(|i| {
            let dropped: Vec<_> = i.raw.dropped_files.iter().map(|file| file.path().to_owned()).collect();
            (!i.raw.hovered_files.is_empty(), dropped)
        });
        if blocked || self.workspace.is_err() || (!hovering && dropped.is_empty()) {
            return;
        }
        let target = self.drag_pointer(ui.ctx()).and_then(|at| self.scripts.folder_at(at));
        if dropped.is_empty() {
            mark_drop_target(ui, target.map_or(panel, |(row, _)| row));
            ui.ctx().request_repaint_after(DRAG_TICK);
        } else {
            let folder = target.map(|(_, folder)| folder.to_owned());
            self.import_dropped(ui.ctx(), dropped, folder.as_deref());
        }
    }

    /// Where the pointer is during a file drag. The window gets no mouse events then, so egui
    /// does not know.
    fn drag_pointer(&self, ctx: &egui::Context) -> Option<egui::Pos2> {
        #[cfg(test)]
        if let Some(at) = self.drag_pointer_at {
            return Some(at);
        }
        let (x, y) = self.root_window?.cursor_position()?;
        Some(egui::pos2(x as f32, y as f32) / ctx.pixels_per_point())
    }

    /// Imports each dropped file that can be, and says which could not.
    fn import_dropped(&mut self, ctx: &egui::Context, dropped: Vec<PathBuf>, folder: Option<&str>) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let (supported, unsupported) = file_drop::by_support(dropped);
        for source in supported {
            match workspace.import(&source, folder) {
                Ok(split) => report_split(ctx, split),
                Err(error) => {
                    log::error!("a dropped file was not imported: {error}");
                    toast(ctx, format!("{} could not be imported: {error}", file_drop::name(&source)));
                }
            }
        }
        if !unsupported.is_empty() {
            toast(ctx, file_drop::unsupported_message(&unsupported));
        }
        // What is already drawn this frame shows the old library.
        ctx.request_repaint();
    }

    /// Looks for messages from later copies of the app, which exit after sending them.
    fn hear_other_copies(&self, ctx: &egui::Context) {
        let Some(instance) = &self.instance else {
            return;
        };
        while let Some(message) = instance.try_recv() {
            handle_instance_message(ctx, &message);
        }
        ctx.request_repaint_after(INSTANCE_POLL);
    }

    /// Draws the open library dialog, if there is one.
    fn show_dialog(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Ok(workspace)) = (&mut self.dialog, &mut self.workspace) else {
            return;
        };
        let (open, failure) = dialog.show(ctx, workspace);
        if let Some(error) = failure {
            report(ctx, Err(error));
        }
        if !open {
            self.dialog = None;
            ctx.request_repaint();
        }
    }

    /// The open script as the title bar shows it.
    fn document(&self) -> Option<Document> {
        let workspace = self.workspace.as_ref().ok()?;
        let open = workspace.open_script()?;
        Some(Document { title: workspace.title(&open.id).to_owned(), unsaved: open.unsaved(), caption: open.count.words().map(stats::caption) })
    }

    /// The Run panel's view controls and diagnostics line.
    fn view_controls(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
        let state = self.menu_state();
        let mut picked = None;
        ui.add_space(space::SCALE_8);
        ui.horizontal_wrapped(|ui| {
            for (item, label) in [(Item::ShowView, "Show"), (Item::HideView, "Hide")] {
                if ui.add_enabled(item.enabled(state), egui::Button::new(label)).clicked() {
                    picked = Some(item);
                }
            }
            let button = egui::Button::new(Item::ClickThrough.label()).selected(self.click_through);
            if ui.add(button).on_hover_text(Item::ClickThrough.chord()).clicked() {
                picked = Some(Item::ClickThrough);
            }
        });
        if let Some(item) = picked {
            self.run(ui.ctx(), frame, item);
        }
        ui.add_space(space::SCALE_8);
        let status = self.view.as_ref().map(|view| view.status());
        ui.label(diagnostics(self.alpha.get().map(|mode| format!("{mode:?}")), status));
    }
}

/// The Prompter's folder, `%APPDATA%\Cutmark\prompter`.
fn data_folder() -> anyhow::Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("the user's config folder could not be found")?;
    Ok(dirs.config_dir().join("Cutmark").join("prompter"))
}

/// Opens the library kept in `folder`.
fn open_library(folder: &Path) -> anyhow::Result<Workspace> {
    Ok(Workspace::new(Store::open(folder)?))
}

fn toast(ctx: &egui::Context, text: impl Into<String>) {
    ui_kit::widgets::toast(ctx, text);
}

/// Tells the user when an imported file was split into `parts` scripts.
fn report_split(ctx: &egui::Context, parts: Option<usize>) {
    if let Some(parts) = parts {
        toast(ctx, format!("The file was over the word limit, so it was split into {parts} scripts."));
    }
}

/// Does what a message from another copy of the app asks.
fn handle_instance_message(ctx: &egui::Context, message: &str) {
    if message != FOCUS_MESSAGE {
        log::warn!("another copy sent an unknown message");
        return;
    }
    // A minimized window cannot take the focus, so it is restored first.
    for command in [egui::ViewportCommand::Minimized(false), egui::ViewportCommand::Focus] {
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, command);
    }
}

/// Marks what dragged files would be dropped on.
pub(crate) fn mark_drop_target(ui: &egui::Ui, target: egui::Rect) {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let outline = egui::Stroke::new(DROP_STROKE, accent);
    ui.painter().rect(target, radius::SEGMENT, accent.gamma_multiply(DROP_TINT), outline, egui::StrokeKind::Inside);
}

/// Tells the user about a library operation that failed.
fn report(ctx: &egui::Context, result: Result<(), StoreError>) {
    if let Err(error) = result {
        log::error!("{error}");
        toast(ctx, error.to_string());
    }
}

/// The file name Export suggests for a title.
fn export_name(title: &str) -> String {
    let safe: String = title.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    format!("{safe}.md")
}

/// What the tab bar or its keys ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TabChange {
    Click(tabs::Action),
    CloseActive,
    Cycle { forward: bool },
}

/// The tab change whose keys were pressed this frame: Ctrl+Tab, Ctrl+Shift+Tab or Ctrl+W.
fn tab_shortcut(ui: &egui::Ui) -> Option<TabChange> {
    let ctrl = egui::Modifiers::COMMAND;
    // A pattern without Shift also matches with it, so the Shift keys go first.
    let shortcuts = [
        (ctrl.plus(egui::Modifiers::SHIFT), egui::Key::Tab, TabChange::Cycle { forward: false }),
        (ctrl, egui::Key::Tab, TabChange::Cycle { forward: true }),
        (ctrl, egui::Key::W, TabChange::CloseActive),
    ];
    ui.input_mut(|i| {
        // AltGr is Ctrl with Alt, and types characters on some layouts.
        if i.modifiers.alt {
            return None;
        }
        shortcuts.into_iter().find(|&(modifiers, key, _)| i.consume_key(modifiers, key)).map(|(_, _, change)| change)
    })
}

/// True on the frame Ctrl+Alt+C is pressed in the Library.
fn click_through_hotkey(ui: &egui::Ui) -> bool {
    ui.input_mut(|i| {
        // egui-winit reports Ctrl with C as a copy even when Alt is held, so no C key arrives.
        let pressed = i.modifiers.ctrl && i.modifiers.alt && i.events.iter().any(|event| matches!(event, egui::Event::Copy));
        if pressed {
            // The menu bar takes Alt released with no key press as a tap and moves the keyboard
            // to File, so the C key is put back.
            i.events.push(egui::Event::Key { key: egui::Key::C, physical_key: None, pressed: true, repeat: false, modifiers: i.modifiers });
        }
        pressed
    })
}

/// The menu item whose shortcut was pressed this frame, for the keys no field handles itself.
fn shortcut(ui: &egui::Ui) -> Option<Item> {
    let ctrl = egui::Modifiers::COMMAND;
    let ctrl_shift = ctrl.plus(egui::Modifiers::SHIFT);
    // A pattern without Shift also matches with it, so the Shift keys go first.
    let shortcuts = [
        (ctrl_shift, egui::Key::F, Item::SearchAll),
        (ctrl, egui::Key::F, Item::FindInScript),
        (ctrl_shift, egui::Key::N, Item::NewFolder),
        (ctrl, egui::Key::N, Item::NewScript),
    ];
    ui.input_mut(|i| {
        // AltGr is Ctrl with Alt, and types characters on some layouts.
        if i.modifiers.alt {
            return None;
        }
        shortcuts.into_iter().find(|&(modifiers, key, _)| i.consume_key(modifiers, key)).map(|(_, _, item)| item)
    })
}

/// The Format menu item whose keys were pressed this frame. With the caret in a list,
/// `in_list`, Tab and Shift+Tab indent and outdent. The keys are consumed.
fn format_shortcut(ui: &egui::Ui, in_list: bool) -> Option<Item> {
    use egui::Key;

    ui.input_mut(|i| {
        let mut found = None;
        let held = i.modifiers;
        i.events.retain(|event| {
            let action = match event {
                // egui-winit reports Ctrl with X as a cut even when Shift is held, so no X key
                // arrives.
                egui::Event::Cut if held.command && held.shift && !held.alt => Some(FormatAction::Inline(Inline::Strike)),
                &egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => key_action(key, physical_key, modifiers, in_list),
                _ => None,
            };
            if let Some(action) = action {
                found.get_or_insert(Item::Format(action));
            }
            action.is_none()
        });
        if found == Some(Item::Format(FormatAction::Inline(Inline::Strike))) {
            // The key itself, should it come too.
            i.consume_key(egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT), Key::X);
        }
        found
    })
}

/// The formatting action of one key press.
fn key_action(key: egui::Key, physical: Option<egui::Key>, modifiers: egui::Modifiers, in_list: bool) -> Option<FormatAction> {
    use egui::Key;

    // AltGr is Ctrl with Alt, and types characters on some layouts.
    if modifiers.alt {
        return None;
    }
    if !modifiers.command {
        return (in_list && key == Key::Tab).then_some(if modifiers.shift { FormatAction::Outdent } else { FormatAction::Indent });
    }
    if modifiers.shift {
        // With Shift, some layouts give these keys other characters, so the key's place counts too.
        let is = |wanted: Key| key == wanted || physical == Some(wanted);
        return match () {
            () if key == Key::X => Some(FormatAction::Inline(Inline::Strike)),
            () if is(Key::Num8) => Some(FormatAction::List(ListKind::Bulleted)),
            () if is(Key::Num7) => Some(FormatAction::List(ListKind::Numbered)),
            () if is(Key::Period) => Some(FormatAction::Style(BlockStyle::Quote)),
            () => None,
        };
    }
    match key {
        Key::B => Some(FormatAction::Inline(Inline::Bold)),
        Key::I => Some(FormatAction::Inline(Inline::Italic)),
        Key::U => Some(FormatAction::Inline(Inline::Underline)),
        Key::Num0 => Some(FormatAction::Style(BlockStyle::Paragraph)),
        Key::Num1 => Some(FormatAction::Style(BlockStyle::Heading(1))),
        Key::Num2 => Some(FormatAction::Style(BlockStyle::Heading(2))),
        Key::Num3 => Some(FormatAction::Style(BlockStyle::Heading(3))),
        Key::CloseBracket => Some(FormatAction::Indent),
        Key::OpenBracket => Some(FormatAction::Outdent),
        _ => None,
    }
}

/// The open script's part of the title bar.
struct Document {
    title: String,
    unsaved: bool,
    caption: Option<String>,
}

/// The 44px title bar (rev 3 "Title bar"). The toolbar adds the separator and the caption
/// buttons. A clicked menu item is reported through `picked`, and an open Edit menu through
/// `edit_menu_open`.
fn title_bar(
    ui: &mut egui::Ui,
    state: MenuState,
    click_through: bool,
    document: Option<&Document>,
    picked: &Cell<Option<Item>>,
    edit_menu_open: &Cell<bool>,
) -> egui::Rect {
    ui_kit::main_toolbar(
        ui,
        |ui| {
            app_mark(ui);
            let mut menus: Vec<ui_kit::Menu<'_>> = MENUS
                .iter()
                .map(|&(title, groups)| {
                    ui_kit::Menu::new(title, move |ui| {
                        if title == "Edit" {
                            edit_menu_open.set(true);
                        }
                        let used = if title == "Format" { menu_columns(ui, groups, state) } else { menu_rows(ui, groups, state, click_through) };
                        if let Some(item) = used {
                            picked.set(Some(item));
                        }
                    })
                })
                .collect();
            ui_kit::menu_bar(ui, &mut menus);
        },
        |ui| {
            let title = match document {
                Some(document) => ui_kit::Title::Document { name: &document.title, unsaved: document.unsaved, caption: document.caption.as_deref() },
                None => ui_kit::Title::App(ROOT_TITLE),
            };
            ui_kit::title_block(ui, &title);
        },
        |ui| {
            ui.add_enabled_ui(false, |ui| ui_kit::primary_button(ui, "Start Prompter"));
        },
    )
    .bar
}

/// A long menu's items in two columns, with its first two groups in the left one. One column
/// would be taller than the window.
fn menu_columns(ui: &mut egui::Ui, groups: &[&[Item]], state: MenuState) -> Option<Item> {
    let (left, right) = groups.split_at(groups.len().min(2));
    let mut picked = None;
    ui.horizontal_top(|ui| {
        for half in [left, right] {
            ui.vertical(|ui| {
                ui.set_width(MENU_COLUMN_WIDTH);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    picked = picked.or(menu_rows(ui, half, state, false));
                });
            });
        }
    });
    picked
}

/// A menu's items, with a separator between groups. Returns the one clicked, which also
/// closes the menu.
fn menu_rows(ui: &mut egui::Ui, groups: &[&[Item]], state: MenuState, click_through: bool) -> Option<Item> {
    let mut picked = None;
    for (index, group) in groups.iter().enumerate() {
        if index > 0 {
            ui.separator();
        }
        for &item in *group {
            let formatted = matches!(item, Item::Format(action) if state.editor.format.has(action));
            let checked = (item == Item::ClickThrough && click_through) || (item == Item::ShowMarkdown && state.show_markdown) || formatted;
            if ui.add_enabled(item.enabled(state), menu_button(item).selected(checked)).clicked() {
                picked = Some(item);
                ui.close();
            }
        }
    }
    picked
}

/// A right-click menu on `response`, in the glass popover look of the menu bar. Returns the
/// item clicked and whether the menu is open.
fn context_menu(response: &egui::Response, groups: &[&[Item]], state: MenuState) -> (Option<Item>, bool) {
    let ctx = response.ctx.clone();
    let frame = ui_kit::glass_frame(&ctx, radius::GROUP).inner_margin(egui::Margin::same(space::SCALE_6 as i8));
    // The first frame of a popover is an invisible sizing pass.
    let was_shown = ctx.read_response(egui::Popup::default_response_id(response)).is_some();
    let Some(shown) = egui::Popup::context_menu(response).frame(frame).show(|ui| menu_rows(ui, groups, state, false)) else {
        return (None, false);
    };
    if was_shown {
        ui_kit::paint_glass_highlight(&ctx.layer_painter(shown.response.layer_id), shown.response.rect, radius::GROUP);
    }
    (shown.inner, true)
}

/// A menu row: the label, then the shortcut if the item has one.
fn menu_button(item: Item) -> egui::Button<'static> {
    let mut job = egui::text::LayoutJob::default();
    job.append(item.label(), 0.0, egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY));
    if !item.chord().is_empty() {
        job.append(item.chord(), space::SCALE_12, egui::TextFormat::simple(ui_kit::fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY));
    }
    egui::Button::new(job)
}

/// The app mark at the left of the title bar: a small rounded square in the accent color.
fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(APP_MARK), egui::Sense::hover());
    ui.painter().rect_filled(rect, radius::SEGMENT, ui_kit::Theme::get(ui.ctx()).accent.accent);
}

/// Splits the body into Scripts, editor and Run, with an 8px gap between them.
fn columns(body: egui::Rect) -> [egui::Rect; 3] {
    let mut scripts = body;
    scripts.max.x = body.left() + SCRIPTS_WIDTH;
    let mut run = body;
    run.min.x = body.right() - RUN_WIDTH;
    let mut editor = body;
    editor.min.x = scripts.right() + space::SCALE_8;
    editor.max.x = run.left() - space::SCALE_8;
    [scripts, editor, run]
}

/// A panel that fills `rect`.
fn column(ui: &mut egui::Ui, rect: egui::Rect, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        ui_kit::panel(ui, |ui| {
            ui.set_min_size(ui.available_size());
            add_contents(ui);
        });
    });
}

/// A 44px row at the top of a panel, with a hairline along its bottom that runs edge to edge.
pub(crate) fn panel_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(row);
        add_contents(ui);
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
}

/// A side panel's header row (rev 3 "Panel header"). `trailing` adds to its right end, from
/// right to left.
fn panel_header(ui: &mut egui::Ui, title: &str, trailing: impl FnOnce(&mut egui::Ui)) {
    panel_row(ui, |ui| {
        ui.add_space(HEADER_PAD);
        let text = egui::RichText::new(title).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY);
        ui.add(egui::Label::new(text).selectable(false));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), trailing);
    });
}

/// The Scripts header's button. True if it was clicked.
fn new_folder_button(ui: &mut egui::Ui, state: MenuState) -> bool {
    let item = Item::NewFolder;
    let icon = egui::TextFormat::simple(ui_kit::fonts::icon_regular(typography::BODY_SIZE), color::TEXT_SECONDARY);
    let text = egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_PRIMARY);
    let mut job = egui::text::LayoutJob::default();
    job.append(ui_kit::icons::regular::FOLDER_PLUS, 0.0, icon);
    job.append(item.label(), space::SCALE_4, text);
    ui.add_enabled(item.enabled(state), egui::Button::new(job)).on_hover_text(item.chord()).clicked()
}

/// The editor's space, and with the preview on, the preview's to its right.
fn split(area: egui::Rect, preview: bool) -> (egui::Rect, Option<egui::Rect>) {
    if !preview {
        return (area, None);
    }
    let half = ((area.width() - space::SCALE_8) / 2.0).max(0.0);
    let mut left = area;
    left.max.x = area.left() + half;
    let mut right = area;
    right.min.x = area.right() - half;
    (left, Some(right))
}

/// The editor panel with no scripts at all. True if New was clicked.
fn first_script(ui: &mut egui::Ui) -> bool {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() / 2.0 - size::LIST_ROW_HEIGHT).max(0.0));
        let text = egui::RichText::new("Create your first script").font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY);
        ui.label(text);
        ui.add_space(space::SCALE_8);
        ui_kit::primary_button(ui, "New").clicked()
    })
    .inner
}

/// A caption in the middle of the space left in a panel.
pub(crate) fn empty_state(ui: &mut egui::Ui, caption: &str) {
    let text = egui::RichText::new(caption).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY);
    ui.centered_and_justified(|ui| ui.label(text));
}

/// The status bar's capture text, or `None` with no view open. `Some(None)` is an open view whose
/// window has not been found, so nothing excludes it yet. With the developer switch, `dev`, the
/// text shows with or without a view.
fn capture_label(dev: bool, view: Option<Option<ViewStatus>>) -> Option<&'static str> {
    if dev {
        return Some("Visible to capture (dev)");
    }
    view.map(|status| if status.is_some_and(|status| status.excluded) { "Hidden from capture" } else { "Visible to capture" })
}

/// The diagnostics line. `view` is `None` with no view open, and `Some(None)` while the open
/// view's window has not been found.
fn diagnostics(alpha: Option<String>, view: Option<Option<ViewStatus>>) -> String {
    let yes_no = |on: bool| if on { "yes" } else { "no" };
    let alpha = alpha.unwrap_or_else(|| "unknown".to_owned());
    let view = match view {
        None => "hidden".to_owned(),
        Some(None) => "window not found".to_owned(),
        Some(Some(s)) => format!(
            "excluded {}, no-activate {}, topmost {}, backing clear {}",
            yes_no(s.excluded),
            yes_no(s.no_activate),
            yes_no(s.topmost),
            yes_no(s.backing_is_clear)
        ),
    };
    format!("Alpha mode: {alpha} | View: {view}")
}

impl eframe::App for PrompterApp {
    /// The root viewport is transparent only so that eframe picks a transparent alpha mode for
    /// the views; the Library itself stays opaque.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        ui_kit::tokens::color::BG_GROUND.to_normalized_gamma_f32()
    }

    /// Runs before each frame, and also while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.hear_other_copies(ctx);
        self.keep_saved(ctx);
    }

    /// The last chance to save, for an exit that asked for no frame.
    fn on_exit(&mut self) {
        if let Ok(workspace) = &mut self.workspace
            && let Err(error) = workspace.flush()
        {
            log::error!("the script was not saved on exit: {error}");
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.monitor.is_none() {
            self.monitor = ui.input(|i| i.viewport().monitor_size);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
            if self.supported {
                self.library(ui, frame);
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

#[cfg(test)]
mod library_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame with the hotkey check ahead of a menu bar, as the Library does.
    fn frame(ctx: &egui::Context, modifiers: egui::Modifiers, events: Vec<egui::Event>) -> (bool, Option<usize>) {
        let mut result = (false, None);
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, ..Default::default() };
        let mut output = ctx.run_ui(input, |ui| {
            result.0 = click_through_hotkey(ui);
            result.1 = ui.horizontal(|ui| ui_kit::menu_bar(ui, &mut [ui_kit::Menu::new("File", |_ui| {})]).open).inner;
        });
        output.textures_delta.clear();
        result
    }

    #[test]
    fn the_click_through_hotkey_does_not_enter_the_menu_bar() {
        let ctx = egui::Context::default();
        ctx.set_fonts(ui_kit::fonts::definitions());
        let held = egui::Modifiers { ctrl: true, command: true, alt: true, ..Default::default() };
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        frame(&ctx, Default::default(), Vec::new());
        frame(&ctx, held, Vec::new());
        assert!(frame(&ctx, held, vec![egui::Event::Copy]).0);
        frame(&ctx, Default::default(), Vec::new());
        // Enter opens the first menu only if releasing Alt moved the keyboard to the bar.
        assert_eq!(frame(&ctx, Default::default(), vec![enter]).1, None);
    }

    #[test]
    fn the_status_bar_says_whether_the_view_is_hidden_from_capture() {
        let hidden = ViewStatus { excluded: true, ..Default::default() };
        assert_eq!(capture_label(false, None), None);
        assert_eq!(capture_label(false, Some(None)), Some("Visible to capture"));
        assert_eq!(capture_label(false, Some(Some(ViewStatus::default()))), Some("Visible to capture"));
        assert_eq!(capture_label(false, Some(Some(hidden))), Some("Hidden from capture"));
        for view in [None, Some(Some(hidden))] {
            assert_eq!(capture_label(true, view), Some("Visible to capture (dev)"));
        }
    }

    /// The commands the root window gets for a message from another copy.
    fn commands_for(message: &str) -> Vec<egui::ViewportCommand> {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| handle_instance_message(ui.ctx(), message));
        output.textures_delta.clear();
        output.viewport_output.remove(&egui::ViewportId::ROOT).map(|viewport| viewport.commands).unwrap_or_default()
    }

    #[test]
    fn a_focus_message_restores_the_library_and_then_focuses_it() {
        let commands = commands_for(prompter_platform::instance::FOCUS_MESSAGE);
        let at = |wanted: egui::ViewportCommand| commands.iter().position(|command| *command == wanted);
        let restore = at(egui::ViewportCommand::Minimized(false)).unwrap();
        assert!(restore < at(egui::ViewportCommand::Focus).unwrap(), "{commands:?}");
    }

    #[test]
    fn any_other_message_is_ignored() {
        for message in ["", "Focus", "focus ", "quit"] {
            let commands = commands_for(message);
            assert!(!commands.contains(&egui::ViewportCommand::Focus), "{message:?}: {commands:?}");
            assert!(!commands.contains(&egui::ViewportCommand::Minimized(false)), "{message:?}: {commands:?}");
        }
    }

    #[test]
    fn the_export_name_drops_characters_windows_does_not_allow() {
        assert_eq!(export_name("Intro: take 1/2?"), "Intro_ take 1_2_.md");
    }

    #[test]
    fn the_side_panels_keep_their_width_with_8px_gaps() {
        let body = egui::Rect::from_min_size(egui::pos2(8.0, 52.0), egui::vec2(1084.0, 634.0));
        let [scripts, editor, run] = columns(body);
        assert_eq!((scripts.width(), run.width()), (260.0, 320.0));
        assert_eq!((editor.left() - scripts.right(), run.left() - editor.right()), (8.0, 8.0));
        assert_eq!((scripts.left(), run.right()), (body.left(), body.right()));
    }
}
