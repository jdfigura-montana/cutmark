//! A script's Markdown line by line, as the formatting actions change it: each line's quote
//! marks, indent, list marker and heading marks, and where its list starts and ends.

use std::ops::Range;

use crate::markup::list_marker_width;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Blank,
    Text,
    /// A horizontal rule.
    Rule,
    /// The `===` or `---` under a heading's text.
    Underline,
    /// A code fence and the lines between a pair of them.
    Fence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    /// The width of the bullet or number, without the space after it.
    pub width: usize,
    /// An ordered item's number, and where it is written.
    pub number: Option<u64>,
    pub digits: Range<usize>,
}

/// One line. Positions are bytes of the whole text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub start: usize,
    /// Before the line break.
    pub end: usize,
    pub kind: Kind,
    /// How many quotes it is in, and where the marks of them end.
    pub quote: usize,
    pub quote_end: usize,
    /// The width of the blanks after the quote marks, and where they end.
    pub indent: usize,
    pub indent_end: usize,
    pub marker: Option<Marker>,
    /// Level 1 to 6, for a heading of either form.
    pub heading: Option<u8>,
    /// The text, without the marks around it.
    pub content: usize,
    pub content_end: usize,
}

/// The lines of `text`. There is always at least one.
pub fn parse(text: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut start = 0;
    // The marks of the code fence that is open.
    let mut fence: Option<&str> = None;
    for whole in text.split('\n') {
        let source = whole.strip_suffix('\r').unwrap_or(whole);
        let mut line = line(source, start);
        let rest = &source[line.indent_end - start..];
        let marks = ["```", "~~~"].into_iter().find(|marks| rest.starts_with(marks));
        if let Some(open) = fence {
            line.set_plain(Kind::Fence);
            if marks == Some(open) {
                fence = None;
            }
        } else if marks.is_some() {
            line.set_plain(Kind::Fence);
            fence = marks;
        } else if let Some(level) = underline_level(&source[line.quote_end - start..]) {
            let above =
                lines.last_mut().filter(|above| above.kind == Kind::Text && above.marker.is_none() && above.heading.is_none() && above.quote == line.quote);
            if let Some(above) = above {
                above.heading = Some(level);
                line.set_plain(Kind::Underline);
            }
        }
        lines.push(line);
        start += whole.len() + 1;
    }
    lines
}

/// One line on its own.
fn line(source: &str, start: usize) -> Line {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut quote = 0;
    loop {
        let spaces = bytes[at..].iter().take_while(|&&byte| byte == b' ').count();
        if spaces > 3 || bytes.get(at + spaces) != Some(&b'>') {
            break;
        }
        at += spaces + 1;
        at += usize::from(bytes.get(at) == Some(&b' '));
        quote += 1;
    }
    let quote_end = at;
    let mut indent = 0;
    while let Some(&byte @ (b' ' | b'\t')) = bytes.get(at) {
        indent += if byte == b'\t' { 4 - indent % 4 } else { 1 };
        at += 1;
    }
    let indent_end = at;
    let end = source.len();
    let mut line = Line {
        start,
        end: start + end,
        kind: Kind::Text,
        quote,
        quote_end: start + quote_end,
        indent,
        indent_end: start + indent_end,
        marker: None,
        heading: None,
        content: start + at,
        content_end: start + source.trim_end().len().max(at),
    };
    if at == end {
        // A bare ">" is an empty line of its quote.
        line.kind = Kind::Blank;
        return line;
    }
    if is_rule(&source[at..]) {
        line.kind = Kind::Rule;
        return line;
    }
    if let Some(width) = list_marker_width(&source[at..]) {
        let word = &source[at..at + width];
        let digits = word.bytes().take_while(u8::is_ascii_digit).count();
        let number = word[..digits].parse().ok();
        line.marker = Some(Marker { width, number, digits: start + at..start + at + digits });
        at += width;
        at += source[at..].len() - source[at..].trim_start_matches([' ', '\t']).len();
    }
    let hashes = source[at..].len() - source[at..].trim_start_matches('#').len();
    let spaced = matches!(bytes.get(at + hashes), None | Some(b' ' | b'\t'));
    if (1..=6).contains(&hashes) && spaced && indent < 4 {
        line.heading = u8::try_from(hashes).ok();
        at += hashes;
        at += source[at..].len() - source[at..].trim_start_matches([' ', '\t']).len();
        // A closing run of "#" counts only with a space before it.
        let body = source[at..].trim_end();
        let open = body.trim_end_matches('#');
        let body = if open.len() < body.len() && (open.is_empty() || open.ends_with([' ', '\t'])) { open.trim_end() } else { body };
        line.content_end = start + at + body.len();
    } else {
        line.content_end = line.content_end.max(start + at);
    }
    line.content = start + at;
    line
}

impl Line {
    /// For a line that is not text, whatever it looks like.
    fn set_plain(&mut self, kind: Kind) {
        self.kind = kind;
        self.marker = None;
        self.heading = None;
        self.content = self.indent_end;
        self.content_end = self.end;
    }
}

fn is_rule(rest: &str) -> bool {
    let mut marks = rest.chars().filter(|c| !c.is_whitespace());
    let Some(first @ ('-' | '*' | '_')) = marks.next() else {
        return false;
    };
    let mut count = 1;
    marks.all(|mark| {
        count += 1;
        mark == first
    }) && count >= 3
}

/// The heading level a line gives the text above it.
fn underline_level(rest: &str) -> Option<u8> {
    if rest.starts_with("    ") {
        return None;
    }
    let marks = rest.trim();
    // "- " could be an empty list item. It is left to be one.
    if !marks.is_empty() && marks.bytes().all(|byte| byte == b'=') {
        Some(1)
    } else if marks.len() > 1 && marks.bytes().all(|byte| byte == b'-') {
        Some(2)
    } else {
        None
    }
}

/// The line that byte `at` is on.
pub fn at(lines: &[Line], at: usize) -> usize {
    lines.partition_point(|line| line.start <= at).saturating_sub(1)
}

/// The line with a heading's text, for its underline. Any other line is itself.
pub fn title_of(lines: &[Line], line: usize) -> usize {
    if lines[line].kind == Kind::Underline { line.saturating_sub(1) } else { line }
}

/// True if a list cannot go on past this line, which is not an item.
fn ends_list(lines: &[Line], at: usize) -> bool {
    let line = &lines[at];
    let after_blank = at > 0 && lines[at - 1].kind == Kind::Blank;
    line.indent == 0 && (line.kind != Kind::Text || line.heading.is_some() || after_blank)
}

/// The nearest item above `line`, in its list, that is indented no further than `indent`.
pub fn previous_item(lines: &[Line], line: usize, indent: usize) -> Option<usize> {
    for above in (0..line).rev() {
        let found = &lines[above];
        if found.quote != lines[line].quote {
            break;
        }
        if found.marker.is_some() {
            if found.indent <= indent {
                return Some(above);
            }
        } else if found.kind != Kind::Blank && ends_list(lines, above) {
            break;
        }
    }
    None
}

/// The list item that `line` is, or that it carries on.
pub fn item_of(lines: &[Line], line: usize) -> Option<usize> {
    let own = &lines[line];
    if own.marker.is_some() {
        return Some(line);
    }
    if own.kind != Kind::Text || own.heading.is_some() {
        return None;
    }
    for above in (0..=line).rev() {
        let found = &lines[above];
        if found.quote != own.quote {
            break;
        }
        if found.marker.is_some() {
            return (above < line).then_some(above);
        }
        if found.kind == Kind::Blank {
            // Past an empty line, only an indented line still belongs to the item.
            if own.indent == 0 {
                break;
            }
        } else if ends_list(lines, above) {
            break;
        }
    }
    None
}

/// An item's lines: its own and those of everything under it.
pub fn subtree(lines: &[Line], item: usize) -> Range<usize> {
    let root = &lines[item];
    let mut end = item + 1;
    let mut after_blank = false;
    for (at, line) in lines.iter().enumerate().skip(item + 1) {
        if line.quote != root.quote {
            break;
        }
        if line.kind == Kind::Blank {
            after_blank = true;
            continue;
        }
        let carries_on = !after_blank && line.marker.is_none() && line.kind == Kind::Text && line.heading.is_none();
        if line.indent <= root.indent && !carries_on {
            break;
        }
        after_blank = false;
        end = at + 1;
    }
    item..end
}

/// The line after the last one of the list that `item` is in.
pub fn list_end(lines: &[Line], item: usize) -> usize {
    (item + 1..lines.len())
        .find(|&at| {
            let line = &lines[at];
            line.quote != lines[item].quote || (line.marker.is_none() && line.kind != Kind::Blank && ends_list(lines, at))
        })
        .unwrap_or(lines.len())
}
