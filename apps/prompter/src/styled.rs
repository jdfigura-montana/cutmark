//! The styled look of the editor's Markdown. The text laid out is the text itself, so that
//! the cursor and selection map to it: hidden syntax is made see-through and next to no
//! width, never left out. Bullets, quote bars and cue chips are drawn around the text.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use egui::text::{ByteIndex, ByteRange, CharIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontId, Rect, Shape, Vec2, pos2, vec2};
use ui_kit::tokens::{color, space, typography};

use crate::markup::{CueKind, ListMarker, Markup, Style};
use crate::renderer::{HEADING_SCALE, LIST_INDENT, MARKER_GAP};

/// The room left of the text, where quote bars go. A bar in the text's own space would run
/// into a quote's wrapped rows, which start at the left edge.
pub const GUTTER: f32 = space::SCALE_12;
const QUOTE_BAR: f32 = space::SCALE_2;
/// The font size of hidden syntax. At zero the text could not be laid out.
const HIDDEN_SIZE: f32 = 0.1;
/// Line height as a multiple of the text size. The room it leaves keeps cue chips apart.
const LINE_HEIGHT: f32 = 1.6;
/// The room a cue chip takes at each side of its cue.
const CHIP_PAD: f32 = space::SCALE_4;
/// How far a chip reaches above and below its text.
const CHIP_REACH: f32 = 1.0;
/// How strongly each cue type's color fills its chip.
const NOTE_TINT: f32 = 0.45;
const ACCENT_TINT: f32 = 0.35;

/// Which lines show their Markdown syntax.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reveal {
    /// The 0-based line the cursor is on.
    pub line: Option<usize>,
    /// The View menu's Show Markdown: every line shows its syntax.
    pub all: bool,
}

impl Reveal {
    /// True if a line's syntax is dimmed, false if it is hidden.
    fn shows(self, line: usize) -> bool {
        self.all || self.line == Some(line)
    }
}

/// The parse of the text last asked for, so that each edit is parsed once.
#[derive(Default)]
pub struct ParseCache {
    last: Option<(u64, Rc<Markup>)>,
    /// How many times text has been parsed.
    #[cfg(test)]
    pub parses: usize,
}

impl ParseCache {
    pub fn get(&mut self, text: &str) -> Rc<Markup> {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some((_, markup)) = self.last.as_ref().filter(|(last, _)| *last == hash) {
            return Rc::clone(markup);
        }
        #[cfg(test)]
        {
            self.parses += 1;
        }
        let markup = Rc::new(Markup::parse(text));
        self.last = Some((hash, Rc::clone(&markup)));
        markup
    }
}

/// The font of body text.
pub fn body_font() -> FontId {
    FontId::proportional(typography::BODY_SIZE)
}

/// The font of a run. Headings are larger by level, and bold.
fn font(style: Style) -> FontId {
    match style.heading {
        Some(level) => {
            let scale = HEADING_SCALE[usize::from(level.saturating_sub(1)).min(HEADING_SCALE.len() - 1)];
            ui_kit::fonts::semibold(typography::BODY_SIZE * scale)
        }
        None if style.bold => ui_kit::fonts::semibold(typography::BODY_SIZE),
        None => body_font(),
    }
}

/// The format of a run. Hidden syntax keeps its line's height, so that revealing it moves
/// nothing up or down.
fn format(style: Style, shown: bool) -> TextFormat {
    let font = font(style);
    let line_height = Some((font.size * LINE_HEIGHT).round());
    if style.marker && !shown {
        return TextFormat { line_height, ..TextFormat::simple(FontId::new(HIDDEN_SIZE, font.family), Color32::TRANSPARENT) };
    }
    let text = if style.marker { color::TEXT_TERTIARY } else { color::TEXT_PRIMARY };
    TextFormat { line_height, italics: style.italic, ..TextFormat::simple(font, text) }
}

fn width(ui: &egui::Ui, font: &FontId, text: &str) -> f32 {
    ui.fonts_mut(|fonts| text.chars().map(|c| fonts.glyph_width(font, c)).sum())
}

/// Where a list item's text starts, from the left edge. A long number widens it.
fn list_indent(ui: &egui::Ui, item: &ListMarker) -> f32 {
    let em = typography::BODY_SIZE;
    let level = LIST_INDENT * em * (item.depth + 1) as f32;
    level.max(width(ui, &body_font(), &item.label) + MARKER_GAP * em)
}

/// The layout of `text`, which `markup` is the parse of. `matches` are byte ranges to back
/// with a color, in order and without overlaps.
pub fn layout_job(ui: &egui::Ui, text: &str, markup: &Markup, reveal: Reveal, matches: &[(Range<usize>, Color32)], wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob { text: text.to_owned(), keep_trailing_whitespace: true, ..Default::default() };
    job.wrap.max_width = wrap_width;
    job.sections.reserve(markup.runs.len() + 2 * matches.len());

    let mut items = markup.items.iter().peekable();
    let mut cues = markup.cues.iter().flat_map(|cue| [cue.range.start, cue.range.end]).peekable();
    let mut matches = matches.iter().peekable();
    for run in &markup.runs {
        let shown = reveal.shows(run.line);
        let format = format(run.style, shown);
        let mut leading_space = 0.0;
        let mut item = None;
        while let Some(next) = items.next_if(|item| item.range.start < run.range.end) {
            item = Some(next);
        }
        if let Some(item) = item {
            // Syntax that shows hangs in the indent, so that the item's text stays put.
            let taken = if shown { width(ui, &format.font_id, &text[item.range.clone()]) } else { 0.0 };
            leading_space = (list_indent(ui, item) - taken).max(0.0);
        }
        let newline = &text[run.range.clone()] == "\n";
        // Room for the chip before a cue's first run and after its last.
        while let Some(edge) = cues.next_if(|&edge| edge <= run.range.start) {
            if edge == run.range.start && !newline {
                leading_space += CHIP_PAD;
            }
        }

        // The run is cut where a match starts or ends.
        let mut from = run.range.start;
        while from < run.range.end {
            while matches.next_if(|(found, _)| found.end <= from).is_some() {}
            let (to, background) = match matches.peek() {
                Some((found, tint)) if found.start <= from => (found.end.min(run.range.end), *tint),
                Some((found, _)) => (found.start.min(run.range.end), Color32::TRANSPARENT),
                None => (run.range.end, Color32::TRANSPARENT),
            };
            let byte_range: ByteRange = ByteIndex(from)..ByteIndex(to);
            job.sections.push(LayoutSection { leading_space, byte_range, format: TextFormat { background, ..format.clone() } });
            leading_space = 0.0;
            from = to;
        }
    }
    job
}

/// What is drawn around the text.
#[derive(Default)]
pub struct Decorations {
    /// Cue chips, which go under the text.
    pub behind: Vec<Shape>,
    /// Bullets, numbers and quote bars.
    pub front: Vec<Shape>,
}

fn chip_color(kind: CueKind, accent: Color32) -> Color32 {
    match kind {
        CueKind::Note => color::TEXT_TERTIARY.gamma_multiply(NOTE_TINT),
        CueKind::Pause => color::BG_HOVER,
        CueKind::Mark => accent.gamma_multiply(ACCENT_TINT),
        CueKind::End => color::SEM_RECORD.gamma_multiply(ACCENT_TINT),
    }
}

/// The decorations of the rows that can be seen. `galley` is the layout of the text `markup`
/// is the parse of, drawn at `at`.
pub fn decorations(ui: &egui::Ui, markup: &Markup, galley: &egui::Galley, at: Vec2, reveal: Reveal) -> Decorations {
    let accent = ui_kit::Theme::get(ui.ctx()).accent.accent;
    let seen = ui.clip_rect().y_range();
    let mut drawn = Decorations::default();
    // The first character of the row, its line, and the first cue not yet passed.
    let (mut start, mut line, mut cue) = (0, 0, 0);
    let mut line_begins = true;
    for row in &galley.rows {
        let end = start + row.glyphs.len();
        while markup.cues.get(cue).is_some_and(|cue| cue.chars.end <= start) {
            cue += 1;
        }
        let rect = row.rect().translate(at);
        if rect.bottom() >= seen.min && rect.top() <= seen.max {
            if markup.quote_lines.binary_search(&line).is_ok() {
                let bar = Rect::from_min_size(pos2(at.x - GUTTER, rect.top()), vec2(QUOTE_BAR, rect.height()));
                drawn.front.push(Shape::rect_filled(bar, 0.0, color::TEXT_TERTIARY));
            }
            if line_begins && !reveal.shows(line) {
                let first = markup.items.partition_point(|item| item.line < line);
                for item in markup.items[first..].iter().take_while(|item| item.line == line) {
                    let mut label = format(Style::default(), true);
                    label.color = color::TEXT_SECONDARY;
                    let label = ui.fonts_mut(|fonts| fonts.layout_job(LayoutJob::single_section(item.label.clone(), label)));
                    // Right of the number is where the text starts, less the gap.
                    let right = at.x + list_indent(ui, item) - MARKER_GAP * typography::BODY_SIZE;
                    drawn.front.push(Shape::galley(pos2(right - label.size().x, rect.top()), label, color::TEXT_SECONDARY));
                }
            }
            for found in markup.cues[cue..].iter().take_while(|cue| cue.chars.start < end) {
                let columns = found.chars.start.saturating_sub(start)..found.chars.end.min(end).saturating_sub(start);
                let Some(glyphs) = row.glyphs.get(columns).filter(|glyphs| !glyphs.is_empty()) else {
                    continue;
                };
                let x = rect.left() + glyphs[0].pos.x - CHIP_PAD..=rect.left() + glyphs[glyphs.len() - 1].max_x() + CHIP_PAD;
                // The chip sits around the text, which is at the top of its row, with the room
                // between the lines below it. Hidden brackets have next to no height.
                let Some(text) = glyphs.iter().max_by(|a, b| a.font_height.total_cmp(&b.font_height)) else {
                    continue;
                };
                let top = rect.top() + text.logical_rect().top();
                let chip = Rect::from_x_y_ranges(x, top - CHIP_REACH..=top + text.font_height + CHIP_REACH);
                drawn.behind.push(Shape::rect_filled(chip, chip.height() / 2.0, chip_color(found.kind, accent)));
            }
        }
        let CharIndex(count) = row.char_count_including_newline();
        start += count;
        line_begins = row.ends_with_newline;
        line += usize::from(row.ends_with_newline);
    }
    drawn
}

#[cfg(test)]
mod tests;
