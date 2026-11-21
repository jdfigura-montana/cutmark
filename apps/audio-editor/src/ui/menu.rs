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
use crate::ui::theme::{THEME, menu_item_style};
use crate::ui::timeline::ViewMenuAction;
use ui_kit::fonts;
use ui_kit::tokens::{color, space, typography};

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

/// A menu separator: 1 px line in the border color with 4 px vertical margin.
fn menu_sep(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), THEME.hairline), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, THEME.control_border);
    ui.add_space(4.0);
}

/// A menu item button with a right-aligned type.value shortcut hint.
fn item(label: impl Into<String>, hint: &str) -> egui::Button<'static> {
    egui::Button::new(label.into()).right_text(egui::RichText::new(hint).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_TERTIARY))
}

/// Diameter of the unsaved dot that follows the project name.
const DOT_D: f32 = space::SCALE_8;
/// Widest the project name is laid out; a longer name truncates with an
/// ellipsis instead of pushing the toolbar's buttons into the caption
/// buttons.
const NAME_MAX_W: f32 = 240.0;

/// The toolbar's document title: the app mark, the project name in
/// type.headline with an 8 px `text.secondary` dot while there are unsaved
/// changes, and the project format caption below it.
pub fn project_title(ui: &mut egui::Ui, name: &str, dirty: bool, caption: &str) {
    app_mark(ui);
    ui.add_space(space::SCALE_4);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = space::SCALE_2;
        // The name and its dot share one measured rect, so the dot always
        // follows the name's last glyph.
        let painter = ui.painter().clone();
        let mut job = egui::text::LayoutJob::simple_singleline(name.to_string(), fonts::semibold(typography::HEADLINE_SIZE), color::TEXT_PRIMARY);
        job.wrap = egui::text::TextWrapping::truncate_at_width(NAME_MAX_W);
        let galley = painter.layout_job(job);
        let dot = u8::from(dirty) as f32 * DOT_D;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + dot, galley.size().y), egui::Sense::hover());
        let mid = rect.center().y;
        painter.galley(egui::pos2(rect.left(), mid - galley.size().y * 0.5), galley, color::TEXT_PRIMARY);
        if dirty {
            painter.circle_filled(egui::pos2(rect.right() - DOT_D * 0.5, mid), DOT_D * 0.5, color::TEXT_SECONDARY);
        }
        ui.label(egui::RichText::new(caption).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    });
}

/// The start screen's toolbar title: the app mark and the app name in
/// type.headline. No project is open, so there is no unsaved dot and no
/// format caption.
pub fn start_title(ui: &mut egui::Ui, name: &str) {
    app_mark(ui);
    ui.add_space(space::SCALE_4);
    ui.label(egui::RichText::new(name).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
}

/// Draws the app mark placeholder: an accent circle with a notch cut from its
/// top right (spec "Open questions" item 1: placeholder until a real logo).
pub(crate) fn app_mark(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
    let c = rect.center();
    let painter = ui.painter();
    painter.circle_filled(c, 9.0, ui_kit::Theme::get(ui.ctx()).accent.accent);
    painter.rect_filled(egui::Rect::from_min_size(egui::pos2(c.x, c.y - 9.0), egui::vec2(9.0, 9.0)), 0.0, color::BG_GROUND);
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
}

impl ProjectMenu {
    /// The toolbar's menu icon button; pass its response to
    /// [`ProjectMenu::popover`].
    pub fn button(ui: &mut egui::Ui) -> egui::Response {
        ui_kit::icon_button(ui, ui_kit::icons::regular::LIST, "Menu")
    }

    /// The menu popover: every top-level menu (Project, Edit, Markers,
    /// Process, View) as a submenu of one glass popup anchored to `button`,
    /// with the same items, shortcuts and enabled states the menu bar had.
    /// Shows the dialogs the items open; returns the Project and View
    /// actions the user chose.
    pub fn popover(&mut self, ui: &mut egui::Ui, button: &egui::Response, editor: &mut Editor) -> (ProjectMenuAction, ViewMenuAction) {
        let mut action = ProjectMenuAction::None;
        let mut view_action = ViewMenuAction::None;
        // While a take is recorded the Edit and Process menus, Import, Export,
        // Save As, both settings dialogs and every Markers item except Add
        // Marker are disabled; Save, New, Open, Close and View stay.
        let recording = editor.is_recording();
        egui::Popup::menu(button).style(menu_item_style as fn(&mut egui::Style)).show(|ui| {
            ui.menu_button("Project", |menu| {
                if menu.add(item("New", "Ctrl+N")).clicked() {
                    action = ProjectMenuAction::New;
                    menu.close();
                }
                if menu.add(item("Open...", "Ctrl+O")).clicked() {
                    action = ProjectMenuAction::Open;
                    menu.close();
                }
                if menu.add(item("Save", "Ctrl+S")).clicked() {
                    self.save(editor);
                    menu.close();
                }
                if menu.add_enabled(!recording, item("Save As...", "Ctrl+Shift+S")).clicked() {
                    self.open_save_as(editor);
                    menu.close();
                }
                if menu.add_enabled(!recording, item("Import Media...", "Ctrl+I")).clicked() {
                    action = ProjectMenuAction::Import;
                    menu.close();
                }
                if !editor.is_locked() && !recording {
                    if menu.add(item("Export...", "Ctrl+E")).clicked() {
                        self.open_export(editor);
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Export...", "Ctrl+E"));
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
            });
            ui.menu_button("Edit", |menu| {
                if recording {
                    menu.disable();
                }
                match editor.undo_label() {
                    Some(label) => {
                        if menu.add(item(format!("Undo {label}"), "Ctrl+Z")).clicked() {
                            let _ = editor.undo();
                            menu.close();
                        }
                    }
                    None => {
                        menu.add_enabled(false, item("Undo", "Ctrl+Z"));
                    }
                }
                match editor.redo_label() {
                    Some(label) => {
                        if menu.add(item(format!("Redo {label}"), "Ctrl+Y")).clicked() {
                            let _ = editor.redo();
                            menu.close();
                        }
                    }
                    None => {
                        menu.add_enabled(false, item("Redo", "Ctrl+Y"));
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
                    if menu.add(item("Cut", "Ctrl+X")).clicked() {
                        let _ = editor.cut_selection();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Cut", "Ctrl+X"));
                }
                if has_selection {
                    if menu.add(item("Copy", "Ctrl+C")).clicked() {
                        editor.copy_selection();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Copy", "Ctrl+C"));
                }
                if can_paste {
                    if menu.add(item("Paste", "Ctrl+V")).clicked() {
                        let _ = editor.paste();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Paste", "Ctrl+V"));
                }
                if has_selection {
                    if menu.add(item("Delete", "Del")).clicked() {
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
                    menu.add_enabled(false, item("Delete", "Del"));
                }
                menu_sep(menu);
                if can_split {
                    if menu.add(item("Split", "S")).clicked() {
                        let _ = editor.split();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Split", "S"));
                }
                if can_trim {
                    if menu.add(item("Trim to Selection", "Ctrl+T")).clicked() {
                        let _ = editor.trim_to_selection();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Trim to Selection", "Ctrl+T"));
                }
                menu_sep(menu);
                if can_select_all {
                    if menu.add(item("Select All", "Ctrl+A")).clicked() {
                        editor.select_all();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Select All", "Ctrl+A"));
                }
                if has_selection {
                    if menu.add(item("Clear Selection", "Esc")).clicked() {
                        editor.clear_selection();
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Clear Selection", "Esc"));
                }
                menu_sep(menu);
                let mut ripple = editor.view.ripple;
                if menu.add(egui::Checkbox::new(&mut ripple, "Ripple (R)")).changed() {
                    editor.toggle_ripple();
                }
                let mut snap = editor.view.snap;
                if menu.add(egui::Checkbox::new(&mut snap, "Snap (N)")).changed() {
                    editor.toggle_snap();
                }
            });
            ui.menu_button("Markers", |menu| {
                if menu.add(item("Add Marker", "M")).clicked() {
                    let _ = editor.add_marker_at_playhead();
                    menu.close();
                }
                if menu.add_enabled(!recording, item("Previous Marker", ",")).clicked() {
                    editor.jump_prev_marker();
                    menu.close();
                }
                if menu.add_enabled(!recording, item("Next Marker", ".")).clicked() {
                    editor.jump_next_marker();
                    menu.close();
                }
                if let Some(id) = editor.selected_marker().filter(|_| !recording) {
                    if menu.add(item("Delete Marker", "Del")).clicked() {
                        let _ = editor.apply(crate::editor::action::Action::DeleteMarkers { ids: vec![id] });
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Delete Marker", "Del"));
                }
            });
            ui.menu_button("Process", |menu| {
                if recording {
                    menu.disable();
                }
                let has_target = editor.process_target().is_some();
                let has_range = matches!(editor.selection(), crate::editor::selection::Selection::Range { .. });
                if has_target {
                    if menu.add(item("Gain...", "Ctrl+G")).clicked() {
                        crate::ui::dialogs::gain::request_process(menu.ctx());
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Gain...", "Ctrl+G"));
                }
                if has_target {
                    if menu.add(item("Normalize...", "Ctrl+Shift+N")).clicked() {
                        crate::ui::dialogs::normalize::request(menu.ctx());
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Normalize...", "Ctrl+Shift+N"));
                }
                if has_target {
                    if menu.add(item("Fade In", "Ctrl+Shift+I")).clicked() {
                        if let Err(error) = editor.fade_in() {
                            self.show_process_error(error);
                        }
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Fade In", "Ctrl+Shift+I"));
                }
                if has_target {
                    if menu.add(item("Fade Out", "Ctrl+Shift+O")).clicked() {
                        if let Err(error) = editor.fade_out() {
                            self.show_process_error(error);
                        }
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Fade Out", "Ctrl+Shift+O"));
                }
                if has_range {
                    if menu.add(item("Silence", "Ctrl+L")).clicked() {
                        if let Err(error) = editor.silence() {
                            self.show_process_error(error);
                        }
                        menu.close();
                    }
                } else {
                    menu.add_enabled(false, item("Silence", "Ctrl+L"));
                }
            });
            ui.menu_button("View", |menu| {
                if menu.add(item("Zoom In", "+")).clicked() {
                    view_action = ViewMenuAction::ZoomIn;
                    menu.close();
                }
                if menu.add(item("Zoom Out", "-")).clicked() {
                    view_action = ViewMenuAction::ZoomOut;
                    menu.close();
                }
                if !matches!(editor.selection(), crate::editor::selection::Selection::Range { .. }) {
                    menu.add_enabled(false, item("Zoom to Selection", "Ctrl+\\"));
                } else if menu.add(item("Zoom to Selection", "Ctrl+\\")).clicked() {
                    view_action = ViewMenuAction::ZoomToSelection;
                    menu.close();
                }
                if menu.add(item("Zoom to Fit", "\\")).clicked() {
                    view_action = ViewMenuAction::ZoomToFit;
                    menu.close();
                }
            });
        });
        let ctx = ui.ctx().clone();
        if let Some(dialog) = &mut self.save_as_dialog {
            dialog.show(&ctx, editor);
            if dialog.is_closed() {
                self.save_as_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.settings_dialog {
            dialog.show(&ctx, editor);
            if dialog.is_closed() {
                self.settings_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.audio_settings_dialog {
            dialog.show(&ctx, editor);
            if dialog.is_closed() {
                self.audio_settings_dialog = None;
            }
        }
        if let Some(dialog) = &mut self.export_dialog {
            dialog.show(&ctx, editor);
        }
        if let Some(dialog) = &mut self.error_dialog {
            dialog.show(&ctx);
        }
        (action, view_action)
    }

    /// True while one of the menu's dialogs (save error, Save As, Project or
    /// Audio Settings) is up. The app keeps Esc and Space away from a running
    /// take then, so closing a dialog never discards a recording.
    pub fn modal_open(&self) -> bool {
        self.error_dialog.as_ref().is_some_and(|dialog| dialog.open)
            || self.save_as_dialog.is_some()
            || self.settings_dialog.is_some()
            || self.audio_settings_dialog.is_some()
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

    /// Slack for float rounding when comparing painted rectangles.
    const EPS: f32 = 0.01;

    fn editor(name: &str, dirty: bool) -> Editor {
        let mut editor =
            Editor::with_project(Project::new(name.to_string(), 48_000, BitDepth::Int24), ViewState::default(), ProjectPaths { root: PathBuf::from("unused") });
        editor.dirty = dirty;
        editor
    }

    /// The menu button in its 52px bar plus the popover, driven frame by
    /// frame the way the app draws them: the button in the panel, the popover
    /// after it, once per frame.
    struct MenuRig<'a> {
        /// The menu state with its dialogs.
        menu: ProjectMenu,
        /// The edited project.
        editor: &'a mut Editor,
        /// The menu button's response, redrawn every frame.
        button: Option<egui::Response>,
        /// The Project actions chosen so far.
        actions: Vec<ProjectMenuAction>,
    }

    impl<'a> MenuRig<'a> {
        fn new(editor: &'a mut Editor) -> Self {
            Self { menu: ProjectMenu::default(), editor, button: None, actions: Vec::new() }
        }

        /// One frame: the bar with the menu button, then the popover.
        fn ui(&mut self, ui: &mut egui::Ui) {
            egui::Panel::top("menu_bar").exact_size(52.0).frame(egui::Frame::new().fill(THEME.bar_bg)).show(ui, |ui| {
                self.button = Some(ProjectMenu::button(ui));
            });
            if let Some(button) = self.button.take() {
                let (action, _) = self.menu.popover(ui, &button, self.editor);
                if action != ProjectMenuAction::None {
                    self.actions.push(action);
                }
            }
        }

        fn frame(&mut self, h: &mut Headless) -> Out {
            h.frame(Vec::new(), |ui| self.ui(ui))
        }

        /// Clicks at `at` and returns the settled frame after the click.
        fn click(&mut self, h: &mut Headless, at: egui::Pos2) -> Out {
            let _ = h.click(at, |ui| self.ui(ui));
            self.frame(h)
        }
    }

    #[test]
    fn audio_settings_opens_from_the_project_menu_and_cancel_closes_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut rig = MenuRig::new(&mut editor);
        let out = rig.frame(&mut h);
        assert!(out.text_rect("Audio Settings").is_none());

        // Menu button > Project > Audio Settings...
        let icon = out.text_rect(ui_kit::icons::regular::LIST).expect("the menu button").center();
        let out = rig.click(&mut h, icon);
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
    fn the_project_title_shows_the_name_dot_and_caption() {
        for width in [1280.0_f32, 520.0] {
            let mut h = Headless::new(width, 400.0);
            let caption = "48 kHz / 24-bit";
            let _ = h.frame(Vec::new(), |ui| project_title(ui, "Test 2", true, caption));
            let out = h.frame(Vec::new(), |ui| project_title(ui, "Test 2", true, caption));

            let dots: Vec<_> = out.circles().into_iter().filter(|c| c.fill == ui_kit::tokens::color::TEXT_SECONDARY).collect();
            assert_eq!(dots.len(), 1, "one unsaved dot at width {width}");
            let dot = dots[0];
            assert_eq!(dot.radius * 2.0, ui_kit::tokens::space::SCALE_8, "8 px dot");

            let name = out.text_rect("Test 2").expect("project name");
            assert!(name.right() <= dot.center.x - dot.radius + EPS, "the dot follows the name");
            assert!(out.text_rect(caption).is_some(), "the format caption");
            assert!(out.text_rect("unsaved").is_none(), "no literal unsaved text");
            assert!(out.texts().iter().all(|t| !t.text.contains('\u{25CF}')), "the dot is painted, never a glyph");
        }
    }

    #[test]
    fn a_clean_project_title_shows_only_name_and_caption() {
        let mut h = Headless::new(1280.0, 400.0);
        let _ = h.frame(Vec::new(), |ui| project_title(ui, "Test 2", false, "48 kHz / 24-bit"));
        let out = h.frame(Vec::new(), |ui| project_title(ui, "Test 2", false, "48 kHz / 24-bit"));
        assert!(out.text_rect("Test 2").is_some());
        assert!(out.text_rect("48 kHz / 24-bit").is_some());
        assert!(out.circles().iter().all(|c| c.fill != ui_kit::tokens::color::TEXT_SECONDARY), "no dot while clean");
    }

    #[test]
    fn a_long_name_is_truncated_not_clipped() {
        let mut h = Headless::new(520.0, 400.0);
        let long = "A very long project name that cannot fit the toolbar";
        let _ = h.frame(Vec::new(), |ui| project_title(ui, long, true, "48 kHz / 24-bit"));
        let out = h.frame(Vec::new(), |ui| project_title(ui, long, true, "48 kHz / 24-bit"));
        let name = out.text_rect(long).expect("project name");
        let full_w = h.ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(long.to_string(), ui_kit::fonts::semibold(ui_kit::tokens::typography::HEADLINE_SIZE), ui_kit::tokens::color::TEXT_PRIMARY)
                .size()
                .x
        });
        assert!(name.width() < full_w - 20.0, "name is shortened to {} from {full_w}", name.width());
        let dot = out.circles().into_iter().find(|c| c.fill == ui_kit::tokens::color::TEXT_SECONDARY).expect("unsaved dot");
        assert!(name.right() <= dot.center.x - dot.radius + EPS);
        assert!(name.right() <= 520.0, "nothing runs past the window");
    }

    // ---- While a take is recorded ----

    /// On a fresh bar: opens the menu button, then the `title` submenu, and
    /// clicks `item_text`. Returns the menu (to look at its dialogs) and the
    /// Project actions chosen.
    fn choose(editor: &mut Editor, title: &str, item_text: &str) -> (ProjectMenu, Vec<ProjectMenuAction>) {
        let mut h = Headless::new(1280.0, 800.0);
        let mut rig = MenuRig::new(editor);
        let out = rig.frame(&mut h);
        let icon = out.text_rect(ui_kit::icons::regular::LIST).unwrap_or_else(|| panic!("no menu button")).center();
        let out = rig.click(&mut h, icon);
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
