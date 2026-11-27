//! The styled look of the editor's Markdown. The text laid out is the text itself, so that
//! the cursor and selection map to it: hidden syntax is made see-through and next to no
//! width, never left out. Bullets, quote bars and cue chips are drawn around the text.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use egui::text::{ByteIndex, ByteRange, CharIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{Color32, FontId, Galley, Rect, Shape, Stroke, Vec2, pos2, vec2};
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
/// The weight of an underline and a strikethrough.
const TEXT_LINE: f32 = 1.0;
/// The least width wrapped rows keep, however deep their hanging indent is.
const MIN_WRAP: f32 = 4.0 * typography::BODY_SIZE;
/// How strongly each cue type's color fills its chip.
const NOTE_TINT: f32 = 0.45;
const ACCENT_TINT: f32 = 0.35;

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
    let line_if = |on: bool| if on { Stroke::new(TEXT_LINE, text) } else { Stroke::NONE };
    let (underline, strikethrough) = (line_if(style.underline), line_if(style.strike));
    TextFormat { line_height, italics: style.italic, underline, strikethrough, ..TextFormat::simple(font, text) }
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

/// The layout of `text`, which `markup` is the parse of. With `shown`, the Markdown syntax is
/// dimmed, and otherwise hidden. `matches` are byte ranges to back
/// with a color, in order and without overlaps.
pub fn layout_job(ui: &egui::Ui, text: &str, markup: &Markup, shown: bool, matches: &[(Range<usize>, Color32)], wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob { text: text.to_owned(), keep_trailing_whitespace: true, ..Default::default() };
    job.wrap.max_width = wrap_width;
    job.sections.reserve(markup.runs.len() + 2 * matches.len());

    let mut items = markup.items.iter().peekable();
    let mut cues = markup.cues.iter().flat_map(|cue| [cue.range.start, cue.range.end]).peekable();
    let mut matches = matches.iter().peekable();
    for run in &markup.runs {
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

/// The laid out text. The wrapped rows of a list item or quote line start where its text
/// does. egui has no such hanging indent, so each of those lines is laid out on its own, as
/// narrow as its wrapped rows are, and they are then moved in. The cursor, the selection and
/// clicks all go by where a row is, so they move with it.
pub fn layout(ui: &egui::Ui, markup: &Markup, job: LayoutJob) -> Arc<Galley> {
    if markup.hangs.is_empty() {
        return ui.fonts_mut(|fonts| fonts.layout_job(job));
    }
    let starts: Vec<usize> = std::iter::once(0).chain(job.text.match_indices('\n').map(|(at, _)| at + 1)).collect();
    // A last line that is empty goes with the line before it, as it does in egui's layout.
    let count = starts.len() - usize::from(starts.len() > 1 && job.text.ends_with('\n'));
    let end_of = |line: usize| if line + 1 < count { starts[line + 1] - 1 } else { job.text.len() };

    let mut galleys = Vec::new();
    let mut hangs = markup.hangs.iter().peekable();
    // The section to look from, and the first line not laid out yet.
    let (mut section, mut plain) = (0, 0);
    for line in 0..=count {
        while hangs.next_if(|&&(hung, _)| hung < line).is_some() {}
        let content = hangs.next_if(|&&(hung, _)| hung == line).map(|&(_, content)| content);
        if line < count && content.is_none() {
            continue;
        }
        // The lines between two hanging ones are laid out together.
        if plain < line {
            let piece = piece(&job, &mut section, starts[plain]..end_of(line - 1));
            galleys.push(ui.fonts_mut(|fonts| fonts.layout_job(piece)));
        }
        if let Some(content) = content.filter(|_| line < count) {
            let piece = piece(&job, &mut section, starts[line]..end_of(line));
            galleys.push(hanging(ui, piece, content - starts[line]));
        }
        plain = line + 1;
    }
    let pixels_per_point = ui.ctx().pixels_per_point();
    Arc::new(Galley::concat(Arc::new(job), &galleys, pixels_per_point))
}

/// The part of `job` in `range`, cut the way egui cuts a job into paragraphs. `from` is the
/// first section that may reach into it, and is moved up.
fn piece(job: &LayoutJob, from: &mut usize, range: Range<usize>) -> LayoutJob {
    let (start, end) = (ByteIndex(range.start), ByteIndex(range.end));
    while job.sections.get(*from).is_some_and(|section| section.byte_range.end <= start) {
        *from += 1;
    }
    let sections = job.sections[*from..]
        .iter()
        .take_while(|section| section.byte_range.start <= end)
        .map(|section| LayoutSection {
            leading_space: if start <= section.byte_range.start { section.leading_space } else { 0.0 },
            byte_range: section.byte_range.start.saturating_sub(range.start)..section.byte_range.end.min(end).saturating_sub(range.start),
            format: section.format.clone(),
        })
        .collect();
    LayoutJob { text: job.text[range].to_owned(), sections, wrap: job.wrap.clone(), keep_trailing_whitespace: true, ..Default::default() }
}

/// Lays out one line whose text starts at byte `content`, past its markers.
fn hanging(ui: &egui::Ui, mut job: LayoutJob, content: usize) -> Arc<Galley> {
    // Where the text starts: the line's indent, then whatever width its markers have.
    let lead = job.sections.first().map_or(0.0, |section| section.leading_space);
    let markers: f32 = job
        .sections
        .iter()
        .enumerate()
        .take_while(|(_, section)| section.byte_range.start.0 < content)
        .map(|(index, section)| {
            let part = section.byte_range.start.0..section.byte_range.end.0.min(content);
            let space = if index == 0 { 0.0 } else { section.leading_space };
            space + width(ui, &section.format.font_id, &job.text[part])
        })
        .sum();
    if lead + markers < 1.0 {
        return ui.fonts_mut(|fonts| fonts.layout_job(job));
    }
    // The first row's indent is added after the layout, so that every row can be as wide.
    if let Some(first) = job.sections.first_mut() {
        first.leading_space = 0.0;
    }
    job.wrap.max_width = (job.wrap.max_width - lead - markers).max(MIN_WRAP);
    let chars = job.text[..content.min(job.text.len())].chars().count();
    let mut galley = Arc::unwrap_or_clone(ui.fonts_mut(|fonts| fonts.layout_job(job)));
    // Where the text is on the first row, measured in the layout itself.
    let text_at = galley.rows.first().map_or(0.0, |row| row.x_offset(CharIndex(chars)));
    let mut wrapped = false;
    for (index, row) in galley.rows.iter_mut().enumerate() {
        if index == 0 {
            row.pos.x += lead;
        } else if wrapped {
            row.pos.x += lead + text_at;
        }
        wrapped = !row.ends_with_newline;
    }
    Arc::new(galley)
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
pub fn decorations(ui: &egui::Ui, markup: &Markup, galley: &egui::Galley, at: Vec2, shown: bool) -> Decorations {
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
            if line_begins && !shown {
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
