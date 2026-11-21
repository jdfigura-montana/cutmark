//! ui::dialogs::export: the Export dialog, progress window and clip warning.

use std::path::PathBuf;

use eframe::egui;
use uuid::Uuid;

use super::{FORM_W, NARROW_W};
use crate::editor::Editor;
use crate::editor::jobs::ExportSettings;
use crate::editor::selection::Selection;
use crate::io::export::{ExportFormat, ExportKind, mp3_rate};
use crate::model::project::BitDepth;
use ui_kit::fonts;
use ui_kit::tokens::{color, size, space, typography};

/// How long the finished-export banner stays in the status bar.
pub const BANNER_SECONDS: f64 = 10.0;

/// Which part of the timeline an export covers.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Scope {
    /// From 0 to the end of the last clip.
    #[default]
    Whole,
    /// The current range selection.
    Selection,
}

/// Status-bar banner for a finished export, shown for [`BANNER_SECONDS`]
/// with a Reveal link.
pub struct ExportBanner {
    /// File name shown in the banner.
    pub name: String,
    /// Full path handed to the Reveal link.
    pub path: PathBuf,
    /// egui time in seconds when the export finished.
    pub at: f64,
}

/// The Export dialog (Project > Export, Ctrl+E).
///
/// Lives in the `ProjectMenu` for the whole session, so the choices survive
/// closing and reopening the dialog; they are never saved to disk.
pub struct ExportDialog {
    /// Whether the dialog is shown.
    pub open: bool,
    /// Whole project or current selection.
    scope: Scope,
    /// Restrict the export to the range selection's tracks.
    tracks_only: bool,
    /// MP3 instead of WAV.
    mp3: bool,
    /// WAV bit depth.
    bit_depth: BitDepth,
    /// MP3 bitrate in kbps.
    kbps: u32,
    /// Mono instead of stereo.
    mono: bool,
    /// Target file path as typed or picked.
    path: String,
    /// Path whose overwrite was already confirmed (by the native save dialog
    /// or our own confirm), so Export does not ask about it again.
    confirmed: Option<PathBuf>,
    /// Path waiting for an overwrite confirmation, if any.
    overwrite_ask: Option<PathBuf>,
    /// Red error text shown inside the dialog.
    error: Option<String>,
}

impl ExportDialog {
    /// Creates the dialog open with the project defaults: whole project, WAV
    /// at the project's bit depth, stereo, `exports/<name>.wav`.
    pub fn new_open(editor: &Editor) -> Self {
        let depth = editor.project.export_bit_depth;
        ExportDialog {
            open: true,
            scope: Scope::Whole,
            tracks_only: false,
            mp3: false,
            bit_depth: depth,
            kbps: 192,
            mono: false,
            path: editor.default_export_path(ExportKind::Wav(depth)).to_string_lossy().into_owned(),
            confirmed: None,
            overwrite_ask: None,
            error: None,
        }
    }

    /// Reopens the dialog, keeping the last choices.
    pub fn reopen(&mut self) {
        self.open = true;
        self.error = None;
    }

    /// Shows the dialog when open. Export resolves the choices into
    /// `ExportSettings` and calls `Editor::start_export`; Cancel, the close
    /// box or Esc dismisses it.
    pub fn show(&mut self, ctx: &egui::Context, editor: &mut Editor) {
        if !self.open {
            return;
        }
        // Overwrite confirmed: re-run the export, the ask is now skipped.
        if self.show_overwrite_ask(ctx) == Some(true) {
            self.try_export(editor);
        }
        let mut open = true;
        let (mut clicked, mut enter, mut cancel) = (false, false, false);
        let locked = editor.is_locked();
        let range = has_range(editor);
        // Without a range selection the dialog can only export the whole project.
        if !range {
            self.scope = Scope::Whole;
        }
        let shown_editor: &Editor = editor;
        super::dialog(
            ctx,
            "export_dialog",
            "Export",
            FORM_W,
            &mut open,
            |ui| {
                super::description(ui, "Render the mix to a WAV or MP3 file.");
                super::scroll_form(ui, |ui| {
                    ui_kit::inspector_group(ui, |ui| {
                        ui_kit::inspector_row(
                            ui,
                            "Scope",
                            |ui| {
                                ui.add_enabled_ui(range, |ui| {
                                    super::choice(
                                        ui,
                                        "export_scope",
                                        &[(Scope::Whole, "Whole project"), (Scope::Selection, "Current selection")],
                                        &mut self.scope,
                                    );
                                });
                            },
                            None,
                        );
                        ui_kit::inspector_row(
                            ui,
                            "Selected tracks only",
                            |ui| {
                                ui.add_enabled_ui(self.scope == Scope::Selection, |ui| {
                                    ui_kit::switch(ui, &mut self.tracks_only);
                                });
                            },
                            None,
                        );
                        let was_mp3 = self.mp3;
                        ui_kit::inspector_row(ui, "Format", |ui| super::choice(ui, "export_fmt", &[(false, "WAV"), (true, "MP3")], &mut self.mp3), None);
                        if self.mp3 != was_mp3 {
                            self.path = swap_extension(&self.path, self.extension());
                            self.confirmed = None;
                        }
                        match self.kind() {
                            ExportKind::Wav(_) => {
                                ui_kit::inspector_row(
                                    ui,
                                    "Bit depth",
                                    |ui| {
                                        super::choice(
                                            ui,
                                            "export_depth",
                                            &[(BitDepth::Int16, "16-bit"), (BitDepth::Int24, "24-bit"), (BitDepth::Float32, "32-bit float")],
                                            &mut self.bit_depth,
                                        )
                                    },
                                    None,
                                );
                            }
                            ExportKind::Mp3 { .. } => {
                                ui_kit::inspector_row(
                                    ui,
                                    "Bitrate",
                                    |ui| {
                                        super::choice(ui, "export_kbps", &[(128, "128"), (192, "192"), (256, "256"), (320, "320")], &mut self.kbps);
                                        ui.label(
                                            egui::RichText::new(format!(
                                                "kbps \u{2014} exports at {} kHz",
                                                rate_khz(mp3_rate(shown_editor.project.sample_rate))
                                            ))
                                            .font(fonts::medium(typography::CAPTION_SIZE))
                                            .color(color::TEXT_TERTIARY),
                                        );
                                    },
                                    None,
                                );
                            }
                        }
                        ui_kit::inspector_row(
                            ui,
                            "Channels",
                            |ui| super::choice(ui, "export_channels", &[(false, "Stereo"), (true, "Mono")], &mut self.mono),
                            None,
                        );
                        ui_kit::inspector_row(
                            ui,
                            "File",
                            |ui| {
                                let browse = super::with_browse(ui, |ui| {
                                    ui_kit::text_field(ui, &mut self.path, "File path");
                                });
                                if browse {
                                    self.browse(shown_editor);
                                }
                            },
                            None,
                        );
                    });
                    if let Some(error) = &self.error {
                        super::error_text(ui, error);
                    }
                });
                // Enter confirms (when not locked).
                enter = super::enter_pressed(ui) && !locked;
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if super::primary_button(ui, "Export", !locked).clicked() {
                    clicked = true;
                }
            },
        );
        // Cancel, Esc or the backdrop dismisses the dialog.
        if cancel || !open {
            self.open = false;
        } else if clicked || enter {
            self.try_export(editor);
        }
    }

    /// Shows the overwrite confirmation when a resolved path exists and was
    /// not confirmed yet; asked exactly once per path.
    ///
    /// Returns `Some(true)` when the user chose Overwrite.
    fn show_overwrite_ask(&mut self, ctx: &egui::Context) -> Option<bool> {
        let path = self.overwrite_ask.clone()?;
        let mut choice: Option<bool> = None;
        let mut open = true;
        super::dialog(
            ctx,
            "export_overwrite_dialog",
            "Overwrite?",
            NARROW_W,
            &mut open,
            |ui| {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
                super::description(ui, &format!("\"{name}\" already exists. Overwrite it?"));
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    choice = Some(false);
                }
                if ui_kit::primary_button(ui, "Overwrite").clicked() {
                    choice = Some(true);
                }
            },
        );
        // Esc or the backdrop cancels the overwrite.
        if choice.is_none() && !open {
            choice = Some(false);
        }
        match choice {
            Some(true) => {
                self.confirmed = Some(path);
                self.overwrite_ask = None;
                Some(true)
            }
            Some(false) => {
                self.overwrite_ask = None;
                Some(false)
            }
            None => None,
        }
    }

    /// Resolves the current choices and starts the export, asking once before
    /// overwriting an existing file.
    fn try_export(&mut self, editor: &mut Editor) {
        self.error = None;
        if editor.is_locked() {
            return;
        }
        let (start, end, tracks) = self.scope_range(editor);
        if end <= start {
            self.error = Some("Nothing to export".to_string());
            return;
        }
        let text = self.path.trim().to_string();
        if text.is_empty() {
            self.error = Some("Choose a file name.".to_string());
            return;
        }
        let path = PathBuf::from(with_extension(&text, self.extension()));
        if path.exists() && self.confirmed.as_ref() != Some(&path) {
            self.overwrite_ask = Some(path);
            return;
        }
        let format = ExportFormat { kind: self.kind(), channels: if self.mono { 1 } else { 2 }, in_rate: editor.project.sample_rate };
        let settings = ExportSettings { path, format, start, end, tracks };
        match editor.start_export(settings) {
            Ok(()) => self.open = false,
            Err(error) => {
                let text = error.to_string();
                self.error = Some(text.strip_prefix("invalid operation: ").unwrap_or(&text).to_string());
            }
        }
    }

    /// The export range: the whole project (0 to the last clip) or the range
    /// selection, with its tracks when "Selected tracks only" is set.
    fn scope_range(&self, editor: &Editor) -> (u64, u64, Option<Vec<Uuid>>) {
        if self.scope == Scope::Selection
            && let Selection::Range { start, end, tracks } = editor.selection()
        {
            let tracks = self.tracks_only.then(|| tracks.clone());
            return (*start, *end, tracks);
        }
        (0, editor.project_length(), None)
    }

    /// The chosen file kind from the format toggles.
    fn kind(&self) -> ExportKind {
        if self.mp3 { ExportKind::Mp3 { kbps: self.kbps } } else { ExportKind::Wav(self.bit_depth) }
    }

    /// The file extension of the chosen format.
    fn extension(&self) -> &'static str {
        if self.mp3 { "mp3" } else { "wav" }
    }

    /// Opens the native save dialog in exports/, filtered to the chosen
    /// format and prefilled with the current file name.
    fn browse(&mut self, editor: &Editor) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Export")
            .set_directory(editor.paths.exports())
            .add_filter(if self.mp3 { "MP3 audio" } else { "WAV audio" }, &[self.extension()]);
        if let Some(name) = file_name(&self.path) {
            dialog = dialog.set_file_name(name);
        }
        if let Some(picked) = dialog.save_file() {
            self.path = picked.to_string_lossy().into_owned();
            // The native Windows save dialog asks before overwriting
            // (FOS_OVERWRITEPROMPT is on by default and rfd does not change
            // the dialog options), so this path never needs our own confirm.
            self.confirmed = Some(picked);
        }
    }
}

/// Draws the progress modal and the clipping warning while an export job runs;
/// the modals' backdrops block input to the rest of the UI.
pub fn show_progress(ctx: &egui::Context, editor: &mut Editor) {
    let Some(status) = editor.export_status() else {
        return;
    };
    if status.peak_warning.is_none() {
        // Esc and the backdrop do not dismiss the progress window: it stays
        // for as long as the export job runs.
        let mut open = true;
        let mut cancel = false;
        super::dialog(
            ctx,
            "export_progress",
            "Exporting",
            NARROW_W,
            &mut open,
            |ui| {
                super::description(ui, if status.pass == 1 { "Checking levels..." } else { "Writing file..." });
                progress_bar(ui, status.frac);
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
            },
        );
        if cancel {
            editor.export_cancel();
        }
    }
    if let Some(db) = status.peak_warning {
        let mut open = true;
        let (mut clicked, mut enter, mut cancel) = (false, false, false);
        super::dialog(
            ctx,
            "export_clipping",
            "Clipping",
            NARROW_W,
            &mut open,
            |ui| {
                super::warning_description(ui, &format!("The mix peaks at +{db:.1} dBFS and will clip."));
                // Enter keeps the export going.
                enter = super::enter_pressed(ui);
            },
            |ui| {
                if super::secondary_button(ui, "Cancel").clicked() {
                    cancel = true;
                }
                if ui_kit::primary_button(ui, "Export Anyway").clicked() {
                    clicked = true;
                }
            },
        );
        if clicked || enter {
            editor.export_continue();
        } else if cancel || !open {
            // Cancel, Esc and the backdrop cancel the export.
            editor.export_cancel();
        }
    }
    // The export reports progress through poll_jobs; keep the UI advancing.
    ctx.request_repaint();
}

/// The progress bar: a `bg.control` capsule track, the theme accent fill, and
/// the percent in the mono value type on the right.
fn progress_bar(ui: &mut egui::Ui, frac: f32) {
    let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), size::SEGMENT_HEIGHT), egui::Sense::hover());
    let percent_w = space::SCALE_24 * 2.0 + space::SCALE_4;
    let bar = egui::Rect::from_center_size(
        egui::pos2(row.left() + (row.width() - percent_w) / 2.0, row.center().y),
        egui::vec2(row.width() - percent_w, space::SCALE_6),
    );
    let radius = space::SCALE_6 / 2.0;
    let painter = ui.painter();
    painter.rect_filled(bar, radius, color::BG_CONTROL);
    if frac > 0.0 {
        let fill = egui::Rect::from_min_max(bar.min, egui::pos2(bar.min.x + bar.width() * frac, bar.max.y));
        painter.rect_filled(fill, radius, ui_kit::Theme::get(ui.ctx()).accent.accent);
    }
    painter.text(
        egui::pos2(row.right(), row.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{}%", (frac * 100.0).round() as i32),
        fonts::mono(typography::VALUE_SIZE),
        color::TEXT_SECONDARY,
    );
}

/// True when the editor holds a range selection.
fn has_range(editor: &Editor) -> bool {
    matches!(editor.selection(), Selection::Range { .. })
}

/// Replaces a `.wav`/`.mp3` extension in `path` with `ext`; any other path is
/// left unchanged.
fn swap_extension(path: &str, ext: &str) -> String {
    if ends_with_audio_extension(path) { format!("{}.{}", &path[..path.len() - 4], ext) } else { path.to_string() }
}

/// `path` with its extension forced to `ext`: a wav/mp3 extension is
/// replaced, a missing one is appended.
fn with_extension(path: &str, ext: &str) -> String {
    if ends_with_audio_extension(path) { format!("{}.{}", &path[..path.len() - 4], ext) } else { format!("{path}.{ext}") }
}

/// True when `path` ends in `.wav` or `.mp3` (case-insensitive).
fn ends_with_audio_extension(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".wav") || lower.ends_with(".mp3")
}

/// The file-name part of a path text, if it has one.
fn file_name(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    std::path::Path::new(trimmed).file_name().map(|n| n.to_string_lossy().into_owned())
}

/// A sample rate in kHz, formatted like "48" or "44.1".
fn rate_khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) { (rate / 1000).to_string() } else { format!("{:.1}", rate as f32 / 1000.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::paths::ProjectPaths;
    use crate::model::project::{Project, ViewState};
    use crate::ui::test_util::Headless;

    #[test]
    fn export_dialog_card_is_compact_and_holds_title_format_and_footer() {
        let mut h = Headless::new(1280.0, 800.0);
        let mut editor = Editor::with_project(
            Project::new("Test 2".to_string(), 48_000, BitDepth::Int24),
            ViewState::default(),
            ProjectPaths { root: PathBuf::from("unused") },
        );
        let mut dialog = ExportDialog::new_open(&editor);
        // The modal's first frame is hidden while egui measures it and it
        // then fades in; settle runs past both.
        let out = h.settle(|ui| dialog.show(ui.ctx(), &mut editor));

        // The card: the largest filled, outlined rect (the modal is still
        // fading in, so its colors are not exactly the theme values yet).
        let card = out
            .rects()
            .into_iter()
            .filter(|r| r.stroke.width > 0.0 && r.fill.a() > 0)
            .map(|r| r.rect)
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("the dialog card is painted");
        assert!(card.height() < 640.0, "card is {} px tall on an 800 px window", card.height());
        assert!(card.top() > 0.0 && card.bottom() < 800.0, "card is on screen");

        // The title is the first "Export", the footer button the last one.
        let texts = out.texts();
        let exports: Vec<_> = texts.iter().filter(|t| t.text == "Export").collect();
        assert_eq!(exports.len(), 2, "title and footer button");
        let title = exports[0].rect;
        let footer_export = exports[1].rect;
        let format = out.text_rect("Format").expect("Format row label");
        let cancel = out.text_rect("Cancel").expect("footer Cancel button");
        for (name, rect) in [("title", title), ("Format row", format), ("footer Export", footer_export), ("footer Cancel", cancel)] {
            assert!(card.contains_rect(rect), "{name} {rect:?} must sit inside the card {card:?}");
        }

        // Rows are laid out one under another, not stacked on one spot.
        let scope = out.text_rect("Scope").expect("Scope row label");
        let channels = out.text_rect("Channels").expect("Channels row label");
        let file = out.text_rect("File").expect("File row label");
        let ys = [title.center().y, scope.center().y, format.center().y, channels.center().y, file.center().y, footer_export.center().y];
        assert!(ys.windows(2).all(|pair| pair[0] + 10.0 < pair[1]), "rows must run top to bottom: {ys:?}");
        assert!(cancel.center().x < footer_export.center().x, "Cancel then Export, left to right");

        // The card is exactly FORM_W wide.
        let tol = 0.6;
        let line = out.dialog_content().expect("dialog card");
        assert!((card.width() - FORM_W).abs() < tol, "card is {} wide", card.width());

        // Footer buttons: 8 px apart, the primary one last and flush with the content.
        let export_button = out.filled_rect_at(footer_export.center()).expect("footer Export button");
        let cancel_button = out.filled_rect_at(cancel.center()).expect("footer Cancel button");
        assert!(
            (export_button.left() - cancel_button.right() - ui_kit::tokens::space::SCALE_8).abs() < tol,
            "gap is {}",
            export_button.left() - cancel_button.right()
        );
        assert!((export_button.right() - line.right()).abs() < tol, "Export ends {} short of the content edge", line.right() - export_button.right());

        // The Browse button ends at the row's control edge: the group margin,
        // the row inset and one gap (4 + 4 + 8) inside the content edge.
        let browse = out.text_rect("Browse...").expect("Browse button text");
        let browse_button = out.filled_rect_at(browse.center()).expect("Browse button");
        let edge = line.right() - ui_kit::tokens::space::SCALE_16;
        assert!((browse_button.right() - edge).abs() < tol, "Browse ends {} off the row's control edge", edge - browse_button.right());
        assert!(out.texts().iter().all(|t| t.rect.right() <= line.right() + tol), "no text runs past the card content");
    }

    #[test]
    fn swap_extension_only_touches_wav_and_mp3() {
        assert_eq!(swap_extension("out.wav", "mp3"), "out.mp3");
        assert_eq!(swap_extension("C:\\a\\OUT.WAV", "mp3"), "C:\\a\\OUT.mp3");
        assert_eq!(swap_extension("out.mp3", "wav"), "out.wav");
        assert_eq!(swap_extension("song.flac", "mp3"), "song.flac");
        assert_eq!(swap_extension("noext", "mp3"), "noext");
    }

    #[test]
    fn with_extension_replaces_or_appends() {
        assert_eq!(with_extension("out.wav", "mp3"), "out.mp3");
        assert_eq!(with_extension("out", "wav"), "out.wav");
        assert_eq!(with_extension("song.flac", "mp3"), "song.flac.mp3");
    }

    #[test]
    fn rate_khz_shows_44_1_as_fraction() {
        assert_eq!(rate_khz(48_000), "48");
        assert_eq!(rate_khz(44_100), "44.1");
    }

    #[test]
    fn file_name_takes_the_last_component() {
        assert_eq!(file_name("C:\\a\\out.wav"), Some("out.wav".to_string()));
        assert_eq!(file_name("out.wav"), Some("out.wav".to_string()));
        assert_eq!(file_name("  "), None);
    }
}
