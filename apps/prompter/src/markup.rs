//! Where the Markdown syntax and the styled text are in a script's source, for the styled
//! editor. It uses the parser the views use, and finds cues the way prompter-core's
//! `script.rs` does. All ranges are byte ranges of the source unless they say otherwise.

use std::collections::BTreeMap;
use std::ops::Range;

use prompter_core::script::{self, BlockKind, Cue};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// A list item's bullet. An ordered item has its number instead.
pub const BULLET: &str = "\u{2022}";

/// What a run of text looks like, apart from the cursor and the find matches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    /// Level 1 to 6.
    pub heading: Option<u8>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// Markdown syntax: hidden or dimmed.
    pub marker: bool,
}

/// Text of one style on one line. A line break is a run of its own, with no style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub range: Range<usize>,
    /// The 0-based line the run is on.
    pub line: usize,
    pub style: Style,
}

/// A list item's marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMarker {
    /// The marker with its indentation and the space after it.
    pub range: Range<usize>,
    pub line: usize,
    /// 0 for a top-level list.
    pub depth: usize,
    /// What is drawn in its place: [`BULLET`] or the item's number.
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueKind {
    Note,
    Pause,
    Mark,
    End,
}

/// A `[[...]]` cue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CueMarkup {
    /// With its brackets.
    pub range: Range<usize>,
    /// The same text, counted in characters.
    pub chars: Range<usize>,
    pub kind: CueKind,
}

/// Where a block's hidden syntax is on its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// Ahead of the text: a list marker, a quote mark or a heading's `#`.
    Prefix,
    /// A heading's closing `#` run.
    Suffix,
    /// The `===` or `---` under a heading's text.
    Underline,
}

/// A kind of formatting inside a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inline {
    Bold,
    Italic,
    Underline,
    Strike,
}

/// Formatted text, with the markers around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSpan {
    pub kind: Inline,
    pub open: Range<usize>,
    /// Empty, at the end of the block's text, for a `<u>` left open.
    pub close: Range<usize>,
}

/// Text as the parser reads it: no Markdown syntax, link targets or cues. It is the text
/// that prompter-core makes spoken words from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Said {
    /// The byte of the source where `text` starts.
    pub at: usize,
    /// Text with the same number runs together. A word cannot cross to another number.
    pub block: usize,
    pub text: String,
}

/// One parse of a script.
#[derive(Debug, Default)]
pub struct Markup {
    /// The whole text in order, with no gaps.
    pub runs: Vec<Run>,
    /// In order.
    pub items: Vec<ListMarker>,
    /// The 0-based lines in a block quote, in order.
    pub quote_lines: Vec<usize>,
    /// In order, without overlaps.
    pub cues: Vec<CueMarkup>,
    /// In no set order.
    pub spans: Vec<InlineSpan>,
    /// Inline code, links and images, which formatting goes around and never into.
    pub solid: Vec<Range<usize>>,
    /// Each list item's and quote's line with the byte where its text starts, after the
    /// markers. Its wrapped rows line up there. In order.
    pub hangs: Vec<(usize, usize)>,
    /// The hidden syntax of headings, lists and quotes, in no set order.
    pub edges: Vec<(Range<usize>, Edge)>,
    /// In order.
    pub said: Vec<Said>,
}

impl Markup {
    pub fn parse(text: &str) -> Self {
        let mut scan = Scan::new(text);
        // The same parser options as prompter-core, so that the two agree.
        for (event, range) in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
            let strike = matches!(event, Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough));
            if strike && !text[range.clone()].starts_with("~~") {
                // One tilde is not a marker. The range covers the whole run, so it has both.
                let at = if matches!(event, Event::Start(_)) { range.start } else { range.end - 1 };
                scan.collect("~", at, false);
            } else {
                scan.event(event, range);
            }
        }
        scan.end_block();
        scan.finish()
    }
}

// What each byte is, collected before it is cut into runs.
const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const MARKER: u8 = 4;
const UNDERLINE: u8 = 64;
const STRIKE: u8 = 128;
/// The heading level, or 0, is kept in three bits from here up.
const HEADING_SHIFT: u8 = 3;
const HEADING_MASK: u8 = 7;

/// Text collected for the current block, to look for cues in.
struct Piece {
    /// Where it is in the joined text.
    at: usize,
    /// Where it starts in the source.
    offset: usize,
}

struct Scan<'a> {
    text: &'a str,
    /// The byte offset of each line's start.
    line_starts: Vec<usize>,
    /// One set of the bits above for each byte.
    bytes: Vec<u8>,
    items: Vec<ListMarker>,
    quote_lines: Vec<usize>,
    cues: Vec<(Range<usize>, CueKind)>,
    spans: Vec<InlineSpan>,
    solid: Vec<Range<usize>>,
    /// By line.
    hangs: BTreeMap<usize, usize>,
    edges: Vec<(Range<usize>, Edge)>,
    /// The `<u>` tags of the current block that are still open.
    open_underlines: Vec<Range<usize>>,
    /// Where the current block's last text ends in the source.
    text_end: usize,
    /// The number the next item of each open list gets, or `None` for a bulleted one.
    lists: Vec<Option<u64>>,
    quotes: usize,
    code_block: bool,
    /// The current block's text, the same with the brackets in code hidden, and where each
    /// part of it came from.
    joined: String,
    searchable: String,
    pieces: Vec<Piece>,
    said: Vec<Said>,
    /// The number the next [`Said`] gets.
    said_block: usize,
}

impl<'a> Scan<'a> {
    fn new(text: &'a str) -> Self {
        let line_starts = std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1)).collect();
        Self {
            text,
            line_starts,
            bytes: vec![0; text.len()],
            items: Vec::new(),
            quote_lines: Vec::new(),
            cues: Vec::new(),
            spans: Vec::new(),
            solid: Vec::new(),
            hangs: BTreeMap::new(),
            edges: Vec::new(),
            open_underlines: Vec::new(),
            text_end: 0,
            lists: Vec::new(),
            quotes: 0,
            code_block: false,
            joined: String::new(),
            searchable: String::new(),
            pieces: Vec::new(),
            said: Vec::new(),
            said_block: 0,
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                self.text_end = range.end;
                self.collect(&text, range.start, self.code_block);
            }
            Event::Code(text) => {
                self.text_end = range.end;
                // The text starts after the backticks.
                let inside = self.text[range.clone()].find(&*text).unwrap_or(0);
                self.collect(&text, range.start + inside, true);
                self.solid.push(range);
            }
            Event::InlineHtml(html) => self.inline_html(&html, range),
            Event::SoftBreak => self.collect(" ", range.start, false),
            Event::HardBreak => self.collect("\n", range.start, false),
            Event::Rule => self.end_block(),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Strong => self.emphasis(range, Inline::Bold),
            Tag::Emphasis => self.emphasis(range, Inline::Italic),
            Tag::Strikethrough => self.emphasis(range, Inline::Strike),
            Tag::Link { .. } | Tag::Image { .. } => self.solid.push(range),
            other => {
                // Every other tag starts a block.
                self.end_block();
                match other {
                    Tag::Heading { level, .. } => self.heading(range, level as u8),
                    Tag::List(first) => self.lists.push(first),
                    Tag::Item => self.item(range.start),
                    Tag::BlockQuote(_) => {
                        self.quotes += 1;
                        self.quote(range);
                    }
                    Tag::CodeBlock(_) => self.code_block = true,
                    _ => {}
                }
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        if matches!(tag, TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough | TagEnd::Link | TagEnd::Image) {
            return;
        }
        self.end_block();
        match tag {
            TagEnd::List(_) => {
                self.lists.pop();
            }
            TagEnd::BlockQuote(_) => self.quotes = self.quotes.saturating_sub(1),
            TagEnd::CodeBlock => self.code_block = false,
            _ => {}
        }
    }

    fn add(&mut self, range: Range<usize>, bits: u8) {
        if let Some(bytes) = self.bytes.get_mut(range) {
            bytes.iter_mut().for_each(|byte| *byte |= bits);
        }
    }

    fn edge(&mut self, range: Range<usize>, edge: Edge) {
        self.add(range.clone(), MARKER);
        self.edges.push((range, edge));
    }

    fn line_of(&self, offset: usize) -> usize {
        self.line_starts.partition_point(|&start| start <= offset).saturating_sub(1)
    }

    /// Where a line's text ends, before its line break.
    fn line_end(&self, line: usize) -> usize {
        let end = self.line_starts.get(line + 1).map_or(self.text.len(), |next| next - 1);
        if self.text[..end].ends_with('\r') { end - 1 } else { end }
    }

    /// The end of the spaces and tabs at `from`.
    fn after_blanks(&self, from: usize) -> usize {
        from + self.text[from..].bytes().take_while(|&byte| byte == b' ' || byte == b'\t').count()
    }

    /// `range` covers the delimiters.
    fn emphasis(&mut self, range: Range<usize>, kind: Inline) {
        let width = if kind == Inline::Italic { 1 } else { 2 };
        let (open, close) = (range.start..range.start + width, range.end.saturating_sub(width)..range.end);
        self.span(kind, open, close);
    }

    fn span(&mut self, kind: Inline, open: Range<usize>, close: Range<usize>) {
        let style = match kind {
            Inline::Bold => BOLD,
            Inline::Italic => ITALIC,
            Inline::Underline => UNDERLINE,
            Inline::Strike => STRIKE,
        };
        self.add(open.end..close.start, style);
        self.add(open.clone(), MARKER);
        self.add(close.clone(), MARKER);
        self.spans.push(InlineSpan { kind, open, close });
    }

    /// `<u>` and `</u>` are markers. Other HTML shows as it is written.
    fn inline_html(&mut self, html: &str, range: Range<usize>) {
        match underline_tag(html) {
            Some(true) => {
                self.text_end = range.end;
                self.open_underlines.push(range);
            }
            Some(false) => match self.open_underlines.pop() {
                Some(open) => self.span(Inline::Underline, open, range),
                // The views drop a stray closing tag too.
                None => self.add(range, MARKER),
            },
            None => {}
        }
    }

    /// Finds the cues of the block just ended and ends each `<u>` left open in it.
    fn end_block(&mut self) {
        while let Some(open) = self.open_underlines.pop() {
            let end = self.text_end.max(open.end);
            self.span(Inline::Underline, open, end..end);
        }
        self.find_cues();
    }

    /// Wrapped rows of `line` line up at `at`, or further in if the line has more markers.
    fn hang(&mut self, line: usize, at: usize) {
        let hang = self.hangs.entry(line).or_insert(at);
        *hang = (*hang).max(at);
    }

    fn heading(&mut self, range: Range<usize>, level: u8) {
        let source = self.text[range.clone()].trim_end();
        let end = range.start + source.len();
        let level = level << HEADING_SHIFT;
        if source.starts_with('#') {
            let hashes = source.len() - source.trim_start_matches('#').len();
            // With no text yet, the space after the "#" is part of the marker.
            let open_end = self.after_blanks(range.start + hashes);
            self.add(range.start..end, level);
            self.edge(range.start..open_end, Edge::Prefix);
            // A closing run of "#" counts only with a space before it.
            let before = source.trim_end_matches('#');
            if before.len() < source.len() && before.ends_with([' ', '\t']) {
                let close_start = (range.start + before.trim_end().len()).max(open_end);
                if close_start < end {
                    self.edge(close_start..end, Edge::Suffix);
                }
            }
        } else if let Some(split) = source.rfind('\n') {
            // A setext heading: the last line is the underline, after any quote or list prefix.
            self.add(range.start..range.start + split, level);
            let last = &source[split + 1..];
            if let Some(at) = last.find(['=', '-']) {
                self.edge(range.start + split + 1 + at..end, Edge::Underline);
            }
        }
    }

    fn item(&mut self, from: usize) {
        let at = self.after_blanks(from);
        let Some(width) = list_marker_width(&self.text[at..]) else {
            return;
        };
        let line = self.line_of(at);
        let indent = self.text[self.line_starts[line]..at].bytes().rev().take_while(|&byte| byte == b' ' || byte == b'\t').count();
        let range = at - indent..self.after_blanks(at + width);
        let depth = self.lists.len().saturating_sub(1);
        let label = match self.lists.last_mut() {
            Some(Some(next)) => {
                let label = format!("{next}.");
                *next = next.saturating_add(1);
                label
            }
            _ => BULLET.to_owned(),
        };
        self.edge(range.clone(), Edge::Prefix);
        self.hang(line, range.end);
        self.items.push(ListMarker { range, line, depth, label });
    }

    /// Marks each line of a quote, and the `>` of this quote's depth where the line has one.
    fn quote(&mut self, range: Range<usize>) {
        let end = range.start + self.text[range.clone()].trim_end().len();
        for line in self.line_of(range.start)..=self.line_of(end.saturating_sub(1).max(range.start)) {
            self.quote_lines.push(line);
            if let Some(at) = quote_marker(&self.text[..self.line_end(line)], self.line_starts[line], self.quotes) {
                let space = usize::from(self.text[at + 1..].starts_with(' '));
                self.edge(at..at + 1 + space, Edge::Prefix);
                self.hang(line, at + 1 + space);
            }
        }
    }

    fn collect(&mut self, text: &str, offset: usize, code: bool) {
        self.pieces.push(Piece { at: self.joined.len(), offset });
        self.joined.push_str(text);
        if code {
            self.searchable.extend(text.chars().map(|c| if c == '[' || c == ']' { '_' } else { c }));
        } else {
            self.searchable.push_str(text);
        }
    }

    /// The source offset of position `at` in the joined text.
    fn source_of(&self, at: usize) -> Option<usize> {
        let piece = self.pieces.get(self.pieces.partition_point(|piece| piece.at <= at).checked_sub(1)?)?;
        Some(piece.offset + (at - piece.at))
    }

    /// Finds the cues in the block just ended. A cue ends at the last `]]` of a run of closing
    /// brackets. The same rules as prompter-core's.
    fn find_cues(&mut self) {
        let mut from = 0;
        // Where each cue is in the joined text.
        let mut found = Vec::new();
        while let Some(open) = self.searchable[from..].find("[[").map(|at| from + at) {
            let Some(mut close) = self.searchable[open + 2..].find("]]").map(|at| open + 2 + at) else {
                break;
            };
            while self.searchable[close + 2..].starts_with(']') {
                close += 1;
            }
            let inner = open + 2..close;
            let kind = if self.searchable[inner.clone()].contains("[[") { None } else { cue_kind(&self.joined[inner]) };
            match kind {
                Some(kind) => {
                    self.add_cue([open, open + 1, close, close + 1], kind);
                    found.push(open..close + 2);
                    from = close + 2;
                }
                None => from = open + 1,
            }
        }
        self.keep_said(&found);
        self.joined.clear();
        self.searchable.clear();
        self.pieces.clear();
    }

    /// Keeps the block's text around `cues`, which are in order. As in the views, a cue
    /// ends the text before it, even in the middle of a word.
    fn keep_said(&mut self, cues: &[Range<usize>]) {
        let ends = self.pieces.iter().skip(1).map(|piece| piece.at).chain(std::iter::once(self.joined.len()));
        for (piece, end) in self.pieces.iter().zip(ends) {
            let mut from = piece.at;
            let mut keep = |range: Range<usize>, block: usize| {
                if !range.is_empty() {
                    self.said.push(Said { at: piece.offset + (range.start - piece.at), block, text: self.joined[range].to_owned() });
                }
            };
            for (index, cue) in cues.iter().enumerate() {
                if cue.end <= from || cue.start >= end {
                    continue;
                }
                keep(from..cue.start.max(from), self.said_block + index);
                from = cue.end.min(end);
            }
            let passed = cues.iter().take_while(|cue| cue.end <= from).count();
            keep(from..end, self.said_block + passed);
        }
        self.said_block += cues.len() + 1;
    }

    /// Takes the positions of a cue's four brackets in the joined text.
    fn add_cue(&mut self, brackets: [usize; 4], kind: CueKind) {
        let found: Vec<usize> = brackets
            .iter()
            .zip(*b"[[]]")
            .filter_map(|(&at, bracket)| self.source_of(at).filter(|&at| self.text.as_bytes().get(at) == Some(&bracket)))
            .collect();
        // A bracket written as an entity is not one in the source, so such a cue stays as text.
        let &[first, second, third, last] = found.as_slice() else {
            return;
        };
        for at in [first, second, third, last] {
            self.add(at..at + 1, MARKER);
        }
        self.cues.push((first..last + 1, kind));
    }

    fn finish(mut self) -> Markup {
        let text = self.text;
        let mut runs: Vec<Run> = Vec::new();
        let mut line = 0;
        let mut start = 0;
        // A cue starts and ends a run, so that the layout can make room around it.
        let mut edges = self.cues.iter().flat_map(|(range, _)| [range.start, range.end]).peekable();
        for at in 1..=text.len() {
            let before = text.as_bytes()[at - 1];
            let edge = edges.next_if_eq(&at).is_some();
            while edges.next_if_eq(&at).is_some() {}
            let breaks = before == b'\n' || text.as_bytes().get(at) == Some(&b'\n');
            if at < text.len() && !edge && !breaks && self.bytes[at] == self.bytes[at - 1] {
                continue;
            }
            let newline = &text[start..at] == "\n";
            let bits = if newline { 0 } else { self.bytes[start] };
            let level = (bits >> HEADING_SHIFT) & HEADING_MASK;
            let has = |bit: u8| bits & bit != 0;
            let style = Style {
                heading: (level > 0).then_some(level),
                bold: has(BOLD),
                italic: has(ITALIC),
                underline: has(UNDERLINE),
                strike: has(STRIKE),
                marker: has(MARKER),
            };
            runs.push(Run { range: start..at, line, style });
            line += usize::from(newline);
            start = at;
        }

        let (mut bytes, mut chars) = (0, 0);
        let mut count_to = |at: usize| {
            chars += text[bytes..at].chars().count();
            bytes = at;
            chars
        };
        let cues = self.cues.drain(..).map(|(range, kind)| CueMarkup { chars: count_to(range.start)..count_to(range.end), range, kind }).collect();

        // Nested quotes list a line once each.
        self.quote_lines.sort_unstable();
        self.quote_lines.dedup();
        let hangs = self.hangs.into_iter().collect();
        let (items, quote_lines, spans, solid, edges, said) = (self.items, self.quote_lines, self.spans, self.solid, self.edges, self.said);
        Markup { runs, items, quote_lines, cues, spans, solid, hangs, edges, said }
    }
}

/// `Some(true)` for `<u>`, `Some(false)` for `</u>` and `None` for any other HTML. The same
/// rule as prompter-core's.
fn underline_tag(html: &str) -> Option<bool> {
    let name = html.strip_prefix('<')?.strip_suffix('>')?.trim_end();
    let (name, opens) = name.strip_prefix('/').map_or((name, true), |name| (name, false));
    name.eq_ignore_ascii_case("u").then_some(opens)
}

/// The length of the list marker that `text` starts with, without the space after it.
pub fn list_marker_width(text: &str) -> Option<usize> {
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let width = match text.as_bytes().get(digits)? {
        b'-' | b'+' | b'*' if digits == 0 => 1,
        b'.' | b')' if digits > 0 => digits + 1,
        _ => return None,
    };
    matches!(text.as_bytes().get(width), None | Some(b' ' | b'\t' | b'\r' | b'\n')).then_some(width)
}

/// Where the `depth`th `>` of a line's prefix is. The prefix is made of blanks, `>` and list
/// markers. `text` ends where the line does. A lazy continuation line has no such `>`.
fn quote_marker(text: &str, line_start: usize, depth: usize) -> Option<usize> {
    let mut at = line_start;
    let mut seen = 0;
    while let Some(&byte) = text.as_bytes().get(at) {
        match byte {
            b' ' | b'\t' => at += 1,
            b'>' => {
                seen += 1;
                if seen == depth {
                    return Some(at);
                }
                at += 1;
            }
            _ => at += list_marker_width(&text[at..])?,
        }
    }
    None
}

/// The kind of cue `inner`, the text between the brackets, makes, if any. prompter-core is
/// asked about the keyword, with a stand-in for a label.
fn cue_kind(inner: &str) -> Option<CueKind> {
    let (keyword, label) = match inner.split_once(':') {
        Some((keyword, _)) => (keyword.trim(), ":x"),
        None => (inner.trim(), ""),
    };
    // Anything else could be read as Markdown by the probe below.
    if keyword.is_empty() || !keyword.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    match script::parse(&format!("[[{keyword}{label}]]")).blocks.first()?.kind {
        BlockKind::Cue(Cue::Note(_)) => Some(CueKind::Note),
        BlockKind::Cue(Cue::Pause) => Some(CueKind::Pause),
        BlockKind::Cue(Cue::Mark { .. }) => Some(CueKind::Mark),
        BlockKind::Cue(Cue::End) => Some(CueKind::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each marker, with runs that touch taken as one.
    fn markers(text: &str) -> Vec<&str> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for run in Markup::parse(text).runs.iter().filter(|run| run.style.marker) {
            match ranges.last_mut() {
                Some(last) if last.end == run.range.start => last.end = run.range.end,
                _ => ranges.push(run.range.clone()),
            }
        }
        ranges.into_iter().map(|range| &text[range]).collect()
    }

    /// The text that has a style.
    fn styled(text: &str, has: impl Fn(Style) -> bool) -> String {
        Markup::parse(text).runs.iter().filter(|run| !run.style.marker && has(run.style)).map(|run| &text[run.range.clone()]).collect()
    }

    #[test]
    fn heading_markers_are_found() {
        assert_eq!(markers("# One\n\n###### Six"), ["# ", "###### "]);
        assert_eq!(markers("## Closed ##\n"), ["## ", " ##"]);
        assert_eq!(markers("# C#"), ["# "]);
        assert_eq!(markers("Title\n=====\n\nSub\n---"), ["=====", "---"]);
        // The second quote marker touches the underline.
        assert_eq!(markers("> Quoted\n> ==="), ["> ", "> ==="]);
        assert_eq!(styled("## Two ##\nbody", |style| style.heading == Some(2)), "Two");
        assert_eq!(styled("Title\n===\nbody", |style| style.heading == Some(1)), "Title");
    }

    #[test]
    fn bold_and_italic_markers_are_found_and_their_text_styled() {
        let text = "a **bold** b *slanted* c __more__ d _also_";
        assert_eq!(markers(text), ["**", "**", "*", "*", "__", "__", "_", "_"]);
        assert_eq!(styled(text, |style| style.bold), "boldmore");
        assert_eq!(styled(text, |style| style.italic), "slantedalso");
        assert_eq!(markers("***both***"), ["***", "***"]);
        assert_eq!(styled("***both***", |style| style.bold && style.italic), "both");
        // Across lines, with each end on its own.
        assert_eq!(markers("**one\ntwo**"), ["**", "**"]);
        assert_eq!(markers("2 * 3 = 6, snake_case, `**code**`, **open"), Vec::<&str>::new());
    }

    #[test]
    fn underline_and_strikethrough_markers_are_found_and_their_text_styled() {
        let text = "a <u>under</u> b ~~gone~~ c <U >**both**</u>";
        assert_eq!(markers(text), ["<u>", "</u>", "~~", "~~", "<U >**", "**</u>"]);
        assert_eq!(styled(text, |style| style.underline), "underboth");
        assert_eq!(styled(text, |style| style.strike), "gone");
        assert_eq!(styled("## <u>~~Head~~</u>", |style| style.underline && style.strike && style.heading == Some(2)), "Head");
        // One tilde, a tag with attributes and other HTML are text.
        assert_eq!(markers("~one~ a <u class=x>b <br> c"), Vec::<&str>::new());
        // A tag left open ends with its block, and a stray closing tag is still a marker.
        assert_eq!(styled("a <u>open\n\nnext", |style| style.underline), "open");
        assert_eq!(markers("a</u> b"), ["</u>"]);
    }

    #[test]
    fn the_underline_and_strike_text_is_what_the_views_draw() {
        let text = "- <u>a **b** c\n- ~~d~~ ~e~ </u>\n\n> x <u>y</u> z";
        let drawn = |pick: fn(&prompter_core::script::Span) -> bool| -> String {
            script::parse(text).blocks.iter().flat_map(|block| &block.spans).filter(|span| pick(span)).map(|span| span.text.as_str()).collect()
        };
        assert_eq!(styled(text, |style| style.underline), drawn(|span| span.underline));
        assert_eq!(styled(text, |style| style.strike), drawn(|span| span.strike));
    }

    #[test]
    fn each_list_and_quote_line_hangs_where_its_text_starts() {
        let text = "- a\n  - b\n\ntext\n\n> c\n> > d\n> - e\nlazy";
        let before = |at: usize| &text[text[..at].rfind('\n').map_or(0, |newline| newline + 1)..at];
        let hangs: Vec<(usize, &str)> = Markup::parse(text).hangs.into_iter().map(|(line, at)| (line, before(at))).collect();
        assert_eq!(hangs, [(0, "- "), (1, "  - "), (5, "> "), (6, "> > "), (7, "> - ")]);
    }

    #[test]
    fn list_markers_are_found_with_their_depth_and_label() {
        let text = "- a\n  * b\n+ c\n\ntext\n\n3. d\n3. e\n   1) f\n- ";
        assert_eq!(markers(text), ["- ", "  * ", "+ ", "3. ", "3. ", "   1) ", "- "]);
        let items: Vec<(usize, usize, String)> = Markup::parse(text).items.into_iter().map(|item| (item.line, item.depth, item.label)).collect();
        let expected = [(0, 0, BULLET), (1, 1, BULLET), (2, 0, BULLET), (6, 0, "3."), (7, 0, "4."), (8, 1, "1."), (9, 0, BULLET)];
        assert_eq!(items, expected.map(|(line, depth, label)| (line, depth, label.to_owned())));
        // A rule is not a list.
        assert!(Markup::parse("---\n\n***").items.is_empty());
        assert_eq!(markers("---\n\n***"), Vec::<&str>::new());
    }

    #[test]
    fn quote_markers_are_found_on_each_line() {
        let text = "> one\n> two\nlazy\n>\n> > deeper\n\nplain > text";
        assert_eq!(markers(text), ["> ", "> ", ">", "> > "]);
        assert_eq!(Markup::parse(text).quote_lines, [0, 1, 2, 3, 4]);
        assert_eq!(markers("> - item\n>   more"), ["> - ", "> "]);
        assert_eq!(markers("- > in a list\n  > again"), ["- > ", "> "]);
        // A ">" that is code is not a marker.
        assert_eq!(markers("> ```\n> > code\n> ```"), ["> ", "> ", "> "]);
        assert_eq!(markers("> a\r\n> b\r\n"), ["> ", "> "]);
    }

    #[test]
    fn each_cue_form_has_its_brackets_marked_and_its_kind() {
        for (cue, kind) in [
            ("[[note: wave]]", CueKind::Note),
            ("[[pause]]", CueKind::Pause),
            ("[[ PAUSE ]]", CueKind::Pause),
            ("[[mark: B-roll: desk]]", CueKind::Mark),
            ("[[mark:]]", CueKind::Mark),
            ("[[end]]", CueKind::End),
        ] {
            let text = format!("caf\u{e9} {cue} after");
            let markup = Markup::parse(&text);
            assert_eq!(markers(&text), ["[[", "]]"], "{cue}");
            assert_eq!(markup.cues.len(), 1, "{cue}");
            assert_eq!((&text[markup.cues[0].range.clone()], markup.cues[0].kind), (cue, kind));
            assert_eq!(markup.cues[0].chars, 5..5 + cue.len(), "{cue}");
        }
    }

    #[test]
    fn cues_are_found_where_the_views_find_them() {
        let text = "# Head [[pause]]\n\n- item [[mark: x]]\n> [[note: **loud**\n> twice]]\n\n[[pause]]]\n\n`[[pause]]` [[later]] [[pause: now]] [[note]]\n\n```\n[[end]]\n```\n\n[[x [[end]]\n\n[[note: [a] b]]]";
        let found: Vec<CueKind> = Markup::parse(text).cues.iter().map(|cue| cue.kind).collect();
        let views: Vec<CueKind> = script::parse(text)
            .blocks
            .iter()
            .filter_map(|block| match &block.kind {
                BlockKind::Cue(Cue::Note(_)) => Some(CueKind::Note),
                BlockKind::Cue(Cue::Pause) => Some(CueKind::Pause),
                BlockKind::Cue(Cue::Mark { .. }) => Some(CueKind::Mark),
                BlockKind::Cue(Cue::End) => Some(CueKind::End),
                _ => None,
            })
            .collect();
        assert_eq!(found, views);
        assert_eq!(found, [CueKind::Pause, CueKind::Mark, CueKind::Note, CueKind::End, CueKind::Note]);
    }

    #[test]
    fn the_runs_cover_the_text_with_one_line_each() {
        let text = "# T\n\n\n- **a** [[pause]][[end]]\r\n> q\n";
        let markup = Markup::parse(text);
        let joined: String = markup.runs.iter().map(|run| &text[run.range.clone()]).collect();
        assert_eq!(joined, text);
        for run in &markup.runs {
            assert!(&text[run.range.clone()] == "\n" || !text[run.range.clone()].contains('\n'), "{run:?}");
            assert_eq!(run.line, text[..run.range.start].matches('\n').count(), "{run:?}");
        }
        assert!(Markup::parse("").runs.is_empty());
    }
}
