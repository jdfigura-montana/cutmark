//! ui::start_screen: the start screen (Design Language rev 3 "Start screen"):
//! two panels side by side, centered, at most 1120 wide. The left hero panel
//! holds the app mark, name and version over a decorative waveform band, the
//! New project and Open project cards and the drop zone; the right panel lists
//! the recent projects. The window chrome and the title bar above are drawn by
//! the app (app.rs); the ui-kit status bar sits below.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local};
use eframe::egui;
use egui::{CornerRadius, Pos2, Rect, Sense, Stroke, StrokeKind, UiBuilder, Vec2, pos2, vec2};
use ui_kit::fonts;
use ui_kit::tokens::{color, radius, space, typography};

use crate::editor::editor::EditorError;
use crate::editor::{Editor, OpenOutcome, RecentEntry, recent_projects, remove_missing_recent, remove_recent};
use crate::ui::dialogs::new_project::NewProjectDialog;
use crate::ui::dialogs::{self, FooterButton};
use crate::ui::menu;
use crate::ui::status_bar;
use crate::ui::theme::THEME;

/// Fixed seeds of the two decorative band waveforms (back layer, front layer).
const BAND_SEEDS: [u32; 2] = [0x9E37_79B1, 0x85EB_CA6B];

/// Width of one decorative band column, in points.
const BAND_COLUMN_W: f32 = 3.0;

/// Where the start screen reads and changes the recent list and opens
/// projects. The app uses [`LIVE`]; the tests swap in doubles so they
/// never touch the user's real recent list.
struct Backend {
    /// Loads the recent list, newest first.
    load: fn() -> Vec<RecentEntry>,
    /// Removes one entry from the recent list.
    remove: fn(&Path),
    /// Opens a project with the autosave-recovery check.
    open: fn(&Path) -> Result<OpenOutcome, EditorError>,
}

/// The real recent list and the real project open.
static LIVE: Backend = Backend { load: recent_projects, remove: remove_recent, open: Editor::open_with_recovery_check };

/// Start screen: New, Open and the recent projects list.
pub struct StartScreen {
    /// The New Project dialog, when open.
    dialog: Option<NewProjectDialog>,
    /// Cached recent list; None means it must be loaded from disk.
    recent: Option<Vec<RecentEntry>>,
    /// Last open error, shown in the drop zone.
    error: Option<String>,
    /// True when the Project > Open picker must be shown on the next frame.
    pending_open: bool,
    /// True when the keyboard-selected recent row must open on the next
    /// frame; deferred so Enter never also reaches the recovery modal.
    enter_open: bool,
    /// Project path awaiting an autosave-recovery decision, if any.
    recovery: Option<PathBuf>,
    /// Keyboard-selected recent row, when the list is navigated by keys.
    sel: Option<usize>,
    /// True when the selected row must be scrolled into view this frame.
    scroll_to_sel: bool,
    /// The recent list and open calls.
    backend: &'static Backend,
}

impl Default for StartScreen {
    fn default() -> Self {
        StartScreen {
            dialog: None,
            recent: None,
            error: None,
            pending_open: false,
            enter_open: false,
            recovery: None,
            sel: None,
            scroll_to_sel: false,
            backend: &LIVE,
        }
    }
}

impl StartScreen {
    /// Creates the start screen with the New Project dialog already open.
    pub fn with_new_dialog() -> Self {
        StartScreen { dialog: Some(NewProjectDialog::new_open()), ..StartScreen::default() }
    }

    /// Creates the start screen showing the Open Project picker next frame.
    pub fn with_open_request() -> Self {
        StartScreen { pending_open: true, ..StartScreen::default() }
    }

    /// True when the New Project dialog is currently open.
    pub fn dialog_is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Draws the start screen; returns the Editor to edit when one opens.
    pub fn show(&mut self, ui: &mut egui::Ui, output_device: Option<&str>) -> Option<Editor> {
        let mut opened = None;
        if self.enter_open {
            self.enter_open = false;
            self.open_selected(&mut opened);
        }
        if self.pending_open {
            self.pending_open = false;
            self.open_project_file(&mut opened);
        }
        if let Some(dialog) = &mut self.dialog {
            let ctx = ui.ctx().clone();
            if let Some(editor) = dialog.show(&ctx) {
                self.recent = None;
                opened = Some(editor);
            }
        }
        self.handle_drops(ui, &mut opened);

        // The 26 px status bar under the panels.
        egui::Panel::bottom("start_status_bar")
            .exact_size(THEME.status_bar_h)
            .frame(egui::Frame::new().fill(THEME.bar_bg).inner_margin(egui::Margin { left: 8, ..Default::default() }))
            .show(ui, |ui| {
                status_bar::show_start(ui, output_device);
            });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(THEME.app_bg)).show(ui, |ui| {
            self.show_panels(ui, &mut opened);
        });
        self.show_recovery_modal(ui, &mut opened);
        opened
    }

    /// Loads the recent list when it is not cached and returns a copy.
    fn entries(&mut self) -> Vec<RecentEntry> {
        if self.recent.is_none() {
            self.recent = Some((self.backend.load)());
        }
        self.recent.clone().unwrap_or_default()
    }

    /// The two panels, centered in the window: the hero (420 wide) and the
    /// recent projects (the rest), 16 apart, at most 1120 wide and 640 tall.
    fn show_panels(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        let _ = self.entries();
        self.handle_list_keys(ui);
        let entries = self.entries();

        let avail = ui.max_rect();
        let margin = THEME.start_card_margin;
        let min_w = THEME.start_panel_w + THEME.start_gap + THEME.start_panel_w / 2.0;
        let w = THEME.start_max_w.min(avail.width() - 2.0 * margin).max(min_w);
        let h = THEME.start_panel_h.min(avail.height() - 2.0 * margin).max(THEME.start_panel_h * 0.75);
        let content = Rect::from_center_size(avail.center(), vec2(w, h));
        let hero_w = THEME.start_panel_w.min(w * 0.45);
        let hero = Rect::from_min_size(content.min, vec2(hero_w, h));
        let recent = Rect::from_min_max(pos2(hero.right() + THEME.start_gap, content.top()), content.max);

        self.show_hero(ui, hero, opened);
        self.show_recent(ui, recent, &entries, opened);
        self.scroll_to_sel = false;
    }

    /// Arrow keys move the keyboard selection through the recent list; Enter
    /// opens the selected row (deferred a frame, see `enter_open`); Delete
    /// removes it, the same action as the row's remove button.
    fn handle_list_keys(&mut self, ui: &egui::Ui) {
        let len = self.recent.as_ref().map_or(0, Vec::len);
        self.sel = self.sel.filter(|_| len > 0).map(|s| s.min(len.saturating_sub(1)));
        if self.dialog.is_some() || self.recovery.is_some() || len == 0 {
            return;
        }
        let (up, down, enter, delete) = ui.input(|i| {
            (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Delete))
        });
        let last = len - 1;
        if up {
            self.sel = Some(match self.sel {
                Some(s) => s.saturating_sub(1),
                None => last,
            });
            self.scroll_to_sel = true;
        } else if down {
            self.sel = Some(match self.sel {
                Some(s) => (s + 1).min(last),
                None => 0,
            });
            self.scroll_to_sel = true;
        } else if enter {
            self.enter_open = self.sel.is_some();
        } else if delete && let Some(s) = self.sel {
            self.remove_at(s);
        }
    }

    /// Removes the recent entry at `index` from the list and reloads it, then
    /// keeps the selection on the row that took its place.
    fn remove_at(&mut self, index: usize) {
        let Some(root) = self.recent.as_ref().and_then(|entries| entries.get(index)).map(|entry| entry.root.clone()) else {
            return;
        };
        (self.backend.remove)(&root);
        let entries = (self.backend.load)();
        self.sel = self.sel.filter(|_| !entries.is_empty()).map(|s| s.min(entries.len().saturating_sub(1)));
        self.recent = Some(entries);
    }

    /// Opens the keyboard-selected recent row, if any.
    fn open_selected(&mut self, opened: &mut Option<Editor>) {
        let entries = self.entries();
        if let Some(s) = self.sel
            && let Some(entry) = entries.get(s.min(entries.len().saturating_sub(1)))
        {
            let root = entry.root.clone();
            self.try_open(root, opened);
        }
    }

    /// The hero panel: app mark, name and version over the waveform band, the
    /// New project and Open project cards and the drop zone.
    fn show_hero(&mut self, ui: &mut egui::Ui, rect: Rect, opened: &mut Option<Editor>) {
        paint_panel(ui, rect);
        let pad = space::SCALE_24;
        let inner = rect.shrink(pad);

        // The decorative band sits behind the heading, under the top edge.
        let mark_side = THEME.start_mark;
        let band_top = inner.top() + mark_side + pad;
        paint_band(ui, Rect::from_min_size(pos2(rect.left(), band_top), vec2(rect.width(), THEME.start_band_h)));

        // Heading: mark, then the name over the version.
        let mark = Rect::from_min_size(inner.min, Vec2::splat(mark_side));
        menu::paint_app_mark(ui, mark, color::BG_PANEL);
        let name = ui.painter().layout_no_wrap(crate::app::APP_SHORT_NAME.to_owned(), fonts::semibold(THEME.start_title_size), color::TEXT_PRIMARY);
        let version = ui.painter().layout_no_wrap(format!("v{}", env!("CARGO_PKG_VERSION")), fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY);
        let block_h = name.size().y + version.size().y;
        let text_x = mark.right() + space::SCALE_12;
        let text_y = mark.center().y - block_h / 2.0;
        let version_y = text_y + name.size().y;
        ui.painter().galley(pos2(text_x, text_y), name, color::TEXT_PRIMARY);
        ui.painter().galley(pos2(text_x, version_y), version, color::TEXT_TERTIARY);

        // Cards and the drop zone, anchored to the bottom of the panel.
        let drop = Rect::from_min_size(pos2(inner.left(), inner.bottom() - THEME.start_drop_h), vec2(inner.width(), THEME.start_drop_h));
        let card_h = THEME.start_card_tile + 2.0 * THEME.start_card_pad.y;
        let open_card = Rect::from_min_size(pos2(inner.left(), drop.top() - space::SCALE_12 - card_h), vec2(inner.width(), card_h));
        let new_card = Rect::from_min_size(pos2(inner.left(), open_card.top() - space::SCALE_10 - card_h), vec2(inner.width(), card_h));

        let new_card_spec = Card {
            id: "start_new_card",
            icon: ui_kit::icons::regular::PLUS,
            title: "New project",
            subtitle: "Start a new audio project",
            shortcut: "Ctrl+N",
            primary: true,
        };
        if action_card(ui, new_card, &new_card_spec).clicked() {
            self.dialog = Some(NewProjectDialog::new_open());
        }
        let open_card_spec = Card {
            id: "start_open_card",
            icon: ui_kit::icons::regular::FOLDER_OPEN,
            title: "Open project",
            subtitle: "Pick a project.json from disk",
            shortcut: "Ctrl+O",
            primary: false,
        };
        if action_card(ui, open_card, &open_card_spec).clicked() {
            self.open_project_file(opened);
        }

        let hovering = valid_drop_hover(&ui.input(|i| i.raw.hovered_files.clone()));
        paint_drop_zone(ui, drop, hovering, self.error.as_deref());
    }

    /// The recent projects panel: title and count, the rows, then the footer
    /// hint line under a hairline.
    fn show_recent(&mut self, ui: &mut egui::Ui, rect: Rect, entries: &[RecentEntry], opened: &mut Option<Editor>) {
        paint_panel(ui, rect);
        let pad = space::SCALE_24;
        let inner = rect.shrink(pad);

        // Header: title left, "Clear missing" and the count right.
        let header = Rect::from_min_size(inner.min, vec2(inner.width(), THEME.start_header_h));
        let title = ui.painter().layout_no_wrap("Recent projects".to_owned(), fonts::semibold(THEME.start_card_title_size), color::TEXT_PRIMARY);
        ui.painter().galley(pos2(header.left(), header.center().y - title.size().y / 2.0), title, color::TEXT_PRIMARY);
        let count = ui.painter().layout_no_wrap(format!("{} of 10", entries.len()), fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY);
        let count_left = header.right() - count.size().x;
        ui.painter().galley(pos2(count_left, header.center().y - count.size().y / 2.0), count, color::TEXT_TERTIARY);
        if entries.iter().any(|e| !e.exists()) {
            let right = count_left - space::SCALE_12;
            if self.clear_missing_link(ui, header, right).clicked() {
                remove_missing_recent();
                self.recent = None;
            }
        }

        // Footer: a hairline across the panel and the key hints under it.
        let footer_h = THEME.start_header_h * 2.0;
        let line_y = rect.bottom() - footer_h;
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), line_y), pos2(rect.right(), line_y + THEME.hairline)), 0.0, color::STROKE_HAIRLINE);
        let hint = ui.painter().layout_no_wrap(footer_hint(!entries.is_empty()).to_owned(), fonts::medium(THEME.start_hint_size), color::TEXT_TERTIARY);
        let hint_y = line_y + (footer_h - hint.size().y) / 2.0;
        ui.painter().galley(pos2(inner.left(), hint_y), hint, color::TEXT_TERTIARY);

        // The rows, in a scroll area between the header and the footer.
        let list = Rect::from_min_max(pos2(inner.left(), header.bottom() + space::SCALE_12), pos2(inner.right(), line_y - space::SCALE_8));
        let mut list_ui = ui.new_child(UiBuilder::new().max_rect(list));
        if entries.is_empty() {
            let text = ui.painter().layout_no_wrap("No recent projects yet".to_owned(), fonts::medium(THEME.start_hint_size), color::TEXT_TERTIARY);
            ui.painter().galley(pos2(list.left(), list.top()), text, color::TEXT_TERTIARY);
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut list_ui, |ui| {
            ui.spacing_mut().item_spacing.y = THEME.start_row_gap;
            for (index, entry) in entries.iter().enumerate() {
                self.show_row(ui, index, entry, opened);
            }
        });
    }

    /// The "Clear missing" text link in the header, right edge at `right`.
    fn clear_missing_link(&self, ui: &egui::Ui, header: Rect, right: f32) -> egui::Response {
        let galley = ui.painter().layout_no_wrap("Clear missing".to_owned(), fonts::medium(THEME.start_hint_size), color::TEXT_SECONDARY);
        let rect = Rect::from_min_size(pos2(right - galley.size().x, header.center().y - galley.size().y / 2.0), galley.size());
        let response = ui.interact(rect, ui.id().with("start_clear_missing"), Sense::click());
        let tint = if response.hovered() { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
        ui.painter().galley(rect.min, galley, tint);
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }

    /// One recent-project row: waveform tile, the project name over its
    /// middle-truncated folder path, the last-opened date (or "Missing") and,
    /// on hover or selection, the remove button. Clicking the row opens the
    /// project.
    fn show_row(&mut self, ui: &mut egui::Ui, index: usize, entry: &RecentEntry, opened: &mut Option<Editor>) {
        let selected = self.sel == Some(index);
        let exists = entry.exists();
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), THEME.start_row_h), Sense::click());
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &entry.name));
        let shown = ui.is_rect_visible(rect);
        let hovered = response.contains_pointer();
        if selected && self.scroll_to_sel {
            response.scroll_to_me(None);
        }
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);

        // The remove button: visible on hover or selection. It is registered
        // after the row, so it wins the clicks over its own rectangle.
        let side = THEME.start_x_btn;
        let remove_rect = Rect::from_center_size(pos2(rect.right() - space::SCALE_10 - side / 2.0, rect.center().y), Vec2::splat(side));
        let remove = (selected || hovered).then(|| ui.interact(remove_rect, response.id.with("remove"), Sense::click()));

        if shown {
            let accent = ui_kit::Theme::get(ui.ctx()).accent;
            let radius = CornerRadius::from(THEME.start_row_radius);
            if selected {
                ui.painter().rect_filled(rect, radius, color::BG_ROW_SELECTED);
            } else if hovered {
                ui.painter().rect_filled(rect, radius, color::BG_GROUP);
            }
            // Tile.
            let tile = Rect::from_center_size(pos2(rect.left() + space::SCALE_10 + THEME.start_tile / 2.0, rect.center().y), Vec2::splat(THEME.start_tile));
            let (tile_fill, glyph_color) = match (selected, exists) {
                (true, _) => (accent.tint, accent.accent),
                (false, true) => (color::BG_CONTROL, color::TEXT_TERTIARY),
                (false, false) => (color::BG_CONTROL, color::TEXT_DISABLED),
            };
            ui.painter().rect_filled(tile, CornerRadius::from(radius::GROUP), tile_fill);
            let glyph = ui.painter().layout_no_wrap(ui_kit::icons::regular::WAVEFORM.to_owned(), fonts::icon_regular(THEME.start_icon_size), glyph_color);
            ui.painter().galley(tile.center() - glyph.size() / 2.0, glyph, glyph_color);

            // Date column, right of the text and left of the button slot.
            let (date_text, date_color) = if exists {
                (format_last_opened(entry.last_opened).unwrap_or_default(), color::TEXT_TERTIARY)
            } else {
                ("Missing".to_owned(), THEME.error_soft)
            };
            let date = ui.painter().layout_no_wrap(date_text, fonts::medium(typography::CAPTION_SIZE), date_color);
            let date_left = remove_rect.left() - space::SCALE_4 - date.size().x;
            ui.painter().galley(pos2(date_left, rect.center().y - date.size().y / 2.0), date, date_color);

            // Name over the folder path, both limited to the text column.
            let text_left = tile.right() + space::SCALE_12;
            let text_w = (date_left - space::SCALE_12 - text_left).max(0.0);
            let name_color = if exists { color::TEXT_PRIMARY } else { color::TEXT_TERTIARY };
            let name = ui.painter().layout_no_wrap(entry.name.clone(), fonts::semibold(typography::BODY_SIZE), name_color);
            let path_font = fonts::mono(THEME.start_small_size);
            let path_text = middle_truncate(ui.painter(), &entry.root.display().to_string(), &path_font, text_w);
            let path = ui.painter().layout_no_wrap(path_text, path_font, color::TEXT_TERTIARY);
            let block_h = name.size().y + space::SCALE_2 + path.size().y;
            let name_y = rect.center().y - block_h / 2.0;
            let path_y = name_y + name.size().y + space::SCALE_2;
            let text_clip = Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_left + text_w, rect.bottom()));
            ui.painter().with_clip_rect(text_clip).galley(pos2(text_left, name_y), name, name_color);
            ui.painter().galley(pos2(text_left, path_y), path, color::TEXT_TERTIARY);

            if let Some(remove) = &remove {
                let hot = remove.hovered();
                if hot {
                    ui.painter().circle_filled(remove_rect.center(), side / 2.0, color::BG_HOVER);
                }
                let tint = if hot { color::TEXT_PRIMARY } else { color::TEXT_SECONDARY };
                let x = ui.painter().layout_no_wrap(ui_kit::icons::regular::X.to_owned(), fonts::icon_regular(typography::BODY_SIZE), tint);
                ui.painter().galley(remove_rect.center() - x.size() / 2.0, x, tint);
            }
        }

        if let Some(remove) = remove {
            let remove = remove.on_hover_text("Remove from list");
            remove.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Remove from list"));
            if remove.clicked() {
                self.remove_at(index);
                ui.ctx().request_repaint();
                return;
            }
        }
        if response.clicked() {
            self.sel = Some(index);
            let root = entry.root.clone();
            self.try_open(root, opened);
        }
    }

    /// Files dropped on the start screen: a project folder or project.json
    /// opens the project, anything else shows the error in the drop zone.
    fn handle_drops(&mut self, ui: &egui::Ui, opened: &mut Option<Editor>) {
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(path) = dropped.first() {
            if is_project_path(path) {
                self.try_open(path.clone(), opened);
            } else {
                self.error = Some("Not a CutMark Audio Editor project: drop a project folder or a project.json file.".to_owned());
            }
        }
    }

    /// Draws the autosave-recovery modal when a project needs a decision.
    fn show_recovery_modal(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        let Some(path) = self.recovery.clone() else {
            return;
        };
        #[derive(PartialEq)]
        enum Choice {
            Recover,
            LastSaved,
            Cancel,
        }
        let mut choice = None;
        let response = dialogs::modal("autosave_recovery_dialog").show(ui.ctx(), |ui| {
            dialogs::card_width(ui, THEME.dialog_w);
            dialogs::title(ui, "Recover autosave");
            ui.add_space(THEME.space_xs);
            dialogs::description(ui, "This project has unsaved changes from a session that didn't close properly.");
            dialogs::before_form(ui);
            dialogs::footer(ui, |ui| {
                if dialogs::footer_button(ui, "Recover autosave", FooterButton::Primary, true).clicked() {
                    choice = Some(Choice::Recover);
                }
                if dialogs::footer_button(ui, "Open last saved", FooterButton::Cancel, true).clicked() {
                    choice = Some(Choice::LastSaved);
                }
                if dialogs::footer_button(ui, "Cancel", FooterButton::Cancel, true).clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
            // Enter recovers the autosave.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                choice = Some(Choice::Recover);
            }
        });
        self.recovery = None;
        // Esc or the backdrop cancels the open.
        if choice.is_none() && response.should_close() {
            choice = Some(Choice::Cancel);
        }
        if choice != Some(Choice::Cancel) {
            self.finish_open(&path, choice == Some(Choice::Recover), opened);
        }
    }

    /// Opens the project either from the autosave or from the last save.
    fn finish_open(&mut self, path: &Path, recover: bool, opened: &mut Option<Editor>) {
        self.error = None;
        let result = if recover { Editor::open_recovered(path) } else { Editor::open_discarding_autosave(path) };
        match result {
            Ok(editor) => {
                self.recent = None;
                *opened = Some(editor);
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Opens a project picked with a file dialog filtered to project.json.
    fn open_project_file(&mut self, opened: &mut Option<Editor>) {
        self.error = None;
        let picked = rfd::FileDialog::new().add_filter("Project file (project.json)", &["json"]).pick_file();
        if let Some(path) = picked {
            self.try_open(path, opened);
        }
    }

    /// Attempts the recovery-checked open; a newer autosave raises the modal.
    fn try_open(&mut self, path: PathBuf, opened: &mut Option<Editor>) {
        self.error = None;
        match (self.backend.open)(&path) {
            Ok(OpenOutcome::Opened(editor)) => {
                self.recent = None;
                *opened = Some(*editor);
            }
            Ok(OpenOutcome::NeedsRecoveryChoice(path)) => self.recovery = Some(path),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// True when `path` is a project.json file or a folder holding one.
fn is_project_path(path: &Path) -> bool {
    let is_project_file = path.is_file() && path.file_name().is_some_and(|n| n == std::ffi::OsStr::new("project.json"));
    let is_project_dir = path.is_dir() && path.join("project.json").is_file();
    is_project_file || is_project_dir
}

/// True while a file being dragged over the window is a project the drop zone
/// would open (a project.json or a project folder).
fn valid_drop_hover(hovered: &[egui::HoveredFile]) -> bool {
    hovered.first().and_then(|file| file.path.as_deref()).is_some_and(is_project_path)
}

/// The footer hint line: only keys that work. Up, Down, Enter and Delete act
/// on the selected recent row, so they are listed only while rows exist.
fn footer_hint(has_rows: bool) -> &'static str {
    if has_rows { "Up and Down select \u{b7} Enter opens \u{b7} Delete removes" } else { "Ctrl+N new project \u{b7} Ctrl+O open project" }
}

/// A panel: `bg.panel` fill with a hairline rim, radius `radius.panel`.
fn paint_panel(ui: &egui::Ui, rect: Rect) {
    let rounding = CornerRadius::from(radius::PANEL);
    ui.painter().rect_filled(rect, rounding, THEME.panel_bg);
    ui.painter().rect_stroke(rect, rounding, Stroke::new(THEME.hairline, color::STROKE_HAIRLINE), StrokeKind::Inside);
}

/// One action card's content.
struct Card<'a> {
    /// Widget id salt.
    id: &'a str,
    /// Phosphor regular glyph for the tile.
    icon: &'a str,
    /// Title, `type` 15 / 600.
    title: &'a str,
    /// Subtitle, `type.caption`.
    subtitle: &'a str,
    /// Shortcut label, `type.value`; only shortcuts that exist.
    shortcut: &'a str,
    /// True for the accent card.
    primary: bool,
}

/// An action card: a 14 radius row with a 44 icon tile, title over subtitle and
/// the shortcut on the right. The primary card fills with the accent and uses
/// `accent.on` text; the other card uses `bg.group` with a hairline rim and
/// `bg.hover` on hover.
fn action_card(ui: &mut egui::Ui, rect: Rect, card: &Card<'_>) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(card.id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, card.title));
    let accent = ui_kit::Theme::get(ui.ctx()).accent;
    let rounding = CornerRadius::from(THEME.start_card_radius);
    let (fill, title_color, subtitle_color, tile_fill, glyph_color) = if card.primary {
        let fill = if response.is_pointer_button_down_on() {
            accent.pressed
        } else if response.hovered() {
            accent.hover
        } else {
            accent.accent
        };
        (fill, accent.on, accent.on.gamma_multiply(0.7), accent.on.gamma_multiply(0.14), accent.on)
    } else {
        let fill = if response.hovered() { color::BG_HOVER } else { color::BG_GROUP };
        (fill, color::TEXT_PRIMARY, color::TEXT_TERTIARY, color::BG_HOVER, color::TEXT_SECONDARY)
    };
    ui.painter().rect_filled(rect, rounding, fill);
    if !card.primary {
        ui.painter().rect_stroke(rect, rounding, Stroke::new(THEME.hairline, color::STROKE_HAIRLINE), StrokeKind::Inside);
    }
    let tile =
        Rect::from_min_size(pos2(rect.left() + THEME.start_card_pad.x, rect.center().y - THEME.start_card_tile / 2.0), Vec2::splat(THEME.start_card_tile));
    ui.painter().rect_filled(tile, CornerRadius::from(radius::GROUP), tile_fill);
    let glyph = ui.painter().layout_no_wrap(card.icon.to_owned(), fonts::icon_regular(THEME.start_icon_size), glyph_color);
    ui.painter().galley(tile.center() - glyph.size() / 2.0, glyph, glyph_color);

    let title = ui.painter().layout_no_wrap(card.title.to_owned(), fonts::semibold(THEME.start_card_title_size), title_color);
    let subtitle = ui.painter().layout_no_wrap(card.subtitle.to_owned(), fonts::medium(typography::CAPTION_SIZE), subtitle_color);
    let block_h = title.size().y + subtitle.size().y;
    let text_x = tile.right() + space::SCALE_12;
    let title_y = rect.center().y - block_h / 2.0;
    let subtitle_y = title_y + title.size().y;
    ui.painter().galley(pos2(text_x, title_y), title, title_color);
    ui.painter().galley(pos2(text_x, subtitle_y), subtitle, subtitle_color);

    let shortcut = ui.painter().layout_no_wrap(card.shortcut.to_owned(), fonts::mono(typography::VALUE_SIZE), subtitle_color);
    let shortcut_pos = pos2(rect.right() - THEME.start_card_pad.x - shortcut.size().x, rect.center().y - shortcut.size().y / 2.0);
    ui.painter().galley(shortcut_pos, shortcut, subtitle_color);

    if response.has_focus() {
        ui_kit::widgets::focus::focus_ring(ui, rect, rounding);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The drop zone: a dashed 14 radius box with an upload icon and the drop hint,
/// or the last open error in its place. The rim turns accent while a valid
/// drop hovers.
fn paint_drop_zone(ui: &egui::Ui, rect: Rect, hovering: bool, error: Option<&str>) {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let rim = if hovering { accent } else { THEME.start_drop_border };
    let outline = rounded_rect_path(rect.shrink(THEME.start_drop_stroke / 2.0), THEME.start_drop_radius);
    ui.painter().extend(egui::Shape::dashed_line(&outline, Stroke::new(THEME.start_drop_stroke, rim), THEME.start_dash, THEME.start_dash_gap));

    let font = fonts::medium(THEME.start_hint_size);
    if let Some(message) = error {
        let mut job = egui::text::LayoutJob::single_section(message.to_owned(), egui::TextFormat::simple(font, THEME.error_soft));
        job.wrap = egui::text::TextWrapping {
            max_width: rect.width() - 2.0 * space::SCALE_16,
            max_rows: 3,
            break_anywhere: false,
            overflow_character: Some('\u{2026}'),
        };
        let galley = ui.painter().layout_job(job);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, THEME.error_soft);
        return;
    }
    let tint = if hovering { color::TEXT_SECONDARY } else { color::TEXT_TERTIARY };
    let icon = ui.painter().layout_no_wrap(ui_kit::icons::regular::UPLOAD_SIMPLE.to_owned(), fonts::icon_regular(THEME.start_icon_size), tint);
    let hint = ui.painter().layout_no_wrap("Drop a project folder or project.json here".to_owned(), font, tint);
    let gap = space::SCALE_6;
    let total = icon.size().y + gap + hint.size().y;
    let top = rect.center().y - total / 2.0;
    ui.painter().galley(pos2(rect.center().x - icon.size().x / 2.0, top), icon.clone(), tint);
    ui.painter().galley(pos2(rect.center().x - hint.size().x / 2.0, top + icon.size().y + gap), hint, tint);
}

/// The closed outline of a rounded rectangle as a point list, for the dashed
/// drop zone border.
fn rounded_rect_path(rect: Rect, corner: f32) -> Vec<Pos2> {
    const STEPS: usize = 6;
    let r = corner.min(rect.width() / 2.0).min(rect.height() / 2.0).max(0.0);
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -std::f32::consts::FRAC_PI_2),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (pos2(rect.left() + r, rect.bottom() - r), std::f32::consts::FRAC_PI_2),
        (pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI),
    ];
    let mut points = Vec::with_capacity(4 * (STEPS + 1) + 1);
    for (center, start) in corners {
        for step in 0..=STEPS {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / STEPS as f32;
            points.push(center + vec2(angle.cos(), angle.sin()) * r);
        }
    }
    if let Some(first) = points.first().copied() {
        points.push(first);
    }
    points
}

/// Fixed-seed decorative peaks: `columns` min/max pairs that swell toward the
/// middle and fade to nothing at both ends, so nothing ever reaches the
/// panel's rounded corners. The same seed always gives the same peaks.
fn band_peaks(seed: u32, columns: usize) -> Vec<(f32, f32)> {
    let mut state = seed.max(1);
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state % 10_000) as f32 / 10_000.0
    };
    let last = columns.saturating_sub(1).max(1) as f32;
    (0..columns)
        .map(|i| {
            let t = i as f32 / last;
            let swell = (std::f32::consts::PI * t).sin().max(0.0).powf(0.8);
            let phrase = 0.65 + 0.35 * (t * 9.0 + seed as f32).sin();
            let top = swell * phrase * (0.25 + 0.75 * next());
            let bottom = swell * phrase * (0.25 + 0.75 * next());
            (-bottom, top)
        })
        .collect()
}

/// The hero's decorative band: two layered filled waveforms from fixed seeds
/// (`bg.hover` behind, accent in front, the front one shorter), each at 55%
/// opacity and the whole band at 55% again. Painted once per frame; nothing
/// animates and nothing asks for a repaint.
fn paint_band(ui: &egui::Ui, rect: Rect) {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let layer = THEME.start_band_alpha * THEME.start_band_alpha;
    let columns = (rect.width() / BAND_COLUMN_W).max(2.0) as usize;
    let painter = ui.painter().with_clip_rect(rect);
    let front = rect.shrink2(vec2(0.0, rect.height() * 0.2));
    ui_kit::widgets::timeline::paint_waveform(&painter, rect, &band_peaks(BAND_SEEDS[0], columns), color::BG_HOVER.gamma_multiply(layer));
    ui_kit::widgets::timeline::paint_waveform(&painter, front, &band_peaks(BAND_SEEDS[1], columns), accent.gamma_multiply(layer));
}

/// `text` cut in the middle with an ellipsis so it fits `max_width` in `font`:
/// the start and the end of a folder path stay readable. Text that fits is
/// returned unchanged.
fn middle_truncate(painter: &egui::Painter, text: &str, font: &egui::FontId, max_width: f32) -> String {
    let width = |s: &str| painter.layout_no_wrap(s.to_owned(), font.clone(), color::TEXT_TERTIARY).size().x;
    if width(text) <= max_width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| -> String {
        let head: String = chars.iter().take(keep - keep / 2).collect();
        let tail: String = chars.iter().skip(chars.len() - keep / 2).collect();
        format!("{head}\u{2026}{tail}")
    };
    // The widest cut that still fits: the width grows with `keep`.
    let (mut low, mut high) = (0usize, chars.len());
    while low < high {
        let mid = (low + high).div_ceil(2);
        if width(&build(mid)) <= max_width {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    build(low)
}

/// Local-time label for a last-opened Unix timestamp: "Today 2:14 PM",
/// "Yesterday", "Oct 9" (this year) or "Oct 9, 2025"; None for entries
/// that predate the timestamp.
fn format_last_opened(ts: Option<i64>) -> Option<String> {
    format_last_opened_at(ts?, Local::now())
}

/// `format_last_opened` with an injectable "now", for tests.
fn format_last_opened_at(ts: i64, now: DateTime<Local>) -> Option<String> {
    use chrono::TimeZone;
    let when = Local.timestamp_opt(ts, 0).single()?;
    let today = now.date_naive();
    let day = when.date_naive();
    if day == today {
        Some(format!("Today {}", when.format("%l:%M %p").to_string().trim_start()))
    } else if day == today - Duration::days(1) {
        Some("Yesterday".to_owned())
    } else if day.year() == today.year() {
        Some(when.format("%b %-d").to_string())
    } else {
        Some(when.format("%b %-d, %Y").to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_util::{Headless, Out};
    use chrono::TimeZone;
    use std::cell::RefCell;

    thread_local! {
        /// The recent list the test backend serves (one per test thread).
        static LIST: RefCell<Vec<RecentEntry>> = const { RefCell::new(Vec::new()) };
        /// The paths the test backend was asked to open.
        static OPENED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    }

    fn fake_load() -> Vec<RecentEntry> {
        LIST.with(|list| list.borrow().clone())
    }

    fn fake_remove(root: &Path) {
        LIST.with(|list| list.borrow_mut().retain(|entry| entry.root != root));
    }

    fn fake_open(path: &Path) -> Result<OpenOutcome, EditorError> {
        OPENED.with(|opened| opened.borrow_mut().push(path.to_path_buf()));
        Err(EditorError::Locked("test open".to_owned()))
    }

    fn root_of(name: &str) -> PathBuf {
        PathBuf::from(format!("C:\\cm_test\\{name}"))
    }

    fn names() -> Vec<String> {
        LIST.with(|list| list.borrow().iter().map(|entry| entry.name.clone()).collect())
    }

    fn opened() -> Vec<PathBuf> {
        OPENED.with(|opened| opened.borrow().clone())
    }

    /// A start screen over a fake recent list, driven frame by frame.
    struct Rig {
        h: Headless,
        screen: StartScreen,
    }

    impl Rig {
        fn new(projects: &[&str]) -> Rig {
            LIST.with(|list| {
                *list.borrow_mut() = projects.iter().map(|name| RecentEntry { name: (*name).to_owned(), root: root_of(name), last_opened: None }).collect();
            });
            OPENED.with(|opened| opened.borrow_mut().clear());
            static FAKE: Backend = Backend { load: fake_load, remove: fake_remove, open: fake_open };
            let screen = StartScreen { backend: &FAKE, ..StartScreen::default() };
            Rig { h: Headless::new(1280.0, 800.0), screen }
        }

        fn frame_with(&mut self, events: Vec<egui::Event>) -> Out {
            let screen = &mut self.screen;
            self.h.frame(events, |ui| {
                let _ = screen.show(ui, None);
            })
        }

        fn frame(&mut self) -> Out {
            self.frame_with(Vec::new())
        }

        fn key(&mut self, key: egui::Key) -> Out {
            let event = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            self.frame_with(vec![event])
        }

        fn click_text(&mut self, label: &str) -> Out {
            let out = self.frame();
            let at = out.text_rect(label).unwrap_or_else(|| panic!("no {label} on screen")).center();
            let screen = &mut self.screen;
            self.h.click(at, |ui| {
                let _ = screen.show(ui, None);
            })
        }
    }

    #[test]
    fn enter_opens_the_selected_recent_project() {
        let mut rig = Rig::new(&["alpha", "bravo", "charlie"]);
        let _ = rig.frame();
        let _ = rig.key(egui::Key::ArrowDown);
        let _ = rig.key(egui::Key::ArrowDown);
        assert!(opened().is_empty(), "arrows only select");
        let _ = rig.key(egui::Key::Enter);
        let _ = rig.frame();
        assert_eq!(opened(), vec![root_of("bravo")], "Enter opens the second row, once");
    }

    #[test]
    fn enter_with_nothing_selected_opens_nothing() {
        let mut rig = Rig::new(&["alpha", "bravo"]);
        let _ = rig.frame();
        let _ = rig.key(egui::Key::Enter);
        let _ = rig.frame();
        assert!(opened().is_empty());
    }

    #[test]
    fn the_remove_button_removes_its_row_and_only_shows_on_hover_or_selection() {
        let mut rig = Rig::new(&["alpha", "bravo", "charlie"]);
        let out = rig.frame();
        assert!(out.text_rect(ui_kit::icons::regular::X).is_none(), "no remove button at rest");
        let _ = rig.key(egui::Key::ArrowDown);
        let out = rig.frame();
        let buttons = out.texts().into_iter().filter(|t| t.text == ui_kit::icons::regular::X).count();
        assert_eq!(buttons, 1, "only the selected row shows its remove button");
        let _ = rig.click_text(ui_kit::icons::regular::X);
        assert_eq!(names(), vec!["bravo", "charlie"], "the selected row alpha was removed");
        assert!(opened().is_empty(), "removing never opens the row");
        let out = rig.frame();
        assert!(out.text_rect("alpha").is_none(), "the row is gone from the list");
    }

    #[test]
    fn delete_removes_the_selected_row_and_keeps_the_selection_in_place() {
        let mut rig = Rig::new(&["alpha", "bravo", "charlie"]);
        let _ = rig.frame();
        let _ = rig.key(egui::Key::Delete);
        assert_eq!(names().len(), 3, "Delete with no selection removes nothing");
        let _ = rig.key(egui::Key::ArrowDown);
        let _ = rig.key(egui::Key::ArrowDown);
        let _ = rig.key(egui::Key::Delete);
        assert_eq!(names(), vec!["alpha", "charlie"], "Delete removed the selected bravo");
        assert_eq!(rig.screen.sel, Some(1), "the selection moved onto the row that took its place");
        let _ = rig.key(egui::Key::Delete);
        assert_eq!(names(), vec!["alpha"], "the last row went, so the selection stepped back onto alpha");
        assert_eq!(rig.screen.sel, Some(0));
    }

    #[test]
    fn deleting_the_last_row_clears_the_selection() {
        let mut rig = Rig::new(&["alpha"]);
        let _ = rig.frame();
        let _ = rig.key(egui::Key::ArrowDown);
        let _ = rig.key(egui::Key::Delete);
        assert!(names().is_empty());
        let out = rig.frame();
        assert_eq!(rig.screen.sel, None);
        assert!(out.text_rect("No recent projects yet").is_some());
    }

    #[test]
    fn the_band_is_fixed_seed_and_requests_no_repaint() {
        assert_eq!(band_peaks(BAND_SEEDS[0], 140), band_peaks(BAND_SEEDS[0], 140), "the same seed paints the same band");
        assert_ne!(band_peaks(BAND_SEEDS[0], 140), band_peaks(BAND_SEEDS[1], 140), "the two layers differ");
        let peaks = band_peaks(BAND_SEEDS[1], 140);
        assert!(peaks.iter().all(|&(min, max)| (-1.0..=0.0).contains(&min) && (0.0..=1.0).contains(&max)));
        assert_eq!(peaks.first().map(|p| p.1), Some(0.0), "the band fades to nothing at its left end");

        let mut h = Headless::new(1280.0, 800.0);
        let mut painted = false;
        let mut frame = |h: &mut Headless| {
            h.frame(Vec::new(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    paint_band(ui, Rect::from_min_size(pos2(100.0, 100.0), vec2(420.0, THEME.start_band_h)));
                    painted = true;
                });
            })
        };
        let _ = frame(&mut h);
        let out = frame(&mut h);
        assert!(painted, "the band was painted");
        assert!(!out.repaint_requested(), "painting the band never asks for another frame");
    }

    #[test]
    fn an_idle_start_screen_requests_no_repaint() {
        let mut rig = Rig::new(&["alpha", "bravo"]);
        let _ = rig.frame();
        let out = rig.frame();
        assert!(!out.repaint_requested(), "nothing animates on the idle start screen");
    }

    #[test]
    fn the_hero_and_recent_panels_follow_the_board_geometry() {
        let mut rig = Rig::new(&["alpha"]);
        let _ = rig.frame();
        let out = rig.frame();
        let version = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert!(out.text_rect(&version).is_some(), "the version from Cargo");
        let name = out.text_rect(crate::app::APP_SHORT_NAME).expect("the app name");
        let mark_left = name.left() - space::SCALE_12 - THEME.start_mark;
        // 1280 wide: content is 1120 wide, starts at x 80; the hero pads 24.
        assert!((mark_left - 104.0).abs() <= 1.0, "the mark sits in the hero's padding, at {mark_left}");
        let title = out.text_rect("Recent projects").expect("the recent title");
        // Hero 80..500, 16 gap, recent from 516, padding 24.
        assert!((title.left() - 540.0).abs() <= 1.0, "the recent panel starts 16 after the 420 hero, title at {}", title.left());
        for label in ["New project", "Open project", "Ctrl+N", "Ctrl+O", "Drop a project folder or project.json here"] {
            assert!(out.text_rect(label).is_some(), "the hero shows {label}");
        }
    }

    #[test]
    fn the_footer_hint_lists_only_keys_that_work() {
        let mut rig = Rig::new(&["alpha"]);
        let _ = rig.frame();
        let out = rig.frame();
        assert!(out.text_rect(footer_hint(true)).is_some());
        assert!(footer_hint(true).contains("Delete") && footer_hint(true).contains("Enter"));
        assert!(!footer_hint(false).contains("Delete") && !footer_hint(false).contains("Enter"), "no row keys while the list is empty");
        let mut empty = Rig::new(&[]);
        let _ = empty.frame();
        assert!(empty.frame().text_rect(footer_hint(false)).is_some());
    }

    #[test]
    fn a_failed_open_shows_its_message_in_the_drop_zone() {
        let mut rig = Rig::new(&["alpha"]);
        let _ = rig.frame();
        let _ = rig.key(egui::Key::ArrowDown);
        let _ = rig.key(egui::Key::Enter);
        let out = rig.frame();
        assert_eq!(opened(), vec![root_of("alpha")]);
        assert!(out.text_rect("test open").is_some(), "the open error replaces the drop hint");
        assert!(out.text_rect("Drop a project folder or project.json here").is_none());
    }

    #[test]
    fn long_folder_paths_are_cut_in_the_middle() {
        let mut h = Headless::new(400.0, 200.0);
        let font = fonts::mono(THEME.start_small_size);
        let long = "C:\\Users\\someone\\Documents\\Podcasts\\Season 4\\Episode 12 final mix\\project";
        let mut cut = String::new();
        let mut same = String::new();
        let _ = h.frame(Vec::new(), |ui| {
            cut = middle_truncate(ui.painter(), long, &font, 150.0);
            same = middle_truncate(ui.painter(), "C:\\short", &font, 150.0);
            let width = ui.painter().layout_no_wrap(cut.clone(), font.clone(), color::TEXT_TERTIARY).size().x;
            assert!(width <= 150.0, "the cut text fits: {width}");
        });
        assert!(cut.starts_with("C:\\") && cut.ends_with("project") && cut.contains('\u{2026}'), "head and tail stay: {cut}");
        assert_eq!(same, "C:\\short", "short paths are untouched");
    }

    #[test]
    fn only_project_files_and_project_folders_are_valid_drops() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("project.json");
        std::fs::write(&file, "{}").expect("write");
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, "x").expect("write");
        assert!(is_project_path(&file) && is_project_path(dir.path()));
        assert!(!is_project_path(&other) && !is_project_path(&dir.path().join("missing")));
        let hover = |path: Option<PathBuf>| vec![egui::HoveredFile { path, mime: String::new() }];
        assert!(valid_drop_hover(&hover(Some(file))));
        assert!(!valid_drop_hover(&hover(Some(other))));
        assert!(!valid_drop_hover(&hover(None)), "an unknown path does not highlight");
        assert!(!valid_drop_hover(&[]));
    }

    #[test]
    fn today_shows_the_clock_time() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 14, 30, 0).unwrap();
        let ts = Local.with_ymd_and_hms(2026, 10, 16, 14, 14, 0).unwrap().timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Today 2:14 PM".to_owned()));
    }

    #[test]
    fn yesterday_shows_no_time() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 0, 30, 0).unwrap();
        let ts = Local.with_ymd_and_hms(2026, 10, 15, 23, 59, 0).unwrap().timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Yesterday".to_owned()));
    }

    #[test]
    fn this_year_shows_month_and_day() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let ts = Local.with_ymd_and_hms(2026, 10, 9, 8, 0, 0).unwrap().timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Oct 9".to_owned()));
    }

    #[test]
    fn other_years_include_the_year() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let ts = Local.with_ymd_and_hms(2025, 10, 9, 8, 0, 0).unwrap().timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Oct 9, 2025".to_owned()));
    }

    #[test]
    fn missing_timestamp_formats_to_none() {
        assert_eq!(format_last_opened(None), None);
    }
}
