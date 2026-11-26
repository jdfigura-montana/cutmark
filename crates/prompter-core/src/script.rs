//! Markdown to blocks (docs/suite/prompter.md, "Scripts").

use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::{mem, slice};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

/// A parsed script.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Script {
    pub blocks: Vec<Block>,
    /// Hash of the Markdown, to check a cached word index against. A new Rust release may
    /// change the value, which only means the cache is rebuilt.
    pub source_hash: u64,
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
        /// The rest of an item that a cue split. It gets no bullet or number.
        continues: bool,
        /// The list's first number, set only on the first item of an ordered list.
        start: Option<u64>,
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
    /// `<u>text</u>`.
    pub underline: bool,
    /// `~~text~~`.
    pub strike: bool,
}

impl Span {
    fn same_style(&self, other: &Self) -> bool {
        (self.bold, self.italic, self.underline, self.strike) == (other.bold, other.italic, other.underline, other.strike)
    }
}

/// A `[[...]]` cue. Cues are never spoken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cue {
    Note(String),
    Pause,
    /// `label` is trimmed. A leading category and its colon are not part of it.
    Mark {
        label: String,
        category: MarkCategory,
    },
    End,
}

/// The suite mark types a script can use, named and spelled as in the marks crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkCategory {
    Highlight,
    Cut,
    Retake,
    Chapter,
    Note,
    BRoll,
}

/// The names a label may use. Writers type "B-roll"; "b_roll" is the marks crate's spelling.
const CATEGORY_NAMES: [(&str, MarkCategory); 7] = [
    ("highlight", MarkCategory::Highlight),
    ("cut", MarkCategory::Cut),
    ("retake", MarkCategory::Retake),
    ("chapter", MarkCategory::Chapter),
    ("note", MarkCategory::Note),
    ("b-roll", MarkCategory::BRoll),
    ("b_roll", MarkCategory::BRoll),
];

impl MarkCategory {
    fn from_name(text: &str) -> Option<Self> {
        CATEGORY_NAMES.iter().find(|(name, _)| text.eq_ignore_ascii_case(name)).map(|&(_, category)| category)
    }

    /// Splits a trimmed mark label into its category and the label to keep.
    ///
    /// "B-roll: desk" gives "desk". Any other label is kept as written, and its first word
    /// may still name the category: "B-roll here" matches and "Cutaway" does not. A label
    /// that names none is a [`MarkCategory::Note`].
    fn split_label(label: &str) -> (Self, &str) {
        let typed = label.split_once(':').and_then(|(name, rest)| Some((Self::from_name(name.trim_end())?, rest.trim_start())));
        typed.unwrap_or_else(|| {
            let end = label.find(|c: char| c == ':' || c.is_whitespace()).unwrap_or(label.len());
            (Self::from_name(&label[..end]).unwrap_or(Self::Note), label)
        })
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

/// The Markdown extensions a script uses. With strikethrough on, pulldown-cmark also reads
/// `~one tilde~` as struck, which [`parse`] turns back into text.
pub(crate) const MARKDOWN_OPTIONS: Options = Options::ENABLE_STRIKETHROUGH;

/// `Some(true)` for the inline HTML `<u>`, `Some(false)` for `</u>` and `None` for any other.
/// A `<u>` with attributes is one of the others.
pub(crate) fn underline_tag(html: &str) -> Option<bool> {
    let name = html.strip_prefix('<')?.strip_suffix('>')?.trim_end();
    let (name, opens) = name.strip_prefix('/').map_or((name, true), |name| (name, false));
    name.eq_ignore_ascii_case("u").then_some(opens)
}

/// Parses a Markdown script into blocks.
///
/// Links and images become their plain text. HTML is dropped, except that an inline `<u>`
/// underlines the text up to its `</u>`, or to the end of its block.
///
/// A cue inside a block splits it into the text before, the cue block and the text after.
/// Cues inside a heading do not split it, so that one heading stays one section. Brackets
/// inside inline code and code blocks never open or close a cue.
pub fn parse(md: &str) -> Script {
    let mut builder = Builder::new(md);
    for (event, range) in Parser::new_ext(md, MARKDOWN_OPTIONS).into_offset_iter() {
        let strike = matches!(event, Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough));
        if strike && !md[range.clone()].starts_with("~~") {
            // One tilde is not a marker. The range covers the whole run, so it has both.
            let at = if matches!(event, Event::Start(_)) { range.start } else { range.end - 1 };
            builder.text("~", at, false);
        } else {
            builder.event(event, range);
        }
    }
    builder.flush();
    Script { blocks: builder.blocks, source_hash: hash_source(md) }
}

fn hash_source(md: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    md.hash(&mut hasher);
    hasher.finish()
}

/// Text collected for the current block, before cues are cut out of it.
struct Piece {
    span: Span,
    /// Byte offset in the Markdown where this text starts.
    offset: usize,
    /// Inline code or code block text.
    code: bool,
}

/// An open list.
struct List {
    ordered: bool,
    /// The first number of an ordered list, until a block or an empty item has used it.
    start: Option<u64>,
}

struct Builder {
    /// Byte offset of every newline in the Markdown.
    newlines: Vec<usize>,
    blocks: Vec<Block>,
    pieces: Vec<Piece>,
    heading: Option<u8>,
    lists: Vec<List>,
    quotes: usize,
    code_block: bool,
    bold: usize,
    italic: usize,
    /// Open `<u>` tags. They never reach past their block.
    underline: usize,
    strike: usize,
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
            code_block: false,
            bold: 0,
            italic: 0,
            underline: 0,
            strike: 0,
        }
    }

    fn event(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text, range.start, self.code_block),
            Event::Code(text) => self.text(&text, range.start, true),
            Event::SoftBreak => self.text(" ", range.start, false),
            Event::HardBreak => self.text("\n", range.start, false),
            Event::Rule => {
                self.flush();
                let line = self.line(range.start);
                self.push_block(BlockKind::Rule, Vec::new(), line);
            }
            Event::InlineHtml(html) => match underline_tag(&html) {
                Some(true) => self.underline += 1,
                Some(false) => self.underline = self.underline.saturating_sub(1),
                None => {}
            },
            // Other HTML is dropped. The other events need parser options that are not enabled.
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { .. } | Tag::Image { .. } => {}
            // Every other tag starts a block. A tight list item has no paragraph of its own,
            // so its text must end here when a nested block begins.
            other => {
                self.flush();
                match other {
                    Tag::Heading { level, .. } => self.heading = Some(heading_number(level)),
                    Tag::List(start) => self.lists.push(List { ordered: start.is_some(), start }),
                    Tag::BlockQuote(_) => self.quotes += 1,
                    Tag::CodeBlock(_) => self.code_block = true,
                    _ => {}
                }
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {}
            other => {
                // Flush first: the block's kind depends on the containers still open.
                self.flush();
                match other {
                    TagEnd::Heading(_) => self.heading = None,
                    TagEnd::List(_) => {
                        self.lists.pop();
                    }
                    // An item that made no text block still uses up its number.
                    TagEnd::Item => {
                        if let Some(start) = self.lists.last_mut().and_then(|list| list.start.as_mut()) {
                            *start = start.saturating_add(1);
                        }
                    }
                    TagEnd::CodeBlock => self.code_block = false,
                    TagEnd::BlockQuote(_) => self.quotes = self.quotes.saturating_sub(1),
                    _ => {}
                }
            }
        }
    }

    fn text(&mut self, text: &str, offset: usize, code: bool) {
        let span = Span { text: text.to_owned(), bold: self.bold > 0, italic: self.italic > 0, underline: self.underline > 0, strike: self.strike > 0 };
        self.pieces.push(Piece { span, offset, code });
    }

    fn text_kind(&self) -> BlockKind {
        if let Some(level) = self.heading {
            BlockKind::Heading(level)
        } else if let Some(list) = self.lists.last() {
            let depth = u8::try_from(self.lists.len() - 1).unwrap_or(u8::MAX);
            BlockKind::ListItem { ordered: list.ordered, depth, continues: false, start: list.start }
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
        // A `<u>` left open ends here.
        self.underline = 0;
        if self.pieces.is_empty() {
            return;
        }
        let pieces = mem::take(&mut self.pieces);
        let text: String = pieces.iter().map(|p| p.span.text.as_str()).collect();
        // The same text with the brackets in code hidden, byte for byte.
        let searchable: String = pieces.iter().map(|p| if p.code { p.span.text.replace(['[', ']'], "_") } else { p.span.text.clone() }).collect();
        let mut kind = self.text_kind();
        let cues = find_cues(&text, &searchable);

        if matches!(kind, BlockKind::Heading(_)) {
            self.push_heading(&pieces, &text, cues, kind);
            return;
        }

        let mut done = 0;
        for (range, cue) in cues {
            self.push_part(&pieces, &text, done..range.start, &mut kind);
            self.push_cue(&pieces, range.start, cue);
            done = range.end;
        }
        self.push_part(&pieces, &text, done..text.len(), &mut kind);
    }

    /// Pushes the text between two cues. Once a list item has a block, `kind` becomes that of
    /// its continuation.
    fn push_part(&mut self, pieces: &[Piece], text: &str, range: Range<usize>, kind: &mut BlockKind) {
        if !self.push_text(pieces, text, slice::from_ref(&range), kind.clone()) {
            return;
        }
        if let BlockKind::ListItem { continues, start, .. } = kind {
            *continues = true;
            *start = None;
            if let Some(list) = self.lists.last_mut() {
                list.start = None;
            }
        }
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
    /// Returns true if it pushed one.
    fn push_text(&mut self, pieces: &[Piece], text: &str, ranges: &[Range<usize>], kind: BlockKind) -> bool {
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
        let Some(offset) = first else {
            return false;
        };
        let line = self.line(offset);
        self.push_block(kind, spans, line);
        true
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
                Some(last) if last.same_style(&piece.span) => {
                    last.text.push_str(part);
                }
                _ => spans.push(Span { text: part.to_owned(), ..piece.span }),
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
///
/// Brackets are looked for in `searchable`, which has the same byte positions as `text`.
/// A cue ends at the last `]]` of a run of closing brackets.
fn find_cues(text: &str, searchable: &str) -> Vec<(Range<usize>, Cue)> {
    let mut cues = Vec::new();
    let mut from = 0;
    while let Some(open) = searchable[from..].find("[[").map(|at| from + at) {
        let Some(mut close) = searchable[open + 2..].find("]]").map(|at| open + 2 + at) else {
            break;
        };
        while searchable[close + 2..].starts_with(']') {
            close += 1;
        }
        let inner = open + 2..close;
        // An inside that holds "[[" is not a cue; the inner one is found on its own.
        let cue = if searchable[inner.clone()].contains("[[") { None } else { parse_cue(&text[inner]) };
        match cue {
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
    match (keyword.to_ascii_lowercase().as_str(), label) {
        ("note", Some(label)) => Some(Cue::Note(label.to_owned())),
        ("pause", None) => Some(Cue::Pause),
        ("mark", Some(label)) => {
            let (category, label) = MarkCategory::split_label(label);
            Some(Cue::Mark { label: label.to_owned(), category })
        }
        ("end", None) => Some(Cue::End),
        _ => None,
    }
}
