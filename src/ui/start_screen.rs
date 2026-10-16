//! ui::start_screen: the start screen (docs/ui-start.png): a centered card
//! with the New/Open buttons and drop zone on the left and the recent
//! projects list on the right, over a 26 px status bar.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration, Local};
use eframe::egui;
use egui::UiBuilder;
use egui_phosphor::fill;

use crate::editor::{
    Editor, OpenOutcome, RecentEntry, recent_projects, remove_missing_recent, remove_recent,
};
use crate::ui::dialogs::new_project::NewProjectDialog;
use crate::ui::dialogs::{self, FooterButton};
use crate::ui::status_bar;
use crate::ui::theme::{THEME, Theme};

/// Start screen: New Project, Open Project and the Recent Projects list.
#[derive(Default)]
pub struct StartScreen {
    /// The New Project dialog, when open.
    dialog: Option<NewProjectDialog>,
    /// Cached recent list; None means it must be loaded from disk.
    recent: Option<Vec<RecentEntry>>,
    /// Last open error, shown inside the drop zone.
    error: Option<String>,
    /// True when the Project > Open picker must be shown on the next frame.
    pending_open: bool,
    /// Project path awaiting an autosave-recovery decision, if any.
    recovery: Option<PathBuf>,
}

impl StartScreen {
    /// Creates the start screen with the New Project dialog already open.
    pub fn with_new_dialog() -> Self {
        StartScreen {
            dialog: Some(NewProjectDialog::new_open()),
            ..StartScreen::default()
        }
    }

    /// Creates the start screen showing the Open Project picker next frame.
    pub fn with_open_request() -> Self {
        StartScreen {
            pending_open: true,
            ..StartScreen::default()
        }
    }

    /// True when the New Project dialog is currently open.
    pub fn dialog_is_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Draws the start screen; returns the Editor to edit when one opens.
    pub fn show(&mut self, ui: &mut egui::Ui, output_device: Option<&str>) -> Option<Editor> {
        let mut opened = None;
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

        // The 26 px status bar; no menu bar or toolbar on the start screen.
        egui::Panel::bottom("start_status_bar")
            .exact_size(THEME.status_bar_h)
            .frame(
                egui::Frame::new()
                    .fill(THEME.bar_bg)
                    .inner_margin(egui::Margin {
                        left: 8,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                status_bar::show_start(ui, output_device);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(THEME.app_bg))
            .show(ui, |ui| {
                self.show_card(ui, &mut opened);
            });
        self.show_recovery_modal(ui, &mut opened);
        opened
    }

    /// The centered 1000x600 card, shrunk in width when the window is narrow.
    fn show_card(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        let avail = ui.max_rect();
        let max_w = (avail.width() - 2.0 * THEME.start_card_margin).max(THEME.start_card_min_w);
        let w = THEME.start_card_w.min(max_w);
        let h = THEME.start_card_h;
        let rect = egui::Rect::from_center_size(avail.center(), egui::vec2(w, h));
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            egui::Frame::new()
                .fill(THEME.panel_bg)
                .stroke(egui::Stroke::new(THEME.hairline, THEME.border))
                .corner_radius(THEME.start_card_radius)
                .shadow(THEME.card_shadow)
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(
                        w - 2.0 * THEME.hairline,
                        h - 2.0 * THEME.hairline,
                    ));
                    let inner = ui.max_rect();
                    let left = egui::Rect::from_min_size(
                        inner.min,
                        egui::vec2(THEME.start_col_w, inner.height()),
                    );
                    let right = egui::Rect::from_min_max(
                        egui::pos2(inner.min.x + THEME.start_col_w, inner.min.y),
                        egui::pos2(inner.max.x, inner.max.y),
                    );
                    let p = ui.painter();
                    p.rect_filled(left, 0.0, THEME.ruler_bg);
                    p.line_segment(
                        [left.right_top(), left.right_bottom()],
                        egui::Stroke::new(THEME.hairline, THEME.border),
                    );
                    ui.scope_builder(
                        UiBuilder::new().max_rect(egui::Rect::from_min_max(
                            egui::pos2(
                                left.left() + THEME.start_pad_side,
                                left.top() + THEME.start_pad_top,
                            ),
                            egui::pos2(
                                left.right() - THEME.start_pad_side,
                                left.bottom() - THEME.start_pad_bottom,
                            ),
                        )),
                        |ui| self.show_left(ui, opened),
                    );
                    ui.scope_builder(
                        UiBuilder::new().max_rect(egui::Rect::from_min_max(
                            egui::pos2(
                                right.left() + THEME.start_pad_r_side,
                                right.top() + THEME.start_pad_r_top,
                            ),
                            egui::pos2(
                                right.right() - THEME.start_pad_r_side,
                                right.bottom() - THEME.start_pad_r_bottom,
                            ),
                        )),
                        |ui| self.show_right(ui, opened),
                    );
                });
        });
    }

    /// The left column: logo, title, buttons and drop zone.
    fn show_left(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        let p = ui.painter().clone();
        // Logo: 20 rounded bars, vertically centered in a 40 px box.
        let (logo, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), THEME.start_logo_h),
            egui::Sense::hover(),
        );
        for (i, &bh) in THEME.start_bar_heights.iter().enumerate() {
            let x = logo.left() + i as f32 * THEME.start_bar_pitch;
            let bar = egui::Rect::from_center_size(
                egui::pos2(x + THEME.start_bar_w / 2.0, logo.center().y),
                egui::vec2(THEME.start_bar_w, bh),
            );
            p.rect_filled(bar, THEME.start_bar_w / 2.0, THEME.track_color(i / 3));
        }
        ui.add_space(THEME.start_title_gap);
        ui.label(
            egui::RichText::new("Audio Editor")
                .font(Theme::semibold(THEME.start_title_size))
                .color(THEME.text_primary),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} \u{B7} 64-bit \u{B7} WASAPI",
                env!("CARGO_PKG_VERSION")
            ))
            .font(Theme::mono(THEME.start_small_size))
            .color(THEME.text_faint),
        );
        ui.add_space(THEME.start_buttons_gap);
        self.big_button(ui, true, fill::PLUS, "New Project", "Ctrl+N", opened);
        ui.add_space(THEME.start_btn_gap);
        self.big_button(
            ui,
            false,
            fill::FOLDER_OPEN,
            "Open Project\u{2026}",
            "Ctrl+O",
            opened,
        );
        ui.add_space(THEME.start_drop_gap);
        self.show_drop_zone(ui);
    }

    /// The dashed drop zone, sized so the two footnotes sit at the bottom.
    fn show_drop_zone(&mut self, ui: &mut egui::Ui) {
        let p = ui.painter().clone();
        let hint = "Or drop a project folder or project.json here";
        let foot1 = "New projects default to 48 kHz, 24-bit export.";
        let foot2 = "Sample rate is fixed after creation.";
        let hint_font = egui::FontId::proportional(THEME.start_hint_size);
        let g1 = p.layout_no_wrap(foot1.to_owned(), hint_font.clone(), THEME.text_faint);
        let g2 = p.layout_no_wrap(foot2.to_owned(), hint_font.clone(), THEME.text_faint);
        let foot_h = g1.size().y + THEME.space_sm + g2.size().y;
        let dz_h = (ui.available_height() - foot_h - THEME.space_md).max(THEME.space_xxl);
        let (dz, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), dz_h), egui::Sense::hover());
        p.rect_filled(
            dz,
            THEME.start_drop_radius,
            THEME.control_fill.gamma_multiply(0.4),
        );
        dashed_rect(&p, dz, THEME.start_drop_radius, THEME.control_border);
        let (text, color) = match &self.error {
            Some(error) => (error.clone(), THEME.error_soft),
            None => (hint.to_owned(), THEME.text_faint),
        };
        p.text(
            egui::pos2(dz.left() + THEME.space_lg, dz.top() + THEME.space_lg),
            egui::Align2::LEFT_TOP,
            text,
            hint_font.clone(),
            color,
        );
        ui.add_space(THEME.space_md);
        for foot in [foot1, foot2] {
            ui.label(
                egui::RichText::new(foot)
                    .font(hint_font.clone())
                    .color(THEME.text_faint),
            );
        }
    }

    /// One full-width 52 px start-screen button, drawn with the painter.
    fn big_button(
        &mut self,
        ui: &mut egui::Ui,
        accent: bool,
        icon: &str,
        label: &str,
        hint: &str,
        opened: &mut Option<Editor>,
    ) {
        let t = &*THEME;
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), t.start_btn_h),
            egui::Sense::click(),
        );
        let (fill, text_col) = if accent {
            (
                if resp.hovered() {
                    t.accent_hover
                } else {
                    t.accent
                },
                t.start_on_accent,
            )
        } else {
            (
                if resp.hovered() {
                    t.control_hover
                } else {
                    t.control_fill
                },
                t.text_normal,
            )
        };
        let p = ui.painter();
        p.rect_filled(rect, t.start_btn_radius, fill);
        if !accent {
            p.rect_stroke(
                rect,
                t.start_btn_radius,
                egui::Stroke::new(t.hairline, t.control_border),
                egui::StrokeKind::Inside,
            );
        }
        let cy = rect.center().y;
        let label_x = rect.left() + t.space_lg + t.start_icon_size + t.space_md;
        p.text(
            egui::pos2(rect.left() + t.space_lg, cy),
            egui::Align2::LEFT_CENTER,
            icon,
            egui::FontId::proportional(t.start_icon_size),
            text_col,
        );
        let label_font = if accent {
            Theme::semibold(t.start_btn_text_size)
        } else {
            Theme::medium(t.start_btn_text_size)
        };
        p.text(
            egui::pos2(label_x, cy),
            egui::Align2::LEFT_CENTER,
            label,
            label_font,
            text_col,
        );
        p.text(
            egui::pos2(rect.right() - t.space_lg, cy),
            egui::Align2::RIGHT_CENTER,
            hint,
            Theme::mono(t.start_small_size),
            if accent { text_col } else { t.text_faint },
        );
        if resp.clicked() {
            if accent {
                self.dialog = Some(NewProjectDialog::new_open());
            } else {
                self.open_project_file(opened);
            }
        }
    }

    /// The right column: the recent projects header and scrollable list.
    fn show_right(&mut self, ui: &mut egui::Ui, opened: &mut Option<Editor>) {
        if self.recent.is_none() {
            self.recent = Some(recent_projects());
        }
        let entries = self.recent.clone().unwrap_or_default();
        let missing = entries.iter().filter(|e| !e.exists()).count();

        // Header row: title, count, optional "Clear missing" on the right.
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), THEME.start_header_h),
            egui::Sense::hover(),
        );
        let p = ui.painter().clone();
        let title_font = egui::FontId::proportional(THEME.start_small_size);
        p.text(
            egui::pos2(row.left(), row.center().y),
            egui::Align2::LEFT_CENTER,
            "RECENT PROJECTS",
            title_font.clone(),
            THEME.text_muted,
        );
        let title_w = p
            .layout_no_wrap("RECENT PROJECTS".to_owned(), title_font, THEME.text_muted)
            .size()
            .x;
        p.text(
            egui::pos2(row.left() + title_w + THEME.space_md, row.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} of 10", entries.len()),
            Theme::mono(THEME.start_small_size),
            THEME.text_faint,
        );
        if missing > 0 {
            self.clear_missing_button(ui, row);
        }
        ui.add_space(THEME.space_md);
        let (line, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), THEME.hairline),
            egui::Sense::hover(),
        );
        p.line_segment(
            [line.left_center(), line.right_center()],
            egui::Stroke::new(THEME.hairline, THEME.border),
        );
        ui.add_space(THEME.space_sm);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = THEME.start_row_gap;
            if entries.is_empty() {
                let (empty, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 3.0 * THEME.start_row_h),
                    egui::Sense::hover(),
                );
                p.text(
                    empty.center(),
                    egui::Align2::CENTER_CENTER,
                    "No recent projects yet",
                    egui::FontId::proportional(THEME.start_hint_size),
                    THEME.text_faint,
                );
            } else {
                for (i, entry) in entries.iter().enumerate() {
                    self.show_row(ui, i, entry, opened);
                }
            }
        });
    }

    /// The right-aligned "Clear missing" text button in the header row.
    fn clear_missing_button(&mut self, ui: &mut egui::Ui, row: egui::Rect) {
        let p = ui.painter().clone();
        let label = "Clear missing";
        let font = egui::FontId::proportional(THEME.start_hint_size);
        let bw = p
            .layout_no_wrap(label.to_owned(), font.clone(), THEME.text_muted)
            .size()
            .x
            + 2.0 * THEME.space_sm;
        let brect = egui::Rect::from_min_size(
            egui::pos2(row.right() - bw, row.top()),
            egui::vec2(bw, row.height()),
        );
        let resp = ui.interact(
            brect,
            egui::Id::new("start_clear_missing"),
            egui::Sense::click(),
        );
        let color = if resp.hovered() {
            THEME.text_primary
        } else {
            THEME.text_muted
        };
        p.text(
            brect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            font,
            color,
        );
        if resp.clicked() {
            remove_missing_recent();
            self.recent = None;
        }
    }

    /// One 64 px recent-project row: tile, name, path, date and remove button.
    fn show_row(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        entry: &RecentEntry,
        opened: &mut Option<Editor>,
    ) {
        let t = &*THEME;
        let p = ui.painter().clone();
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), t.start_row_h),
            egui::Sense::click(),
        );
        let exists = entry.exists();
        let dim = |c: egui::Color32| if exists { c } else { c.gamma_multiply(0.7) };
        if resp.hovered() {
            p.rect_filled(rect, t.start_row_radius, t.control_fill);
        }

        // Tile: track color at low alpha, or grey for a missing project.
        let tile = egui::Rect::from_min_size(
            egui::pos2(
                rect.left() + t.space_md + t.space_xs,
                rect.center().y - t.start_tile / 2.0,
            ),
            egui::vec2(t.start_tile, t.start_tile),
        );
        let tc = t.track_color(index);
        let (tile_fill, tile_border, icon_col) = if exists {
            (
                egui::Color32::from_rgba_unmultiplied(tc.r(), tc.g(), tc.b(), 0x1F),
                egui::Color32::from_rgba_unmultiplied(tc.r(), tc.g(), tc.b(), 0x55),
                tc,
            )
        } else {
            (t.lane_b, t.control_border, t.start_missing_icon)
        };
        p.rect_filled(tile, t.start_row_radius, tile_fill);
        p.rect_stroke(
            tile,
            t.start_row_radius,
            egui::Stroke::new(t.hairline, tile_border),
            egui::StrokeKind::Inside,
        );
        p.text(
            tile.center(),
            egui::Align2::CENTER_CENTER,
            fill::WAVEFORM,
            egui::FontId::proportional(t.start_icon_size),
            icon_col,
        );

        // Fixed right-side columns: date, then the remove button.
        let x_center = egui::pos2(
            rect.right() - t.space_md - t.start_x_btn / 2.0,
            rect.center().y,
        );
        let date_right = rect.right() - t.space_md - t.start_x_btn - t.space_md;
        let text_left = tile.right() + t.space_md + t.space_xs;
        let text_right = date_right - t.start_last_col_w;

        // Name (truncated) with a MISSING pill after it when missing.
        let name_color = dim(if exists { t.text_primary } else { t.text_faint });
        let pill = (!exists).then(|| {
            p.layout_no_wrap(
                "MISSING".to_owned(),
                Theme::semibold(t.start_pill_size),
                t.start_missing_text,
            )
        });
        let pill_w = pill.as_ref().map_or(0.0, |g| g.size().x + 2.0 * t.space_sm);
        let name_g = fit_galley(
            &p,
            &entry.name,
            Theme::medium(t.start_name_size),
            name_color,
            text_right - text_left - pill_w,
        );
        let name_y = rect.center().y - name_g.size().y - t.start_row_gap;
        p.galley(egui::pos2(text_left, name_y), name_g.clone(), name_color);
        if let Some(pill_g) = pill {
            let pill_rect = egui::Rect::from_center_size(
                egui::pos2(
                    text_left + name_g.size().x + t.space_sm + pill_g.size().x / 2.0 + t.space_sm,
                    name_y + name_g.size().y / 2.0,
                ),
                egui::vec2(
                    pill_g.size().x + 2.0 * t.space_sm,
                    pill_g.size().y + 2.0 * t.space_xs,
                ),
            );
            p.rect_filled(pill_rect, 9.0, t.start_missing_fill);
            p.galley(
                egui::pos2(
                    pill_rect.left() + t.space_sm,
                    pill_rect.center().y - pill_g.size().y / 2.0,
                ),
                pill_g,
                t.start_missing_text,
            );
        }

        // Path, home-relative, truncated, never wrapping.
        let path_g = fit_galley(
            &p,
            &display_path(&entry.root),
            Theme::mono(t.start_small_size),
            dim(t.text_faint),
            text_right - text_left,
        );
        p.galley(
            egui::pos2(text_left, rect.center().y + t.start_row_gap),
            path_g,
            dim(t.text_faint),
        );

        // Last opened, right-aligned in its fixed 96 px column.
        if let Some(label) = format_last_opened(entry.last_opened) {
            p.text(
                egui::pos2(date_right, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                label,
                egui::FontId::proportional(t.start_hint_size),
                dim(t.text_muted),
            );
        }

        // Remove button: removes the list entry only, never touches files.
        let x_rect =
            egui::Rect::from_center_size(x_center, egui::vec2(t.start_x_btn, t.start_x_btn));
        let x_resp = ui.interact(
            x_rect,
            egui::Id::new(("start_remove", index)),
            egui::Sense::click(),
        );
        if x_resp.hovered() {
            p.rect_filled(x_rect, t.control_rounding, t.control_fill);
        }
        p.text(
            x_rect.center(),
            egui::Align2::CENTER_CENTER,
            fill::X,
            egui::FontId::proportional(t.start_hint_size),
            if x_resp.hovered() {
                t.text_primary
            } else {
                dim(t.text_faint)
            },
        );
        x_resp.clone().on_hover_text("Remove from list");
        if x_resp.clicked() {
            remove_recent(&entry.root);
            self.recent = None;
        } else if resp.clicked() {
            self.try_open(entry.root.clone(), opened);
        }
    }

    /// Files dropped on the start screen: a project folder or project.json
    /// opens the project, anything else shows the error in the drop zone.
    fn handle_drops(&mut self, ui: &egui::Ui, opened: &mut Option<Editor>) {
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.first() {
            let is_project_file = path.is_file()
                && path
                    .file_name()
                    .is_some_and(|n| n == std::ffi::OsStr::new("project.json"));
            let is_project_dir = path.is_dir() && path.join("project.json").is_file();
            if is_project_file || is_project_dir {
                self.try_open(path.clone(), opened);
            } else {
                self.error = Some(
                    "Not an Audio Editor project: drop a project folder or a project.json file."
                        .to_owned(),
                );
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
            dialogs::description(
                ui,
                "This project has unsaved changes from a session that didn't close properly.",
            );
            dialogs::before_form(ui);
            dialogs::footer(ui, |ui| {
                if dialogs::footer_button(ui, "Recover autosave", FooterButton::Primary, true)
                    .clicked()
                {
                    choice = Some(Choice::Recover);
                }
                if dialogs::footer_button(ui, "Open last saved", FooterButton::Cancel, true)
                    .clicked()
                {
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
        let result = if recover {
            Editor::open_recovered(path)
        } else {
            Editor::open_discarding_autosave(path)
        };
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
        let picked = rfd::FileDialog::new()
            .add_filter("Project file (project.json)", &["json"])
            .pick_file();
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

/// Truncating galley: `text` in `font`, cut with an ellipsis so it fits
/// `max_w` (binary search over the kept prefix; always at least one char).
fn fit_galley(
    p: &egui::Painter,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> std::sync::Arc<egui::text::Galley> {
    let full = p.layout_no_wrap(text.to_owned(), font.clone(), color);
    if full.size().x <= max_w {
        return full;
    }
    let chars: Vec<char> = text.chars().collect();
    let fits = |keep: usize| -> bool {
        let candidate: String = chars[..keep].iter().collect::<String>() + "\u{2026}";
        p.layout_no_wrap(candidate, font.clone(), color).size().x <= max_w
    };
    let mut lo = 1usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let kept: String = chars[..lo].iter().collect::<String>() + "\u{2026}";
    p.layout_no_wrap(kept, font, color)
}

/// A dashed 1 px rectangle drawn with the painter (drop zone border).
fn dashed_rect(p: &egui::Painter, rect: egui::Rect, radius: f32, color: egui::Color32) {
    let stroke = egui::Stroke::new(THEME.hairline, color);
    let dash = THEME.start_dash;
    let gap = THEME.start_dash_gap;
    let dashed_line = |a: egui::Pos2, b: egui::Pos2| {
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut d = 0.0;
        while d < len {
            let end = (d + dash).min(len);
            p.line_segment(
                [
                    egui::pos2(a.x + ux * d, a.y + uy * d),
                    egui::pos2(a.x + ux * end, a.y + uy * end),
                ],
                stroke,
            );
            d = end + gap;
        }
    };
    dashed_line(
        egui::pos2(rect.left() + radius, rect.top()),
        egui::pos2(rect.right() - radius, rect.top()),
    );
    dashed_line(
        egui::pos2(rect.left() + radius, rect.bottom()),
        egui::pos2(rect.right() - radius, rect.bottom()),
    );
    dashed_line(
        egui::pos2(rect.left(), rect.top() + radius),
        egui::pos2(rect.left(), rect.bottom() - radius),
    );
    dashed_line(
        egui::pos2(rect.right(), rect.top() + radius),
        egui::pos2(rect.right(), rect.bottom() - radius),
    );
}

/// Shows a path relative to the user's home folder when it is inside it.
fn display_path(root: &Path) -> String {
    if let Ok(home) = std::env::var("USERPROFILE")
        && let Ok(rest) = root.strip_prefix(&home)
    {
        let rest = rest.to_string_lossy();
        if rest.is_empty() {
            return "~".to_owned();
        }
        return format!("~\\{rest}");
    }
    root.to_string_lossy().into_owned()
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
        Some(format!(
            "Today {}",
            when.format("%l:%M %p").to_string().trim_start()
        ))
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
        let ts = Local
            .with_ymd_and_hms(2026, 10, 16, 14, 14, 0)
            .unwrap()
            .timestamp();
        assert_eq!(
            format_last_opened_at(ts, now),
            Some("Today 2:14 PM".to_owned())
        );
    }

    #[test]
    fn yesterday_shows_no_time() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 0, 30, 0).unwrap();
        let ts = Local
            .with_ymd_and_hms(2026, 10, 15, 23, 59, 0)
            .unwrap()
            .timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Yesterday".to_owned()));
    }

    #[test]
    fn this_year_shows_month_and_day() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let ts = Local
            .with_ymd_and_hms(2026, 10, 9, 8, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(format_last_opened_at(ts, now), Some("Oct 9".to_owned()));
    }

    #[test]
    fn other_years_include_the_year() {
        let now = Local.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let ts = Local
            .with_ymd_and_hms(2025, 10, 9, 8, 0, 0)
            .unwrap()
            .timestamp();
        assert_eq!(
            format_last_opened_at(ts, now),
            Some("Oct 9, 2025".to_owned())
        );
    }

    #[test]
    fn missing_timestamp_formats_to_none() {
        assert_eq!(format_last_opened(None), None);
    }
}
