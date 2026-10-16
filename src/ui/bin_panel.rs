//! ui::bin_panel: the Media Bin panel (docs/ui-target.png look).

use eframe::egui;
use egui_phosphor::fill;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::model::media::MediaItem;
use crate::ui::dialogs::{self, FooterButton};
use crate::ui::theme::{THEME, Theme};
use crate::ui::timeline::interaction::MediaDrag;

/// Sort order of the bin list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum BinSort {
    /// Order in project.media (date added).
    #[default]
    DateAdded,
    /// Alphabetical by display name.
    Name,
}

/// Key under which another panel requests the bin to reveal a media item.
const REVEAL_KEY: &str = "bin_reveal_request";

/// Requests selecting and scrolling to a media item in the bin panel
/// (used by the clip context menu's "Reveal in Bin").
pub fn request_reveal(ctx: &egui::Context, media_id: Uuid) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(REVEAL_KEY), media_id));
}

/// State of the Media Bin panel.
#[derive(Default)]
pub struct BinPanel {
    /// Currently selected media item, if any.
    selected: Option<Uuid>,
    /// Media item being renamed inline, if any.
    renaming: Option<Uuid>,
    /// Buffer of the inline rename text field.
    rename_text: String,
    /// True once the rename field was focused for the first time.
    rename_focused_once: bool,
    /// Current sort order.
    sort: BinSort,
    /// Pending remove confirmation: media id and its clip usage count.
    confirm_remove: Option<(Uuid, usize)>,
    /// Media item to scroll into view this frame (Reveal in Bin).
    reveal: Option<Uuid>,
}

impl BinPanel {
    /// Draws the bin panel; returns human-readable messages for the user.
    pub fn show(&mut self, ui: &mut egui::Ui, editor: &mut Editor) -> Vec<String> {
        let mut messages = Vec::new();

        // 1 px border on the right edge of the panel.
        let outer = ui.max_rect();
        ui.painter().line_segment(
            [
                egui::pos2(outer.right() - 0.5, outer.top()),
                egui::pos2(outer.right() - 0.5, outer.bottom()),
            ],
            egui::Stroke::new(THEME.hairline, THEME.border),
        );

        // A Reveal in Bin request from another panel: select and scroll to it.
        let reveal_request = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<Uuid>(egui::Id::new(REVEAL_KEY)));
        if let Some(id) = reveal_request {
            self.selected = Some(id);
            self.reveal = Some(id);
        }

        self.header(ui, editor, &mut messages);
        ui.add_space(THEME.space_md);

        // The scroll area leaves room for the fixed drop zone (10 px above
        // it, 44 px tall, 10 px off the panel bottom).
        let scroll_h = (ui.available_height() - 10.0 - THEME.bin_drop_h - 10.0).max(80.0);
        egui::ScrollArea::vertical()
            .max_height(scroll_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = THEME.bin_row_gap;
                for item in sorted_items(&editor.project.media, self.sort) {
                    self.show_item(ui, editor, &item, &mut messages);
                }
                for (label, frac) in editor.pending_jobs() {
                    ui.add_space(THEME.space_sm);
                    ui.label(&label);
                    ui.add(egui::ProgressBar::new(frac).show_percentage());
                }
            });

        drop_zone(ui);

        self.show_remove_confirm(ui.ctx(), editor, &mut messages);
        messages
    }

    /// Header row: MEDIA caption, Import button and the sort button.
    fn header(&mut self, ui: &mut egui::Ui, editor: &mut Editor, messages: &mut Vec<String>) {
        ui.horizontal(|ui| {
            // 10 px from the panel edge; the caption sits centered against
            // the button height.
            ui.add_space(10.0);
            ui.set_min_height(THEME.bin_btn_h);
            ui.add(egui::Label::new(
                egui::RichText::new("MEDIA")
                    .font(Theme::semibold(THEME.bin_header_size))
                    .color(THEME.text_muted),
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = THEME.space_sm;

                let sort_label = egui::RichText::new(format!(
                    "{} {}",
                    match self.sort {
                        BinSort::DateAdded => "Date",
                        BinSort::Name => "Name",
                    },
                    fill::CARET_DOWN
                ))
                .size(THEME.bin_btn_text_size);
                let sort_button = ui
                    .add(egui::Button::new(sort_label).min_size(egui::vec2(0.0, THEME.bin_btn_h)));
                egui::Popup::menu(&sort_button).show(|ui| {
                    ui.selectable_value(&mut self.sort, BinSort::DateAdded, "Date added");
                    ui.selectable_value(&mut self.sort, BinSort::Name, "Name");
                });

                let import_label = egui::RichText::new(format!("{} Import", fill::DOWNLOAD))
                    .size(THEME.bin_btn_text_size);
                if ui
                    .add(egui::Button::new(import_label).min_size(egui::vec2(0.0, THEME.bin_btn_h)))
                    .clicked()
                {
                    messages.extend(pick_and_import(editor));
                }
            });
        });
    }

    /// Draws one bin item row with its interactions.
    fn show_item(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        item: &MediaItem,
        messages: &mut Vec<String>,
    ) {
        let offline = editor.is_media_offline(item.id);
        let usage = editor.media_usage(item.id);
        let selected = self.selected == Some(item.id);
        let previewing_this = editor.previewing() == Some(item.id);

        // Reveal in Bin: scroll the requested item into view.
        if self.reveal == Some(item.id) {
            ui.scroll_to_cursor(Some(egui::Align::Center));
            self.reveal = None;
        }

        // The usage pill sits on the row's right edge; the name stops short of it.
        let pill_w = pill_width(ui, usage_text(usage));
        let row_w = ui.available_width();
        let name_max_w = (row_w
            - THEME.space_md
            - THEME.bin_tile_sz
            - THEME.space_md
            - pill_w
            - THEME.space_sm
            - THEME.space_md)
            .max(40.0);

        // Row background: control fill while hovered or selected.
        let row_rect = egui::Rect::from_min_size(
            ui.next_widget_position(),
            egui::vec2(row_w, THEME.bin_row_h),
        );
        // Center of the NAME text line, for the usage pill; falls back to the
        // first content line when the rename field replaces the name.
        let mut name_line_center =
            row_rect.top() + THEME.space_md + THEME.bin_tile_sz.max(THEME.bin_name_size) * 0.5;
        let active = selected || ui.rect_contains_pointer(row_rect);

        let inner = egui::Frame::new()
            .fill(if active {
                THEME.control_fill
            } else {
                egui::Color32::TRANSPARENT
            })
            .corner_radius(THEME.bin_row_rounding)
            .inner_margin(THEME.space_md)
            .show(ui, |ui| {
                ui.set_min_size(egui::vec2(
                    row_w - 2.0 * THEME.space_md,
                    THEME.bin_row_h - 2.0 * THEME.space_md,
                ));
                ui.horizontal(|ui| {
                    // 30x30 tile with the waveform icon.
                    let (tile_rect, _) = ui.allocate_exact_size(
                        egui::vec2(THEME.bin_tile_sz, THEME.bin_tile_sz),
                        egui::Sense::hover(),
                    );
                    let painter = ui.painter();
                    painter.rect(
                        tile_rect,
                        THEME.control_rounding,
                        THEME.control_fill,
                        egui::Stroke::new(THEME.hairline, THEME.control_border),
                        egui::StrokeKind::Inside,
                    );
                    // The phosphor glyph lives in the proportional family
                    // (egui-phosphor only adds itself there), so use it here
                    // like the header buttons do.
                    painter.text(
                        tile_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        fill::WAVEFORM,
                        egui::FontId::proportional(THEME.bin_tile_icon_size),
                        THEME.text_normal,
                    );

                    ui.add_space(THEME.space_md);
                    ui.vertical(|ui| {
                        if self.renaming == Some(item.id) {
                            self.rename_field(ui, editor, item.id, messages);
                        } else {
                            // The name stops short of the usage pill.
                            ui.scope(|ui| {
                                ui.set_max_width(name_max_w);
                                let mut name = item.display_name.clone();
                                if offline {
                                    name.push_str(" (offline)");
                                }
                                let text = egui::RichText::new(name).size(THEME.bin_name_size);
                                let text = if offline {
                                    text.color(THEME.bin_offline)
                                } else {
                                    text.color(THEME.text_primary)
                                };
                                let name_response = ui.add(
                                    egui::Label::new(text)
                                        .sense(egui::Sense::click())
                                        .truncate(),
                                );
                                if name_response.clicked() {
                                    self.selected = Some(item.id);
                                }
                                if name_response.double_clicked() {
                                    self.begin_rename(item);
                                }
                                name_line_center = name_response.rect.center().y;
                            });
                            ui.add_space(THEME.space_xs);
                            // The meta line spans the full row width under
                            // the name, out to the row's right padding.
                            let meta = egui::RichText::new(meta_line(editor, item))
                                .font(Theme::mono(THEME.bin_meta_size))
                                .color(THEME.text_muted);
                            ui.add(egui::Label::new(meta).truncate());
                        }
                    });
                });
            });
        let row = inner.response.rect;

        // Usage pill on the right edge of the row, centered on the NAME line
        // only so it never touches the meta line below it.
        let pill_galley = pill_galley(ui, usage_text(usage));
        draw_pill(ui.painter(), row, pill_galley, name_line_center);

        // Thin progress bar along the row bottom while this item previews.
        if previewing_this {
            let frac = editor.preview_progress();
            let y = row.bottom() - THEME.space_md;
            let bar = egui::Rect::from_min_max(
                egui::pos2(row.left() + THEME.space_md, y),
                egui::pos2(
                    row.left() + THEME.space_md + (row.width() - 2.0 * THEME.space_md) * frac,
                    y + THEME.bin_progress_h,
                ),
            );
            ui.painter().rect_filled(bar, THEME.corner_xs, THEME.accent);
        }

        let row_response = ui.interact(
            inner.response.rect,
            egui::Id::new(("bin_row", item.id)),
            if offline {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        // Online rows are drag sources for the timeline; offline ones are not.
        if !offline && row_response.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), MediaDrag { media_id: item.id });
        }
        if row_response.clicked() {
            self.selected = Some(item.id);
        }
        egui::Popup::context_menu(&row_response)
            .style(crate::ui::theme::menu_item_style as fn(&mut egui::Style))
            .show(|ui| {
                if !offline
                    && ui
                        .button(if previewing_this {
                            "Stop Preview"
                        } else {
                            "Preview"
                        })
                        .clicked()
                {
                    if previewing_this {
                        editor.stop_preview();
                    } else {
                        editor.preview_media(item.id);
                    }
                    ui.close();
                }
                if offline && ui.button("Relink...").clicked() {
                    ui.close();
                    messages.extend(pick_and_relink(editor, item.id));
                }
                if ui.button("Rename").clicked() {
                    self.begin_rename(item);
                    ui.close();
                }
                if ui.button("Reveal in Explorer").clicked() {
                    editor.reveal_media(item.id);
                    ui.close();
                }
                if ui.button("Remove from Project").clicked() {
                    ui.close();
                    if usage > 0 {
                        self.confirm_remove = Some((item.id, usage));
                    } else if let Err(error) = editor.apply(Action::RemoveMedia { id: item.id }) {
                        messages.push(error.to_string());
                    }
                }
            });
    }

    /// Starts an inline rename of the given item.
    fn begin_rename(&mut self, item: &MediaItem) {
        self.renaming = Some(item.id);
        self.rename_text = item.display_name.clone();
        self.rename_focused_once = false;
    }
    /// Draws the inline rename text field and applies or cancels it.
    fn rename_field(
        &mut self,
        ui: &mut egui::Ui,
        editor: &mut Editor,
        id: Uuid,
        messages: &mut Vec<String>,
    ) {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.renaming = None;
            return;
        }
        let width = (ui.available_width() - THEME.bin_rename_pad).max(THEME.bin_rename_min_w);
        let response =
            ui.add(egui::TextEdit::singleline(&mut self.rename_text).desired_width(width));
        if !self.rename_focused_once {
            response.request_focus();
            if response.has_focus() {
                self.rename_focused_once = true;
            }
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                match editor.apply(Action::RenameMedia {
                    id,
                    name: self.rename_text.clone(),
                }) {
                    Ok(()) => self.renaming = None,
                    Err(error) => messages.push(error.to_string()),
                }
            } else {
                self.renaming = None;
            }
        }
    }

    /// Draws the remove confirmation window when one is pending.
    fn show_remove_confirm(
        &mut self,
        ctx: &egui::Context,
        editor: &mut Editor,
        messages: &mut Vec<String>,
    ) {
        let Some((id, usage)) = self.confirm_remove else {
            return;
        };
        let Some(name) = editor
            .project
            .media
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.display_name.clone())
        else {
            self.confirm_remove = None;
            return;
        };
        let open = true;
        let mut remove = false;
        let mut cancel = false;
        let response = dialogs::modal("bin_remove_confirm").show(ctx, |ui| {
            dialogs::card_width(ui, THEME.dialog_w);
            dialogs::title(ui, "Remove from Project");
            ui.add_space(THEME.space_xs);
            dialogs::description(
                ui,
                &format!(
                    "Remove \"{name}\"? This item is used by {usage} clips. Removing it also removes those clips."
                ),
            );
            dialogs::before_form(ui);
            dialogs::footer(ui, |ui| {
                if dialogs::footer_button(ui, "Remove", FooterButton::Destructive, true)
                    .clicked()
                {
                    remove = true;
                }
                if dialogs::footer_button(ui, "Cancel", FooterButton::Cancel, true).clicked() {
                    cancel = true;
                }
            });
            // Enter confirms the removal.
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                remove = true;
            }
        });
        // Esc or the backdrop cancels.
        if response.should_close() {
            cancel = true;
        }
        if remove {
            self.selected = None;
            if let Err(error) = editor.apply(Action::RemoveMedia { id }) {
                messages.push(error.to_string());
            }
            self.confirm_remove = None;
        } else if !open || cancel {
            self.confirm_remove = None;
        }
    }
}

/// The dashed drop zone anchored to the bottom of the panel, 10 px in from
/// the panel's left, right and bottom edges, 44 px tall.
fn drop_zone(ui: &mut egui::Ui) {
    let margin = 10.0;
    let outer = ui.max_rect();
    let rect = egui::Rect::from_min_max(
        egui::pos2(
            outer.left() + margin,
            outer.bottom() - margin - THEME.bin_drop_h,
        ),
        egui::pos2(outer.right() - margin, outer.bottom() - margin),
    );
    // Advance the cursor below the zone so later widgets cannot land on it.
    let pad = (rect.top() - ui.cursor().top()).max(0.0);
    ui.allocate_exact_size(egui::vec2(1.0, pad), egui::Sense::hover());
    ui.allocate_exact_size(
        egui::vec2(rect.width(), rect.height()),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    dashed_rect(
        painter,
        rect,
        egui::Stroke::new(THEME.hairline, THEME.control_border),
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "Drop WAV or MP3 files here",
        egui::FontId::proportional(THEME.bin_drop_text_size),
        THEME.text_faint,
    );
}

/// Draws a complete dashed 1 px rectangle: all four sides drawn end to end
/// so no corner gaps are left.
fn dashed_rect(painter: &egui::Painter, rect: egui::Rect, stroke: egui::Stroke) {
    dashed_line(painter, rect.left_top(), rect.right_top(), stroke);
    dashed_line(painter, rect.left_bottom(), rect.right_bottom(), stroke);
    dashed_line(painter, rect.left_top(), rect.left_bottom(), stroke);
    dashed_line(painter, rect.right_top(), rect.right_bottom(), stroke);
}

/// Draws a dashed line segment.
fn dashed_line(painter: &egui::Painter, a: egui::Pos2, b: egui::Pos2, stroke: egui::Stroke) {
    let length = b.distance(a);
    if length <= 0.0 {
        return;
    }
    let dir = (b - a) / length;
    let mut d = 0.0;
    while d < length {
        let end = (d + THEME.ghost_dash).min(length);
        painter.line_segment([a + dir * d, a + dir * end], stroke);
        d += THEME.ghost_dash + THEME.ghost_gap;
    }
}

/// Measured width of the usage pill (text plus its side padding).
fn pill_width(ui: &egui::Ui, text: String) -> f32 {
    pill_galley(ui, text).size().x + 12.0
}

/// The usage pill text, pre-laid-out for measuring and drawing.
fn pill_galley(ui: &egui::Ui, text: String) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(
        text,
        egui::FontId::proportional(THEME.bin_pill_text_size),
        THEME.text_normal,
    )
}

/// Draws the usage pill centered vertically on the given line (the row's
/// NAME line) at the row's right edge.
fn draw_pill(
    painter: &egui::Painter,
    row: egui::Rect,
    galley: std::sync::Arc<egui::Galley>,
    line_center_y: f32,
) {
    let pill_w = galley.size().x + 12.0;
    let pill_h = THEME.bin_pill_h;
    let pill = egui::Rect::from_min_max(
        egui::pos2(
            row.right() - THEME.space_md - pill_w,
            line_center_y - pill_h * 0.5,
        ),
        egui::pos2(row.right() - THEME.space_md, line_center_y + pill_h * 0.5),
    );
    painter.rect_filled(pill, THEME.bin_pill_rounding, THEME.bin_pill_fill);
    painter.galley(
        pill.left_top() + egui::vec2(6.0, (pill_h - galley.size().y) * 0.5),
        galley,
        THEME.text_normal,
    );
}

/// Opens the import file picker (WAV and MP3) and starts one import job per file.
///
/// Returns the names of files rejected because of an unsupported extension.
pub fn pick_and_import(editor: &mut Editor) -> Vec<String> {
    let Some(files) = rfd::FileDialog::new()
        .add_filter("Audio (WAV, MP3)", &["wav", "mp3"])
        .pick_files()
    else {
        return Vec::new();
    };
    editor.import_files(files)
}

/// Opens the relink file picker (WAV and MP3) and starts one relink job.
///
/// The job copies the chosen file into media/ for the offline item.
fn pick_and_relink(editor: &mut Editor, id: Uuid) -> Vec<String> {
    let Some(file) = rfd::FileDialog::new()
        .add_filter("Audio (WAV, MP3)", &["wav", "mp3"])
        .pick_file()
    else {
        return Vec::new();
    };
    editor.relink(id, file);
    Vec::new()
}

/// Returns the bin items in the given sort order.
fn sorted_items(items: &[MediaItem], sort: BinSort) -> Vec<MediaItem> {
    let mut items = items.to_vec();
    if sort == BinSort::Name {
        items.sort_by(|a, b| {
            a.display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase())
        });
    }
    items
}

/// The monospace meta line: time, rate, channels and format separated by
/// middle dots (`\u{B7}`), e.g. `0:06.200 48k St MP3` with dots between.
fn meta_line(editor: &Editor, item: &MediaItem) -> String {
    let rate = editor.project.sample_rate;
    let total_ms = item.length * 1000 / u64::from(rate);
    let min = total_ms / 60_000;
    let sec = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    let channels = if item.channels <= 1 { "Mo" } else { "St" };
    format!(
        "{min}:{sec:02}.{ms:03} \u{B7} {}k \u{B7} {channels} \u{B7} {}",
        item.original_rate / 1000,
        media_format(&item.file_name)
    )
}

/// The original container format, taken from the file extension.
fn media_format(file_name: &str) -> &'static str {
    let lower = file_name.to_lowercase();
    if lower.ends_with(".wav") {
        "WAV"
    } else if lower.ends_with(".mp3") {
        "MP3"
    } else {
        "Other"
    }
}

/// Human-readable usage count, e.g. "1 use" or "3 uses".
fn usage_text(usage: usize) -> String {
    if usage == 1 {
        "1 use".to_string()
    } else {
        format!("{usage} uses")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> MediaItem {
        MediaItem {
            id: Uuid::new_v4(),
            display_name: name.to_string(),
            file_name: format!("{name}.wav"),
            original_rate: 44_100,
            channels: 1,
            length: 48_000,
        }
    }

    #[test]
    fn date_added_keeps_project_order() {
        let items = vec![item("b"), item("a")];
        let sorted = sorted_items(&items, BinSort::DateAdded);
        assert_eq!(sorted[0].display_name, "b");
        assert_eq!(sorted[1].display_name, "a");
    }

    #[test]
    fn name_sort_is_case_insensitive() {
        let items = vec![item("beta"), item("Alpha"), item("alpha2")];
        let sorted = sorted_items(&items, BinSort::Name);
        let names: Vec<&str> = sorted.iter().map(|i| i.display_name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "alpha2", "beta"]);
    }

    #[test]
    fn media_format_from_extension() {
        assert_eq!(media_format("song.MP3"), "MP3");
        assert_eq!(media_format("take.wav"), "WAV");
        assert_eq!(media_format("song.flac"), "Other");
    }

    #[test]
    fn usage_text_singular_and_plural() {
        assert_eq!(usage_text(1), "1 use");
        assert_eq!(usage_text(3), "3 uses");
    }

    #[test]
    fn meta_line_separates_with_real_middle_dots() {
        let editor = Editor::with_project(
            crate::model::project::Project::new(
                "Test".to_string(),
                48_000,
                crate::model::project::BitDepth::Int24,
            ),
            crate::model::project::ViewState::default(),
            crate::io::paths::ProjectPaths {
                root: std::path::PathBuf::from("unused"),
            },
        );
        let line = meta_line(&editor, &item("tone"));
        assert_eq!(line, "0:01.000 \u{B7} 44k \u{B7} Mo \u{B7} WAV");
        // UTF-8 for U+00B7 is C2 B7; the double-encoded mojibake starts C3 82.
        assert!(line.contains('\u{B7}'));
        assert!(!line.contains('\u{C2}'), "no double-encoded middle dot");
    }
}
