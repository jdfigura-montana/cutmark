//! ui::bin_panel: the resizable Media Bin panel.

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::model::media::MediaItem;
use crate::ui::theme::THEME;
use crate::ui::timeline::interaction::MediaDrag;
use crate::util::time;

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

        // A Reveal in Bin request from another panel: select and scroll to it.
        let reveal_request = ui
            .ctx()
            .data_mut(|d| d.remove_temp::<Uuid>(egui::Id::new(REVEAL_KEY)));
        if let Some(id) = reveal_request {
            self.selected = Some(id);
            self.reveal = Some(id);
        }

        ui.horizontal(|ui| {
            if ui.button("Import...").clicked() {
                messages.extend(pick_and_import(editor));
            }
            egui::ComboBox::from_id_salt("bin_sort")
                .selected_text(match self.sort {
                    BinSort::DateAdded => "Date added",
                    BinSort::Name => "Name",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.sort, BinSort::DateAdded, "Date added");
                    ui.selectable_value(&mut self.sort, BinSort::Name, "Name");
                });
        });
        ui.add_space(THEME.space_sm);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for item in sorted_items(&editor.project.media, self.sort) {
                self.show_item(ui, editor, &item, &mut messages);
            }
        });

        for (label, frac) in editor.pending_jobs() {
            ui.add_space(THEME.space_sm);
            ui.label(&label);
            ui.add(egui::ProgressBar::new(frac).show_percentage());
        }

        self.show_remove_confirm(ui.ctx(), editor, &mut messages);
        messages
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

        let inner = ui.allocate_ui(egui::vec2(ui.available_width(), THEME.bin_row_h), |ui| {
            ui.horizontal(|ui| {
                if !offline
                    && ui
                        .small_button(if previewing_this { "â¹" } else { "â–¶" })
                        .clicked()
                {
                    if previewing_this {
                        editor.stop_preview();
                    } else {
                        editor.preview_media(item.id);
                    }
                }
                if selected {
                    let (bar, _) = ui.allocate_exact_size(
                        egui::vec2(THEME.bin_sel_bar_w, THEME.bin_sel_bar_h),
                        egui::Sense::hover(),
                    );
                    ui.painter()
                        .rect_filled(bar, THEME.corner_xs, ui.visuals().selection.bg_fill);
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        if self.renaming == Some(item.id) {
                            self.rename_field(ui, editor, item.id, messages);
                        } else {
                            let mut name = item.display_name.clone();
                            if offline {
                                name.push_str(" (offline)");
                            }
                            let text = egui::RichText::new(name);
                            let text = if offline {
                                text.color(THEME.bin_offline)
                            } else if selected {
                                text.strong()
                            } else {
                                text
                            };
                            let name_response =
                                ui.add(egui::Label::new(text).sense(egui::Sense::click()));
                            if name_response.clicked() {
                                self.selected = Some(item.id);
                            }
                            if name_response.double_clicked() {
                                self.begin_rename(item);
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak(usage_text(usage));
                        });
                    });
                    ui.label(
                        egui::RichText::new(details_line(editor, item))
                            .small()
                            .color(THEME.text_faint),
                    );
                });
            });
            // Thin progress bar under the row while this item previews.
            if previewing_this {
                let frac = editor.preview_progress();
                let (bar, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), THEME.bin_progress_h),
                    egui::Sense::hover(),
                );
                let filled = egui::Rect::from_min_size(
                    bar.min,
                    egui::vec2(bar.width() * frac, THEME.bin_progress_h),
                );
                ui.painter()
                    .rect_filled(filled, THEME.corner_xs, ui.visuals().selection.bg_fill);
            }
        });
        let row = ui.interact(
            inner.response.rect,
            egui::Id::new(("bin_row", item.id)),
            if offline {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        // Online rows are drag sources for the timeline; offline ones are not.
        if !offline && row.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), MediaDrag { media_id: item.id });
        }
        if row.clicked() {
            self.selected = Some(item.id);
        }
        row.context_menu(|menu| {
            if !offline
                && menu
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
                menu.close();
            }
            if offline && menu.button("Relink...").clicked() {
                menu.close();
                messages.extend(pick_and_relink(editor, item.id));
            }
            if menu.button("Rename").clicked() {
                self.begin_rename(item);
                menu.close();
            }
            if menu.button("Reveal in Explorer").clicked() {
                editor.reveal_media(item.id);
                menu.close();
            }
            if menu.button("Remove from Project").clicked() {
                menu.close();
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
        let mut open = true;
        let mut remove = false;
        let mut cancel = false;
        egui::Window::new("Remove from Project")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "Remove \"{name}\"? This item is used by {usage} clips. Removing it also removes those clips."
                ));
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui.button("Remove").clicked() {
                        remove = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
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

/// The second, grey detail line of a bin row.
fn details_line(editor: &Editor, item: &MediaItem) -> String {
    let duration = time::samples_to_timecode(item.length, editor.project.sample_rate);
    let khz = item.original_rate as f32 / 1000.0;
    let channels = if item.channels <= 1 { "Mono" } else { "Stereo" };
    let format = media_format(&item.file_name);
    format!("{duration}  {khz:.1} kHz  {channels}  {format}")
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
}
