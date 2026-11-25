//! The Prompter's root window: the Library (spec "Library window").

use std::cell::Cell;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use prompter_core::store::{Store, StoreError};
use prompter_platform::os::OsError;
use prompter_platform::view::{NativeWindow, ViewStatus, ViewWindow};
use ui_kit::tokens::{color, radius, size, space, typography};

use crate::dialogs::{Dialog, Naming};
use crate::editor;
use crate::menu::{Item, MENUS, MenuState};
use crate::renderer::AlphaInUse;
use crate::scripts_panel::{Action, ScriptsPanel};
use crate::stats;
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
/// The hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// How often to repaint while a toast shows, so that it goes away on time.
const TOAST_TICK: Duration = Duration::from_millis(250);

const ABOUT: ui_kit::widgets::AboutInfo = ui_kit::widgets::AboutInfo {
    app_name: "Cutmark Prompter",
    version: env!("CARGO_PKG_VERSION"),
    copyright: "Copyright 2026 Cutmark. All rights reserved.",
    notices: "Inter and JetBrains Mono are licensed under the SIL Open Font License 1.1.\n\
              Phosphor Icons and the egui-phosphor crate are licensed under MIT or Apache-2.0.\n\
              egui, eframe, epaint and wgpu are licensed under MIT or Apache-2.0.",
};

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
    appearance_open: bool,
    about_open: bool,
    /// The script library, or why it could not be opened.
    workspace: Result<Workspace, String>,
    scripts: ScriptsPanel,
    dialog: Option<Dialog>,
    /// An Edit menu item picked this frame, to be done in the next one: the click that picked
    /// it would take the keyboard back from the field it is for.
    queued: Option<Item>,
    /// A save failed while closing, so closing again quits without saving.
    close_refused: bool,
}

impl PrompterApp {
    /// Takes the result of the startup OS check.
    pub fn new(os_check: Result<(), OsError>, alpha: AlphaInUse) -> Self {
        if let Err(err) = &os_check {
            log::error!("unsupported system: {err}");
        }
        // An unsupported system only ever sees its dialog, so no library is made on it.
        let workspace = if os_check.is_ok() {
            open_library().map_err(|err| {
                log::error!("the script library could not be opened: {err:#}");
                format!("The script library could not be opened.\n{err:#}")
            })
        } else {
            Err(String::new())
        };
        Self {
            supported: os_check.is_ok(),
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
            dialog: None,
            queued: None,
            close_refused: false,
        }
    }

    fn menu_state(&self) -> MenuState {
        let workspace = self.workspace.as_ref().ok();
        MenuState {
            view_open: self.view.is_some(),
            library_open: workspace.is_some(),
            script_open: workspace.is_some_and(|workspace| workspace.open_script().is_some()),
        }
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

    /// Does what a menu item or a Run panel button asks. Items not wired yet are disabled, so
    /// they never get here.
    fn run(&mut self, ctx: &egui::Context, frame: &eframe::Frame, item: Item) {
        match item {
            Item::New | Item::Import | Item::Export | Item::Duplicate | Item::Delete => self.file_command(ctx, frame, item),
            Item::Undo | Item::Redo | Item::Cut | Item::Copy | Item::Paste | Item::Find => {
                self.queued = Some(item);
                ctx.request_repaint();
            }
            Item::Exit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Item::ShowView if self.view.is_none() => {
                self.last_view_number += 1;
                self.view = Some(FloatingView::new(self.last_view_number, self.monitor.map(view::default_position)));
            }
            Item::HideView => self.view = None,
            Item::ClickThrough => self.click_through = !self.click_through,
            Item::Appearance => self.appearance_open = true,
            Item::About => self.about_open = true,
            _ => {}
        }
    }

    /// The File menu's script items. All but New and Import act on the open script.
    fn file_command(&mut self, ctx: &egui::Context, frame: &eframe::Frame, item: Item) {
        let Ok(workspace) = &mut self.workspace else {
            return;
        };
        let open = workspace.open_script().map(|open| open.id.clone());
        let result = match (item, open) {
            (Item::New, _) => workspace.new_script(),
            (Item::Import, _) => {
                let picker = rfd::FileDialog::new().set_parent(frame).add_filter("Scripts", &["txt", "md"]);
                match picker.pick_file() {
                    Some(source) => workspace.import(&source).map(|split| {
                        if let Some(parts) = split {
                            toast(ctx, format!("The file was over the word limit, so it was split into {parts} scripts."));
                        }
                    }),
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
            Action::Move(id, folder) => workspace.move_to_folder(&id, folder.as_deref()),
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
    fn scripts_column(&mut self, ui: &mut egui::Ui, focus_search: bool) {
        let mut new_folder = false;
        let library_open = self.workspace.is_ok();
        panel_header(ui, "Scripts", |ui| {
            ui.add_enabled_ui(library_open, |ui| {
                new_folder = ui_kit::icon_button(ui, ui_kit::icons::regular::FOLDER_PLUS, "New folder").clicked();
            });
        });
        if new_folder {
            self.dialog = Some(Dialog::name(Naming::NewFolder, ""));
        }
        let action = match &self.workspace {
            Ok(workspace) => self.scripts.show(ui, workspace, focus_search),
            Err(_) => None,
        };
        if let Some(action) = action {
            self.script_action(ui.ctx(), action);
        }
    }

    /// The editor panel: its toolbar, then the open script or what to do without one.
    /// `command` is an Edit menu item picked in the last frame.
    fn editor_column(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, command: Option<Item>) {
        if let Some(item) = panel_toolbar(ui, self.menu_state()) {
            self.run(ui.ctx(), frame, item);
        }
        let workspace = match &mut self.workspace {
            Ok(workspace) => workspace,
            Err(error) => {
                empty_state(ui, error);
                return;
            }
        };
        let no_scripts = workspace.store().library().scripts.is_empty();
        if let Some(open) = workspace.open_script_mut() {
            let id = editor::id(open.serial);
            if let Some(item) = command {
                editor::command(ui, id, item);
            }
            if editor::show(ui, id, &mut open.text) {
                workspace.text_changed(Instant::now());
                // The title bar, already drawn, has to show the unsaved dot.
                ui.ctx().request_repaint();
            }
        } else if no_scripts {
            if first_script(ui) {
                self.run(ui.ctx(), frame, Item::New);
            }
        } else {
            empty_state(ui, "Pick a script or create one");
        }
    }

    /// Saves the open script when due, keeps its word count up to date, and saves it before
    /// the window closes.
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
        if !workspace.open_script().is_some_and(|open| open.unsaved()) {
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
        let command = self.queued.take();
        let picked = Cell::new(None);
        let bar = title_bar(ui, self.menu_state(), self.click_through, self.document().as_ref(), &picked);
        if let Some(item) = picked.get() {
            self.run(ui.ctx(), frame, item);
        }

        let mut rest = ui.max_rect();
        rest.min.y = bar.bottom();
        let (body, status) = rest.split_top_bottom_at_y(rest.bottom() - size::STATUS_BAR_HEIGHT);
        let [scripts, editor, run] = columns(body.shrink(space::SCALE_8));
        column(ui, scripts, |ui| self.scripts_column(ui, command == Some(Item::Find)));
        column(ui, editor, |ui| self.editor_column(ui, frame, command));
        column(ui, run, |ui| {
            panel_header(ui, "Run", |_| {});
            self.view_controls(ui, frame);
        });

        let capture = capture_label(self.view.as_ref().map(|view| view.status()));
        let mut items = vec!["Ready"];
        items.extend(capture);
        ui.scope_builder(egui::UiBuilder::new().max_rect(status.shrink2(egui::vec2(space::SCALE_8, 0.0))), |ui| {
            ui_kit::status_bar(ui, &items);
        });

        ui_kit::appearance_dialog(ui.ctx(), &mut self.appearance_open);
        ui_kit::about_dialog(ui.ctx(), &mut self.about_open, &ABOUT);
        self.show_dialog(ui.ctx());
        if ui_kit::widgets::show_toasts(ui.ctx()).is_some() {
            ui.ctx().request_repaint_after(TOAST_TICK);
        }
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

/// Opens the library in `%APPDATA%\Cutmark\prompter`.
fn open_library() -> anyhow::Result<Workspace> {
    let dirs = directories::BaseDirs::new().context("the user's config folder could not be found")?;
    let store = Store::open(&dirs.config_dir().join("Cutmark").join("prompter"))?;
    Ok(Workspace::new(store))
}

fn toast(ctx: &egui::Context, text: impl Into<String>) {
    ui_kit::widgets::toast(ctx, text);
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

/// The open script's part of the title bar.
struct Document {
    title: String,
    unsaved: bool,
    caption: Option<String>,
}

/// The 44px title bar (rev 3 "Title bar"). The toolbar adds the separator and the caption
/// buttons. A clicked menu item is reported through `picked`.
fn title_bar(ui: &mut egui::Ui, state: MenuState, click_through: bool, document: Option<&Document>, picked: &Cell<Option<Item>>) -> egui::Rect {
    ui_kit::main_toolbar(
        ui,
        |ui| {
            app_mark(ui);
            let mut menus: Vec<ui_kit::Menu<'_>> = MENUS
                .iter()
                .map(|&(title, groups)| {
                    ui_kit::Menu::new(title, move |ui| {
                        for (index, group) in groups.iter().enumerate() {
                            if index > 0 {
                                ui.separator();
                            }
                            for &item in *group {
                                let checked = item == Item::ClickThrough && click_through;
                                if ui.add_enabled(item.enabled(state), menu_button(item).selected(checked)).clicked() {
                                    picked.set(Some(item));
                                    ui.close();
                                }
                            }
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
fn panel_row(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
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

/// The editor's toolbar (rev 3 "Panel toolbar"). Returns the button clicked, as its menu item.
fn panel_toolbar(ui: &mut egui::Ui, state: MenuState) -> Option<Item> {
    let mut picked = None;
    panel_row(ui, |ui| {
        let buttons = [(Item::Undo, ui_kit::icons::regular::ARROW_COUNTER_CLOCKWISE), (Item::Redo, ui_kit::icons::regular::ARROW_CLOCKWISE)];
        for (item, icon) in buttons {
            ui.add_enabled_ui(item.enabled(state), |ui| {
                if ui_kit::icon_button(ui, icon, item.label()).clicked() {
                    picked = Some(item);
                }
            });
        }
    });
    picked
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
/// window has not been found, so nothing excludes it yet.
fn capture_label(view: Option<Option<ViewStatus>>) -> Option<&'static str> {
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
        assert_eq!(capture_label(None), None);
        assert_eq!(capture_label(Some(None)), Some("Visible to capture"));
        assert_eq!(capture_label(Some(Some(ViewStatus::default()))), Some("Visible to capture"));
        assert_eq!(capture_label(Some(Some(hidden))), Some("Hidden from capture"));
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
