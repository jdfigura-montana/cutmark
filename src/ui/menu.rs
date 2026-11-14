//! ui::menu: the top Project, Edit, Process and View menus.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::ui::dialogs::audio_settings::AudioSettingsDialog;
use crate::ui::dialogs::confirm::ConfirmDialog;
use crate::ui::dialogs::export::ExportDialog;
use crate::ui::dialogs::project_settings::ProjectSettingsDialog;
use crate::ui::dialogs::save_as::SaveAsDialog;
use crate::ui::theme::{THEME, Theme};
use crate::ui::timeline::ViewMenuAction;

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
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), THEME.hairline),
        egui::Sense::hover(),
    );
    ui.painter().rect_filled(rect, 0.0, THEME.control_border);
    ui.add_space(4.0);
}

/// A menu item button with a right-aligned JetBrains Mono shortcut hint.
fn item(label: impl Into<String>, hint: &str) -> egui::Button<'static> {
    egui::Button::new(label.into()).right_text(
        egui::RichText::new(hint)
            .font(Theme::mono(11.0))
            .color(THEME.text_faint),
    )
}

/// The right end of the menu bar: the project name and, while there are
/// unsaved changes, a painted amber dot and "unsaved", reading left to right
/// ("Test 2 (dot) unsaved") and ending `menu_right_pad` before the window
/// edge. The dot is a painted circle, not a font glyph (the UI fonts have no
/// U+25CF). A name too long for the bar is truncated instead of clipped.
pub fn project_status(ui: &mut egui::Ui, name: &str, dirty: bool) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_space(THEME.menu_right_pad);
        let painter = ui.painter().clone();
        let font = egui::FontId::proportional(THEME.clip_name_size);
        let unsaved = dirty
            .then(|| painter.layout_no_wrap("unsaved".to_string(), font.clone(), THEME.text_muted));
        let dot = if dirty { THEME.unsaved_dot_d } else { 0.0 };
        let gap = THEME.space_sm;
        let tail_w = unsaved
            .as_ref()
            .map_or(0.0, |text| gap + dot + gap + text.size().x);
        let mut job =
            egui::text::LayoutJob::simple_singleline(name.to_string(), font, THEME.text_muted);
        job.wrap =
            egui::text::TextWrapping::truncate_at_width((ui.available_width() - tail_w).max(0.0));
        let name_galley = painter.layout_job(job);
        let height = unsaved
            .as_ref()
            .map_or(0.0, |text| text.size().y)
            .max(name_galley.size().y);
        // One rect for the whole readout, painted left to right inside it.
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(name_galley.size().x + tail_w, height),
            egui::Sense::hover(),
        );
        let mid = rect.center().y;
        let mut x = rect.left();
        let name_w = name_galley.size().x;
        let name_pos = egui::pos2(x, mid - name_galley.size().y * 0.5);
        painter.galley(name_pos, name_galley, THEME.text_muted);
        x += name_w;
        if let Some(text) = unsaved {
            x += gap;
            painter.circle_filled(
                egui::pos2(x + dot * 0.5, mid),
                dot * 0.5,
                THEME.marker_color,
            );
            x += dot + gap;
            let text_pos = egui::pos2(x, mid - text.size().y * 0.5);
            painter.galley(text_pos, text, THEME.text_muted);
        }
    });
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
    /// Shows the menus; returns the Project and View actions the user chose.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
    ) -> (ProjectMenuAction, ViewMenuAction) {
        let mut action = ProjectMenuAction::None;
        let mut view_action = ViewMenuAction::None;
        // One horizontal menu bar row: menus on the left, project status on
        // the right.
        egui::MenuBar::new()
            .config(
                egui::containers::menu::MenuConfig::new()
                    .style(crate::ui::theme::menu_item_style as fn(&mut egui::Style)),
            )
            .ui(ui, |ui| {
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
                    if menu.add(item("Save As...", "Ctrl+Shift+S")).clicked() {
                        self.open_save_as(editor);
                        menu.close();
                    }
                    if menu.add(item("Import Media...", "Ctrl+I")).clicked() {
                        action = ProjectMenuAction::Import;
                        menu.close();
                    }
                    if !editor.is_locked() {
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
                    if menu.button("Project Settings...").clicked() {
                        self.open_settings(editor);
                        menu.close();
                    }
                    if menu.button("Audio Settings...").clicked() {
                        self.audio_settings_dialog = Some(AudioSettingsDialog::new_open(editor));
                        menu.close();
                    }
                });
                ui.menu_button("Edit", |menu| {
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
                    let has_selection =
                        !matches!(selection, crate::editor::selection::Selection::None);
                    let can_paste = editor
                        .clipboard()
                        .is_some_and(|c| c.tracks.iter().any(|t| !t.is_empty()));
                    let can_split = {
                        let pos = editor.view.playhead;
                        let range_tracks: Option<Vec<Uuid>> = match &selection {
                            crate::editor::selection::Selection::Range { tracks, .. } => {
                                Some(tracks.clone())
                            }
                            _ => None,
                        };
                        editor.project.tracks.iter().any(|track| {
                            range_tracks
                                .as_ref()
                                .is_none_or(|ids| ids.contains(&track.id))
                                && track
                                    .clips
                                    .iter()
                                    .any(|clip| clip.start < pos && pos < clip.end())
                        })
                    };
                    let can_trim =
                        matches!(selection, crate::editor::selection::Selection::Range { .. });
                    let can_select_all =
                        !editor.project.tracks.is_empty() && editor.project_length() > 0;

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
                                    let _ =
                                        editor.apply(crate::editor::action::Action::DeleteClips {
                                            clip_ids: ids.clone(),
                                            ripple: editor.view.ripple,
                                        });
                                }
                                crate::editor::selection::Selection::Range {
                                    start,
                                    end,
                                    tracks,
                                } => {
                                    let _ =
                                        editor.apply(crate::editor::action::Action::DeleteRange {
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
                    if menu
                        .add(egui::Checkbox::new(&mut ripple, "Ripple (R)"))
                        .changed()
                    {
                        editor.toggle_ripple();
                    }
                    let mut snap = editor.view.snap;
                    if menu
                        .add(egui::Checkbox::new(&mut snap, "Snap (N)"))
                        .changed()
                    {
                        editor.toggle_snap();
                    }
                });
                ui.menu_button("Markers", |menu| {
                    if menu.add(item("Add Marker", "M")).clicked() {
                        let _ = editor.add_marker_at_playhead();
                        menu.close();
                    }
                    if menu.add(item("Previous Marker", ",")).clicked() {
                        editor.jump_prev_marker();
                        menu.close();
                    }
                    if menu.add(item("Next Marker", ".")).clicked() {
                        editor.jump_next_marker();
                        menu.close();
                    }
                    if let Some(id) = editor.selected_marker() {
                        if menu.add(item("Delete Marker", "Del")).clicked() {
                            let _ = editor.apply(crate::editor::action::Action::DeleteMarkers {
                                ids: vec![id],
                            });
                            menu.close();
                        }
                    } else {
                        menu.add_enabled(false, item("Delete Marker", "Del"));
                    }
                });
                ui.menu_button("Process", |menu| {
                    let has_target = editor.process_target().is_some();
                    let has_range = matches!(
                        editor.selection(),
                        crate::editor::selection::Selection::Range { .. }
                    );
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
                    if !matches!(
                        editor.selection(),
                        crate::editor::selection::Selection::Range { .. }
                    ) {
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
                project_status(ui, &editor.project.name, editor.dirty);
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

    /// Shows a failed Process menu operation in the error modal, with the
    /// ops prefix stripped so the SPEC's message shows verbatim.
    fn show_process_error(&mut self, error: crate::editor::editor::EditorError) {
        let text = error.to_string();
        self.error_dialog = Some(ConfirmDialog::new(
            "Process",
            text.strip_prefix("invalid operation: ")
                .unwrap_or(&text)
                .to_string(),
        ));
    }

    /// Saves the project; on failure shows the error modal.
    pub fn save(&mut self, editor: &mut Editor) {
        if let Err(error) = editor.save() {
            self.error_dialog = Some(ConfirmDialog::new("Save Error", error.to_string()));
        }
    }

    /// Opens the Save As dialog prefilled with the current name and folder.
    pub fn open_save_as(&mut self, editor: &mut Editor) {
        self.save_as_dialog = Some(SaveAsDialog::new_open(
            &editor.project.name,
            &editor.paths.root,
        ));
    }

    /// Opens the Project Settings dialog prefilled with the current settings.
    pub fn open_settings(&mut self, editor: &mut Editor) {
        self.settings_dialog = Some(ProjectSettingsDialog::new_open(
            &editor.project.name,
            editor.project.export_bit_depth,
        ));
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
        let mut editor = Editor::with_project(
            Project::new(name.to_string(), 48_000, BitDepth::Int24),
            ViewState::default(),
            ProjectPaths {
                root: PathBuf::from("unused"),
            },
        );
        editor.dirty = dirty;
        editor
    }

    /// The real menu bar, in the same 30 px panel the app uses.
    fn menu_panel(ui: &mut egui::Ui, menu: &mut ProjectMenu, editor: &mut Editor) {
        egui::Panel::top("menu_bar")
            .exact_size(30.0)
            .frame(
                egui::Frame::new()
                    .fill(THEME.bar_bg)
                    .inner_margin(egui::Margin {
                        left: 8,
                        ..Default::default()
                    }),
            )
            .show(ui, |ui| {
                let _ = menu.show(ui, editor);
            });
    }

    /// One frame of a fresh menu bar.
    fn menu_bar(h: &mut Headless, editor: &mut Editor) -> Out {
        let mut menu = ProjectMenu::default();
        h.frame(Vec::new(), |ui| menu_panel(ui, &mut menu, editor))
    }

    #[test]
    fn audio_settings_opens_from_the_project_menu_and_cancel_closes_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = editor("Test", false);
        let mut menu = ProjectMenu::default();
        let out = h.settle(|ui| menu_panel(ui, &mut menu, &mut editor));
        assert!(out.text_rect("Audio Settings").is_none());

        // Project > Audio Settings...
        let project = out.text_rect("Project").expect("Project menu").center();
        let _ = h.click(project, |ui| menu_panel(ui, &mut menu, &mut editor));
        let out = h.settle(|ui| menu_panel(ui, &mut menu, &mut editor));
        let item = out
            .text_rect("Audio Settings...")
            .expect("the menu item")
            .center();
        assert!(menu.audio_settings_dialog.is_none());
        let _ = h.click(item, |ui| menu_panel(ui, &mut menu, &mut editor));
        assert!(menu.audio_settings_dialog.is_some(), "the dialog opened");
        let out = h.settle(|ui| menu_panel(ui, &mut menu, &mut editor));
        assert!(out.text_rect("Audio Settings").is_some(), "its title");

        // Cancel closes it and leaves the config alone.
        let cancel = out.text_rect("Cancel").expect("Cancel").center();
        let _ = h.click(cancel, |ui| menu_panel(ui, &mut menu, &mut editor));
        assert!(menu.audio_settings_dialog.is_none(), "the dialog closed");
        assert_eq!(editor.config(), &crate::io::config::AppConfig::default());
        let out = h.settle(|ui| menu_panel(ui, &mut menu, &mut editor));
        assert!(out.text_rect("Audio Settings").is_none());
    }

    #[test]
    fn unsaved_readout_is_a_painted_dot_between_name_and_text() {
        for width in [1280.0_f32, 520.0] {
            let mut h = Headless::new(width, 400.0);
            let mut editor = editor("Test 2", true);
            let _ = menu_bar(&mut h, &mut editor);
            let out = menu_bar(&mut h, &mut editor);

            let dots: Vec<_> = out
                .circles()
                .into_iter()
                .filter(|c| c.fill == THEME.marker_color)
                .collect();
            assert_eq!(dots.len(), 1, "one amber dot at width {width}");
            let dot = dots[0];
            assert_eq!(dot.radius * 2.0, THEME.unsaved_dot_d, "6 px dot");

            let name = out.text_rect("Test 2").expect("project name");
            let unsaved = out.text_rect("unsaved").expect("unsaved text");
            assert!(
                name.right() <= dot.center.x - dot.radius + EPS,
                "name left of the dot"
            );
            assert!(
                dot.center.x + dot.radius <= unsaved.left() + EPS,
                "dot left of the text"
            );
            assert!(
                (unsaved.right() - (width - THEME.menu_right_pad)).abs() < 1.0,
                "8 px right padding: text ends at {} of {width}",
                unsaved.right()
            );
            assert!(
                unsaved.right() <= width - THEME.menu_right_pad + EPS,
                "nothing runs past the padding"
            );
            assert!(
                out.texts().iter().all(|t| !t.text.contains('\u{25CF}')),
                "the dot is painted, never a glyph"
            );
        }
    }

    #[test]
    fn clean_project_shows_only_the_name() {
        let mut h = Headless::new(1280.0, 400.0);
        let mut editor = editor("Test 2", false);
        let _ = menu_bar(&mut h, &mut editor);
        let out = menu_bar(&mut h, &mut editor);
        assert!(out.text_rect("Test 2").is_some());
        assert!(out.text_rect("unsaved").is_none());
        assert!(out.circles().iter().all(|c| c.fill != THEME.marker_color));
    }

    #[test]
    fn a_long_name_is_truncated_not_clipped() {
        let mut h = Headless::new(520.0, 400.0);
        let long = "A very long project name that cannot fit the menu bar";
        let mut editor = editor(long, true);
        let _ = menu_bar(&mut h, &mut editor);
        let out = menu_bar(&mut h, &mut editor);
        let unsaved = out.text_rect("unsaved").expect("unsaved text");
        assert!(unsaved.right() <= 520.0 - THEME.menu_right_pad + EPS);
        let name = out.text_rect(long).expect("project name");
        let full_w = h.ctx.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(
                    long.to_string(),
                    egui::FontId::proportional(THEME.clip_name_size),
                    THEME.text_muted,
                )
                .size()
                .x
        });
        assert!(
            name.width() < full_w - 20.0,
            "name is shortened to {} from {full_w}",
            name.width()
        );
        let dot = out
            .circles()
            .into_iter()
            .find(|c| c.fill == THEME.marker_color)
            .expect("amber dot");
        assert!(name.right() <= dot.center.x - dot.radius + EPS);
    }
}
