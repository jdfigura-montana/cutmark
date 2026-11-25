//! The Snapshots dialog: the open script's kept copies, with preview and restore.

use chrono::Local;
use prompter_core::script;
use prompter_core::store::{Result, SnapshotMeta};
use ui_kit::tokens::{color, size, space, typography};

use crate::preview;
use crate::renderer::{self, RenderStyle};
use crate::stats;
use crate::workspace::Workspace;

const LIST_WIDTH: f32 = 440.0;
const PREVIEW_WIDTH: f32 = 560.0;
const PREVIEW_HEIGHT: f32 = 360.0;
/// The window height left to the dialog's title, buttons and margins.
const DIALOG_CHROME: f32 = 200.0;

/// One snapshot as the list shows it.
#[derive(Debug)]
struct Entry {
    meta: SnapshotMeta,
    /// `None` if its file could not be read.
    text: Option<String>,
    words: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    List,
    /// The entry at this index.
    Preview(usize),
    /// Asking before the entry at this index is restored.
    Confirm(usize),
}

#[derive(Debug)]
pub struct Snapshots {
    /// The script's id.
    id: String,
    /// Newest first.
    entries: Vec<Entry>,
    page: Page,
}

impl Snapshots {
    pub fn load(workspace: &Workspace, id: String) -> Result<Self> {
        let entries = entries(workspace, &id)?;
        Ok(Self { id, entries, page: Page::List })
    }

    /// Draws the dialog. Returns false once it is closed.
    pub fn show(&mut self, ctx: &egui::Context, workspace: &mut Workspace) -> (bool, Result<()>) {
        let mut open = true;
        let (mut cancelled, mut take, mut restore) = (false, false, false);
        // The page a button in the body or in the footer asks for.
        let (mut from_body, mut from_footer) = (None, None);
        let page = self.page;
        let entry = match page {
            Page::List => None,
            Page::Preview(at) | Page::Confirm(at) => self.entries.get(at),
        };
        let title = match page {
            Page::List => "Snapshots",
            Page::Preview(_) => "Snapshot preview",
            Page::Confirm(_) => "Restore snapshot?",
        };
        ui_kit::dialog(
            ctx,
            egui::Id::new("snapshots-dialog"),
            title,
            &mut open,
            |ui| match page {
                Page::List => {
                    ui.set_width(LIST_WIDTH);
                    let height = room(ui);
                    let rows = egui::ScrollArea::vertical().max_height(height).auto_shrink([false, true]);
                    from_body = rows.show(ui, |ui| list(ui, &self.entries)).inner;
                }
                Page::Preview(at) => {
                    ui.set_width(PREVIEW_WIDTH);
                    if let Some(entry) = entry {
                        ui.label(summary(entry));
                        snapshot_preview(ui, at, entry.text.as_deref().unwrap_or_default());
                    }
                }
                Page::Confirm(_) => {
                    ui.set_width(LIST_WIDTH);
                    if let Some(entry) = entry {
                        ui.label(format!("The script will go back to its text from {}. The current text is saved as a snapshot first.", time(entry.meta)));
                    }
                }
            },
            |ui| match page {
                Page::List => {
                    cancelled = ui.button("Close").clicked();
                    take = ui_kit::primary_button(ui, "Snapshot now").clicked();
                }
                Page::Preview(at) => {
                    if ui.button("Back").clicked() {
                        from_footer = Some(Page::List);
                    }
                    if ui_kit::primary_button(ui, "Restore").clicked() {
                        from_footer = Some(Page::Confirm(at));
                    }
                }
                Page::Confirm(_) => {
                    if ui.button("Cancel").clicked() {
                        from_footer = Some(Page::List);
                    }
                    restore = ui_kit::primary_button(ui, "Restore").clicked();
                }
            },
        );

        let chosen = entry.map(|entry| entry.meta);
        let mut next = from_body.or(from_footer);
        let mut result = Ok(());
        if take {
            result = workspace.snapshot(&self.id).and_then(|()| {
                self.entries = entries(workspace, &self.id)?;
                Ok(())
            });
            ctx.request_repaint();
        }
        if restore && let Some(meta) = chosen {
            result = workspace.restore(&self.id, meta);
            if result.is_ok() {
                ui_kit::widgets::toast(ctx, "Snapshot restored.");
                open = false;
            } else {
                next = Some(Page::List);
            }
            ctx.request_repaint();
        }
        if let Some(next) = next {
            self.page = next;
            ctx.request_repaint();
        }
        (open && !cancelled, result)
    }
}

fn entries(workspace: &Workspace, id: &str) -> Result<Vec<Entry>> {
    let store = workspace.store();
    let entry = |meta| {
        let text = store.snapshot_text(id, meta).inspect_err(|error| log::warn!("a snapshot could not be read: {error}")).ok();
        let words = text.as_deref().map(stats::spoken_words);
        Entry { meta, text, words }
    };
    Ok(store.snapshots(id)?.into_iter().map(entry).collect())
}

/// A snapshot's time on this computer's clock.
fn time(meta: SnapshotMeta) -> String {
    meta.taken.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()
}

fn summary(entry: &Entry) -> String {
    let words = entry.words.map_or_else(|| "unreadable".to_owned(), stats::word_label);
    format!("{} · {words}", time(entry.meta))
}

/// The height a dialog's body can take in this window.
fn room(ui: &egui::Ui) -> f32 {
    (ui.ctx().content_rect().height() - DIALOG_CHROME).max(size::LIST_ROW_HEIGHT)
}

/// The list of snapshots. Returns the page a row's button asks for.
fn list(ui: &mut egui::Ui, entries: &[Entry]) -> Option<Page> {
    if entries.is_empty() {
        let text = egui::RichText::new("No snapshots yet.").font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY);
        ui.label(text);
        return None;
    }
    let mut next = None;
    for (at, entry) in entries.iter().enumerate() {
        let row = egui::vec2(ui.available_width(), size::LIST_ROW_HEIGHT);
        ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_size(row);
            let text = egui::RichText::new(summary(entry)).font(ui_kit::fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_PRIMARY);
            ui.add(egui::Label::new(text).selectable(false));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::SCALE_6;
                ui.add_enabled_ui(entry.text.is_some(), |ui| {
                    if ui.button("Restore").clicked() {
                        next = Some(Page::Confirm(at));
                    }
                    if ui.button("Preview").clicked() {
                        next = Some(Page::Preview(at));
                    }
                });
            });
        });
    }
    next
}

/// A snapshot as the views would show it.
fn snapshot_preview(ui: &mut egui::Ui, at: usize, text: &str) {
    let style = RenderStyle::preview();
    let area = egui::vec2(ui.available_width(), PREVIEW_HEIGHT.min(room(ui)));
    ui.allocate_ui(area, |ui| {
        ui.set_min_size(area);
        preview::pane(ui, ("snapshot-preview", at), &style, |ui| {
            let blocks = renderer::layout_script(&script::parse(text), &style, ui.available_width());
            renderer::show_blocks(ui, &blocks, &style);
        });
    });
}
