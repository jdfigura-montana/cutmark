//! Markdown to blocks (docs/suite/prompter.md, "Scripts").

use std::ops::Range;
use std::{mem, slice};

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

/// A parsed script.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    pub blocks: Vec<Block>,
}

/// One block of a script, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Stable order index: the block's position in [`Script::blocks`].
    pub id: u32,
    pub kind: BlockKind,
    /// The block's text. Empty for rules and cues.
    pub spans: Vec<Span>,
    /// 1-based line in the Markdown where the block's content starts.
    pub source_line: u32,
}

impl Block {
    /// The block's text with formatting removed.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlockKind {
    /// Level 1 to 6.
    Heading(u8),
    Paragraph,
    /// `depth` is 0 for a top-level list. `ordered` describes the innermost list.
    ListItem {
        ordered: bool,
        depth: u8,
    },
    Quote,
    Rule,
    Cue(Cue),
}

/// A run of text with one style. A hard line break is a `\n` in `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
}

/// A `[[...]]` cue. Cues are never spoken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cue {
    Note(String),
    Pause,
    /// `label` is the whole trimmed label, category prefix included.
    Mark {
        label: String,
        category: MarkCategory,
    },
    End,
}

/// The suite mark types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkCategory {
    Highlight,
    Cut,
    Retake,
    Chapter,
    Note,
    BRoll,
}

const CATEGORY_PREFIXES: [(&str, MarkCategory); 6] = [
    ("highlight", MarkCategory::Highlight),
    ("cut", MarkCategory::Cut),
    ("retake", MarkCategory::Retake),
    ("chapter", MarkCategory::Chapter),
    ("note", MarkCategory::Note),
    ("b-roll", MarkCategory::BRoll),
];

impl MarkCategory {
    /// The category named at the start of a mark label, or [`MarkCategory::Note`].
    ///
    /// The name must be followed by a colon, a space or the end of the label, so both
    /// "B-roll: desk" and the spec's "B-roll here" match, and "Cutaway" does not.
    fn from_label(label: &str) -> Self {
        let end = label.find(|c: char| c == ':' || c.is_whitespace()).unwrap_or(label.len());
        let prefix = &label[..end];
        CATEGORY_PREFIXES.iter().find(|(name, _)| prefix.eq_ignore_ascii_case(name)).map_or(Self::Note, |&(_, category)| category)
    }
}

/// A heading, as listed in the Outline and the Jump to section menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub block_id: u32,
    pub level: u8,
    pub title: String,
}

/// The script's headings in reading order.
pub fn sections(script: &Script) -> Vec<Section> {
    script
        .blocks
        .iter()
        .filter_map(|block| match block.kind {
            BlockKind::Heading(level) => Some(Section { block_id: block.id, level, title: block.text() }),
            _ => None,
        })
        .collect()
}

/// Parses a Markdown script into blocks.
///
/// Links and images become their plain text and HTML is dropped. A cue inside a block splits
/// it into the text before, the cue block and the text after. Cues inside a heading do not
/// split it, so that one heading stays one section.
pub fn parse(md: &str) -> Script {
    let mut builder = Builder::new(md);
    for (event, range) in Parser::new(md).into_offset_iter() {
        builder.event(event, range);
    }
    builder.flush();
    Script { blocks: builder.blocks }
}

/// Text collected for the current block, before cues are cut out of it.
struct Piece {
    span: Span,
    /// Byte offset in the Markdown where this text starts.
    offset: usize,
}

struct Builder {
    /// Byte offset of every newline in the Markdown.
    newlines: Vec<usize>,
    blocks: Vec<Block>,
    pieces: Vec<Piece>,
    heading: Option<u8>,
    /// One entry per open list; true when it is ordered.
    lists: Vec<bool>,
    quotes: usize,
    bold: usize,
    italic: usize,
}

impl Builder {
    fn new(md: &str) -> Self {
        Self {
            newlines: md.match_indices('\n').map(|(at, _)| at).collect(),
            blocks: Vec::new(),
            pieces: Vec::new(),
            heading: None,
            lists: Vec::new(),
            quotes: 0,
            bold: 0,
            italic: 0,
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Code(text) => self.text(&text, range.start),
            Event::SoftBreak => self.text(" ", range.start),
            Event::HardBreak => self.text("\n", range.start),
            Event::Rule => {
                self.flush();
                let line = self.line(range.start);
                self.push_block(BlockKind::Rule, Vec::new(), line);
            }
            // HTML is dropped. The other events need parser options that are not enabled.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            Tag::Link { .. } | Tag::Image { .. } => {}
            // Every other tag starts a block. A tight list item has no paragraph of its own,
            // so its text must end here when a nested block begins.
            other => {
                self.flush();
                match other {
                    Tag::Heading { level, .. } => self.heading = Some(heading_number(level)),
                    Tag::List(first_number) => self.lists.push(first_number.is_some()),
                    Tag::BlockQuote(_) => self.quotes += 1,
                    _ => {}
                }
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {}
            other => {
                // Flush first: the block's kind depends on the containers still open.
                self.flush();
                match other {
                    TagEnd::Heading(_) => self.heading = None,
                    TagEnd::List(_) => {
                        self.lists.pop();
                    }
                    TagEnd::BlockQuote(_) => self.quotes = self.quotes.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }

    fn text(&mut self, text: &str, offset: usize) {
        self.pieces.push(Piece { span: Span { text: text.to_owned(), bold: self.bold > 0, italic: self.italic > 0 }, offset });
    }

    fn text_kind(&self) -> BlockKind {
        if let Some(level) = self.heading {
            BlockKind::Heading(level)
        } else if let Some(&ordered) = self.lists.last() {
            let depth = u8::try_from(self.lists.len() - 1).unwrap_or(u8::MAX);
            BlockKind::ListItem { ordered, depth }
        } else if self.quotes > 0 {
            BlockKind::Quote
        } else {
            BlockKind::Paragraph
        }
    }

    /// Turns the collected text into blocks.
    ///
    /// Cues are found in the joined text because the parser reports each bracket as its own
    /// text event.
    fn flush(&mut self) {
        if self.pieces.is_empty() {
            return;
        }
        let pieces = mem::take(&mut self.pieces);
        let text: String = pieces.iter().map(|p| p.span.text.as_str()).collect();
        let kind = self.text_kind();
        let cues = find_cues(&text);

        if matches!(kind, BlockKind::Heading(_)) {
            self.push_heading(&pieces, &text, cues, kind);
            return;
        }

        let mut done = 0;
        for (range, cue) in cues {
            let before = done..range.start;
            self.push_text(&pieces, &text, slice::from_ref(&before), kind.clone());
            self.push_cue(&pieces, range.start, cue);
            done = range.end;
        }
        let rest = done..text.len();
        self.push_text(&pieces, &text, slice::from_ref(&rest), kind);
    }

    /// Pushes one heading made of all the text around its cues. Cues that come before any
    /// heading text go before the heading block and the rest go after it.
    fn push_heading(&mut self, pieces: &[Piece], text: &str, mut cues: Vec<(Range<usize>, Cue)>, kind: BlockKind) {
        let mut gaps = Vec::with_capacity(cues.len() + 1);
        let mut done = 0;
        for (range, _) in &cues {
            gaps.push(done..range.start);
            done = range.end;
        }
        gaps.push(done..text.len());

        let text_start = gaps.iter().find(|gap| !text[(*gap).clone()].trim().is_empty()).map_or(text.len(), |gap| gap.start);
        let before = cues.partition_point(|(range, _)| range.start < text_start);
        let after = cues.split_off(before);

        for (range, cue) in cues {
            self.push_cue(pieces, range.start, cue);
        }
        self.push_text(pieces, text, &gaps, kind);
        for (range, cue) in after {
            self.push_cue(pieces, range.start, cue);
        }
    }

    /// Pushes one text block made of `ranges` of the joined text, unless they are blank.
    fn push_text(&mut self, pieces: &[Piece], text: &str, ranges: &[Range<usize>], kind: BlockKind) {
        let mut spans: Vec<Span> = Vec::new();
        let mut first = None;
        for range in ranges {
            let mut start = range.start;
            // Cutting a cue out of "a [[pause]] b" must not leave two spaces.
            if spans.last().is_some_and(|s| s.text.ends_with(' ')) {
                let rest = &text[range.clone()];
                start += rest.len() - rest.trim_start().len();
            }
            let found = collect_spans(pieces, start..range.end, &mut spans);
            first = first.or(found);
        }
        trim_spans(&mut spans);
        if let Some(offset) = first {
            let line = self.line(offset);
            self.push_block(kind, spans, line);
        }
    }

    fn push_cue(&mut self, pieces: &[Piece], at: usize, cue: Cue) {
        let line = self.line(offset_at(pieces, at));
        self.push_block(BlockKind::Cue(cue), Vec::new(), line);
    }

    fn push_block(&mut self, kind: BlockKind, spans: Vec<Span>, source_line: u32) {
        let id = u32::try_from(self.blocks.len()).unwrap_or(u32::MAX);
        self.blocks.push(Block { id, kind, spans, source_line });
    }

    fn line(&self, offset: usize) -> u32 {
        let before = self.newlines.partition_point(|&at| at < offset);
        u32::try_from(before + 1).unwrap_or(u32::MAX)
    }
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Appends the styled text inside `range` of the joined text, merging runs of one style.
/// Returns the Markdown offset of the first character that is not whitespace.
fn collect_spans(pieces: &[Piece], range: Range<usize>, spans: &mut Vec<Span>) -> Option<usize> {
    let mut first = None;
    let mut piece_start = 0;
    for piece in pieces {
        let piece_end = piece_start + piece.span.text.len();
        let from = range.start.max(piece_start);
        let to = range.end.min(piece_end);
        if from < to {
            let part = &piece.span.text[from - piece_start..to - piece_start];
            let visible = part.trim_start();
            if first.is_none() && !visible.is_empty() {
                first = Some(piece.offset + (from - piece_start) + (part.len() - visible.len()));
            }
            match spans.last_mut() {
                Some(last) if last.bold == piece.span.bold && last.italic == piece.span.italic => {
                    last.text.push_str(part);
                }
                _ => spans.push(Span { text: part.to_owned(), bold: piece.span.bold, italic: piece.span.italic }),
            }
        }
        piece_start = piece_end;
    }
    first
}

/// The Markdown offset of position `at` in the joined text.
fn offset_at(pieces: &[Piece], at: usize) -> usize {
    let mut piece_start = 0;
    for piece in pieces {
        let piece_end = piece_start + piece.span.text.len();
        if at < piece_end {
            return piece.offset + (at - piece_start);
        }
        piece_start = piece_end;
    }
    pieces.last().map_or(0, |piece| piece.offset)
}

/// Trims whitespace from both ends of a block, dropping spans that become empty.
fn trim_spans(spans: &mut Vec<Span>) {
    while let Some(last) = spans.last_mut() {
        last.text.truncate(last.text.trim_end().len());
        if !last.text.is_empty() {
            break;
        }
        spans.pop();
    }
    let blank = spans.iter().take_while(|s| s.text.trim().is_empty()).count();
    spans.drain(..blank);
    if let Some(first) = spans.first_mut() {
        let extra = first.text.len() - first.text.trim_start().len();
        first.text.drain(..extra);
    }
}

/// Every cue in `text` with the range it covers. Unknown `[[...]]` text is left alone.
fn find_cues(text: &str) -> Vec<(Range<usize>, Cue)> {
    let mut cues = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("[[").map(|at| from + at) {
        let Some(close) = text[open + 2..].find("]]").map(|at| open + 2 + at) else {
            break;
        };
        match parse_cue(&text[open + 2..close]) {
            Some(cue) => {
                cues.push((open..close + 2, cue));
                from = close + 2;
            }
            // Step one byte, so a cue that starts inside this text is still found.
            None => from = open + 1,
        }
    }
    cues
}

/// Parses the inside of `[[...]]`.
fn parse_cue(inner: &str) -> Option<Cue> {
    let (keyword, label) = match inner.split_once(':') {
        Some((keyword, label)) => (keyword.trim(), Some(label.trim())),
        None => (inner.trim(), None),
    };
    // An unknown inside that holds "[[" is not a cue; the inner one is found on its own.
    match (keyword.to_ascii_lowercase().as_str(), label) {
        (_, Some(label)) if label.contains("[[") => None,
        ("note", Some(label)) => Some(Cue::Note(label.to_owned())),
        ("pause", None) => Some(Cue::Pause),
        ("mark", Some(label)) => Some(Cue::Mark { label: label.to_owned(), category: MarkCategory::from_label(label) }),
        ("end", None) => Some(Cue::End),
        _ => None,
    }
}
