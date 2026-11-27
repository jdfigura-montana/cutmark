//! How the Recorder draws the numbers a user reads as data: in Martian Mono, inside a sentence
//! or as a stat block (docs/suite/Recorder-E3.md, section 5).

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Context, Label, RichText, TextStyle, Ui};
use ui_kit::Theme;
use ui_kit::tokens::typography;

/// The size of a stat block's value (docs/suite/Design-Language-E3.md, section 4).
const STAT_VALUE_SIZE: f32 = 15.0;

/// Draws every number field (drag values and slider value boxes) in Martian Mono.
pub fn apply(ctx: &Context) {
    ctx.all_styles_mut(|style| style.drag_value_text_style = TextStyle::Monospace);
}

/// A picture size as it is shown, with a multiplication sign between width and height.
pub fn size_text(width: u32, height: u32) -> String {
    format!("{width}\u{d7}{height}")
}

/// One line of caption text in which each of `data`, found in order, is in Martian Mono. Pass
/// [`Color32::PLACEHOLDER`] to let the widget that draws the job pick the color.
pub fn with_data(text: &str, data: &[&str], color: Color32) -> LayoutJob {
    let words = TextFormat::simple(ui_kit::fonts::medium(typography::CAPTION_SIZE), color);
    let mono = ui_kit::fonts::mono(typography::VALUE_SIZE);
    let extra_letter_spacing = mono.size * ui_kit::fonts::MONO_LETTER_SPACING_EM;
    let mono = TextFormat { extra_letter_spacing, ..TextFormat::simple(mono, color) };
    let mut job = LayoutJob::default();
    let mut rest = text;
    for part in data {
        let Some((before, after)) = rest.split_once(part) else { continue };
        if !before.is_empty() {
            job.append(before, 0.0, words.clone());
        }
        job.append(part, 0.0, mono.clone());
        rest = after;
    }
    if !rest.is_empty() {
        job.append(rest, 0.0, words);
    }
    job
}

/// One stat block: the value in Martian Mono 15 with its label in a caption under it.
pub fn stat_block(ui: &mut Ui, value: &str, label: &str) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let value = RichText::new(value).font(ui_kit::fonts::mono(STAT_VALUE_SIZE)).color(neutrals.text_primary);
    let label = RichText::new(label).font(ui_kit::fonts::medium(typography::CAPTION_SIZE)).color(neutrals.text_tertiary);
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(Label::new(value).extend().selectable(false));
        ui.add(Label::new(label).extend().selectable(false));
    });
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// The sections of `job` as `(text, in Martian Mono)`.
    fn parts(job: &LayoutJob) -> Vec<(&str, bool)> {
        job.sections
            .iter()
            .map(|section| (&job.text[section.byte_range.start.0..section.byte_range.end.0], section.format.font_id.family == egui::FontFamily::Monospace))
            .collect()
    }

    #[test]
    fn only_the_data_in_a_line_is_in_mono() {
        let job = with_data("Offset +83 ms over 12 flashes", &["+83 ms", "12"], Color32::WHITE);
        assert_eq!(parts(&job), [("Offset ", false), ("+83 ms", true), (" over ", false), ("12", true), (" flashes", false)]);
        assert_eq!(parts(&with_data("5.0 GB free", &["5.0 GB"], Color32::WHITE)), [("5.0 GB", true), (" free", false)]);
        assert_eq!(parts(&with_data("Off", &[], Color32::WHITE)), [("Off", false)]);
    }

    #[test]
    fn a_size_is_written_with_a_multiplication_sign() {
        assert_eq!(size_text(2560, 1440), "2560\u{d7}1440");
    }

    /// Every Rust file under `dir`.
    fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    /// No line of code in the Recorder joins strings with a spaced middle dot, written either
    /// as the character or as its escape. Comments are skipped, and so is a line marked
    /// "allow: spaced middle dot".
    #[test]
    fn no_recorder_string_is_joined_with_a_middle_dot() {
        let mut files = Vec::new();
        sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
        assert!(files.len() > 20, "the sources were found: {files:?}");
        let needles = [format!(" {} ", '\u{b7}'), format!(" \\u{{{}}} ", "b7"), format!(" \\u{{{}}} ", "00b7")];
        for file in files {
            let text = std::fs::read_to_string(&file).unwrap();
            for (number, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") || line.ends_with("// allow: spaced middle dot") {
                    continue;
                }
                let lower = line.to_lowercase();
                assert!(!needles.iter().any(|needle| lower.contains(needle)), "{}:{} joins strings with a dot: {line}", file.display(), number + 1);
            }
        }
    }
}
