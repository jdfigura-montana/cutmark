//! ui::start_screen: the start screen: a centered panel with the app mark, a
//! type.title heading, the New and Open buttons and the recent projects as
//! ui-kit list rows, over a ui-kit status bar. The window chrome and the 52px
//! main toolbar above the panel are drawn by the app (app.rs).

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local};
use eframe::egui;
use egui::UiBuilder;
use ui_kit::tokens::{size, space, typography};

use crate::editor::{Editor, OpenOutcome, RecentEntry, recent_projects, remove_missing_recent, remove_recent};
use crate::ui::dialogs::new_project::NewProjectDialog;
use crate::ui::dialogs::{self, FooterButton};
use crate::ui::menu;
use crate::ui::status_bar;
use crate::ui::theme::{THEME, Theme};

/// Start screen: New, Open and the recent projects list.
#[derive(Default)]
pub struct StartScreen {
    /// The New Project dialog, when open.
    dialog: Option<NewProjectDialog>,
    /// Cached recent list; None means it must be loaded from disk.
    recent: Option<Vec<RecentEntry>>,
    /// Last open error, shown under the buttons.
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

        // The 26 px status bar under the centered panel.
        egui::Panel::bottom("start_status_bar")
            .exact_size(THEME.status_bar_h)
            .frame(egui::Frame::new().fill(THEME.bar_bg).inner_margin(egui::Margin { left: 8, ..Default::default() }))
            .show(ui, |ui| {
                status_bar::show_start(ui, output_device);
            });

        egui::CentralPanel::default().frame(egui::Frame::new().fill(THEME.app_bg)).show(ui, |ui| {
            self.show_panel(ui, &mut opened);
        });
        self.show_recovery_modal(ui, &mut opened);
        opened
    }

    /// The centered panel, about 560 wide: app mark, title, New/Open, drop
    /// hint and the recent projects.
    fn show_panel(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        if self.recent.is_none() {
            self.recent = Some(recent_projects());
        }
        let entries = self.recent.clone().unwrap_or_default();
        if let Some(s) = self.sel {
            self.sel = Some(s.min(entries.len().saturating_sub(1)));
        }
        self.handle_list_keys(ui, &entries);

        let avail = ui.max_rect();
        let w = THEME.start_panel_w.min((avail.width() - 2.0 * THEME.start_card_margin).max(THEME.start_panel_w / 2.0));
        let h = THEME.start_panel_h.min((avail.height() - 2.0 * THEME.start_card_margin).max(THEME.start_panel_h / 2.0));
        let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            egui::Frame::new()
                .fill(THEME.panel_bg)
                .stroke(egui::Stroke::new(THEME.hairline, THEME.border))
                .corner_radius(THEME.card_radius)
                .shadow(THEME.card_shadow)
                .inner_margin(THEME.card_pad)
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(w - 2.0 * (THEME.hairline + THEME.card_pad), h - 2.0 * (THEME.hairline + THEME.card_pad)));
                    self.show_body(ui, &entries, opened);
                });
        });
    }

    /// Arrow keys move the keyboard selection through the recent list; Enter
    /// opens the selected row (deferred a frame, see `enter_open`).
    fn handle_list_keys(&mut self, ui: &egui::Ui, entries: &[RecentEntry]) {
        if self.dialog.is_some() || self.recovery.is_some() || entries.is_empty() {
            return;
        }
        let (up, down, enter) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter)));
        let last = entries.len() - 1;
        if up {
            self.sel = Some(match self.sel {
                Some(s) => s.saturating_sub(1),
                None => last,
            });
        } else if down {
            self.sel = Some(match self.sel {
                Some(s) => (s + 1).min(last),
                None => 0,
            });
        } else if enter {
            self.enter_open = self.sel.is_some();
        }
    }

    /// Opens the keyboard-selected recent row, if any.
    fn open_selected(&mut self, opened: &mut Option<Editor>) {
        if self.recent.is_none() {
            self.recent = Some(recent_projects());
        }
        let entries = self.recent.clone().unwrap_or_default();
        if let Some(s) = self.sel
            && let Some(entry) = entries.get(s.min(entries.len().saturating_sub(1)))
        {
            let root = entry.root.clone();
            self.try_open(root, opened);
        }
    }

    /// The panel body: the app mark and title, the New and Open buttons, the
    /// drop hint (or the last open error) and the recent projects list.
    fn show_body(&mut self, ui: &mut egui::Ui, entries: &[RecentEntry], opened: &mut Option<Editor>) {
        menu::app_mark(ui);
        ui.add_space(space::SCALE_8);
        ui.label(egui::RichText::new(crate::app::APP_SHORT_NAME).font(Theme::semibold(typography::TITLE_SIZE)).color(THEME.text_primary));
        ui.add_space(space::SCALE_16);
        ui.horizontal(|ui| {
            if ui_kit::primary_button(ui, "New").clicked() {
                self.dialog = Some(NewProjectDialog::new_open());
            }
            ui.add_space(space::SCALE_4);
            if secondary_button(ui, "Open").clicked() {
                self.open_project_file(opened);
            }
        });
        ui.add_space(space::SCALE_4);
        match &self.error {
            Some(error) => ui.label(egui::RichText::new(error.clone()).font(Theme::medium(typography::CAPTION_SIZE)).color(THEME.error_soft)),
            None => ui.label(
                egui::RichText::new("Or drop a project folder or project.json here").font(Theme::medium(typography::CAPTION_SIZE)).color(THEME.text_faint),
            ),
        };
        ui.add_space(space::SCALE_16);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Recent projects").font(Theme::medium(typography::CAPTION_SIZE)).color(THEME.text_muted));
            ui.label(egui::RichText::new(format!("{} of 10", entries.len())).font(Theme::mono(typography::VALUE_SIZE)).color(THEME.text_faint));
            let missing = entries.iter().filter(|e| !e.exists()).count();
            if missing > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link(egui::RichText::new("Clear missing").font(Theme::medium(typography::CAPTION_SIZE)).color(THEME.text_muted)).clicked() {
                        remove_missing_recent();
                        self.recent = None;
                    }
                });
            }
        });
        ui.add_space(space::SCALE_4);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = space::SCALE_2;
            if entries.is_empty() {
                ui.label(egui::RichText::new("No recent projects yet").font(Theme::medium(typography::CAPTION_SIZE)).color(THEME.text_faint));
            } else {
                let sel = self.sel;
                for (i, entry) in entries.iter().enumerate() {
                    self.show_row(ui, i, entry, sel, opened);
                }
            }
        });
    }

    /// One recent-project list row: the project name with the last-opened
    /// date (or "Missing") as its trailing value and a remove button beside
    /// it. Clicking the row opens the project.
    fn show_row(&mut self, ui: &mut egui::Ui, index: usize, entry: &RecentEntry, sel: Option<usize>, opened: &mut Option<Editor>) {
        let trailing = if entry.exists() { format_last_opened(entry.last_opened).unwrap_or_default() } else { "Missing".to_owned() };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::SCALE_2;
            let row_w = ui.available_width() - size::ICON_BUTTON - space::SCALE_2;
            let response = ui
                .allocate_ui(egui::vec2(row_w, size::LIST_ROW_HEIGHT), |ui| ui_kit::list_row(ui, sel == Some(index), None, &entry.name, Some(&trailing)))
                .inner;
            if ui_kit::icon_button(ui, ui_kit::icons::regular::X, "Remove from list").clicked() {
                remove_recent(&entry.root);
                self.recent = None;
            }
            if response.clicked() {
                self.sel = Some(index);
                let root = entry.root.clone();
                self.try_open(root, opened);
            }
        });
    }

    /// Files dropped on the start screen: a project folder or project.json
    /// opens the project, anything else shows the error under the buttons.
    fn handle_drops(&mut self, ui: &egui::Ui, opened: &mut Option<Editor>) {
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if let Some(path) = dropped.first() {
            let is_project_file = path.is_file() && path.file_name().is_some_and(|n| n == std::ffi::OsStr::new("project.json"));
            let is_project_dir = path.is_dir() && path.join("project.json").is_file();
            if is_project_file || is_project_dir {
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
        match Editor::open_with_recovery_check(&path) {
            Ok(OpenOutcome::Opened(editor)) => {
                self.recent = None;
                *opened = Some(*editor);
            }
            Ok(OpenOutcome::NeedsRecoveryChoice(path)) => self.recovery = Some(path),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

/// The secondary button: a control-fill capsule with a hairline stroke and
/// `text.secondary` medium type.body text, `bg.hover` on hover; the same 32 px
/// height and 16 px padding as the ui-kit primary button.
fn secondary_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), Theme::medium(typography::BODY_SIZE), THEME.text_normal);
    let height = size::TOOLBAR_BUTTON;
    let width = galley.size().x + 2.0 * space::SCALE_16;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let p = ui.painter();
    let fill = if response.hovered() { THEME.control_hover } else { THEME.control_fill };
    p.rect_filled(rect, height / 2.0, fill);
    p.rect_stroke(rect, height / 2.0, egui::Stroke::new(THEME.hairline, THEME.control_border), egui::StrokeKind::Inside);
    p.galley(rect.center() - galley.size() / 2.0, galley, THEME.text_normal);
    response
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
    use chrono::TimeZone;

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
