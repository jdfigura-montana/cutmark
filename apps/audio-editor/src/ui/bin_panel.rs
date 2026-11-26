//! ui::bin_panel: the Media panel (ui-kit side panel, 44 tall header, list rows).

use eframe::egui;
use ui_kit::icons::regular;
use ui_kit::tokens::{color, radius, size, space, typography};
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::action::Action;
use crate::model::media::MediaItem;
use crate::ui::dialogs;
use crate::ui::timeline::interaction::MediaDrag;

/// Side of the icon tile at the left of a row.
const TILE: f32 = 28.0;
/// Size of the waveform glyph inside the tile.
const TILE_ICON: f32 = 16.0;
/// Left padding of the header title from the panel's inner edge; with the
/// panel's own 8px padding the title sits 14px from the panel edge (spec
/// "Panel header").
const HEADER_PAD: f32 = 6.0;
/// Gap between two rows.
const ROW_GAP: f32 = 2.0;
/// Height of the preview progress bar along the bottom of a row.
const PROGRESS_H: f32 = 2.0;
/// Height of the dashed drop zone at the bottom of the panel.
const DROP_H: f32 = 44.0;
/// Dash and gap length of the drop zone outline.
const DASH: f32 = 6.0;
/// Gap length of the drop zone outline.
const DASH_GAP: f32 = 4.0;
/// Width of the Remove from Project confirmation, the same as the other
/// confirmation dialogs.
const CONFIRM_W: f32 = 420.0;
/// The drop hint, also the empty state message.
const DROP_HINT: &str = "Drop WAV or MP3 files here";
/// The 1 px hairline stroke width.
const HAIRLINE: f32 = 1.0;

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
        // F1 over the panel opens Importing audio; the tour spotlights it.
        ui_kit::help::context(ui, crate::help::CONTEXT_MEDIA);
        ui_kit::tour::anchor(ui, crate::help::ANCHOR_MEDIA, ui.max_rect());

        // A Reveal in Bin request from another panel: select and scroll to it.
        let reveal_request = ui.ctx().data_mut(|d| d.remove_temp::<Uuid>(egui::Id::new(REVEAL_KEY)));
        if let Some(id) = reveal_request {
            self.selected = Some(id);
            self.reveal = Some(id);
        }

        // While a take is recorded the bin ignores drags, preview, remove,
        // rename, relink and import: pending prompts and edits go away.
        if editor.is_recording() {
            self.renaming = None;
            self.confirm_remove = None;
        }

        ui_kit::side_panel(ui, |ui| {
            // The panel fills the area the app gives it.
            ui.set_min_size(ui.available_size());
            self.header(ui, editor, &mut messages);
            ui.add_space(space::SCALE_8);
            self.list(ui, editor, &mut messages);
        });

        self.show_remove_confirm(ui.ctx(), editor, &mut messages);
        messages
    }

    /// Header row (44 tall): the "Media" title, then the Sort and Import icon
    /// buttons on the right and a hairline along the bottom.
    fn header(&mut self, ui: &mut egui::Ui, editor: &mut Editor, messages: &mut Vec<String>) {
        let neutral = crate::ui::neutrals(ui.ctx());
        let recording = editor.is_recording();
        let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
        let inner = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = space::SCALE_4;
            ui.add_space(HEADER_PAD);
            ui.add(
                egui::Label::new(egui::RichText::new("Media").font(ui_kit::fonts::semibold(typography::BODY_SIZE)).color(neutral.text_primary))
                    .selectable(false),
            );
            // Added right to left, so on screen the Sort button sits left of
            // the Import button.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(!recording, |ui| {
                    if ui_kit::icon_button(ui, regular::DOWNLOAD_SIMPLE, "Import (Ctrl+I)").clicked() {
                        messages.extend(pick_and_import(editor));
                    }
                });
                let sort_tooltip = format!(
                    "Sort: {}",
                    match self.sort {
                        BinSort::DateAdded => "Date added",
                        BinSort::Name => "Name",
                    }
                );
                let sort_button = ui_kit::icon_button(ui, regular::ARROWS_DOWN_UP, &sort_tooltip);
                show_glass(egui::Popup::menu(&sort_button).gap(space::SCALE_4), &sort_button, |ui| {
                    ui.selectable_value(&mut self.sort, BinSort::DateAdded, "Date added");
                    ui.selectable_value(&mut self.sort, BinSort::Name, "Name");
                });
            });
        });
        // The hairline runs edge to edge of the panel, through its padding.
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range().expand(space::PANEL_PADDING), rect.bottom(), egui::Stroke::new(HAIRLINE, neutral.stroke_hairline));
    }

    /// The item list with the drop zone under it, or the empty state when
    /// there is nothing to list.
    fn list(&mut self, ui: &mut egui::Ui, editor: &mut Editor, messages: &mut Vec<String>) {
        let neutral = crate::ui::neutrals(ui.ctx());
        if editor.project.media.is_empty() && !editor.has_pending_jobs() {
            empty_state(ui, editor, messages);
            return;
        }
        // The scroll area leaves room for the fixed drop zone under it.
        let scroll_h = (ui.available_height() - space::SCALE_8 - DROP_H).max(80.0);
        egui::ScrollArea::vertical().max_height(scroll_h).auto_shrink([false, false]).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = ROW_GAP;
            for item in sorted_items(&editor.project.media, self.sort) {
                self.show_item(ui, editor, &item, messages);
            }
            for (label, frac) in editor.pending_jobs() {
                ui.add_space(space::SCALE_4);
                ui.label(egui::RichText::new(&label).font(egui::FontId::proportional(typography::CAPTION_SIZE)).color(neutral.text_tertiary));
                ui.add(egui::ProgressBar::new(frac).show_percentage());
            }
        });
        drop_zone(ui);
    }

    /// Draws one bin item as a list row with its interactions.
    fn show_item(&mut self, ui: &mut egui::Ui, editor: &mut Editor, item: &MediaItem, messages: &mut Vec<String>) {
        let offline = editor.is_media_offline(item.id);
        let recording = editor.is_recording();
        let usage = editor.media_usage(item.id);
        let previewing_this = editor.previewing() == Some(item.id);
        let renaming = self.renaming == Some(item.id);

        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), size::LIST_ROW_HEIGHT), egui::Sense::hover());
        // Reveal in Bin: scroll the requested item into view.
        if self.reveal == Some(item.id) {
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
            self.reveal = None;
        }

        // The row's own interaction sits under its contents, so the rename
        // field above it keeps the pointer.
        let row_response =
            ui.interact(rect, egui::Id::new(("bin_row", item.id)), if offline || recording { egui::Sense::click() } else { egui::Sense::click_and_drag() });
        // Online rows are drag sources for the timeline; offline ones are not,
        // and nothing is dragged to the timeline while a take is recorded.
        if !offline && !recording && row_response.drag_started() {
            egui::DragAndDrop::set_payload(ui.ctx(), MediaDrag { media_id: item.id });
        }
        if row_response.clicked() {
            self.selected = Some(item.id);
        }
        if row_response.double_clicked() && !recording && !renaming {
            self.begin_rename(item);
        }

        if ui.is_rect_visible(rect) {
            self.paint_row(ui, editor, item, rect, &row_response, offline);
        }
        if renaming {
            let field = egui::Rect::from_center_size(
                egui::pos2(rect.center().x + (TILE + space::SCALE_8) / 2.0, rect.center().y),
                egui::vec2(rect.width() - TILE - space::SCALE_8 - 2.0 * space::SCALE_6, size::PANEL_BUTTON_HEIGHT),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| {
                self.rename_field(ui, editor, item.id, messages);
            });
        } else {
            // The details the row no longer shows: format, rate, channels and
            // how many clips use the item.
            let tip = format!("{}\n{}", meta_line(editor, item), usage_text(usage));
            let _ = row_response.clone().on_hover_text(tip);
        }

        show_glass(egui::Popup::context_menu(&row_response), &row_response, |ui| {
            // Preview, Relink, Rename and Remove are off while recording.
            if !offline && ui.add_enabled(!recording, egui::Button::new(if previewing_this { "Stop Preview" } else { "Preview" })).clicked() {
                if previewing_this {
                    editor.stop_preview();
                } else {
                    editor.preview_media(item.id);
                }
                ui.close();
            }
            if offline && ui.add_enabled(!recording, egui::Button::new("Relink...")).clicked() {
                ui.close();
                messages.extend(pick_and_relink(editor, item.id));
            }
            if ui.add_enabled(!recording, egui::Button::new("Rename")).clicked() {
                self.begin_rename(item);
                ui.close();
            }
            if ui.button("Reveal in Explorer").clicked() {
                if let Err(error) = editor.reveal_media(item.id) {
                    messages.push(error.to_string());
                }
                ui.close();
            }
            if ui.add_enabled(!recording, egui::Button::new("Remove from Project")).clicked() {
                ui.close();
                if usage > 0 {
                    self.confirm_remove = Some((item.id, usage));
                } else if let Err(error) = editor.apply(Action::RemoveMedia { id: item.id }) {
                    messages.push(error.to_string());
                }
            }
        });
    }

    /// Paints a row: state fill, icon tile, name (unless it is being
    /// renamed), the duration on the right and the preview progress bar.
    fn paint_row(&self, ui: &egui::Ui, editor: &Editor, item: &MediaItem, rect: egui::Rect, response: &egui::Response, offline: bool) {
        let neutral = crate::ui::neutrals(ui.ctx());
        let selected = self.selected == Some(item.id);
        let painter = ui.painter();
        let fill = if selected {
            Some(neutral.bg_row_selected)
        } else if response.hovered() {
            Some(neutral.bg_group)
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect, radius::SEGMENT, fill);
        }

        // The 28 icon tile with the waveform glyph.
        let tile = egui::Rect::from_min_size(egui::pos2(rect.left() + space::SCALE_6, rect.center().y - TILE / 2.0), egui::Vec2::splat(TILE));
        painter.rect_filled(tile, radius::SEGMENT, neutral.bg_control);
        let glyph = painter.layout_no_wrap(regular::WAVEFORM.to_owned(), ui_kit::fonts::icon_regular(TILE_ICON), neutral.text_secondary);
        painter.galley(tile.center() - glyph.size() / 2.0, glyph, neutral.text_secondary);

        // The duration, right aligned in type.value text.tertiary.
        let duration =
            painter.layout_no_wrap(duration_text(editor.project.sample_rate, item.length), ui_kit::fonts::mono(typography::VALUE_SIZE), neutral.text_tertiary);
        let duration_left = rect.right() - space::SCALE_8 - duration.size().x;
        painter.galley(egui::pos2(duration_left, rect.center().y - duration.size().y / 2.0), duration, neutral.text_tertiary);

        // The name, cut with an ellipsis before the duration.
        if self.renaming != Some(item.id) {
            let name_x = tile.right() + space::SCALE_8;
            let mut name = item.display_name.clone();
            if offline {
                name.push_str(" (offline)");
            }
            // No offline token exists in the design language; the red of the
            // record meaning marks a file that is gone.
            let name_color = if offline {
                color::SEM_RECORD
            } else if selected {
                neutral.text_primary
            } else {
                neutral.text_secondary
            };
            let mut job = egui::text::LayoutJob::single_section(name, egui::TextFormat::simple(ui_kit::fonts::medium(typography::BODY_SIZE), name_color));
            job.wrap = egui::text::TextWrapping {
                max_width: (duration_left - space::SCALE_8 - name_x).max(0.0),
                max_rows: 1,
                break_anywhere: true,
                overflow_character: Some('\u{2026}'),
            };
            let galley = painter.layout_job(job);
            painter.galley(egui::pos2(name_x, rect.center().y - galley.size().y / 2.0), galley, name_color);
        }

        // Thin progress bar along the row bottom while this item previews.
        if editor.previewing() == Some(item.id) {
            let inset = space::SCALE_6;
            let y = rect.bottom() - PROGRESS_H - 1.0;
            let width = (rect.width() - 2.0 * inset) * editor.preview_progress();
            let bar = egui::Rect::from_min_size(egui::pos2(rect.left() + inset, y), egui::vec2(width, PROGRESS_H));
            painter.rect_filled(bar, 0.0, ui_kit::Theme::get(ui.ctx()).accent.accent);
        }
        if response.has_focus() {
            ui_kit::focus_ring(ui, rect, radius::SEGMENT.into());
        }
    }

    /// Starts an inline rename of the given item.
    fn begin_rename(&mut self, item: &MediaItem) {
        self.renaming = Some(item.id);
        self.rename_text = item.display_name.clone();
        self.rename_focused_once = false;
    }

    /// Draws the inline rename text field and applies or cancels it.
    fn rename_field(&mut self, ui: &mut egui::Ui, editor: &mut Editor, id: Uuid, messages: &mut Vec<String>) {
        if crate::shortcuts::peek(ui.ctx(), "cancel_edit") {
            self.renaming = None;
            return;
        }
        let response = ui_kit::text_field(ui, &mut self.rename_text, "Name");
        if !self.rename_focused_once {
            response.request_focus();
            if response.has_focus() {
                self.rename_focused_once = true;
            }
        }
        if response.lost_focus() {
            if crate::shortcuts::peek(ui.ctx(), "confirm_edit") {
                match editor.apply(Action::RenameMedia { id, name: self.rename_text.clone() }) {
                    Ok(()) => self.renaming = None,
                    Err(error) => messages.push(error.to_string()),
                }
            } else {
                self.renaming = None;
            }
        }
    }

    /// Draws the remove confirmation window when one is pending.
    fn show_remove_confirm(&mut self, ctx: &egui::Context, editor: &mut Editor, messages: &mut Vec<String>) {
        let Some((id, usage)) = self.confirm_remove else {
            return;
        };
        let Some(name) = editor.project.media.iter().find(|m| m.id == id).map(|m| m.display_name.clone()) else {
            self.confirm_remove = None;
            return;
        };
        let mut open = true;
        let (mut clicked, mut enter, mut cancel) = (false, false, false);
        dialogs::dialog(
            ctx,
            "bin_remove_confirm",
            "Remove from Project",
            CONFIRM_W,
            &mut open,
            |ui| {
                dialogs::description(ui, &format!("Remove \"{name}\"? This item is used by {usage} clips. Removing it also removes those clips."));
                // Enter confirms the removal.
                enter = dialogs::enter_pressed(ui);
            },
            |ui| {
                if dialogs::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if ui_kit::primary_button(ui, "Remove").clicked() {
                    clicked = true;
                }
            },
        );
        // Esc or the backdrop cancels.
        let remove = clicked || enter;
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

/// Shows `popup` as a ui-kit glass popover: the glass frame with 6px padding,
/// plus the top highlight once the popover has had its sizing pass.
fn show_glass<R>(popup: egui::Popup<'_>, response: &egui::Response, add_contents: impl FnOnce(&mut egui::Ui) -> R) {
    let ctx = response.ctx.clone();
    let was_shown = ctx.read_response(egui::Popup::default_response_id(response)).is_some();
    let frame = ui_kit::glass_frame(&ctx, radius::GROUP).inner_margin(egui::Margin::same(space::SCALE_6 as i8));
    let shown = popup.frame(frame).show(add_contents);
    // The first frame of a popover is an invisible sizing pass.
    if let (Some(shown), true) = (shown, was_shown) {
        ui_kit::paint_glass_highlight(&ctx.layer_painter(shown.response.layer_id), shown.response.rect, radius::GROUP);
    }
}

/// The empty bin: the drop hint centered in `type.caption` `text.tertiary`
/// with the Import action as a normal button under it.
fn empty_state(ui: &mut egui::Ui, editor: &mut Editor, messages: &mut Vec<String>) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let area = ui.available_rect_before_wrap();
    ui.allocate_rect(area, egui::Sense::hover());
    let hint = ui.painter().layout_no_wrap(DROP_HINT.to_owned(), egui::FontId::proportional(typography::CAPTION_SIZE), neutral.text_tertiary);
    let label = ui.painter().layout_no_wrap("Import".to_owned(), ui_kit::fonts::medium(typography::BODY_SIZE), neutral.text_primary);
    let button_size = egui::vec2(label.size().x + 2.0 * space::SCALE_16, size::PANEL_BUTTON_HEIGHT);
    let total = hint.size().y + space::SCALE_12 + button_size.y;
    let top = area.center().y - total / 2.0;
    let hint_height = hint.size().y;
    ui.painter().galley(egui::pos2(area.center().x - hint.size().x / 2.0, top), hint, neutral.text_tertiary);
    let button_rect = egui::Rect::from_center_size(egui::pos2(area.center().x, top + hint_height + space::SCALE_12 + button_size.y / 2.0), button_size);
    let enabled = !editor.is_recording();
    if ui.put(button_rect, egui::Button::new("Import")).clicked() && enabled {
        messages.extend(pick_and_import(editor));
    }
}

/// The dashed drop zone anchored to the bottom of the panel, 44 tall.
fn drop_zone(ui: &egui::Ui) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let outer = ui.max_rect();
    let rect = egui::Rect::from_min_max(egui::pos2(outer.left(), outer.bottom() - DROP_H), outer.right_bottom());
    let outline = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    let stroke = egui::Stroke::new(HAIRLINE, neutral.stroke_separator_strong);
    ui.painter().extend(egui::Shape::dashed_line(&outline, stroke, DASH, DASH_GAP));
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, DROP_HINT, egui::FontId::proportional(typography::CAPTION_SIZE), neutral.text_tertiary);
}

/// Opens the import file picker (WAV and MP3) and starts one import job per file.
///
/// Returns the names of files rejected because of an unsupported extension.
/// Does nothing (no picker) while a take is being recorded.
pub fn pick_and_import(editor: &mut Editor) -> Vec<String> {
    if editor.is_recording() {
        return Vec::new();
    }
    let Some(files) = rfd::FileDialog::new().add_filter("Audio (WAV, MP3)", &["wav", "mp3"]).pick_files() else {
        return Vec::new();
    };
    editor.import_files(files)
}

/// Opens the relink file picker (WAV and MP3) and starts one relink job.
///
/// The job copies the chosen file into media/ for the offline item. Does
/// nothing (no picker) while a take is being recorded.
fn pick_and_relink(editor: &mut Editor, id: Uuid) -> Vec<String> {
    if editor.is_recording() {
        return Vec::new();
    }
    let Some(file) = rfd::FileDialog::new().add_filter("Audio (WAV, MP3)", &["wav", "mp3"]).pick_file() else {
        return Vec::new();
    };
    editor.relink(id, file);
    Vec::new()
}

/// Returns the bin items in the given sort order.
fn sorted_items(items: &[MediaItem], sort: BinSort) -> Vec<MediaItem> {
    let mut items = items.to_vec();
    if sort == BinSort::Name {
        items.sort_by_key(|a| a.display_name.to_lowercase());
    }
    items
}

/// The length as `m:ss.mmm` at the project sample rate.
fn duration_text(rate: u32, length: u64) -> String {
    let total_ms = length * 1000 / u64::from(rate);
    let min = total_ms / 60_000;
    let sec = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{min}:{sec:02}.{ms:03}")
}

/// The meta line: time, rate, channels and format, two spaces apart, e.g.
/// `0:06.200  48k  St  MP3`.
fn meta_line(editor: &Editor, item: &MediaItem) -> String {
    let channels = if item.channels <= 1 { "Mo" } else { "St" };
    format!("{}  {}k  {channels}  {}", duration_text(editor.project.sample_rate, item.length), item.original_rate / 1000, media_format(&item.file_name))
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
    if usage == 1 { "1 use".to_string() } else { format!("{usage} uses") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_util::Headless;

    fn item(name: &str) -> MediaItem {
        MediaItem { id: Uuid::new_v4(), display_name: name.to_string(), file_name: format!("{name}.wav"), original_rate: 44_100, channels: 1, length: 48_000 }
    }

    fn test_editor() -> Editor {
        Editor::with_project(
            crate::model::project::Project::new("Test".to_string(), 48_000, crate::model::project::BitDepth::Int24),
            crate::model::project::ViewState::default(),
            crate::io::paths::ProjectPaths { root: std::path::PathBuf::from("unused") },
        )
    }

    fn with_items(names: &[&str]) -> Editor {
        let mut editor = test_editor();
        let project = std::sync::Arc::make_mut(&mut editor.project);
        project.media.extend(names.iter().map(|name| item(name)));
        editor
    }

    /// Runs one frame of the panel on a 1280 x 800 screen.
    fn frame(h: &mut Headless, panel: &mut BinPanel, editor: &mut Editor) -> crate::ui::test_util::Out {
        h.frame(Vec::new(), |ui| {
            ui.set_max_width(236.0);
            let _ = panel.show(ui, editor);
        })
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
    fn meta_line_separates_the_values_with_spaces() {
        let editor = test_editor();
        let line = meta_line(&editor, &item("tone"));
        assert_eq!(line, "0:01.000  44k  Mo  WAV");
        assert!(line.is_ascii(), "no dot or other separator glyph");
    }

    #[test]
    fn import_and_relink_open_no_picker_while_recording() {
        // A picker would block on a native dialog: the test thread would not
        // come back, and `within_secs` fails it.
        let result = crate::ui::test_util::within_secs(10, || {
            let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[]], "Rec");
            editor.start_recording().expect("a take");
            let imported = pick_and_import(&mut editor);
            let relinked = pick_and_relink(&mut editor, uuid::Uuid::nil());
            (imported.len(), relinked.len(), editor.take_is_live())
        });
        assert_eq!(result, (0, 0, true));
    }

    #[test]
    fn header_has_the_media_title_and_the_sort_and_import_buttons() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut panel, mut editor) = (BinPanel::default(), with_items(&["tone"]));
        let out = frame(&mut h, &mut panel, &mut editor);
        let title = out.texts().into_iter().find(|t| t.text == "Media").expect("the title");
        assert_eq!(title.color, color::TEXT_PRIMARY);
        let sort = out.text_rect(regular::ARROWS_DOWN_UP).expect("the Sort button");
        let import = out.text_rect(regular::DOWNLOAD_SIMPLE).expect("the Import button");
        assert!(title.rect.right() < sort.left() && sort.right() < import.left(), "title, then Sort, then Import");
        assert!((sort.center().y - title.rect.center().y).abs() < 1.0, "one header row");
    }

    #[test]
    fn rows_show_the_name_and_the_duration_on_the_right() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut panel, mut editor) = (BinPanel::default(), with_items(&["tone", "hum"]));
        let out = frame(&mut h, &mut panel, &mut editor);
        let texts = out.texts();
        let name = texts.iter().find(|t| t.text == "tone").expect("the name");
        let duration = texts.iter().find(|t| t.text == "0:01.000").expect("the duration");
        assert_eq!(duration.color, color::TEXT_TERTIARY);
        assert!(duration.rect.left() > name.rect.right(), "the duration is right of the name");
        assert!(out.text_rect(regular::WAVEFORM).is_some(), "the tile glyph");
        assert!(out.text_rect(DROP_HINT).is_some(), "the drop zone stays under the list");
        assert!(out.text_rect("Import").is_none(), "no empty state button");
    }

    #[test]
    fn a_long_name_is_cut_before_the_duration() {
        let long = "a_very_long_recording_name_that_cannot_fit_in_a_narrow_panel_at_all";
        let fits = crate::ui::test_util::within_secs(30, move || {
            let mut h = Headless::new(1280.0, 800.0);
            let (mut panel, mut editor) = (BinPanel::default(), with_items(&[long]));
            let out = frame(&mut h, &mut panel, &mut editor);
            let texts = out.texts();
            let name = texts.iter().find(|t| t.text.starts_with("a_very")).expect("the name");
            let duration = texts.iter().find(|t| t.text == "0:01.000").expect("the duration");
            name.rect.right() < duration.rect.left()
        });
        assert!(fits, "the cut name stops before the duration");
    }

    #[test]
    fn clicking_a_row_selects_it() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut panel, mut editor) = (BinPanel::default(), with_items(&["tone", "hum"]));
        let out = frame(&mut h, &mut panel, &mut editor);
        let target = out.text_rect("hum").expect("the second row").center();
        let _ = h.click(target, |ui| {
            ui.set_max_width(236.0);
            let _ = panel.show(ui, &mut editor);
        });
        let out = frame(&mut h, &mut panel, &mut editor);
        let hum = out.text_rect("hum").expect("the row").center();
        let row = out.filled_rect_at(hum).expect("a row fill");
        assert_eq!(row.height(), size::LIST_ROW_HEIGHT);
        let selected = out.rects().into_iter().any(|r| r.fill == color::BG_ROW_SELECTED && r.rect == row);
        assert!(selected, "the clicked row has the selected fill");
    }

    #[test]
    fn the_empty_bin_shows_the_hint_once_and_an_import_button() {
        let mut h = Headless::new(1280.0, 800.0);
        let (mut panel, mut editor) = (BinPanel::default(), test_editor());
        let out = frame(&mut h, &mut panel, &mut editor);
        let hints: Vec<_> = out.texts().into_iter().filter(|t| t.text == DROP_HINT).collect();
        assert_eq!(hints.len(), 1, "no second hint in a drop zone");
        assert_eq!(hints[0].color, color::TEXT_TERTIARY);
        let button = out.text_rect("Import").expect("the Import button");
        assert!(button.top() > hints[0].rect.bottom(), "the button sits under the hint");
        assert!((button.center().x - hints[0].rect.center().x).abs() < 1.0, "both centered");
    }
}
