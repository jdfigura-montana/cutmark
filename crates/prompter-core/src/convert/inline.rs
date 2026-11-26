//! One block's text as Markdown: escapes, cues and styles.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use super::{Glyph, Style};
use crate::script::{MARKDOWN_OPTIONS, underline_tag};
use crate::store::escape_plan;

/// Stands for a cue while the styles are checked. It is punctuation, as a cue's brackets
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
    strike: bool,
    underline: bool,
    /// Every other marker closes where an underline starts, so that `<u>` is outside them
    /// all. A tag works anywhere, but a '*' or '~' beside one may not.
    split: bool,
}

/// No styles at all.
const PLAIN: Attempt = Attempt { bold: false, italic: None, strike: false, underline: false, split: false };

/// The ways to write bold and italic, from the best to the plainest.
const EMPHASIS: [(bool, Option<&str>); 5] = [(true, Some("*")), (true, Some("_")), (true, None), (false, Some("*")), (false, None)];

/// From the best to the plainest. Markdown cannot express every mix of styles, so each is
/// read back before it is used: a style may be lost, but never turned into stray '*' or '~'.
/// Strikethrough is given up after bold and italic, and underline only with all the rest.
fn attempts() -> impl Iterator<Item = Attempt> {
    [true, false].into_iter().flat_map(|strike| {
        EMPHASIS.into_iter().flat_map(move |(bold, italic)| [false, true].map(|split| Attempt { bold, italic, strike, underline: true, split }))
    })
}

/// A style as the Markdown marks it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Marker {
    Underline,
    Bold,
    Italic,
    Strike,
}

/// Of two that open together and last as long, the first here goes outside.
const MARKERS: [Marker; 4] = [Marker::Underline, Marker::Bold, Marker::Italic, Marker::Strike];

impl Marker {
    fn wanted(self, style: Style) -> bool {
        match self {
            Self::Underline => style.underline,
            Self::Bold => style.bold,
            Self::Italic => style.italic,
            Self::Strike => style.strike,
        }
    }
}

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
        return render(&marks, PLAIN, false).0;
    }
    join_over_spaces(&mut marks);
    for attempt in attempts() {
        let mut styled = marks.clone();
        for glyph in styled.iter_mut().map(|mark| &mut mark.glyph) {
            glyph.style.bold &= attempt.bold;
            glyph.style.italic &= attempt.italic.is_some();
            glyph.style.strike &= attempt.strike;
            glyph.style.underline &= attempt.underline;
        }
        if attempt.split {
            part_at_underlines(&mut styled);
        }
        // An underline's tags need no fitting.
        fit_to_words(&mut styled, |style| &mut style.bold);
        fit_to_words(&mut styled, |style| &mut style.italic);
        fit_to_words(&mut styled, |style| &mut style.strike);
        let (checked, expected) = render(&styled, attempt, true);
        if reads_as(&checked, &expected) {
            return render(&styled, attempt, false).0;
        }
    }
    for mark in &mut marks {
        mark.glyph.style = Style::default();
    }
    render(&marks, PLAIN, false).0
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
        let shared = Style {
            bold: before.bold && after.bold,
            italic: before.italic && after.italic,
            underline: before.underline && after.underline,
            strike: before.strike && after.strike,
        };
        for mark in &mut marks[at..end] {
            mark.glyph.style = shared;
        }
        at = end;
    }
}

/// Takes every style off the spaces beside the start or end of an underline, so that no
/// marker has to close after one of them.
fn part_at_underlines(marks: &mut [Mark]) {
    let underlined = |mark: Option<&Mark>| mark.is_some_and(|mark| mark.glyph.style.underline);
    let mut at = 0;
    while at < marks.len() {
        if !marks[at].glyph.c.is_whitespace() {
            at += 1;
            continue;
        }
        let end = marks[at..].iter().position(|mark| !mark.glyph.c.is_whitespace()).map_or(marks.len(), |n| at + n);
        if underlined(at.checked_sub(1).map(|n| &marks[n])) != underlined(marks.get(end)) {
            for mark in &mut marks[at..end] {
                mark.glyph.style = Style::default();
            }
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
    let opener = |marker: Marker| match marker {
        Marker::Underline => "<u>",
        Marker::Bold => "**",
        Marker::Italic => attempt.italic.unwrap_or("*"),
        Marker::Strike => "~~",
    };
    let closer = |marker: Marker| if marker == Marker::Underline { "</u>" } else { opener(marker) };
    let mut markdown = String::new();
    let mut expected = Vec::with_capacity(marks.len());
    // The open styles, outermost first.
    let mut open: Vec<Marker> = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        if check && mark.cue && !mark.cue_start {
            continue;
        }
        let Glyph { c, style } = mark.glyph;
        let parts = attempt.split && style.underline && !c.is_whitespace() && !open.contains(&Marker::Underline);
        if let Some(first) = open.iter().position(|marker| parts || !marker.wanted(style)) {
            for marker in open.drain(first..).rev() {
                markdown.push_str(closer(marker));
            }
        }
        // Nothing opens in front of a space.
        if !c.is_whitespace() {
            let mut opening: Vec<Marker> = MARKERS.into_iter().filter(|marker| marker.wanted(style) && !open.contains(marker)).collect();
            // The one that lasts longer goes outside. When splitting, an underline always does.
            opening.sort_by_key(|&marker| {
                let lasts = marks[at..].iter().take_while(|mark| marker.wanted(mark.glyph.style)).count();
                (!(attempt.split && marker == Marker::Underline), std::cmp::Reverse(lasts))
            });
            for marker in opening {
                markdown.push_str(opener(marker));
                open.push(marker);
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
    for marker in open.into_iter().rev() {
        markdown.push_str(closer(marker));
    }
    (markdown, expected)
}

/// True if `markdown` shows exactly the expected text, styled as expected. The style of a
/// space does not show.
fn reads_as(markdown: &str, expected: &[Glyph]) -> bool {
    let mut read = Vec::with_capacity(expected.len());
    let (mut bold, mut italic, mut underline, mut strike) = (0_usize, 0_usize, 0_usize, 0_usize);
    for event in Parser::new_ext(markdown, MARKDOWN_OPTIONS) {
        let style = Style { bold: bold > 0, italic: italic > 0, underline: underline > 0, strike: strike > 0 };
        match event {
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::InlineHtml(html) => match underline_tag(&html) {
                Some(true) => underline += 1,
                Some(false) => underline = underline.saturating_sub(1),
                None => return false,
            },
            Event::Text(text) => read.extend(text.chars().map(|c| Glyph { c, style })),
            Event::HardBreak => read.push(Glyph { c: '\n', style }),
            _ => return false,
        }
    }
    read.len() == expected.len() && read.iter().zip(expected).all(|(a, b)| a.c == b.c && (a.c.is_whitespace() || a.style == b.style))
}
