//! The Review mode panels (spec "Review mode"): the takes list on the left,
//! the take's picture in the center and the take info on the right.
//!
//! Playback arrives with R9, so the center shows a placeholder with the take's
//! name: media-io can decode live frames but offers no cheap one-frame decode
//! of a file, so there is no thumbnail yet. Clicking a mark does nothing for
//! the same reason; the seek comes with the playback. The Actions group and
//! the delete confirmation dialog report through [`TakeAction`] and the app
//! does the file work.

use egui::{Align, Button, Color32, Layout, Rect, RichText, Ui, vec2};
use marks::MarkType;
use ui_kit::tokens::{color, radius, size, space, typography};
use ui_kit::{MarkerKind, inspector_group, inspector_row, marker_color};

use crate::review::{self, Day, Take};

/// Left padding of a panel header's title, the same inset the app's other
/// headers use.
const HEADER_PAD: f32 = 6.0;

/// The 1px hairline stroke width.
const HAIRLINE: f32 = 1.0;

/// What the takes panel's header buttons did this frame.
#[derive(Debug, PartialEq, Eq)]
pub enum TakesAction {
    /// Nothing was clicked.
    None,
    /// Open the project folder.
    OpenFolder,
    /// Rescan the takes.
    Refresh,
}

/// What the take panel's Actions group did this frame (spec "Review mode").
#[derive(Debug, PartialEq)]
pub enum TakeAction {
    /// Nothing was clicked.
    None,
    /// Open Explorer with the take's main file selected.
    ShowInFolder,
    /// Confirm the rename to this name; the app validates it and renames every file.
    Rename(String),
}

/// The take panel's editing state: the open rename field and the delete dialog.
#[derive(Default)]
pub struct TakePanelState {
    /// The rename buffer while the take's name is being edited.
    pub renaming: Option<String>,
    /// Whether the rename field already took focus, so focus is only requested once.
    rename_focused: bool,
    /// The reason the last rename was refused, shown under the field.
    pub rename_error: Option<String>,
    /// Whether the delete confirmation dialog is open.
    pub confirm_delete: bool,
}

impl TakePanelState {
    /// Closes the rename field and clears its reason, after a rename or when the take goes away.
    pub fn close_rename(&mut self) {
        self.renaming = None;
        self.rename_focused = false;
        self.rename_error = None;
    }
}

/// The left Takes panel (spec "Review mode"): the header with the folder and
/// refresh buttons, then the days with their takes, newest first. Returns
/// what the header buttons did.
pub fn takes_panel(ui: &mut Ui, days: &[Day], selected: &mut Option<String>) -> TakesAction {
    let action = takes_header(ui);
    ui.add_space(space::SCALE_8);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        let mut picked = None;
        for day in days {
            group_caption(ui, &day.date.format("%Y-%m-%d").to_string());
            for take in &day.takes {
                let trailing = take_row_trailing(take);
                let is_selected = selected.as_deref() == Some(take.name.as_str());
                if ui_kit::list_row(ui, is_selected, None, &take.name, Some(&trailing)).clicked() {
                    picked = Some(take.name.clone());
                }
            }
            ui.add_space(space::SCALE_8);
        }
        if let Some(name) = picked {
            *selected = Some(name);
        }
    });
    action
}

/// The takes panel's header row (44 tall): "Takes" and the open-folder and
/// refresh icon buttons on the right, with the hairline along the bottom of
/// the row.
fn takes_header(ui: &mut Ui) -> TakesAction {
    let mut action = TakesAction::None;
    let row = vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.add(egui::Label::new(RichText::new("Takes").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY)).selectable(false));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Right to left, so the pair still reads "open folder, refresh" left to right.
            if ui_kit::icon_button(ui, ui_kit::icons::regular::ARROWS_CLOCKWISE, "Refresh takes").clicked() {
                action = TakesAction::Refresh;
            }
            if ui_kit::icon_button(ui, ui_kit::icons::regular::FOLDER_OPEN, "Open project folder").clicked() {
                action = TakesAction::OpenFolder;
            }
        });
    });
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    action
}

/// The trailing caption of a take row: the start time, the mark count and the
/// duration when the sidecar knows it.
fn take_row_trailing(take: &Take) -> String {
    let mut trailing = format!("{} \u{b7} {} marks", take.start.format("%H:%M"), take.marks.len());
    if let Some(duration) = take.duration_ms {
        trailing.push_str(&format!(" \u{b7} {}", review::format_duration(duration)));
    }
    trailing
}

/// A group caption ("2026-11-21", "File", "Marks") in the caption style.
fn group_caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY)).selectable(false));
}

/// The Review center (spec "Review mode"): the take's first frame letterboxed
/// in 16:9. Until R9 the placeholder shows the take's name, because media-io
/// offers no cheap one-frame decode of a file.
pub fn center(ui: &mut Ui, days: &[Day], selected: Option<&str>) {
    let available = ui.available_rect_before_wrap();
    let Some(take) = review::find(days, selected) else {
        placeholder_caption(ui, available.center(), "No take selected");
        return;
    };
    let box_rect = fit_16_9(available);
    let painter = ui.painter();
    painter.rect_filled(box_rect, radius::GROUP, color::BG_GROUND);
    // The name wraps inside the box instead of spilling over its edges.
    let galley =
        painter.layout(take.name.clone(), ui_kit::fonts::medium(typography::BODY_SIZE), color::TEXT_TERTIARY, box_rect.width() - 2.0 * space::SCALE_16);
    painter.galley(box_rect.center() - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
}

/// A centered tertiary caption where the picture would be.
fn placeholder_caption(ui: &Ui, at: egui::Pos2, text: &str) {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), ui_kit::fonts::medium(typography::CAPTION_SIZE), color::TEXT_TERTIARY);
    ui.painter().galley(at - galley.size() / 2.0, galley, color::TEXT_TERTIARY);
}

/// The largest 16:9 box that fits in `available`, centered in it.
fn fit_16_9(available: Rect) -> Rect {
    const ASPECT: f32 = 16.0 / 9.0;
    let width = available.width().min(available.height() * ASPECT).max(0.0);
    Rect::from_center_size(available.center(), vec2(width, width / ASPECT))
}

/// The right take info panel (spec "Review mode"): the header with the take's
/// name and date, then the File group, the sidecar's marks and the Actions
/// group at the bottom. Returns what the Actions group or the rename field
/// did; `recording` disables Rename and Delete while a take runs.
pub fn take_panel(ui: &mut Ui, days: &[Day], selected: Option<&str>, state: &mut TakePanelState, recording: bool) -> TakeAction {
    let take = review::find(days, selected);
    let mut action = take_header(ui, take, state).map_or(TakeAction::None, TakeAction::Rename);
    ui.add_space(space::SCALE_8);
    let Some(take) = take else { return action };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.set_max_width(ui.available_width());
        group_caption(ui, "File");
        inspector_group(ui, |ui| {
            let duration = take.duration_ms.map(review::format_duration).unwrap_or_else(|| "\u{2014}".to_owned());
            row(ui, "Duration", &duration);
            // The hover names every file of the take with its own size.
            let file_list = take
                .files
                .iter()
                .map(|file| {
                    let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    format!("{name} ({})", review::format_size(file.size))
                })
                .collect::<Vec<_>>()
                .join("\n");
            row(ui, "Size", &review::format_size(take.total_size)).on_hover_text(file_list);
            row(ui, "Video format", &format_label(&take.video));
            if let Some(iso) = &take.iso {
                row(ui, "Camera ISO format", &format_label(iso));
            }
        });
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Marks");
        if take.marks.is_empty() {
            placeholder_caption(ui, ui.cursor().left_top() + egui::vec2(ui.available_width() / 2.0, 10.0), "No marks");
            ui.add_space(20.0);
        }
        // Clicking a mark seeks to it only with R9's playback, so the row
        // carries no action yet.
        for mark in &take.marks {
            let label = mark.label.clone().unwrap_or_else(|| type_name(mark.mark_type, mark.original_type.as_deref()));
            let trailing = marks::format_tc(mark.t_ms);
            let _ = ui_kit::list_row(ui, false, Some(mark_dot(mark.mark_type)), &label, Some(&trailing));
        }
        ui.add_space(space::SCALE_8);
        group_caption(ui, "Actions");
        actions_group(ui, take, state, recording, &mut action);
        ui.add_space(space::SCALE_8);
    });
    action
}

/// The Actions group at the bottom of the take panel (spec "Review mode"):
/// Show in folder, Rename and Delete take as text buttons; Rename and Delete
/// are disabled while a take runs.
fn actions_group(ui: &mut Ui, take: &Take, state: &mut TakePanelState, recording: bool, action: &mut TakeAction) {
    inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        ui.horizontal(|ui| {
            if ui.button("Show in folder").clicked() {
                *action = TakeAction::ShowInFolder;
            }
            ui.add_enabled_ui(!recording, |ui| {
                if ui.button("Rename").clicked() {
                    state.renaming = Some(take.name.clone());
                    state.rename_focused = false;
                    state.rename_error = None;
                }
                let delete = Button::new(RichText::new("Delete take").color(color::SEM_RECORD));
                if ui.add(delete).clicked() {
                    state.confirm_delete = true;
                }
            });
        });
        ui.add_space(space::SCALE_4);
    });
}

/// The take info header row: the take's name over its start date, with the
/// hairline along the bottom of the row. While the name is being edited the
/// name is an inline text field instead: Enter confirms (the new name is
/// returned), Esc cancels, and a refused name shows its reason under the field.
fn take_header(ui: &mut Ui, take: Option<&Take>, state: &mut TakePanelState) -> Option<String> {
    let mut submitted = None;
    let mut cancelled = false;
    let renaming = state.renaming.is_some() && take.is_some();
    // The field grows past the 44px row, so the fixed height is only reserved
    // when the plain name is shown.
    let height = if renaming { 0.0 } else { size::PANEL_TOOLBAR_HEIGHT };
    let row = vec2(ui.available_width(), height);
    let inner = ui.allocate_ui_with_layout(row, Layout::left_to_right(Align::Center), |ui| {
        ui.add_space(HEADER_PAD);
        ui.vertical(|ui| {
            match take {
                Some(take) => match &mut state.renaming {
                    Some(buffer) => {
                        let edit = ui.add(egui::TextEdit::singleline(buffer).desired_width(ui.available_width()));
                        if !state.rename_focused {
                            edit.request_focus();
                            state.rename_focused = true;
                        }
                        if edit.lost_focus() {
                            let (enter, escape) = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Escape)));
                            if escape {
                                cancelled = true;
                            } else if enter {
                                submitted = Some(buffer.clone());
                            }
                        }
                    }
                    None => {
                        ui.add(
                            egui::Label::new(RichText::new(take.name.clone()).font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                                .selectable(false)
                                .truncate(),
                        );
                    }
                },
                None => {
                    ui.add(
                        egui::Label::new(RichText::new("No take selected").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY))
                            .selectable(false),
                    );
                }
            }
            if !renaming && let Some(take) = take {
                ui.add(
                    egui::Label::new(
                        RichText::new(take.start.format("%b %d, %Y").to_string())
                            .font(ui_kit::fonts::medium(typography::CAPTION_SIZE))
                            .color(color::TEXT_TERTIARY),
                    )
                    .selectable(false),
                );
            }
            if let Some(reason) = &state.rename_error {
                ui.colored_label(ui.visuals().error_fg_color, reason);
            }
        });
    });
    if cancelled {
        state.close_rename();
    }
    let rect = inner.response.rect;
    ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE));
    submitted
}

/// One read-only inspector row with its value.
fn row(ui: &mut Ui, label: &str, value: &str) -> egui::Response {
    inspector_row(ui, label, |_ui| {}, Some(value))
}

/// The format of a take file: its extension uppercased, or a dash when it has none.
fn format_label(path: &std::path::Path) -> String {
    path.extension().map(|extension| extension.to_string_lossy().to_uppercase()).unwrap_or_else(|| "\u{2014}".to_owned())
}

/// The dot color of a sidecar mark: the marker palette has no Highlight or
/// Chapter tokens, so they take the Retake and Note colors (the same mapping
/// the live marks list uses).
fn mark_dot(mark_type: MarkType) -> Color32 {
    let kind = match mark_type {
        MarkType::Cut => MarkerKind::Cut,
        MarkType::Retake | MarkType::Highlight => MarkerKind::Retake,
        MarkType::Note | MarkType::Chapter => MarkerKind::Note,
        MarkType::BRoll => MarkerKind::BRoll,
        MarkType::System => MarkerKind::System,
    };
    marker_color(kind)
}

/// The display name of a mark type; an unknown type keeps the original string
/// the reader saved.
fn type_name(mark_type: MarkType, original: Option<&str>) -> String {
    match original {
        Some(original) => original.to_owned(),
        None => match mark_type {
            MarkType::Highlight => "Highlight".to_owned(),
            MarkType::Cut => "Cut".to_owned(),
            MarkType::Retake => "Retake".to_owned(),
            MarkType::Chapter => "Chapter".to_owned(),
            MarkType::Note => "Note".to_owned(),
            MarkType::BRoll => "B-roll".to_owned(),
            MarkType::System => "System".to_owned(),
        },
    }
}

/// The delete confirmation dialog (spec "Review mode"): every file of the take
/// with its size, Cancel and "Move to Recycle Bin". Returns whether the move
/// was confirmed this frame; the app does the file work.
pub fn delete_dialog(ctx: &egui::Context, take: Option<&Take>, open: &mut bool) -> bool {
    let mut confirmed = false;
    if !*open {
        return false;
    }
    let Some(take) = take else {
        *open = false;
        return false;
    };
    // The buttons report through cells; the dialog itself closes `open` on Esc
    // and a backdrop click, so the closures must not capture it.
    let closed = std::cell::Cell::new(false);
    ui_kit::dialog(
        ctx,
        egui::Id::new("review-delete-take"),
        "Delete take",
        &mut *open,
        |ui| {
            ui.label(format!("Move the files of \u{201c}{}\u{201d} to the Recycle Bin?", take.name));
            ui.add_space(space::SCALE_8);
            egui::ScrollArea::vertical().max_height(220.0).auto_shrink(false).show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                for file in &take.files {
                    let name = file.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.label(format!("{name} ({})", review::format_size(file.size)));
                }
            });
        },
        |ui| {
            if ui.button("Cancel").clicked() {
                closed.set(true);
            }
            if ui_kit::primary_button(ui, "Move to Recycle Bin").clicked() {
                closed.set(true);
                confirmed = true;
            }
        },
    );
    if closed.get() {
        *open = false;
    }
    confirmed
}
