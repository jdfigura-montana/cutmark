//! What Copy and Cut put on the clipboard while Markdown syntax is hidden: the selection as
//! Markdown with every marker it cuts through closed, and the same as HTML.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::caret::Doc;
use crate::markup::{CueKind, Inline, underline_tag};
use crate::typing;

/// The HTML that goes with a text the editor has copied.
#[derive(Debug, Clone, Default)]
struct Rich {
    text: String,
    html: String,
}

fn rich_id() -> egui::Id {
    egui::Id::new("rich-copy")
}

/// Copies `selection`, through egui, and keeps its HTML for whoever writes the clipboard.
pub fn put(ctx: &egui::Context, doc: &Doc<'_>, selection: Range<usize>) {
    let text = markdown(doc, selection);
    if text.is_empty() {
        return;
    }
    let rich = Rich { html: html(&text), text: text.clone() };
    ctx.data_mut(|data| data.insert_temp(rich_id(), rich));
    ctx.copy_text(text);
}

/// The HTML of `text`, if the editor was what copied it.
pub fn take_html(ctx: &egui::Context, text: &str) -> Option<String> {
    ctx.data_mut(|data| data.remove_temp::<Rich>(rich_id())).filter(|rich| rich.text == text).map(|rich| rich.html)
}

/// A pair of markers, or of a code block's fences.
struct Pair {
    open: Range<usize>,
    /// Empty for a `<u>` left open.
    close: Range<usize>,
    /// Between them.
    content: Range<usize>,
    /// What is written where a marker has to be added.
    opener: String,
    closer: String,
    /// Blanks cannot be just inside the markers.
    trims: bool,
    /// It means something only with all that is between its markers.
    whole_only: bool,
}

impl Pair {
    fn new(text: &str, open: Range<usize>, close: Range<usize>) -> Self {
        let (opener, closer) = (text[open.clone()].to_owned(), text[close.clone()].to_owned());
        Self { content: open.end..close.start, open, close, opener, closer, trims: false, whole_only: false }
    }
}

fn pairs(doc: &Doc<'_>) -> Vec<Pair> {
    let (text, markup) = (doc.text, doc.markup);
    let formats = markup.spans.iter().map(|span| {
        let mut pair = Pair::new(text, span.open.clone(), span.close.clone());
        pair.trims = span.kind != Inline::Underline;
        if span.close.is_empty() {
            "</u>".clone_into(&mut pair.closer);
        }
        pair
    });
    let wraps = markup.wraps.iter().map(|wrap| Pair::new(text, wrap.open.clone(), wrap.close.clone()));
    let cues = markup.cues.iter().map(|cue| {
        let inner = &text[cue.range.start + 2..cue.range.end - 2];
        // A note or mark keeps its keyword around any part of its label.
        let label =
            inner.split_once(':').filter(|_| matches!(cue.kind, CueKind::Note | CueKind::Mark)).map(|(_, label)| cue.range.end - 2 - label.trim_start().len());
        let mut pair = Pair::new(text, cue.range.start..label.unwrap_or(cue.range.start + 2), cue.range.end - 2..cue.range.end);
        pair.whole_only = label.is_none();
        pair
    });
    let fences = markup.fences.iter().map(|fence| {
        let (first, last) = (doc.line_of(fence.open.start), doc.line_of(fence.close.start));
        Pair::new(text, doc.lines[first].start..doc.lines[first + 1].start, doc.lines[last - 1].end..doc.lines[last].end)
    });
    formats.chain(wraps).chain(cues).chain(fences).collect()
}

/// The lines to copy, from `selection`: where they start and end, and where a marker that has
/// to be added ahead of them goes.
fn reach(doc: &Doc<'_>, selection: &Range<usize>) -> (Range<usize>, usize) {
    let (first, last) = (doc.line_of(selection.start), doc.line_of(selection.end));
    let (start, end) = doc.bounds[first];
    // The caret cannot go ahead of a line's marks, so all of its text takes them too.
    let marked = selection.start <= start && (last > first || selection.end >= end);
    let from = if marked { doc.lines[first].start } else { selection.start };
    let lead = if marked { start.max(selection.start) } else { from };
    let mut to = selection.end;
    // Of a last line with none of its text selected, the marks stay behind.
    if last > first && to <= doc.bounds[last].0 {
        to = doc.lines[last].start;
    }
    // A heading with all its text keeps the line under it.
    let line = doc.line_of(to);
    if doc.underline.get(line + 1) == Some(&true) && to >= doc.bounds[line].1 && from <= doc.lines[line].start {
        to = doc.lines[line + 1].end;
    }
    (from..to.max(from), lead)
}

/// `selection` as Markdown. Formatting, code, links and cues that it cuts through are closed
/// around the part selected. Markers with none of their text selected are left out.
pub fn markdown(doc: &Doc<'_>, selection: Range<usize>) -> String {
    let text = doc.text;
    if selection.is_empty() || (selection.start == 0 && selection.end == text.len()) {
        return text[selection].to_owned();
    }
    let (range, lead) = reach(doc, &selection);
    let (from, to) = (range.start, range.end);
    let pairs = pairs(doc);
    let mut dropped = vec![false; to - from];
    let mut drop = |marker: &Range<usize>| {
        for at in marker.start.max(from)..marker.end.min(to) {
            dropped[at - from] = true;
        }
    };
    // The markers to add, each with its pair and the place its own marker has in the text.
    let (mut openers, mut closers) = (Vec::new(), Vec::new());
    for (index, pair) in pairs.iter().enumerate() {
        let inside = pair.content.start.max(from)..pair.content.end.min(to);
        let some = inside.start < inside.end && doc.visible(inside).next().is_some();
        let all = from <= pair.content.start && to >= pair.content.end;
        if some && (all || !pair.whole_only) {
            if from > pair.open.start {
                drop(&pair.open);
                openers.push((pair.open.start, index));
            }
            let unclosed = if pair.close.is_empty() { to <= pair.close.start } else { to < pair.close.end };
            if unclosed {
                drop(&pair.close);
                closers.push((pair.close.start, index));
            }
        } else if from > pair.open.start || to < pair.close.end {
            drop(&pair.open);
            drop(&pair.close);
        }
    }
    openers.sort_unstable();
    closers.sort_unstable();

    let mut out = String::new();
    // Where each pair's markers are in what is written.
    let mut opened: Vec<Option<Range<usize>>> = vec![None; pairs.len()];
    let mut closed = opened.clone();
    let mut moved = vec![None; to - from];
    let add = |out: &mut String, marker: &str, place: &mut Option<Range<usize>>| {
        *place = Some(out.len()..out.len() + marker.len());
        out.push_str(marker);
    };
    for (at, c) in text[from..to].char_indices().map(|(at, c)| (from + at, c)) {
        if at == lead {
            for &(_, index) in &openers {
                add(&mut out, &pairs[index].opener, &mut opened[index]);
            }
        }
        if !dropped[at - from] {
            moved[at - from] = Some(out.len());
            out.push(c);
        }
    }
    // Closing markers go ahead of any line breaks at the end.
    let breaks = out.split_off(out.trim_end_matches(['\r', '\n']).len());
    for &(_, index) in &closers {
        add(&mut out, &pairs[index].closer, &mut closed[index]);
    }
    out.push_str(&breaks);

    // The markers kept where they were.
    let kept = |marker: &Range<usize>| {
        let at = marker.start.checked_sub(from)?;
        let start = (*moved.get(at)?)?;
        Some(start..start + marker.len())
    };
    let mut trimmed: Vec<(Range<usize>, Range<usize>)> = pairs
        .iter()
        .enumerate()
        .filter(|(_, pair)| pair.trims)
        .filter_map(|(index, pair)| {
            let open = opened[index].clone().or_else(|| kept(&pair.open))?;
            let close = closed[index].clone().or_else(|| kept(&pair.close))?;
            Some((open, close))
        })
        .collect();
    trimmed.sort_by_key(|(open, close)| close.start.saturating_sub(open.end));
    let out = typing::tidy(out, 0, trimmed).change.text;

    // A nested item copied from its own marker on is not left indented.
    let first = &doc.lines[doc.line_of(from)];
    if from == first.start && first.marker.is_some() && first.quote == 0 && first.indent > 0 {
        let indent = first.indent_end - first.start;
        let outdent = |line: &str| line[line.bytes().take(indent).take_while(|&byte| byte == b' ' || byte == b'\t').count()..].to_owned();
        return out.split_inclusive('\n').map(outdent).collect();
    }
    out
}

fn escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// An HTML fragment of Markdown, for word processors: headings, lists, quotes, rules, bold,
/// italic, underline and strikethrough. Links are their text, and cues are text as written.
/// Text that is one paragraph has no paragraph around it, so that it pastes into a line.
pub fn html(markdown: &str) -> String {
    let mut out = String::new();
    // The `<u>` tags still open. None reaches past its block.
    let mut underlines = 0_usize;
    let end_underlines = |out: &mut String, open: &mut usize| {
        out.push_str(&"</u>".repeat(std::mem::take(open)));
    };
    // The same parser options as the views.
    for (event, range) in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        let strike = matches!(event, Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough));
        if strike && !markdown[range].starts_with("~~") {
            out.push('~');
            continue;
        }
        match event {
            Event::Start(Tag::Strong) => out.push_str("<strong>"),
            Event::End(TagEnd::Strong) => out.push_str("</strong>"),
            Event::Start(Tag::Emphasis) => out.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => out.push_str("</em>"),
            Event::Start(Tag::Strikethrough) => out.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => out.push_str("</s>"),
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) | Event::End(TagEnd::Link | TagEnd::Image) => {}
            Event::Start(tag) => {
                end_underlines(&mut out, &mut underlines);
                match tag {
                    Tag::Paragraph | Tag::HtmlBlock => out.push_str("<p>"),
                    Tag::Heading { level, .. } => out.push_str(&format!("<h{}>", level as u8)),
                    Tag::List(Some(1)) => out.push_str("<ol>"),
                    Tag::List(Some(start)) => out.push_str(&format!("<ol start=\"{start}\">")),
                    Tag::List(None) => out.push_str("<ul>"),
                    Tag::Item => out.push_str("<li>"),
                    Tag::BlockQuote(_) => out.push_str("<blockquote>"),
                    Tag::CodeBlock(_) => out.push_str("<pre>"),
                    _ => {}
                }
            }
            Event::End(tag) => {
                end_underlines(&mut out, &mut underlines);
                match tag {
                    TagEnd::Paragraph | TagEnd::HtmlBlock => out.push_str("</p>"),
                    TagEnd::Heading(level) => out.push_str(&format!("</h{}>", level as u8)),
                    TagEnd::List(ordered) => out.push_str(if ordered { "</ol>" } else { "</ul>" }),
                    TagEnd::Item => out.push_str("</li>"),
                    TagEnd::BlockQuote(_) => out.push_str("</blockquote>"),
                    TagEnd::CodeBlock => out.push_str("</pre>"),
                    _ => {}
                }
            }
            Event::InlineHtml(tag) => match underline_tag(&tag) {
                Some(true) => {
                    underlines += 1;
                    out.push_str("<u>");
                }
                Some(false) if underlines > 0 => {
                    underlines -= 1;
                    out.push_str("</u>");
                }
                Some(false) => {}
                None => escape(&mut out, &tag),
            },
            Event::Text(text) | Event::Code(text) | Event::Html(text) => escape(&mut out, &text),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak => out.push_str("<br>"),
            Event::Rule => {
                end_underlines(&mut out, &mut underlines);
                out.push_str("<hr>");
            }
            _ => {}
        }
    }
    end_underlines(&mut out, &mut underlines);
    let alone = out.strip_prefix("<p>").and_then(|rest| rest.strip_suffix("</p>")).filter(|rest| !rest.contains("<p>"));
    match alone {
        Some(inside) => inside.to_owned(),
        None => out,
    }
}

#[cfg(test)]
mod tests;
