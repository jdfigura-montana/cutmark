//! HTML, .docx and .odt to Markdown, for rich paste and import (docs/suite/prompter.md,
//! "Rich paste" and "Import").
//!
//! htmd parses the HTML. The Markdown is written here, because htmd's own writer would escape
//! `[[...]]` cues and does not escape plain text the way the .txt import does. The .docx and
//! .odt readers use the same writer.

mod docx;
mod inline;
mod odt;
mod package;

pub(crate) use docx::docx_to_markdown;
pub(crate) use odt::odt_to_markdown;
pub use package::DocumentError;

use htmd::HtmlToMarkdown;
use htmd::node::Node;

/// How many elements deep the structure is read. Below this only the text is kept, so a
/// hostile document cannot overflow the stack.
const MAX_NESTING: usize = 200;

/// The largest number Markdown reads as a list marker: nine digits.
const MAX_NUMBER: u64 = 999_999_999;

/// Elements whose contents are never text of the document.
const DROPPED: [&str; 16] =
    ["audio", "canvas", "head", "iframe", "meta", "noscript", "object", "script", "select", "style", "svg", "template", "textarea", "title", "video", "xml"];

/// Elements that start and end a line.
const BLOCKS: [&str; 24] = [
    "address",
    "article",
    "aside",
    "body",
    "caption",
    "center",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "form",
    "header",
    "main",
    "nav",
    "p",
    "section",
    "summary",
    "table",
    "td",
    "th",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
}

/// One character of text. A `\n` is a line break.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Glyph {
    c: char,
    style: Style,
}

#[derive(Debug, Clone, Copy)]
struct Item {
    /// 0 for a top-level list.
    depth: usize,
    /// `None` for a bullet.
    number: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Paragraph,
    Heading(usize),
    Item(Item),
    /// A horizontal rule. It has no text.
    Rule,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Quote {
    depth: usize,
    /// Tells the paragraphs of one quote from those of the next.
    id: usize,
}

#[derive(Debug)]
struct Block {
    kind: Kind,
    quote: Quote,
    text: Vec<Glyph>,
}

/// Converts HTML, a whole document or a fragment of one, to the Markdown a script uses.
///
/// Headings, bold, italic, lists, blockquotes, horizontal rules and line breaks are kept, and
/// `[[...]]` cues stay cues. Each table cell becomes its own paragraph, links become their
/// text, and everything else is dropped. The result ends with a newline unless it is empty.
pub fn html_to_markdown(html: &str) -> String {
    // htmd parses a string without reading or decoding anything, so this does not fail.
    let Ok(tree) = HtmlToMarkdown::new().html_to_tree(html) else {
        return String::new();
    };
    let mut reader = Reader::default();
    reader.children(&tree.root(), Style::default(), 0);
    reader.flush();
    write(&reader.blocks)
}

/// Collects the document's text into blocks.
#[derive(Debug, Default)]
struct Reader {
    blocks: Vec<Block>,
    text: Vec<Glyph>,
    heading: Option<usize>,
    /// The list item whose marker has not been used yet.
    item: Option<Item>,
    /// The open lists. Each holds its next number, or `None` for a bullet list.
    lists: Vec<Option<u64>>,
    quote: Quote,
    preformatted: bool,
}

impl Reader {
    fn children(&mut self, node: &Node<'_>, style: Style, nesting: usize) {
        let mut next = node.first_child();
        while let Some(child) = next {
            self.node(&child, style, nesting);
            next = child.next_sibling();
        }
    }

    /// Comments, Word's conditional ones among them, have neither text nor a tag.
    fn node(&mut self, node: &Node<'_>, style: Style, nesting: usize) {
        if let Some(text) = node.text() {
            self.push_str(&text, style);
        } else if let Some(tag) = node.tag_name() {
            self.element(node, tag, style, nesting);
        }
    }

    fn element(&mut self, node: &Node<'_>, tag: &str, style: Style, nesting: usize) {
        // A numbered heading keeps its number as text. Every other marker is rewritten.
        let kept_number = || self.heading.is_some() && self.item.is_none() && is_number(&text_of(node));
        if DROPPED.contains(&tag) || (is_list_marker(node) && !kept_number()) {
            return;
        }
        if nesting >= MAX_NESTING {
            self.push_str(&text_of(node), style);
            return;
        }
        let style = style_of(node, tag, style);
        let nesting = nesting + 1;
        let word_item = if self.item.is_none() && matches!(tag, "p" | "div") { word_item(node) } else { None };
        match tag {
            "br" => self.push_break(),
            // Inside a heading or a list item a rule only ends a line.
            "hr" if self.item.is_none() && self.heading.is_none() => {
                self.flush();
                self.blocks.push(Block { kind: Kind::Rule, quote: self.quote, text: Vec::new() });
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if self.item.is_none() => {
                self.flush();
                let outer = self.heading.replace(usize::from(tag.as_bytes()[1] - b'0'));
                self.children(node, style, nesting);
                self.flush();
                self.heading = outer;
            }
            "ul" | "ol" | "menu" => {
                self.flush();
                let outer = self.item.take();
                let first = node.attr("start").and_then(|start| start.trim().parse::<u64>().ok()).unwrap_or(1);
                self.lists.push((tag == "ol").then_some(first.min(MAX_NUMBER)));
                self.children(node, style, nesting);
                self.flush();
                self.lists.pop();
                self.item = outer;
            }
            "li" => {
                let depth = self.lists.len().saturating_sub(1);
                let number = self.lists.last_mut().and_then(Option::as_mut).map(|next| {
                    let number = *next;
                    *next = number.saturating_add(1).min(MAX_NUMBER);
                    number
                });
                self.in_item(Item { depth, number }, node, style, nesting);
            }
            "blockquote" if self.item.is_none() => {
                self.flush();
                if self.quote.depth == 0 {
                    self.quote.id += 1;
                }
                self.quote.depth += 1;
                self.children(node, style, nesting);
                self.flush();
                self.quote.depth -= 1;
            }
            "pre" => {
                self.boundary();
                let outer = std::mem::replace(&mut self.preformatted, true);
                self.children(node, style, nesting);
                self.preformatted = outer;
                self.boundary();
            }
            _ => {
                if let Some(item) = word_item {
                    self.in_item(item, node, style, nesting);
                } else if is_block(tag) {
                    self.boundary();
                    self.children(node, style, nesting);
                    self.boundary();
                } else {
                    self.children(node, style, nesting);
                }
            }
        }
    }

    fn in_item(&mut self, item: Item, node: &Node<'_>, style: Style, nesting: usize) {
        self.flush();
        let outer = self.item.replace(item);
        self.children(node, style, nesting);
        self.flush();
        self.item = outer;
    }

    fn push_str(&mut self, text: &str, style: Style) {
        for c in text.chars() {
            if self.preformatted && c == '\n' {
                self.push_break();
            } else {
                self.push(c, style);
            }
        }
    }

    /// Adds a character, collapsing white space as a browser does. No-break spaces collapse
    /// too: Word writes an empty paragraph as one.
    fn push(&mut self, c: char, style: Style) {
        if !matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c' | '\u{a0}') {
            self.text.push(Glyph { c, style });
        } else if self.text.last().is_some_and(|last| !matches!(last.c, ' ' | '\n')) {
            self.text.push(Glyph { c: ' ', style });
        }
    }

    fn push_break(&mut self) {
        if self.heading.is_some() && self.item.is_none() {
            // A Markdown heading is one line.
            self.push(' ', Style::default());
            return;
        }
        if self.text.last().is_some_and(|last| last.c == ' ') {
            self.text.pop();
        }
        // Two breaks end a paragraph. A list item stays in one piece, because a second
        // paragraph in it would be drawn with a marker of its own.
        let repeated = self.item.is_some() && self.text.last().is_some_and(|last| last.c == '\n');
        if !self.text.is_empty() && !repeated {
            self.text.push(Glyph { c: '\n', style: Style::default() });
        }
    }

    /// The start or end of a block element. Inside a heading or a list item it only ends a
    /// line.
    fn boundary(&mut self) {
        if self.item.is_none() && self.heading.is_none() {
            self.flush();
        } else if self.text.last().is_some_and(|last| last.c != '\n') {
            self.push_break();
        }
    }

    /// Makes blocks of the collected text: one for each run of lines between empty lines.
    fn flush(&mut self) {
        let text = std::mem::take(&mut self.text);
        let mut block: Vec<Glyph> = Vec::new();
        for line in text.split(|glyph| glyph.c == '\n') {
            let line = trim(line);
            if line.is_empty() {
                self.push_block(std::mem::take(&mut block));
                continue;
            }
            if !block.is_empty() {
                block.push(Glyph { c: '\n', style: Style::default() });
            }
            block.extend_from_slice(line);
        }
        self.push_block(block);
    }

    fn push_block(&mut self, text: Vec<Glyph>) {
        if text.is_empty() {
            return;
        }
        // Only an item's first block gets its marker.
        let kind = match (self.item.take(), self.heading) {
            (Some(item), _) => Kind::Item(item),
            (None, Some(level)) => Kind::Heading(level),
            (None, None) => Kind::Paragraph,
        };
        self.blocks.push(Block { kind, quote: self.quote, text });
    }
}

fn is_block(tag: &str) -> bool {
    // The headings and quotes here are those inside a list item.
    BLOCKS.contains(&tag) || matches!(tag, "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "hr" | "tr")
}

/// The line without the white space at its ends.
fn trim(line: &[Glyph]) -> &[Glyph] {
    let shown = |glyph: &Glyph| !glyph.c.is_whitespace();
    match (line.iter().position(shown), line.iter().rposition(shown)) {
        (Some(first), Some(last)) => &line[first..=last],
        _ => &[],
    }
}

/// All the text under `node`, without recursion. A space stands where each element starts, so
/// words in different elements stay apart.
fn text_of(node: &Node<'_>) -> String {
    let mut text = String::new();
    let mut waiting = vec![node.clone()];
    while let Some(node) = waiting.pop() {
        if let Some(part) = node.text() {
            text.push_str(&part);
        } else if node.tag_name().is_some_and(|tag| !DROPPED.contains(&tag)) {
            if is_block(node.tag_name().unwrap_or_default()) {
                text.push(' ');
            }
            // Last child first, so that the first one is read next.
            let mut child = node.last_child();
            while let Some(current) = child {
                child = current.prev_sibling();
                waiting.push(current);
            }
        }
    }
    text
}

/// The value of a property in the element's `style` attribute, in lowercase. The whole name
/// has to match, so Word's `mso-bidi-font-weight` is not `font-weight`.
fn css(node: &Node<'_>, property: &str) -> Option<String> {
    let style = node.attr("style")?;
    let mut found = None;
    for declaration in style.split(';') {
        if let Some((name, value)) = declaration.split_once(':')
            && name.trim().eq_ignore_ascii_case(property)
        {
            let value = value.split('!').next().unwrap_or_default();
            found = Some(value.trim().to_ascii_lowercase());
        }
    }
    found
}

/// The style of an element's text: what it inherits, then what its tag says, then what its
/// `style` attribute says.
fn style_of(node: &Node<'_>, tag: &str, mut style: Style) -> Style {
    // Google Docs wraps what it copies in a <b> that is not bold.
    if node.attr("id").is_some_and(|id| id.starts_with("docs-internal-guid")) {
        return style;
    }
    style.bold |= matches!(tag, "b" | "strong");
    style.italic |= matches!(tag, "i" | "em");
    if let Some(weight) = css(node, "font-weight") {
        match weight.as_str() {
            "bold" | "bolder" => style.bold = true,
            "normal" | "lighter" => style.bold = false,
            number => style.bold = number.parse::<u16>().map_or(style.bold, |number| number >= 600),
        }
    }
    match css(node, "font-style").as_deref() {
        Some("italic" | "oblique") => style.italic = true,
        Some("normal") => style.italic = false,
        _ => {}
    }
    style
}

/// True for the bullet or number Word writes into a list paragraph's text.
fn is_list_marker(node: &Node<'_>) -> bool {
    css(node, "mso-list").as_deref() == Some("ignore")
}

/// The list item a Word paragraph stands for, if it is one. Word writes lists as paragraphs:
/// the level is in the `mso-list` style, whatever the paragraph's class, and the marker is
/// text. A `MsoListParagraph` is an item even without that style.
fn word_item(node: &Node<'_>) -> Option<Item> {
    let level = css(node, "mso-list").and_then(|list| list.split_whitespace().find_map(|part| part.strip_prefix("level")?.parse::<usize>().ok()));
    let listed = || node.attr("class").is_some_and(|class| class.split_whitespace().any(|name| name.starts_with("MsoListParagraph")));
    if level.is_none() && !listed() {
        return None;
    }
    let marker = node.find(is_list_marker).map(|marker| text_of(&marker));
    Some(Item { depth: level.unwrap_or(1).saturating_sub(1), number: marker.and_then(|marker| number_of(&marker)) })
}

/// True if a heading's marker is a number, such as "1.", "2.1" or "A.", and not a bullet.
fn is_number(marker: &str) -> bool {
    marker.chars().any(|c| c.is_ascii_digit()) || number_of(marker).is_some()
}

/// The number of a marker such as "3." or "b)", or `None` for a bullet. A letter or a Roman
/// numeral gives 1.
fn number_of(marker: &str) -> Option<u64> {
    let marker = marker.trim_matches(|c: char| c.is_whitespace());
    let body = marker.strip_suffix(['.', ')'])?;
    let body = body.strip_prefix('(').unwrap_or(body);
    let last = body.rsplit('.').next().unwrap_or_default();
    if last.is_empty() || !last.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(last.parse::<u64>().ok().filter(|&number| number <= MAX_NUMBER).unwrap_or(1))
}

/// An open level of a Markdown list.
struct Level {
    /// The depth the source gave it, which may be more than the depth it is written at.
    depth: usize,
    /// The width of its marker, which is how far its contents are indented.
    width: usize,
    ordered: bool,
}

/// Writes the blocks, with one blank line between them except inside a list.
fn write(blocks: &[Block]) -> String {
    let mut markdown = String::new();
    let mut levels: Vec<Level> = Vec::new();
    let mut previous: Option<&Block> = None;
    for block in blocks {
        let same_quote = previous.is_some_and(|previous| previous.quote == block.quote);
        let mut tight = false;
        let (indent, marker) = if let Kind::Item(item) = block.kind {
            let in_list = same_quote && previous.is_some_and(|previous| matches!(previous.kind, Kind::Item(_)));
            if !in_list {
                levels.clear();
            }
            // A level deeper than the next one would read as a code block. Counting the open
            // levels above this one also keeps equal depths level with each other in a list
            // that starts, or skips to, a deeper one.
            let depth = levels.iter().take_while(|level| level.depth < item.depth).count();
            let ordered = item.number.is_some();
            let new_level = depth == levels.len();
            let new_type = levels.get(depth).is_some_and(|level| level.ordered != ordered);
            // Without a blank line, a list that starts past 1 would join the text above it.
            let joins_text = (new_level || new_type) && item.number.is_some_and(|number| number != 1);
            tight = in_list && !new_type && !joins_text;
            levels.truncate(depth);
            let indent: usize = levels.iter().map(|level| level.width).sum();
            let marker = item.number.map_or_else(|| "- ".to_owned(), |number| format!("{number}. "));
            levels.push(Level { depth: item.depth, width: marker.len(), ordered });
            (indent, marker)
        } else {
            let marker = match block.kind {
                Kind::Heading(level) => format!("{} ", "#".repeat(level)),
                // The blank line before it keeps it from underlining the text above.
                Kind::Rule => "---".to_owned(),
                _ => String::new(),
            };
            (0, marker)
        };

        let quote = "> ".repeat(block.quote.depth);
        if previous.is_some() && !tight {
            if same_quote && block.quote.depth > 0 {
                // A bare '>' keeps both paragraphs in one quote.
                markdown.push_str(quote.trim_end());
            }
            markdown.push('\n');
        }
        let continuation = format!("\n{quote}{}", " ".repeat(indent + if matches!(block.kind, Kind::Item(_)) { marker.len() } else { 0 }));
        markdown.push_str(&quote);
        markdown.push_str(&" ".repeat(indent));
        markdown.push_str(&marker);
        let text = inline::markdown(&block.text, matches!(block.kind, Kind::Heading(_)));
        markdown.push_str(&text.replace('\n', &continuation));
        markdown.push('\n');
        previous = Some(block);
    }
    markdown
}
