//! One block's text as Markdown: escapes, cues and emphasis.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use super::{Glyph, Style};
use crate::store::escape_plan;

/// Stands for a cue while the emphasis is checked. It is punctuation, as a cue's brackets
/// are, and means nothing to Markdown without a '[' after it.
const CUE_STAND_IN: char = '!';

/// A glyph and how it is written.
#[derive(Clone, Copy)]
struct Mark {
    glyph: Glyph,
    /// It takes a backslash.
    escaped: bool,
    /// It is part of a cue.
    cue: bool,
    /// It is the first character of a cue.
    cue_start: bool,
}

/// One way of writing the styles.
#[derive(Clone, Copy)]
struct Attempt {
    bold: bool,
    italic: Option<&'static str>,
}

/// From the best to the plainest. Markdown cannot express every mix of styles, so each is
/// read back before it is used: a style may be lost, but never turned into stray '*'.
const ATTEMPTS: [Attempt; 5] = [
    Attempt { bold: true, italic: Some("*") },
    Attempt { bold: true, italic: Some("_") },
    Attempt { bold: true, italic: None },
    Attempt { bold: false, italic: Some("*") },
    Attempt { bold: false, italic: None },
];

/// The Markdown for one block's text. Line breaks come out as a backslash and a newline.
pub(super) fn markdown(text: &[Glyph], heading: bool) -> String {
    let mut marks = plan(text);
    // A '#' at the end of a heading would be read as the closing of it.
    if heading
        && let Some(last) = marks.last_mut()
        && last.glyph.c == '#'
    {
        last.escaped = true;
    }
    if marks.iter().all(|mark| mark.glyph.style == Style::default()) {
        return render(&marks, ATTEMPTS[4], false).0;
    }
    join_over_spaces(&mut marks);
    for attempt in ATTEMPTS {
        let mut styled = marks.clone();
        for glyph in styled.iter_mut().map(|mark| &mut mark.glyph) {
            glyph.style.bold &= attempt.bold;
            glyph.style.italic &= attempt.italic.is_some();
        }
        fit_to_words(&mut styled, |style| &mut style.bold);
        fit_to_words(&mut styled, |style| &mut style.italic);
        let (checked, expected) = render(&styled, attempt, true);
        if reads_as(&checked, &expected) {
            return render(&styled, attempt, false).0;
        }
    }
    for mark in &mut marks {
        mark.glyph.style = Style::default();
    }
    render(&marks, ATTEMPTS[4], false).0
}

/// Finds the cues and the characters to escape, line by line. Cues lose their styles, since a
/// marker inside one would change it.
fn plan(text: &[Glyph]) -> Vec<Mark> {
    let mut marks: Vec<Mark> = text.iter().map(|&glyph| Mark { glyph, escaped: false, cue: false, cue_start: false }).collect();
    for line in marks.split_mut(|mark| mark.glyph.c == '\n') {
        let plain: String = line.iter().map(|mark| mark.glyph.c).collect();
        let (escapes, cues) = escape_plan(&plain);
        for (mark, (at, _)) in line.iter_mut().zip(plain.char_indices()) {
            mark.escaped = escapes.binary_search(&at).is_ok();
            if let Some(cue) = cues.iter().find(|cue| cue.contains(&at)) {
                mark.cue = true;
                mark.cue_start = cue.start == at;
                mark.glyph.style = Style::default();
            }
        }
    }
    marks
}

/// Gives each space and line break the styles its neighbors share, so that two bold words
/// make one bold run.
fn join_over_spaces(marks: &mut [Mark]) {
    let mut before = Style::default();
    let mut at = 0;
    while at < marks.len() {
        if !marks[at].glyph.c.is_whitespace() {
            before = marks[at].glyph.style;
            at += 1;
            continue;
        }
        let end = marks[at..].iter().position(|mark| !mark.glyph.c.is_whitespace()).map_or(marks.len(), |n| at + n);
        let after = marks.get(end).map(|mark| mark.glyph.style).unwrap_or_default();
        let shared = Style { bold: before.bold && after.bold, italic: before.italic && after.italic };
        for mark in &mut marks[at..end] {
            mark.glyph.style = shared;
        }
        at = end;
    }
}

/// Shrinks each run of one style until Markdown can open and close it. A marker cannot touch
/// a space on its inner side, or punctuation there with a letter on the other side. The text
/// that falls outside, such as the '(' in "a(**b)**", loses the style.
fn fit_to_words(marks: &mut [Mark], style: fn(&mut Style) -> &mut bool) {
    let mut at = 0;
    while at < marks.len() {
        if !*style(&mut marks[at].glyph.style) {
            at += 1;
            continue;
        }
        let mut end = at;
        while end < marks.len() && *style(&mut marks[end].glyph.style) {
            end += 1;
        }
        let (mut from, mut to) = (at, end);
        while from < to && blocks_marker(marks[from].glyph.c, from.checked_sub(1).map(|n| marks[n].glyph.c)) {
            *style(&mut marks[from].glyph.style) = false;
            from += 1;
        }
        while from < to && blocks_marker(marks[to - 1].glyph.c, marks.get(to).map(|mark| mark.glyph.c)) {
            to -= 1;
            *style(&mut marks[to].glyph.style) = false;
        }
        at = end;
    }
}

/// True if a marker cannot sit between `inside` and `outside`.
fn blocks_marker(inside: char, outside: Option<char>) -> bool {
    inside.is_whitespace() || (!inside.is_alphanumeric() && outside.is_some_and(char::is_alphanumeric))
}

/// Writes the marks. With `check`, each cue is written as [`CUE_STAND_IN`]. The second value
/// is the text and styles the Markdown should read as.
fn render(marks: &[Mark], attempt: Attempt, check: bool) -> (String, Vec<Glyph>) {
    let marker = |bold: bool| if bold { "**" } else { attempt.italic.unwrap_or("*") };
    let wants = |style: Style, bold: bool| if bold { style.bold } else { style.italic };
    let mut markdown = String::new();
    let mut expected = Vec::with_capacity(marks.len());
    // The open styles, outermost first. True is bold and false is italic.
    let mut open: Vec<bool> = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        if check && mark.cue && !mark.cue_start {
            continue;
        }
        let Glyph { c, style } = mark.glyph;
        if let Some(first) = open.iter().position(|&bold| !wants(style, bold)) {
            for bold in open.drain(first..).rev() {
                markdown.push_str(marker(bold));
            }
        }
        // Nothing opens in front of a space.
        if !c.is_whitespace() {
            let mut opening: Vec<bool> = [true, false].into_iter().filter(|&bold| wants(style, bold) && !open.contains(&bold)).collect();
            // The one that lasts longer goes outside.
            opening.sort_by_key(|&bold| std::cmp::Reverse(marks[at..].iter().take_while(|mark| wants(mark.glyph.style, bold)).count()));
            for bold in opening {
                markdown.push_str(marker(bold));
                open.push(bold);
            }
        }
        if c == '\n' {
            markdown.push_str("\\\n");
        } else if check && mark.cue {
            markdown.push(CUE_STAND_IN);
        } else {
            if mark.escaped {
                markdown.push('\\');
            }
            markdown.push(c);
        }
        let c = if check && mark.cue { CUE_STAND_IN } else { c };
        expected.push(Glyph { c, style });
    }
    for bold in open.into_iter().rev() {
        markdown.push_str(marker(bold));
    }
    (markdown, expected)
}

/// True if `markdown` shows exactly the expected text, styled as expected. The style of a
/// space does not show.
fn reads_as(markdown: &str, expected: &[Glyph]) -> bool {
    let mut read = Vec::with_capacity(expected.len());
    let (mut bold, mut italic) = (0_usize, 0_usize);
    for event in Parser::new(markdown) {
        let style = Style { bold: bold > 0, italic: italic > 0 };
        match event {
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Text(text) => read.extend(text.chars().map(|c| Glyph { c, style })),
            Event::HardBreak => read.push(Glyph { c: '\n', style }),
            _ => return false,
        }
    }
    read.len() == expected.len() && read.iter().zip(expected).all(|(a, b)| a.c == b.c && (a.c.is_whitespace() || a.style == b.style))
}
