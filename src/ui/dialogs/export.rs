//! ui::dialogs::export: the Export dialog, progress window and clip warning.

use std::path::PathBuf;

use eframe::egui;
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::jobs::ExportSettings;
use crate::editor::selection::Selection;
use crate::io::export::{ExportFormat, ExportKind, mp3_rate};
use crate::model::project::BitDepth;
use crate::ui::theme::THEME;

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
            path: editor
                .default_export_path(ExportKind::Wav(depth))
                .to_string_lossy()
                .into_owned(),
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
        blocker(ctx);
        let mut open = self.open;
        egui::Window::new("Export")
            .open(&mut open)
            .default_width(THEME.dialog_w)
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("export_grid")
                    .num_columns(2)
                    .spacing([THEME.space_md, THEME.grid_spacing])
                    .show(ui, |ui| {
                        ui.label("Scope:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.scope, Scope::Whole, "Whole project");
                            ui.add_enabled_ui(has_range(editor), |ui| {
                                ui.radio_value(
                                    &mut self.scope,
                                    Scope::Selection,
                                    "Current selection",
                                );
                            });
                        });
                        ui.end_row();

                        ui.label("");
                        ui.add_enabled_ui(self.scope == Scope::Selection, |ui| {
                            ui.checkbox(&mut self.tracks_only, "Selected tracks only");
                        });
                        ui.end_row();

                        ui.label("Format:");
                        let was_mp3 = self.mp3;
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.mp3, false, "WAV");
                            ui.radio_value(&mut self.mp3, true, "MP3");
                        });
                        if self.mp3 != was_mp3 {
                            self.path = swap_extension(&self.path, self.extension());
                            self.confirmed = None;
                        }
                        ui.end_row();

                        match self.kind() {
                            ExportKind::Wav(_) => {
                                ui.label("Bit depth:");
                                egui::ComboBox::from_id_salt("export_bit_depth")
                                    .selected_text(bit_depth_text(self.bit_depth))
                                    .show_ui(ui, |ui| {
                                        for depth in
                                            [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32]
                                        {
                                            ui.selectable_value(
                                                &mut self.bit_depth,
                                                depth,
                                                bit_depth_text(depth),
                                            );
                                        }
                                    });
                                ui.end_row();
                            }
                            ExportKind::Mp3 { .. } => {
                                ui.label("Bitrate:");
                                ui.vertical(|ui| {
                                    egui::ComboBox::from_id_salt("export_bitrate")
                                        .selected_text(format!("{} kbps", self.kbps))
                                        .show_ui(ui, |ui| {
                                            for kbps in [128, 192, 256, 320] {
                                                ui.selectable_value(
                                                    &mut self.kbps,
                                                    kbps,
                                                    format!("{kbps} kbps"),
                                                );
                                            }
                                        });
                                    ui.label(format!(
                                        "Exports at {} kHz",
                                        rate_khz(mp3_rate(editor.project.sample_rate))
                                    ));
                                });
                                ui.end_row();
                            }
                        }

                        ui.label("Channels:");
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut self.mono, false, "Stereo");
                            ui.radio_value(&mut self.mono, true, "Mono");
                        });
                        ui.end_row();

                        ui.label("File:");
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.path)
                                    .desired_width(THEME.dialog_w * 0.55),
                            );
                            if ui.button("Browse...").clicked() {
                                self.browse(editor);
                            }
                        });
                        ui.end_row();
                    });

                if let Some(error) = &self.error {
                    ui.label(egui::RichText::new(error).color(THEME.error));
                }
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!editor.is_locked(), egui::Button::new("Export"))
                        .clicked()
                    {
                        self.try_export(editor);
                    }
                    if ui.button("Cancel").clicked() {
                        self.open = false;
                    }
                });
                // Enter confirms, Esc cancels.
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !editor.is_locked() {
                    self.try_export(editor);
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.open = false;
                }
            });
        self.open = open;
    }

    /// Shows the overwrite confirmation when a resolved path exists and was
    /// not confirmed yet; asked exactly once per path.
    ///
    /// Returns `Some(true)` when the user chose Overwrite.
    fn show_overwrite_ask(&mut self, ctx: &egui::Context) -> Option<bool> {
        let path = self.overwrite_ask.clone()?;
        let mut choice: Option<bool> = None;
        egui::Window::new("Overwrite?")
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());
                ui.label(format!("\"{name}\" already exists. Overwrite it?"));
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui.button("Overwrite").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(false);
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(false);
                }
            });
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
        let format = ExportFormat {
            kind: self.kind(),
            channels: if self.mono { 1 } else { 2 },
            in_rate: editor.project.sample_rate,
        };
        let settings = ExportSettings {
            path,
            format,
            start,
            end,
            tracks,
        };
        match editor.start_export(settings) {
            Ok(()) => self.open = false,
            Err(error) => {
                let text = error.to_string();
                self.error = Some(
                    text.strip_prefix("invalid operation: ")
                        .unwrap_or(&text)
                        .to_string(),
                );
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
        if self.mp3 {
            ExportKind::Mp3 { kbps: self.kbps }
        } else {
            ExportKind::Wav(self.bit_depth)
        }
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
            .add_filter(
                if self.mp3 { "MP3 audio" } else { "WAV audio" },
                &[self.extension()],
            );
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

/// Draws the progress window and the clip warning while an export job runs,
/// with a full-screen blocker so nothing else in the UI reacts to clicks.
pub fn show_progress(ctx: &egui::Context, editor: &mut Editor) {
    let Some(status) = editor.export_status() else {
        return;
    };
    blocker(ctx);
    egui::Window::new("Exporting")
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.label(if status.pass == 1 {
                "Checking levels..."
            } else {
                "Writing file..."
            });
            ui.add(egui::ProgressBar::new(status.frac).show_percentage());
            ui.add_space(THEME.space_md);
            ui.vertical_centered(|ui| {
                if ui.button("Cancel").clicked() {
                    editor.export_cancel();
                }
            });
        });
    if let Some(db) = status.peak_warning {
        egui::Window::new("Clipping")
            .collapsible(false)
            .resizable(false)
            .movable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("The mix peaks at +{db:.1} dBFS and will clip."));
                ui.add_space(THEME.space_md);
                ui.horizontal(|ui| {
                    if ui.button("Export Anyway").clicked() {
                        editor.export_continue();
                    }
                    if ui.button("Cancel").clicked() {
                        editor.export_cancel();
                    }
                });
            });
    }
    // The export reports progress through poll_jobs; keep the UI advancing.
    ctx.request_repaint();
}

/// Full-screen translucent layer that swallows every click so the modal
/// window shown above it is the only thing that reacts.
fn blocker(ctx: &egui::Context) {
    egui::Area::new(egui::Id::new("modal_click_blocker"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::LEFT_TOP, [0.0, 0.0])
        .show(ctx, |ui| {
            let size = ctx.viewport_rect().size();
            let rect = egui::Rect::from_min_size(ui.cursor().min, size);
            let _response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0.0, THEME.modal_shade);
        });
}

/// True when the editor holds a range selection.
fn has_range(editor: &Editor) -> bool {
    matches!(editor.selection(), Selection::Range { .. })
}

/// Replaces a `.wav`/`.mp3` extension in `path` with `ext`; any other path is
/// left unchanged.
fn swap_extension(path: &str, ext: &str) -> String {
    if ends_with_audio_extension(path) {
        format!("{}.{}", &path[..path.len() - 4], ext)
    } else {
        path.to_string()
    }
}

/// `path` with its extension forced to `ext`: a wav/mp3 extension is
/// replaced, a missing one is appended.
fn with_extension(path: &str, ext: &str) -> String {
    if ends_with_audio_extension(path) {
        format!("{}.{}", &path[..path.len() - 4], ext)
    } else {
        format!("{path}.{ext}")
    }
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
    std::path::Path::new(trimmed)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

/// A sample rate in kHz, formatted like "48" or "44.1".
fn rate_khz(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        (rate / 1000).to_string()
    } else {
        format!("{:.1}", rate as f32 / 1000.0)
    }
}

/// Human-readable text for a bit depth.
fn bit_depth_text(depth: BitDepth) -> &'static str {
    match depth {
        BitDepth::Int16 => "16-bit",
        BitDepth::Int24 => "24-bit",
        BitDepth::Float32 => "32-bit float",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
