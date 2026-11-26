//! ui::menu: the toolbar's menu popover (Project, Edit, Markers, Process and
//! View), the toolbar's document title and the dialogs the menus open.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::ui::dialogs::audio_settings::AudioSettingsDialog;
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::dialogs::export::ExportDialog;
use crate::ui::dialogs::project_settings::ProjectSettingsDialog;
use crate::ui::dialogs::save_as::SaveAsDialog;
use crate::ui::timeline::ViewMenuAction;
use ui_kit::fonts;
use ui_kit::tokens::{space, typography};

/// What the user chose in the Project menu this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectMenuAction {
    /// Nothing chosen.
    None,
    /// Close the project and open the New Project dialog on the start screen.
    New,
    /// Close the project and open the project picker on the start screen.
    Open,
    /// Close the project and return to the start screen.
    Close,
    /// Open the media import picker.
    Import,
}

/// What the user chose in the Help menu this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpMenuAction {
    /// Nothing chosen.
    None,
    /// Open the help window on the current topic ("Help topics", F1).
    Topics,
    /// Open the generated "Keyboard shortcuts" page (Ctrl+/).
    Shortcuts,
    /// Show the first-run tour again.
    Tour,
    /// Open the About box.
    About,
}

/// Thickness of a menu separator line.
const SEPARATOR_THICKNESS: f32 = 1.0;

/// A menu separator: 1 px line in the hairline color with 4 px vertical margin.
fn menu_sep(ui: &mut egui::Ui) {
    let neutral = crate::ui::neutrals(ui.ctx());
    ui.add_space(space::SCALE_4);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), SEPARATOR_THICKNESS), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, neutral.stroke_hairline);
    ui.add_space(space::SCALE_4);
}

/// A menu item button with a right-aligned type.value shortcut hint. The hint
/// is the shortcut table's text for `id`, so menus and the sheet never drift.
fn item(ctx: &egui::Context, label: impl Into<String>, id: &str) -> egui::Button<'static> {
    let neutral = crate::ui::neutrals(ctx);
    egui::Button::new(label.into())
        .right_text(egui::RichText::new(crate::shortcuts::text(ctx, id)).font(fonts::mono(typography::VALUE_SIZE)).color(neutral.text_tertiary))
}

/// The app mark's side (spec rev 3 "Title bar": 18).
const APP_MARK_SIZE: f32 = 18.0;

/// Draws the app mark placeholder: an accent circle with a notch cut from its
/// top right (spec "Open questions" item 1: placeholder until a real logo).
pub(crate) fn app_mark(ui: &mut egui::Ui) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(APP_MARK_SIZE, APP_MARK_SIZE), egui::Sense::hover());
    paint_app_mark(ui, rect, neutral.bg_ground);
}

/// Paints the app mark into `rect` (a square); `notch` is the color of the
/// surface behind it, which the cut-out notch shows.
pub(crate) fn paint_app_mark(ui: &egui::Ui, rect: egui::Rect, notch: egui::Color32) {
    let c = rect.center();
    let radius = rect.width() / 2.0;
    let painter = ui.painter();
    painter.circle_filled(c, radius, ui_kit::Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x, c.y - radius), egui::vec2(radius, radius)), 0.0, notch);
}

/// The start screen's menu bar: File with New project and Open project, the
/// same two actions (and shortcuts) the start screen has. Returns the action
/// the user chose (`New` or `Open`). Call it in the left zone of the main
/// toolbar, after the app mark.
pub fn start_menu_bar(ui: &mut egui::Ui) -> ProjectMenuAction {
    let action = std::cell::Cell::new(ProjectMenuAction::None);
    let ctx = ui.ctx().clone();
    let mut menus = [ui_kit::Menu::new("File", |ui| {
        if ui.add(item(&ctx, "New project", "new_project")).clicked() {
            action.set(ProjectMenuAction::New);
            ui.close();
        }
        if ui.add(item(&ctx, "Open project...", "open_project")).clicked() {
            action.set(ProjectMenuAction::Open);
            ui.close();
        }
    })];
    ui_kit::menu_bar(ui, &mut menus);
    action.get()
}

/// The title bar's menu bar: the Project, Edit, Markers, Process, View and
/// Help menus as `ui_kit::menu_bar` titles, with the same items, shortcuts,
/// enabled states and separators the menu button's popover had. Call it once
/// per frame in the left zone of the main toolbar, after the app mark; it runs
/// the open menu's closure inside, so `menu` and `editor` are borrowed only
/// while a menu is open. Returns the Project, View and Help actions the user
/// chose; the dialogs the items open are drawn by [`ProjectMenu::show_dialogs`]
/// once the toolbar is done.
pub fn menu_bar(
    ui: &mut egui::Ui,
    menu: &std::cell::RefCell<&mut ProjectMenu>,
    editor: &std::cell::RefCell<&mut Editor>,
) -> (ProjectMenuAction, ViewMenuAction, HelpMenuAction) {
    let action = std::cell::Cell::new(ProjectMenuAction::None);
    let view_action = std::cell::Cell::new(ViewMenuAction::None);
    let help_action = std::cell::Cell::new(HelpMenuAction::None);
    // While a take is recorded the Edit and Process menus, Import, Export,
    // Save As, both settings dialogs and every Markers item except Add
    // Marker are disabled; Save, New, Open, Close and View stay.
    let mut menus = [
        ui_kit::Menu::new("Project", |ui| {
            let editor = &mut **editor.borrow_mut();
            let recording = editor.is_recording();
            let chosen = menu.borrow_mut().project_menu(ui, editor, recording);
            if chosen != ProjectMenuAction::None {
                action.set(chosen);
            }
        }),
        ui_kit::Menu::new("Edit", |ui| {
            let editor = &mut **editor.borrow_mut();
            let recording = editor.is_recording();
            ProjectMenu::edit_menu(ui, editor, recording);
        }),
        ui_kit::Menu::new("Markers", |ui| {
            let editor = &mut **editor.borrow_mut();
            let recording = editor.is_recording();
            ProjectMenu::markers_menu(ui, editor, recording);
        }),
        ui_kit::Menu::new("Process", |ui| {
            let editor = &mut **editor.borrow_mut();
            let recording = editor.is_recording();
            menu.borrow_mut().process_menu(ui, editor, recording);
        }),
        ui_kit::Menu::new("View", |ui| {
            let chosen = menu.borrow_mut().view_menu(ui, &editor.borrow());
            if chosen != ViewMenuAction::None {
                view_action.set(chosen);
            }
        }),
        // Help topics (F1), Keyboard shortcuts (Ctrl+/), Show the tour, then
        // About (spec: S1 Help "Help menu").
        ui_kit::Menu::new("Help", |ui| {
            let ctx = ui.ctx().clone();
            if ui.add(item(&ctx, "Help topics", "help_topics")).clicked() {
                help_action.set(HelpMenuAction::Topics);
                ui.close();
            }
            if ui.add(item(&ctx, "Keyboard shortcuts", "keyboard_shortcuts")).clicked() {
                help_action.set(HelpMenuAction::Shortcuts);
                ui.close();
            }
            if ui.button("Show the tour").clicked() {
                help_action.set(HelpMenuAction::Tour);
                ui.close();
            }
            menu_sep(ui);
            if ui.button("About Cutmark Audio").clicked() {
                help_action.set(HelpMenuAction::About);
                ui.close();
            }
        }),
    ];
    ui_kit::menu_bar(ui, &mut menus);
    (action.get(), view_action.get(), help_action.get())
}

/// The Project, Edit and View menus plus their modals (save error, Save As).
#[derive(Default)]
pub struct ProjectMenu {
    /// Modal error dialog for a failed save, when shown.
    error_dialog: Option<ConfirmDialog>,
    /// Save As dialog, when open.
    save_as_dialog: Option<SaveAsDialog>,
    /// Project Settings dialog, when open.
    settings_dialog: Option<ProjectSettingsDialog>,
    /// Audio Settings dialog, when open.
    audio_settings_dialog: Option<AudioSettingsDialog>,
    /// Export dialog; kept for the whole session so the last choices are
    /// remembered between opens.
    export_dialog: Option<ExportDialog>,
    /// True while the Appearance dialog (accent and glass) is open.
    appearance_open: bool,
}

impl ProjectMenu {
    /// The Project menu's items; returns the action the user chose.
    fn project_menu(&mut self, menu: &mut egui::Ui, editor: &mut Editor, recording: bool) -> ProjectMenuAction {
        let ctx = menu.ctx().clone();
        let mut action = ProjectMenuAction::None;
        if menu.add(item(&ctx, "New", "new_project")).clicked() {
            action = ProjectMenuAction::New;
            menu.close();
        }
        if menu.add(item(&ctx, "Open...", "open_project")).clicked() {
            action = ProjectMenuAction::Open;
            menu.close();
        }
        if menu.add(item(&ctx, "Save", "save")).clicked() {
            self.save(editor);
            menu.close();
        }
        if menu.add_enabled(!recording, item(&ctx, "Save As...", "save_as")).clicked() {
            self.open_save_as(editor);
            menu.close();
        }
        if menu.add_enabled(!recording, item(&ctx, "Import Media...", "import_media")).clicked() {
            action = ProjectMenuAction::Import;
            menu.close();
        }
        if !editor.is_locked() && !recording {
            if menu.add(item(&ctx, "Export...", "export")).clicked() {
                self.open_export(editor);
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Export...", "export"));
        }
        if menu.button("Close").clicked() {
            action = ProjectMenuAction::Close;
            menu.close();
        }
        if menu.add_enabled(!recording, egui::Button::new("Project Settings...")).clicked() {
            self.open_settings(editor);
            menu.close();
        }
        if menu.add_enabled(!recording, egui::Button::new("Audio Settings...")).clicked() {
            self.audio_settings_dialog = Some(AudioSettingsDialog::new_open(editor));
            menu.close();
        }
        action
    }

    /// The Edit menu's items.
    fn edit_menu(menu: &mut egui::Ui, editor: &mut Editor, recording: bool) {
        let ctx = menu.ctx().clone();
        if recording {
            menu.disable();
        }
        match editor.undo_label() {
            Some(label) => {
                if menu.add(item(&ctx, format!("Undo {label}"), "undo")).clicked() {
                    let _ = editor.undo();
                    menu.close();
                }
            }
            None => {
                menu.add_enabled(false, item(&ctx, "Undo", "undo"));
            }
        }
        match editor.redo_label() {
            Some(label) => {
                if menu.add(item(&ctx, format!("Redo {label}"), "redo")).clicked() {
                    let _ = editor.redo();
                    menu.close();
                }
            }
            None => {
                menu.add_enabled(false, item(&ctx, "Redo", "redo"));
            }
        }
        menu_sep(menu);
        let selection = editor.selection().clone();
        let has_selection = !matches!(selection, crate::editor::selection::Selection::None);
        let can_paste = editor.clipboard().is_some_and(|c| c.tracks.iter().any(|t| !t.is_empty()));
        let can_split = {
            let pos = editor.view.playhead;
            let range_tracks: Option<Vec<Uuid>> = match &selection {
                crate::editor::selection::Selection::Range { tracks, .. } => Some(tracks.clone()),
                _ => None,
            };
            editor.project.tracks.iter().any(|track| {
                range_tracks.as_ref().is_none_or(|ids| ids.contains(&track.id)) && track.clips.iter().any(|clip| clip.start < pos && pos < clip.end())
            })
        };
        let can_trim = matches!(selection, crate::editor::selection::Selection::Range { .. });
        let can_select_all = !editor.project.tracks.is_empty() && editor.project_length() > 0;

        if has_selection {
            if menu.add(item(&ctx, "Cut", "cut")).clicked() {
                let _ = editor.cut_selection();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Cut", "cut"));
        }
        if has_selection {
            if menu.add(item(&ctx, "Copy", "copy")).clicked() {
                editor.copy_selection();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Copy", "copy"));
        }
        if can_paste {
            if menu.add(item(&ctx, "Paste", "paste")).clicked() {
                let _ = editor.paste();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Paste", "paste"));
        }
        if has_selection {
            if menu.add(item(&ctx, "Delete", "delete_selection")).clicked() {
                match &selection {
                    crate::editor::selection::Selection::Clips(ids) => {
                        let _ = editor.apply(crate::editor::action::Action::DeleteClips { clip_ids: ids.clone(), ripple: editor.view.ripple });
                    }
                    crate::editor::selection::Selection::Range { start, end, tracks } => {
                        let _ = editor.apply(crate::editor::action::Action::DeleteRange {
                            s: *start,
                            e: *end,
                            track_ids: tracks.clone(),
                            ripple: editor.view.ripple,
                        });
                        editor.clear_selection();
                        editor.set_playhead(*start);
                    }
                    crate::editor::selection::Selection::None => {}
                }
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Delete", "delete_selection"));
        }
        menu_sep(menu);
        if can_split {
            if menu.add(item(&ctx, "Split", "split_at_playhead")).clicked() {
                let _ = editor.split();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Split", "split_at_playhead"));
        }
        if can_trim {
            if menu.add(item(&ctx, "Trim to Selection", "trim_to_selection")).clicked() {
                let _ = editor.trim_to_selection();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Trim to Selection", "trim_to_selection"));
        }
        menu_sep(menu);
        if can_select_all {
            if menu.add(item(&ctx, "Select All", "select_all")).clicked() {
                editor.select_all();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Select All", "select_all"));
        }
        if has_selection {
            if menu.add(item(&ctx, "Clear Selection", "clear_selection")).clicked() {
                editor.clear_selection();
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Clear Selection", "clear_selection"));
        }
        menu_sep(menu);
        let mut ripple = editor.view.ripple;
        if menu.add(egui::Checkbox::new(&mut ripple, format!("Ripple ({})", crate::shortcuts::text(&ctx, "toggle_ripple")))).changed() {
            editor.toggle_ripple();
        }
        let mut snap = editor.view.snap;
        if menu.add(egui::Checkbox::new(&mut snap, format!("Snap ({})", crate::shortcuts::text(&ctx, "toggle_snap")))).changed() {
            editor.toggle_snap();
        }
    }

    /// The Markers menu's items.
    fn markers_menu(menu: &mut egui::Ui, editor: &mut Editor, recording: bool) {
        let ctx = menu.ctx().clone();
        if menu.add(item(&ctx, "Add Marker", "add_marker")).clicked() {
            let _ = editor.add_marker_at_playhead();
            menu.close();
        }
        if menu.add_enabled(!recording, item(&ctx, "Previous Marker", "previous_marker")).clicked() {
            editor.jump_prev_marker();
            menu.close();
        }
        if menu.add_enabled(!recording, item(&ctx, "Next Marker", "next_marker")).clicked() {
            editor.jump_next_marker();
            menu.close();
        }
        if let Some(id) = editor.selected_marker().filter(|_| !recording) {
            if menu.add(item(&ctx, "Delete Marker", "delete_selection")).clicked() {
                let _ = editor.apply(crate::editor::action::Action::DeleteMarkers { ids: vec![id] });
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Delete Marker", "delete_selection"));
        }
    }

    /// The Process menu's items.
    fn process_menu(&mut self, menu: &mut egui::Ui, editor: &mut Editor, recording: bool) {
        let ctx = menu.ctx().clone();
        if recording {
            menu.disable();
        }
        let has_target = editor.process_target().is_some();
        let has_range = matches!(editor.selection(), crate::editor::selection::Selection::Range { .. });
        if has_target {
            if menu.add(item(&ctx, "Gain...", "process_gain")).clicked() {
                crate::ui::dialogs::gain::request_process(menu.ctx());
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Gain...", "process_gain"));
        }
        if has_target {
            if menu.add(item(&ctx, "Normalize...", "process_normalize")).clicked() {
                crate::ui::dialogs::normalize::request(menu.ctx());
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Normalize...", "process_normalize"));
        }
        if has_target {
            if menu.add(item(&ctx, "Fade In", "fade_in")).clicked() {
                if let Err(error) = editor.fade_in() {
                    self.show_process_error(error);
                }
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Fade In", "fade_in"));
        }
        if has_target {
            if menu.add(item(&ctx, "Fade Out", "fade_out")).clicked() {
                if let Err(error) = editor.fade_out() {
                    self.show_process_error(error);
                }
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Fade Out", "fade_out"));
        }
        if has_range {
            if menu.add(item(&ctx, "Silence", "silence")).clicked() {
                if let Err(error) = editor.silence() {
                    self.show_process_error(error);
                }
                menu.close();
            }
        } else {
            menu.add_enabled(false, item(&ctx, "Silence", "silence"));
        }
    }

    /// The View menu's items; returns the zoom command the user chose.
    fn view_menu(&mut self, menu: &mut egui::Ui, editor: &Editor) -> ViewMenuAction {
        let ctx = menu.ctx().clone();
        let mut view_action = ViewMenuAction::None;
        if menu.add(item(&ctx, "Zoom In", "zoom_in")).clicked() {
            view_action = ViewMenuAction::ZoomIn;
            menu.close();
        }
        if menu.add(item(&ctx, "Zoom Out", "zoom_out")).clicked() {
            view_action = ViewMenuAction::ZoomOut;
            menu.close();
        }
        if !matches!(editor.selection(), crate::editor::selection::Selection::Range { .. }) {
            menu.add_enabled(false, item(&ctx, "Zoom to Selection", "zoom_to_selection"));
        } else if menu.add(item(&ctx, "Zoom to Selection", "zoom_to_selection")).clicked() {
            view_action = ViewMenuAction::ZoomToSelection;
            menu.close();
        }
        if menu.add(item(&ctx, "Zoom to Fit", "zoom_to_fit")).clicked() {
            view_action = ViewMenuAction::ZoomToFit;
            menu.close();
        }
        menu_sep(menu);
        if menu.add(egui::Button::new("Appearance...")).clicked() {
            self.appearance_open = true;
            menu.close();
        }
        view_action
    }

    /// Draws the dialogs the menu items opened (Save As, both settings
    /// dialogs, Export, the save error). Call once per frame, after the main
    /// toolbar.
    pub fn show_dialogs(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if let Some(dialog) = &mut self.save_as_dialog {
            dialog.show(ctx, editor);
            if dialog.is_closed() {
                self.save_as_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.settings_dialog {
            dialog.show(ctx, editor);
            if dialog.is_closed() {
                self.settings_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.audio_settings_dialog {
            dialog.show(ctx, editor);
            if dialog.is_closed() {
                self.audio_settings_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.export_dialog {
            dialog.show(ctx, editor);
        }
        if let Some(dialog) = &mut self.error_dialog {
            dialog.show(ctx);
        }
        if self.appearance_open {
            ui_kit::appearance_dialog(ctx, &mut self.appearance_open);
        }
    }

    /// True while one of the menu's dialogs (save error, Save As, Project or
    /// Audio Settings) is up. The app keeps Esc and Space away from a running
    /// take then, so closing a dialog never discards a recording.
    pub fn modal_open(&self) -> bool {
        self.error_dialog.as_ref().is_some_and(|dialog| dialog.open)
            || self.save_as_dialog.is_some()
            || self.settings_dialog.is_some()
            || self.audio_settings_dialog.is_some()
            || self.appearance_open
    }

    /// Shows a failed Process menu operation in the error modal, with the
    /// ops prefix stripped so the SPEC's message shows verbatim.
    fn show_process_error(&mut self, error: crate::editor::editor::EditorError) {
        let text = error.to_string();
        self.error_dialog = Some(ConfirmDialog::new("Process", text.strip_prefix("invalid operation: ").unwrap_or(&text).to_string()));
    }

    /// Saves the project; on failure shows the error modal.
    pub fn save(&mut self, editor: &mut Editor) {
        if let Err(error) = editor.save() {
            self.error_dialog = Some(ConfirmDialog::new("Save Error", error.to_string()));
        }
    }

    /// Opens the Save As dialog prefilled with the current name and folder.
    pub fn open_save_as(&mut self, editor: &mut Editor) {
        self.save_as_dialog = Some(SaveAsDialog::new_open(&editor.project.name, &editor.paths.root));
    }

    /// Opens the Project Settings dialog prefilled with the current settings.
    pub fn open_settings(&mut self, editor: &mut Editor) {
        self.settings_dialog = Some(ProjectSettingsDialog::new_open(&editor.project.name, editor.project.export_bit_depth));
    }

    /// Opens the Export dialog; created once per session, afterwards it
    /// reopens with the last choices.
    pub fn open_export(&mut self, editor: &Editor) {
        if self.export_dialog.is_none() {
            self.export_dialog = Some(ExportDialog::new_open(editor));
        }
        if let Some(dialog) = &mut self.export_dialog {
            dialog.reopen();
        }
    }

    /// True while the Export dialog exists. It is kept for the whole session
    /// (so the last choices are remembered), so this stays true after the
    /// first open. Test hook for the toolbar's Export button.
    #[cfg(test)]
    pub fn export_dialog_open(&self) -> bool {
        self.export_dialog.is_some()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::project::{BitDepth, Project, ViewState};
    use crate::ui::test_util::{Headless, Out};
    use ui_kit::tokens::color;

    fn editor(name: &str, dirty: bool) -> Editor {
        let mut editor =
            Editor::with_project(Project::new(name.to_string(), 48_000, BitDepth::Int24), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") });
        editor.dirty = dirty;
        editor
    }

    /// The menu bar in a 44px bar plus the dialogs, driven frame by frame the
    /// way the app draws them: the bar in the panel, the dialogs after it,
    /// once per frame.
    struct MenuRig<'a> {
        /// The menu state with its dialogs.
        menu: ProjectMenu,
        /// The edited project.
        editor: &'a mut Editor,
        /// The Project actions chosen so far.
        actions: Vec<ProjectMenuAction>,
    }

    impl<'a> MenuRig<'a> {
        fn new(editor: &'a mut Editor) -> Self {
            Self { menu: ProjectMenu::default(), editor, actions: Vec::new() }
        }

        /// One frame: the bar with the menu bar, then the dialogs.
        fn ui(&mut self, ui: &mut egui::Ui) {
            let Self { menu, editor, actions } = self;
            egui::Panel::top("menu_bar").exact_size(44.0).frame(egui::Frame::new().fill(color::BG_GROUND)).show(ui, |ui| {
                let menu_cell = std::cell::RefCell::new(&mut *menu);
                let editor_cell = std::cell::RefCell::new(&mut **editor);
                ui.horizontal(|ui| {
                    let (action, _, _) = menu_bar(ui, &menu_cell, &editor_cell);
                    if action != ProjectMenuAction::None {
                        actions.push(action);
                    }
                });
            });
            let ctx = ui.ctx().clone();
            menu.show_dialogs(&ctx, editor);
        }

        fn frame(&mut self, h: &mut Headless) -> Out {
            h.frame(Vec::new(), |ui| self.ui(ui))
        }

        /// Clicks at `at` and returns the settled frame after the click.
        fn click(&mut self, h: &mut Headless, at: egui::Pos2) -> Out {
            let _ = h.click(at, |ui| self.ui(ui));
            h.settle(|ui| self.ui(ui))
        }

        /// Moves the pointer to `at` and returns the settled frame.
        fn hover(&mut self, h: &mut Headless, at: egui::Pos2) -> Out {
            let _ = h.frame(vec![egui::Event::PointerMoved(at)], |ui| self.ui(ui));
            h.settle(|ui| self.ui(ui))
        }
    }

    /// Every top-level menu with its items and shortcut hints, as the shortcut
    /// table's text reads them (a hint of "" is an item without a shortcut).
    const MENUS: &[(&str, &[(&str, &str)])] = &[
        (
            "Project",
            &[
                ("New", "Ctrl+N"),
                ("Open...", "Ctrl+O"),
                ("Save", "Ctrl+S"),
                ("Save As...", "Ctrl+Shift+S"),
                ("Import Media...", "Ctrl+I"),
                ("Export...", "Ctrl+E"),
                ("Close", ""),
                ("Project Settings...", ""),
                ("Audio Settings...", ""),
            ],
        ),
        (
            "Edit",
            &[
                ("Undo", "Ctrl+Z"),
                ("Redo", "Ctrl+Y"),
                ("Cut", "Ctrl+X"),
                ("Copy", "Ctrl+C"),
                ("Paste", "Ctrl+V"),
                ("Delete", "Delete"),
                ("Split", "S"),
                ("Trim to Selection", "Ctrl+T"),
                ("Select All", "Ctrl+A"),
                ("Clear Selection", "Escape"),
                ("Ripple (R)", ""),
                ("Snap (N)", ""),
            ],
        ),
        ("Markers", &[("Add Marker", "M"), ("Previous Marker", "Comma"), ("Next Marker", "Period"), ("Delete Marker", "Delete")]),
        (
            "Process",
            &[("Gain...", "Ctrl+G"), ("Normalize...", "Ctrl+Shift+N"), ("Fade In", "Ctrl+Shift+I"), ("Fade Out", "Ctrl+Shift+O"), ("Silence", "Ctrl+L")],
        ),
        ("View", &[("Zoom In", "Plus"), ("Zoom Out", "Minus"), ("Zoom to Selection", "Ctrl+Backslash"), ("Zoom to Fit", "Backslash"), ("Appearance...", "")]),
        ("Help", &[("Help topics", "F1"), ("Keyboard shortcuts", "Ctrl+Slash"), ("Show the tour", ""), ("About Cutmark Audio", "")]),
    ];

    #[test]
    fn the_menu_bar_has_every_top_level_menu_and_no_menu_button() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut rig = MenuRig::new(&mut editor);
        let out = rig.frame(&mut h);
        let mut last_x = f32::NEG_INFINITY;
        for (title, _) in MENUS {
            let rect = out.text_rect(title).unwrap_or_else(|| panic!("no {title} title in the bar"));
            assert!(rect.left() > last_x, "{title} follows the previous title, left to right");
            last_x = rect.left();
        }
        assert!(out.text_rect(ui_kit::icons::regular::LIST).is_none(), "the menu (hamburger) button is gone");
    }

    #[test]
    fn every_menu_keeps_its_items_and_shortcuts() {
        for (title, items) in MENUS {
            let mut h = Headless::new(1280.0, 800.0);
            let mut editor = editor("Test", false);
            let mut rig = MenuRig::new(&mut editor);
            let out = rig.frame(&mut h);
            let at = out.text_rect(title).unwrap_or_else(|| panic!("no {title} title")).center();
            let out = rig.click(&mut h, at);
            for (label, hint) in *items {
                assert!(out.text_rect(label).is_some(), "{title} menu lost its {label} item");
                if !hint.is_empty() {
                    assert!(out.text_rect(hint).is_some(), "{title} > {label} lost its {hint} shortcut hint");
                }
            }
        }
    }

    #[test]
    fn a_click_opens_a_menu_hovering_another_title_switches_and_escape_closes() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut rig = MenuRig::new(&mut editor);
        let out = rig.frame(&mut h);
        let project = out.text_rect("Project").expect("Project title").center();
        let edit = out.text_rect("Edit").expect("Edit title").center();
        assert!(out.text_rect("Save").is_none(), "closed at rest");

        let out = rig.click(&mut h, project);
        assert!(out.text_rect("Save").is_some(), "a click opens Project");
        let out = rig.hover(&mut h, edit);
        assert!(out.text_rect("Cut").is_some(), "hovering Edit while a menu is open switches to Edit");
        assert!(out.text_rect("Save").is_none(), "Project closed");

        let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let _ = h.frame(vec![escape], |ui| rig.ui(ui));
        let out = h.settle(|ui| rig.ui(ui));
        assert!(out.text_rect("Cut").is_none(), "Esc closes the open menu");
    }

    #[test]
    fn audio_settings_opens_from_the_project_menu_and_cancel_closes_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut rig = MenuRig::new(&mut editor);
        let out = rig.frame(&mut h);
        assert!(out.text_rect("Audio Settings").is_none());

        // Project > Audio Settings...
        let project = out.text_rect("Project").expect("Project menu").center();
        let out = rig.click(&mut h, project);
        let item = out.text_rect("Audio Settings...").expect("the menu item").center();
        assert!(rig.menu.audio_settings_dialog.is_none());
        let out = rig.click(&mut h, item);
        assert!(rig.menu.audio_settings_dialog.is_some(), "the dialog opened");
        assert!(out.text_rect("Audio Settings").is_some(), "its title");

        // Cancel closes it and leaves the config alone.
        let cancel = out.text_rect("Cancel").expect("Cancel").center();
        let out = rig.click(&mut h, cancel);
        assert!(rig.menu.audio_settings_dialog.is_none(), "the dialog closed");
        assert_eq!(rig.editor.config(), &crate::io::config::AppConfig::default());
        assert!(out.text_rect("Audio Settings").is_none());
    }

    #[test]
    fn view_appearance_opens_the_ui_kit_dialog_and_done_closes_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut rig = MenuRig::new(&mut editor);
        let out = rig.frame(&mut h);
        assert!(!rig.menu.modal_open());
        let view = out.text_rect("View").expect("View menu").center();
        let out = rig.click(&mut h, view);
        let item = out.text_rect("Appearance...").expect("the menu item").center();
        let out = rig.click(&mut h, item);
        assert!(rig.menu.appearance_open && rig.menu.modal_open(), "the dialog opened");
        assert!(out.text_rect("Appearance").is_some(), "its title");

        let done = out.text_rect("Done").expect("Done").center();
        let _ = rig.click(&mut h, done);
        assert!(!rig.menu.appearance_open, "Done closed the dialog");
    }

    // ---- While a take is recorded ----

    /// On a fresh bar: opens the `title` menu and clicks `item_text`. Returns
    /// the menu (to look at its dialogs) and the Project actions chosen.
    fn choose(editor: &mut Editor, title: &str, item_text: &str) -> (ProjectMenu, Vec<ProjectMenuAction>) {
        let mut h = Headless::new(1280.0, 800.0);
        let mut rig = MenuRig::new(editor);
        let out = rig.frame(&mut h);
        let at = out.text_rect(title).unwrap_or_else(|| panic!("no {title} menu")).center();
        let out = rig.click(&mut h, at);
        let at = out.text_rect(item_text).unwrap_or_else(|| panic!("no {item_text} item in {title}")).center();
        let _ = rig.click(&mut h, at);
        (rig.menu, rig.actions)
    }

    /// A project with a clip and a range selected; recording when `recording`.
    fn take_editor(recording: bool) -> (Editor, tempfile::TempDir) {
        let (mut editor, dir) = crate::ui::test_util::armed_editor(&[&[(0, 48_000)], &[]], "Rec");
        editor.select_all();
        if recording {
            editor.start_recording().expect("a take");
        }
        (editor, dir)
    }

    #[test]
    fn the_project_menu_keeps_save_new_open_and_close_but_not_the_dialogs() {
        let (mut editor, _dir) = take_editor(true);
        let (menu, actions) = choose(&mut editor, "Project", "Audio Settings...");
        assert!(menu.audio_settings_dialog.is_none(), "Audio Settings");
        let (menu, _) = choose(&mut editor, "Project", "Project Settings...");
        assert!(menu.settings_dialog.is_none(), "Project Settings");
        let (menu, _) = choose(&mut editor, "Project", "Save As...");
        assert!(menu.save_as_dialog.is_none(), "Save As");
        let (menu, _) = choose(&mut editor, "Project", "Export...");
        assert!(menu.export_dialog.is_none(), "Export");
        let (_, import) = choose(&mut editor, "Project", "Import Media...");
        assert!(import.is_empty() && actions.is_empty(), "Import");

        // Save stays enabled and works; New and Close still ask the app.
        let (menu, _) = choose(&mut editor, "Project", "Save");
        assert!(menu.error_dialog.is_none(), "Save worked");
        let (_, new) = choose(&mut editor, "Project", "New");
        assert_eq!(new, [ProjectMenuAction::New]);
        let (_, close) = choose(&mut editor, "Project", "Close");
        assert_eq!(close, [ProjectMenuAction::Close]);
        assert!(editor.take_is_live(), "the take was never disturbed");
    }

    #[test]
    fn the_same_menu_items_work_when_nothing_is_recording() {
        // Control for the test above and the two below: the clicks land.
        let (mut editor, _dir) = take_editor(false);
        let (menu, _) = choose(&mut editor, "Project", "Audio Settings...");
        assert!(menu.audio_settings_dialog.is_some());
        let (menu, _) = choose(&mut editor, "Project", "Save As...");
        assert!(menu.save_as_dialog.is_some());
        let (_, import) = choose(&mut editor, "Project", "Import Media...");
        assert_eq!(import, [ProjectMenuAction::Import]);
        let _ = choose(&mut editor, "Edit", "Clear Selection");
        assert!(matches!(editor.selection(), crate::editor::selection::Selection::None));
        editor.select_all();
        let _ = choose(&mut editor, "Edit", "Ripple (R)");
        assert!(editor.view.ripple, "Ripple toggled");
        let (menu, _) = choose(&mut editor, "Process", "Silence");
        assert!(menu.error_dialog.is_none());
        assert_eq!(editor.undo_label(), Some("Silence"), "Silence ran");
    }

    #[test]
    fn the_edit_and_process_menus_are_off_while_recording() {
        let (mut editor, _dir) = take_editor(true);
        let ripple = editor.view.ripple;
        let _ = choose(&mut editor, "Edit", "Clear Selection");
        assert!(!matches!(editor.selection(), crate::editor::selection::Selection::None), "Clear Selection did nothing");
        let _ = choose(&mut editor, "Edit", "Ripple (R)");
        assert_eq!(editor.view.ripple, ripple, "Ripple did nothing");
        let _ = choose(&mut editor, "Edit", "Select All");
        let (menu, _) = choose(&mut editor, "Process", "Silence");
        assert!(menu.error_dialog.is_none(), "Silence did nothing");
        assert!(editor.take_is_live());
        assert_eq!(editor.undo_label(), None, "no edit was made");
    }

    #[test]
    fn only_add_marker_works_in_the_markers_menu_while_recording() {
        let (mut editor, _dir) = take_editor(true);
        editor.apply(crate::editor::action::Action::AddMarker { pos: 100 }).expect("a marker is allowed while recording");
        let id = editor.project.markers[0].id;
        editor.select_marker(Some(id));
        editor.view.playhead = 5_000;

        let _ = choose(&mut editor, "Markers", "Previous Marker");
        let _ = choose(&mut editor, "Markers", "Next Marker");
        assert_eq!(editor.view.playhead, 5_000, "no jump");
        let _ = choose(&mut editor, "Markers", "Delete Marker");
        assert_eq!(editor.project.markers.len(), 1, "not deleted");

        let _ = choose(&mut editor, "Markers", "Add Marker");
        assert_eq!(editor.project.markers.len(), 2, "Add Marker works");
    }
}
