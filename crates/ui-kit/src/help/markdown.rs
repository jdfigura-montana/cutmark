//! The Markdown subset parser and renderer for help topics (spec: S1 Help
//! "Markdown subset"). Headings 1 to 3, paragraphs, bullet and numbered
//! lists, bold, italic, inline code, `<kbd>` keys, topic links, tip blocks and
//! horizontal rules. Everything else renders as plain text. Styling comes
//! only from ui-kit tokens.

use egui::{Label, Pos2, Response, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{color, space, typography};

/// Horizontal gap between two inline runs in a wrapped paragraph.
const INLINE_GAP: f32 = 4.0;
/// Horizontal padding inside a key cap, each side.
const KEY_PAD_X: f32 = 6.0;
/// Vertical padding inside a key cap, each side.
const KEY_PAD_Y: f32 = 2.0;
/// Corner radius of a key cap (smaller than `radius.group`).
const KEY_RADIUS: f32 = 4.0;
/// Width of the marker column of a list row.
const LIST_MARKER: f32 = 20.0;
/// Diameter of a bullet marker.
const BULLET: f32 = 4.0;
/// Height of a horizontal rule stroke.
const RULE: f32 = 1.0;

/// A run of inline text with its style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    /// Plain text.
    Text(String),
    /// **Bold** text.
    Bold(String),
    /// *Italic* text.
    Italic(String),
    /// `Inline code`.
    Code(String),
    /// A keyboard key written as `<kbd>X</kbd>`.
    Key(String),
    /// A `[text](topic:id)` link.
    Link {
        /// The text shown for the link.
        text: String,
        /// The topic id the link opens.
        target: String,
    },
}

/// One block of the Markdown subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A first level heading.
    Heading1(Vec<Span>),
    /// A second level heading.
    Heading2(Vec<Span>),
    /// A third level heading.
    Heading3(Vec<Span>),
    /// A paragraph.
    Paragraph(Vec<Span>),
    /// A bullet list, one span list per item.
    Bullets(Vec<Vec<Span>>),
    /// A numbered list, one span list per item.
    Numbers(Vec<Vec<Span>>),
    /// A `> Tip:` block.
    Tip(Vec<Span>),
    /// A horizontal rule.
    Rule,
}

/// Parses the Markdown subset into blocks. Unsupported lines become paragraphs.
pub fn parse(body: &str) -> Vec<Block> {
    let mut parser = Parser::default();
    for line in body.lines() {
        parser.line(line);
    }
    parser.flush();
    parser.blocks
}

/// Every `[text](topic:id)` target in a Markdown body, in reading order.
pub fn link_targets(body: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for block in parse(body) {
        for span in block.spans() {
            if let Span::Link { target, .. } = span {
                targets.push(target.clone());
            }
        }
    }
    targets
}

impl Block {
    /// The inline spans of this block, in reading order.
    pub fn spans(&self) -> Vec<&Span> {
        match self {
            Self::Heading1(spans) | Self::Heading2(spans) | Self::Heading3(spans) | Self::Paragraph(spans) | Self::Tip(spans) => spans.iter().collect(),
            Self::Bullets(items) | Self::Numbers(items) => items.iter().flatten().collect(),
            Self::Rule => Vec::new(),
        }
    }
}

/// The block parser's open state: one paragraph, list or tip at a time.
#[derive(Default)]
struct Parser {
    /// Finished blocks.
    blocks: Vec<Block>,
    /// The open paragraph's lines.
    paragraph: Vec<String>,
    /// The open bullet list's items.
    bullets: Vec<Vec<Span>>,
    /// The open numbered list's items.
    numbers: Vec<Vec<Span>>,
    /// The open tip's lines.
    tip: Vec<String>,
}

impl Parser {
    /// Feeds one raw line into the parser.
    fn line(&mut self, line: &str) {
        let line = line.trim();
        if line.is_empty() {
            self.flush_paragraph();
            self.flush_tip();
            return;
        }
        if is_rule(line) {
            self.flush();
            self.blocks.push(Block::Rule);
            return;
        }
        if let Some(rest) = line.strip_prefix('>') {
            self.flush_paragraph();
            self.flush_bullets();
            self.flush_numbers();
            let rest = rest.trim_start();
            let text = rest.strip_prefix("Tip:").unwrap_or(rest).trim();
            self.tip.push(text.to_string());
            return;
        }
        self.flush_tip();
        if let Some((level, text)) = heading(line) {
            self.flush();
            let spans = inline(text);
            self.blocks.push(match level {
                1 => Block::Heading1(spans),
                2 => Block::Heading2(spans),
                _ => Block::Heading3(spans),
            });
            return;
        }
        if let Some(item) = bullet_item(line) {
            self.flush_paragraph();
            self.flush_numbers();
            self.bullets.push(inline(item));
            return;
        }
        if let Some(item) = numbered_item(line) {
            self.flush_paragraph();
            self.flush_bullets();
            self.numbers.push(inline(item));
            return;
        }
        self.flush_bullets();
        self.flush_numbers();
        self.paragraph.push(line.to_string());
    }

    /// Closes every open block.
    fn flush(&mut self) {
        self.flush_paragraph();
        self.flush_bullets();
        self.flush_numbers();
        self.flush_tip();
    }

    /// Closes the open paragraph.
    fn flush_paragraph(&mut self) {
        if !self.paragraph.is_empty() {
            let text = self.paragraph.join(" ");
            self.blocks.push(Block::Paragraph(inline(&text)));
            self.paragraph.clear();
        }
    }

    /// Closes the open bullet list.
    fn flush_bullets(&mut self) {
        if !self.bullets.is_empty() {
            self.blocks.push(Block::Bullets(std::mem::take(&mut self.bullets)));
        }
    }

    /// Closes the open numbered list.
    fn flush_numbers(&mut self) {
        if !self.numbers.is_empty() {
            self.blocks.push(Block::Numbers(std::mem::take(&mut self.numbers)));
        }
    }

    /// Closes the open tip.
    fn flush_tip(&mut self) {
        if !self.tip.is_empty() {
            let text = self.tip.join(" ");
            self.blocks.push(Block::Tip(inline(&text)));
            self.tip.clear();
        }
    }
}

/// True for a horizontal rule line of three or more dashes, stars or underscores.
fn is_rule(line: &str) -> bool {
    line.len() >= 3 && (line.chars().all(|c| c == '-') || line.chars().all(|c| c == '*') || line.chars().all(|c| c == '_'))
}

/// A heading line's level and text, for one to three hashes and a space.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if !(1..=3).contains(&hashes) {
        return None;
    }
    let rest = line.get(hashes..)?.strip_prefix(' ')?;
    Some((hashes, rest.trim()))
}

/// A bullet item's text, for `- `, `* ` or `+ `.
fn bullet_item(line: &str) -> Option<&str> {
    ["- ", "* ", "+ "].iter().find_map(|marker| line.strip_prefix(marker))
}

/// A numbered item's text, for `1. ` or `1) `.
fn numbered_item(line: &str) -> Option<&str> {
    let digits = line.find(|c: char| !c.is_ascii_digit())?;
    if digits == 0 {
        return None;
    }
    let rest = line.get(digits..)?;
    rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")).map(str::trim_start)
}

/// Parses inline spans: `code`, **bold**, *italic*, `<kbd>` keys and topic links.
pub fn inline(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '`' => {
                if let Some(end) = find_char(&chars, index + 1, '`') {
                    push_plain(&mut spans, &mut plain);
                    spans.push(Span::Code(chars[index + 1..end].iter().collect()));
                    index = end + 1;
                    continue;
                }
            }
            '*' => {
                let marker = if chars.get(index + 1) == Some(&'*') { "**" } else { "*" };
                if let Some(end) = find_marker(&chars, index + marker.len(), marker) {
                    push_plain(&mut spans, &mut plain);
                    let content: String = chars[index + marker.len()..end].iter().collect();
                    spans.push(if marker == "**" { Span::Bold(content) } else { Span::Italic(content) });
                    index = end + marker.len();
                    continue;
                }
            }
            '<' => {
                if let Some((key, next)) = key_cap(&chars, index) {
                    push_plain(&mut spans, &mut plain);
                    spans.push(Span::Key(key));
                    index = next;
                    continue;
                }
            }
            '[' => {
                if let Some((link, next)) = topic_link(&chars, index) {
                    push_plain(&mut spans, &mut plain);
                    spans.push(link);
                    index = next;
                    continue;
                }
            }
            _ => {}
        }
        plain.push(chars[index]);
        index += 1;
    }
    push_plain(&mut spans, &mut plain);
    spans
}

/// The first index at or after `from` holding `needle`.
fn find_char(chars: &[char], from: usize, needle: char) -> Option<usize> {
    chars.iter().skip(from).position(|c| *c == needle).map(|offset| from + offset)
}

/// The first index at or after `from` starting a `marker` run.
fn find_marker(chars: &[char], from: usize, marker: &str) -> Option<usize> {
    if marker.len() == 1 {
        return find_char(chars, from, marker.chars().next()?);
    }
    (from..chars.len().saturating_sub(1)).find(|index| chars[*index] == '*' && chars[*index + 1] == '*')
}

/// The text of a `<kbd>...</kbd>` run starting at `index`, and the index after it.
fn key_cap(chars: &[char], index: usize) -> Option<(String, usize)> {
    let open: Vec<char> = "<kbd>".chars().collect();
    let close: Vec<char> = "</kbd>".chars().collect();
    if !starts_with(chars, index, &open) {
        return None;
    }
    let text_start = index + open.len();
    let end = find_sequence(chars, text_start, &close)?;
    Some((chars[text_start..end].iter().collect(), end + close.len()))
}

/// A `[text](topic:id)` link starting at `index`, and the index after it.
fn topic_link(chars: &[char], index: usize) -> Option<(Span, usize)> {
    let close = find_char(chars, index + 1, ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = find_char(chars, close + 2, ')')?;
    let target: String = chars[close + 2..end].iter().collect();
    let id = target.strip_prefix("topic:")?;
    let text: String = chars[index + 1..close].iter().collect();
    Some((Span::Link { text, target: id.to_string() }, end + 1))
}

/// True when `chars` holds `needle` at `index`.
fn starts_with(chars: &[char], index: usize, needle: &[char]) -> bool {
    chars.len() >= index + needle.len() && &chars[index..index + needle.len()] == needle
}

/// The first index at or after `from` where `needle` starts.
fn find_sequence(chars: &[char], from: usize, needle: &[char]) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    (from..chars.len().saturating_sub(needle.len() - 1)).find(|index| starts_with(chars, *index, needle))
}

/// Moves the pending plain text into a span.
fn push_plain(spans: &mut Vec<Span>, plain: &mut String) {
    if !plain.is_empty() {
        spans.push(Span::Text(std::mem::take(plain)));
    }
}

/// The text of a span run, without formatting.
fn plain_text(spans: &[Span]) -> String {
    spans
        .iter()
        .map(|span| match span {
            Span::Text(text) | Span::Bold(text) | Span::Italic(text) | Span::Code(text) | Span::Key(text) => text.as_str(),
            Span::Link { text, .. } => text.as_str(),
        })
        .collect()
}

/// Renders one block with ui-kit tokens. Returns the topic id of a clicked link.
pub fn render(ui: &mut Ui, block: &Block) -> Option<String> {
    match block {
        Block::Heading1(spans) => {
            ui.add_space(space::SCALE_8);
            ui.label(RichText::new(plain_text(spans)).font(fonts::semibold(typography::TITLE_SIZE)).color(color::TEXT_PRIMARY));
            None
        }
        Block::Heading2(spans) => {
            ui.add_space(space::SCALE_8);
            ui.label(RichText::new(plain_text(spans)).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
            None
        }
        Block::Heading3(spans) => {
            ui.add_space(space::SCALE_4);
            ui.label(RichText::new(plain_text(spans)).font(fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY));
            None
        }
        Block::Paragraph(spans) => inline_spans(ui, spans),
        Block::Bullets(items) => list(ui, items, false),
        Block::Numbers(items) => list(ui, items, true),
        Block::Tip(spans) => tip(ui, spans),
        Block::Rule => {
            rule(ui);
            None
        }
    }
}

/// Renders a wrapped run of inline spans.
fn inline_spans(ui: &mut Ui, spans: &[Span]) -> Option<String> {
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(INLINE_GAP);
        for span in spans {
            match span {
                Span::Text(text) => {
                    for word in text.split_whitespace() {
                        ui.label(RichText::new(word).font(fonts::medium(typography::BODY_SIZE)).color(color::TEXT_SECONDARY));
                    }
                }
                Span::Bold(text) => {
                    for word in text.split_whitespace() {
                        ui.label(RichText::new(word).font(fonts::semibold(typography::BODY_SIZE)).color(color::TEXT_PRIMARY));
                    }
                }
                Span::Italic(text) => {
                    for word in text.split_whitespace() {
                        ui.label(RichText::new(word).italics().font(fonts::medium(typography::BODY_SIZE)).color(color::TEXT_SECONDARY));
                    }
                }
                Span::Code(text) => {
                    ui.label(RichText::new(text).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_PRIMARY).background_color(color::BG_CONTROL));
                }
                Span::Key(key) => {
                    paint_key_cap(ui, key);
                }
                Span::Link { text, target } => {
                    let accent = Theme::get(ui.ctx()).accent.accent;
                    let response = ui.add(Label::new(RichText::new(text).font(fonts::medium(typography::BODY_SIZE)).color(accent)).extend());
                    if response.clicked() {
                        clicked = Some(target.clone());
                    }
                }
            }
        }
    });
    clicked
}

/// Renders a bullet or numbered list.
fn list(ui: &mut Ui, items: &[Vec<Span>], numbered: bool) -> Option<String> {
    let mut clicked = None;
    for (index, item) in items.iter().enumerate() {
        let mut row = None;
        ui.horizontal_top(|ui| {
            marker(ui, numbered, index);
            row = inline_spans(ui, item);
        });
        if row.is_some() {
            clicked = row;
        }
    }
    clicked
}

/// Draws the bullet or number ahead of a list item.
fn marker(ui: &mut Ui, numbered: bool, index: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(LIST_MARKER, ui.text_style_height(&egui::TextStyle::Body)), Sense::hover());
    if numbered {
        let label = format!("{}.", index + 1);
        let galley = ui.painter().layout_no_wrap(label, fonts::mono(typography::VALUE_SIZE), color::TEXT_TERTIARY);
        let position = Pos2::new(rect.right() - galley.size().x, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(position, galley, color::TEXT_TERTIARY);
    } else {
        ui.painter().circle_filled(rect.center(), BULLET / 2.0, color::TEXT_TERTIARY);
    }
}

/// Renders a `> Tip:` block as an inspector group.
fn tip(ui: &mut Ui, spans: &[Span]) -> Option<String> {
    let mut clicked = None;
    crate::inspector_group(ui, |ui| {
        ui.label(RichText::new("Tip").font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
        clicked = inline_spans(ui, spans);
    });
    clicked
}

/// Draws a horizontal rule in the hairline color.
fn rule(ui: &mut Ui) {
    ui.add_space(space::SCALE_4);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RULE), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, color::STROKE_HAIRLINE);
    ui.add_space(space::SCALE_4);
}

/// Draws a key cap: a small rounded box with a hairline stroke.
fn paint_key_cap(ui: &mut Ui, key: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(key.to_string(), fonts::mono(typography::VALUE_SIZE), color::TEXT_PRIMARY);
    let size = galley.size() + Vec2::new(KEY_PAD_X * 2.0, KEY_PAD_Y * 2.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, KEY_RADIUS, color::BG_CONTROL);
        painter.rect_stroke(rect, KEY_RADIUS, Stroke::new(crate::HAIRLINE, color::STROKE_HAIRLINE), StrokeKind::Inside);
        painter.galley(rect.center() - galley.size() / 2.0, galley, color::TEXT_PRIMARY);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headings of the first three levels become their own blocks, and two
    /// plain lines join into one paragraph.
    #[test]
    fn parses_headings_and_paragraphs() {
        let blocks = parse("# Title\n\nFirst line\nsecond line.\n\n## Section\n");
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0], Block::Heading1(vec![Span::Text("Title".to_string())]));
        assert_eq!(blocks[1], Block::Paragraph(vec![Span::Text("First line second line.".to_string())]));
        assert_eq!(blocks[2], Block::Heading2(vec![Span::Text("Section".to_string())]));
    }

    /// Bullet and numbered lists keep one span list per item.
    #[test]
    fn parses_bullets_and_numbers() {
        let blocks = parse("- one\n- two\n\n1. first\n2. second\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], Block::Bullets(vec![vec![Span::Text("one".to_string())], vec![Span::Text("two".to_string())]]));
        assert_eq!(blocks[1], Block::Numbers(vec![vec![Span::Text("first".to_string())], vec![Span::Text("second".to_string())]]));
    }

    /// A tip block and a horizontal rule are separate blocks.
    #[test]
    fn parses_tip_and_rule() {
        let blocks = parse("> Tip: Press <kbd>Ctrl</kbd>.\n\n---\n");
        assert_eq!(blocks[0], Block::Tip(vec![Span::Text("Press ".to_string()), Span::Key("Ctrl".to_string()), Span::Text(".".to_string()),]));
        assert_eq!(blocks[1], Block::Rule);
    }

    /// Bold, italic, code, keys and topic links all parse inside one line.
    #[test]
    fn parses_inline_spans() {
        let spans = inline("**Bold** and *italic* and `code` and [Recording](topic:recording).");
        assert_eq!(
            spans,
            [
                Span::Bold("Bold".to_string()),
                Span::Text(" and ".to_string()),
                Span::Italic("italic".to_string()),
                Span::Text(" and ".to_string()),
                Span::Code("code".to_string()),
                Span::Text(" and ".to_string()),
                Span::Link { text: "Recording".to_string(), target: "recording".to_string() },
                Span::Text(".".to_string()),
            ]
        );
    }

    /// Every topic link target is listed in reading order.
    #[test]
    fn link_targets_are_listed_in_reading_order() {
        let body = "See [Recording](topic:recording) and [Export](topic:export).\n\n- [Mix](topic:mix)\n";
        assert_eq!(link_targets(body), ["recording", "export", "mix"]);
    }

    /// A line the subset does not know renders as plain paragraph text.
    #[test]
    fn unknown_lines_become_paragraphs() {
        let blocks = parse("| a | b |\n");
        assert_eq!(blocks, [Block::Paragraph(vec![Span::Text("| a | b |".to_string())])]);
    }
}
