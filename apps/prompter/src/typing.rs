//! Typing, deleting and Enter while Markdown syntax is hidden. Only what can be seen is
//! deleted. Hidden syntax goes only with all that it formats, and formatting is kept in a
//! shape that Markdown still reads, so that none of it turns into visible marks.

use std::ops::Range;

use crate::caret::{Bias, Doc};
use crate::formatting::{self, Action, BlockStyle, Change, ListKind};
use crate::lines::{self, Kind, Line};
use crate::markup::{Edge, Inline, Markup};

/// Formatting that a space typed at its end was moved out of. Markdown does not read
/// `**word **` as bold, so the closing marker goes ahead of the space, and what is typed
/// next goes back inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carry {
    /// The text it holds for.
    text: String,
    /// Where the caret is, after the space.
    caret: usize,
    /// The closing markers, ahead of the space.
    close: Range<usize>,
}

impl Carry {
    fn holds(&self, text: &str, selection: &Range<usize>) -> bool {
        selection.is_empty() && selection.start == self.caret && self.text == text
    }
}

/// The text after a key, with the caret, and formatting for what is typed next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Typed {
    pub change: Change,
    pub carry: Option<Carry>,
}

fn caret_at(text: String, caret: usize) -> Typed {
    Typed { change: Change { text, selection: caret..caret }, carry: None }
}

/// Types or pastes `insert` over `selection`.
pub fn insert(doc: &Doc<'_>, selection: Range<usize>, insert: &str, carry: Option<&Carry>) -> Typed {
    if let Some(carry) = carry.filter(|carry| carry.holds(doc.text, &selection) && !insert.contains('\n')) {
        let (text, caret, close) = (doc.text, carry.caret, carry.close.clone());
        if insert.trim().is_empty() {
            let mut new = text.to_owned();
            new.insert_str(caret, insert);
            let caret = caret + insert.len();
            let carry = Carry { text: new.clone(), caret, close };
            return Typed { carry: Some(carry), ..caret_at(new, caret) };
        }
        let gap = &text[close.end..caret];
        let new = format!("{}{gap}{insert}{}{}", &text[..close.start], &text[close.clone()], &text[caret..]);
        let caret = close.start + gap.len() + insert.len();
        return caret_at(new, caret);
    }
    replace(doc, selection, insert)
}

/// [`insert`] for a typed character. A `-` or `=` typed right under a line of text would make
/// that text a heading and itself vanish, so it gets an empty line above it.
pub fn type_text(doc: &Doc<'_>, selection: Range<usize>, typed: &str, carry: Option<&Carry>) -> Typed {
    let done = insert(doc, selection, typed, carry);
    let done = if typed.contains(['-', '=']) { no_new_underline(doc.markup, done) } else { done };
    if matches!(typed, "-" | "`" | "~") { past_hidden_line(done, typed) } else { done }
}

/// Typing the last mark of a `---` rule, or of a code block's closing fence, hides the line
/// the caret is on. The caret goes to an empty line under it, which is made if there is none.
/// A rule of `*` or `_` keeps the caret, as it may be the start of bold or italic.
fn past_hidden_line(mut done: Typed, typed: &str) -> Typed {
    let caret = done.change.selection.start;
    let markup = Markup::parse(&done.change.text);
    let rows = lines::parse(&done.change.text);
    let at = lines::at(&rows, caret);
    let row = &rows[at];
    let dashes = typed == "-"
        && markup.rule_lines.binary_search(&at).is_ok()
        && caret == row.end
        && done.change.text[row.indent_end..row.end].bytes().all(|byte| byte == b'-' || byte == b' ');
    if !dashes && !markup.fences.iter().any(|fence| fence.close.end == caret) {
        return done;
    }
    let below = rows.get(at + 1).filter(|below| below.kind == Kind::Blank && below.quote == row.quote);
    let to = match below {
        Some(below) => below.end,
        None => {
            let put = format!("\n{}", &done.change.text[row.start..row.quote_end]);
            done.change.text.insert_str(row.end, &put);
            row.end + put.len()
        }
    };
    done.change.selection = to..to;
    done.carry = None;
    done
}

/// The places of the lines that are a heading's underline.
pub fn underlines(markup: &Markup) -> impl Iterator<Item = usize> + '_ {
    markup.edges.iter().filter(|(_, edge)| *edge == Edge::Underline).map(|(range, _)| range.start)
}

/// A `-` or an empty list item right under a line of text is read as that text's underline.
/// If a key has made one that `before` did not have, an empty line goes above it.
fn no_new_underline(before: &Markup, mut done: Typed) -> Typed {
    let markup = Markup::parse(&done.change.text);
    if underlines(&markup).count() <= underlines(before).count() {
        return done;
    }
    let text = &done.change.text;
    let rows = lines::parse(text);
    let caret = done.change.selection.start;
    // The new one is on the caret's line or the one next to it.
    let near = lines::at(&rows, caret);
    let found = underlines(&markup).map(|at| lines::at(&rows, at)).min_by_key(|line| line.abs_diff(near));
    if let Some(row) = found.map(|line| &rows[line]) {
        let gap = format!("{}\n", text[row.start..row.quote_end].trim_end());
        let start = row.start;
        done.change.text.insert_str(start, &gap);
        let caret = if caret >= start { caret + gap.len() } else { caret };
        done.change.selection = caret..caret;
        done.carry = None;
    }
    done
}

/// What a pair of hidden markers is around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Format(Inline),
    /// Inline code.
    Code,
    /// A link or a cue.
    Whole,
}

/// The opening and closing markers of each formatted span, inline code, link and cue.
fn spans(markup: &Markup) -> Vec<(Held, Range<usize>, Range<usize>)> {
    let formats = markup.spans.iter().map(|span| (Held::Format(span.kind), span.open.clone(), span.close.clone()));
    let wraps = markup.wraps.iter().map(|wrap| (if wrap.code { Held::Code } else { Held::Whole }, wrap.open.clone(), wrap.close.clone()));
    let cues = markup.cues.iter().map(|cue| (Held::Whole, cue.range.start..cue.range.start + 2, cue.range.end - 2..cue.range.end));
    formats.chain(wraps).chain(cues).collect()
}

/// Puts `insert` where `selection` is, deleting what can be seen of it.
pub fn replace(doc: &Doc<'_>, selection: Range<usize>, insert: &str) -> Typed {
    let (text, a, b) = (doc.text, selection.start, selection.end);
    if a == 0 && b == text.len() {
        return caret_at(insert.to_owned(), insert.len());
    }
    let mut gone = vec![false; text.len()];
    // Marks to put in, by the byte they go ahead of.
    let mut added: Vec<(usize, String)> = Vec::new();

    let (first, last) = (doc.line_of(a), doc.line_of(b));
    // Whole lines go with their marks, and the line after them keeps its own.
    let whole_lines = insert.is_empty() && last > first && a == doc.bounds[first].0 && b <= doc.bounds[last].0;
    if whole_lines {
        gone[doc.lines[first].start..doc.lines[last].start].fill(true);
    } else {
        for found in doc.visible(a..b) {
            gone[found].fill(true);
        }
        join_lines(doc, &selection, &mut gone, &mut added);
    }
    remove_rules(doc, &selection, &mut gone);
    keep_fences(doc, insert.is_empty(), &mut gone);

    // A heading with an underline cannot be left with no text.
    for line in (1..doc.lines.len()).filter(|&line| doc.underline[line]) {
        let title = doc.bounds[line - 1].0..doc.lines[line - 1].end;
        let mut seen = doc.visible(title).peekable();
        if insert.is_empty() && seen.peek().is_some() && seen.all(|found| gone[found.start]) {
            gone[doc.lines[line - 1].end..doc.lines[line].end].fill(true);
        }
    }

    let spans = spans(doc.markup);
    // What formats only deleted text goes, unless new text takes its place.
    for (_, open, close) in &spans {
        let content = open.end..close.start;
        let within = !insert.is_empty() && a >= content.start && b <= content.end;
        let mut seen = doc.visible(content).peekable();
        if !within && seen.peek().is_some() && seen.all(|found| gone[found.start]) {
            gone[open.clone()].fill(true);
            gone[close.clone()].fill(true);
        }
    }
    // Two of a kind that now touch become one.
    for (kind, _, close) in spans.iter().filter(|(kind, ..)| a < b && *kind != Held::Whole) {
        for (other, open, _) in &spans {
            let between = close.end..open.start;
            let typed_between = !insert.is_empty() && (close.end..=open.start).contains(&a);
            let touch = close.end < open.start && between.clone().all(|at| gone[at]);
            // Code joins only code with as many backticks.
            let alike = other == kind && (*kind != Held::Code || text[close.clone()] == text[open.clone()]);
            if alike && touch && !typed_between && !close.is_empty() && !gone[close.start] && !gone[open.start] {
                gone[close.clone()].fill(true);
                gone[open.clone()].fill(true);
            }
        }
    }

    added.sort_by_key(|(at, _)| *at);
    let deleting = insert.is_empty();
    let mut added = added.into_iter().peekable();
    let mut new = String::with_capacity(text.len() + insert.len());
    // Where each byte of the old text is in the new one.
    let mut moved = vec![0; text.len() + 1];
    let mut caret = 0;
    let ends = std::iter::once((text.len(), None));
    for (at, c) in text.char_indices().map(|(at, c)| (at, Some(c))).chain(ends) {
        while let Some((_, mark)) = added.next_if(|(before, _)| *before <= at) {
            new.push_str(&mark);
        }
        if at == a {
            new.push_str(insert);
            caret = new.len();
        }
        moved[at] = new.len();
        if let Some(c) = c.filter(|_| !gone[at]) {
            new.push(c);
        }
    }

    keep_rules(doc, &gone, &mut new, &mut moved, &mut caret);

    // The formatting that is left, innermost first.
    let mut left: Vec<(Range<usize>, Range<usize>)> = spans
        .iter()
        .filter(|(kind, open, close)| {
            matches!(kind, Held::Format(kind) if *kind != Inline::Underline) && !gone[open.start] && !close.is_empty() && !gone[close.start]
        })
        .map(|(_, open, close)| (moved[open.start]..moved[open.start] + open.len(), moved[close.start]..moved[close.start] + close.len()))
        .collect();
    left.sort_by_key(|(open, close)| close.start.saturating_sub(open.end));
    let mut done = tidy(new, caret, left);
    if deleting {
        past_marks(&mut done);
    }
    done
}

/// The lines with a horizontal rule.
fn rules<'a>(doc: &'a Doc<'_>) -> impl DoubleEndedIterator<Item = usize> + 'a {
    doc.markup.rule_lines.iter().copied()
}

/// The line break that shows for the rule on `line`: the one after it, or on the last line,
/// the one ahead.
fn rule_break(doc: &Doc<'_>, line: usize) -> Range<usize> {
    let own = &doc.lines[line];
    match doc.lines.get(line + 1) {
        Some(next) => own.end..next.start,
        None => line.checked_sub(1).map_or(own.end..own.end, |above| doc.lines[above].end..own.start),
    }
}

/// A rule goes as one, with the line break that shows for it, when that break is deleted or
/// the selection runs over the rule.
fn remove_rules(doc: &Doc<'_>, selection: &Range<usize>, gone: &mut [bool]) {
    for line in rules(doc) {
        let (own, shown) = (&doc.lines[line], rule_break(doc, line));
        let broken = selection.start < shown.end && shown.start < selection.end;
        if broken || (!selection.is_empty() && selection.contains(&own.start)) {
            gone[own.start.min(shown.start)..own.end.max(shown.end)].fill(true);
        }
    }
}

/// A rule of dashes that has come to be right under text would make that text a heading. It is
/// written with asterisks, which cannot.
fn keep_rules(doc: &Doc<'_>, gone: &[bool], new: &mut String, moved: &mut [usize], caret: &mut usize) {
    let kept: Vec<Range<usize>> =
        doc.markup.edges.iter().filter(|(range, edge)| *edge == Edge::Rule && !gone[range.start]).map(|(range, _)| range.clone()).collect();
    if kept.is_empty() {
        return;
    }
    let markup = Markup::parse(new);
    let made: Vec<usize> = underlines(&markup).collect();
    // From the last, so that the places ahead stay put.
    for rule in kept.iter().rev() {
        let at = moved[rule.start];
        if !made.contains(&at) {
            continue;
        }
        let (old, put) = (rule.len(), "***");
        new.replace_range(at..at + old, put);
        for place in moved.iter_mut().chain(std::iter::once(&mut *caret)) {
            if *place >= at + old {
                *place = *place - old + put.len();
            }
        }
    }
}

/// True for a line with nothing on it but quote marks.
fn is_empty_line(doc: &Doc<'_>, line: usize) -> bool {
    doc.lines[line].kind == Kind::Blank
}

/// The lines of a code block's fences.
fn fence_lines<'a>(doc: &'a Doc<'_>) -> impl Iterator<Item = (usize, usize)> + 'a {
    doc.markup.fences.iter().map(|fence| (doc.line_of(fence.open.start), doc.line_of(fence.close.start)))
}

/// Both fences of a code block go once all its code is deleted. Until then both stay, each on
/// a line of its own.
fn keep_fences(doc: &Doc<'_>, deleting: bool, gone: &mut [bool]) {
    for (first, last) in fence_lines(doc) {
        let (open, close) = (&doc.lines[first], &doc.lines[last]);
        let (code, closing) = (doc.lines[first + 1].start, doc.lines[last - 1].end);
        let mut seen = doc.visible(code..closing).peekable();
        if deleting && seen.peek().is_some() && seen.all(|found| gone[found.start]) {
            gone[open.start..code].fill(true);
            gone[closing..close.end].fill(true);
        } else {
            gone[open.start..code].fill(false);
            gone[closing..close.end].fill(false);
            // The breaks outside the block stay if text is left to run into a fence.
            let kept = |gone: &[bool], line: &Line| (line.content..line.content_end).any(|at| !gone[at]);
            if first > 0 && kept(gone, &doc.lines[first - 1]) {
                gone[doc.lines[first - 1].end..open.start].fill(false);
            }
            if let Some(below) = doc.lines.get(last + 1).filter(|below| kept(gone, below)) {
                gone[close.end..below.start].fill(false);
            }
        }
    }
}

/// Moves a caret that a deletion has left ahead of its line's marks to the text after them.
fn past_marks(done: &mut Typed) {
    let markup = Markup::parse(&done.change.text);
    let doc = Doc::new(&done.change.text, &markup);
    let caret = done.change.selection.start;
    let line = doc.line_of(caret);
    let start = doc.bounds[line].0;
    let to = match () {
        // On a line it cannot be on, it goes back to the text ahead.
        () if doc.skipped[line] => doc.rest(caret, Bias::Back),
        () => caret.max(start),
    };
    if to != caret {
        done.change.selection = to..to;
    }
}

/// A line break that is deleted joins two lines. The marks that only mean something at a
/// line's start or end go with it.
fn join_lines(doc: &Doc<'_>, selection: &Range<usize>, gone: &mut [bool], added: &mut Vec<(usize, String)>) {
    let breaks: Vec<usize> = doc.visible(selection.clone()).filter(|found| &doc.text[found.clone()] == "\n").map(|found| found.start).collect();
    for at in breaks {
        let above = doc.line_of(at);
        let below = above + 1;
        // A rule goes whole instead.
        if rules(doc).any(|line| rule_break(doc, line).contains(&at)) {
            continue;
        }
        // The break that shows by a fence is the one after it. Nothing can share the fence's
        // line, so all that can go there is an empty line on the far side of the break.
        if fence_lines(doc).any(|(first, _)| first == above) {
            gone[at] = false;
            if let Some(prior) = above.checked_sub(1).filter(|&prior| is_empty_line(doc, prior)) {
                gone[doc.lines[prior].start..doc.lines[above].start].fill(true);
            }
            continue;
        }
        if fence_lines(doc).any(|(_, last)| last == above) {
            if is_empty_line(doc, below) {
                gone[at..doc.lines[below].end].fill(true);
            } else {
                gone[at] = false;
            }
            continue;
        }
        gone[doc.lines[below].start..doc.bounds[below].0].fill(true);
        if doc.underline[above] {
            // The heading takes the other form, which text can follow on its line.
            let title = above - 1;
            gone[doc.lines[title].end..doc.lines[above].end].fill(true);
            if let Some(level) = doc.lines[title].heading {
                added.push((doc.bounds[title].0, format!("{} ", "#".repeat(usize::from(level)))));
            }
        } else {
            gone[doc.bounds[above].1..doc.lines[above].end].fill(true);
            // An underline cannot go under a line that has marks of its own.
            let marked = doc.bounds[above].0 > doc.lines[above].start;
            if marked && doc.underline.get(below + 1) == Some(&true) {
                gone[doc.lines[below].end..doc.lines[below + 1].end].fill(true);
            }
        }
    }
}

/// Takes `range` out of `text`, and moves the positions after it back.
fn cut(text: &mut String, range: Range<usize>, positions: &mut [&mut usize]) {
    text.replace_range(range.clone(), "");
    for at in positions {
        if **at > range.start {
            **at -= range.len().min(**at - range.start);
        }
    }
}

/// Moves the blanks at the ends of formatted text outside its markers, and drops markers
/// with nothing but blanks between them.
pub(crate) fn tidy(mut text: String, mut caret: usize, mut spans: Vec<(Range<usize>, Range<usize>)>) -> Typed {
    // The closing markers moved from behind the caret.
    let mut carried: Option<Range<usize>> = None;
    for index in 0..spans.len() {
        let (open, close) = spans[index].clone();
        let Some(content) = text.get(open.end..close.start) else {
            continue;
        };
        if content.trim().is_empty() {
            for range in [close, open] {
                let (mut start, mut end) = (range.start, range.end);
                cut(&mut text, start..end, &mut [&mut caret, &mut start, &mut end]);
                for (open, close) in &mut spans[index + 1..] {
                    for at in [&mut open.start, &mut open.end, &mut close.start, &mut close.end] {
                        if *at > range.start {
                            *at -= range.len().min(*at - range.start);
                        }
                    }
                }
            }
            continue;
        }
        let leading = content.len() - content.trim_start().len();
        let trailing = content.len() - content.trim_end().len();
        if trailing > 0 {
            let (marker, blank) = (text[close.clone()].to_owned(), text[close.start - trailing..close.start].to_owned());
            let start = close.start - trailing;
            text.replace_range(start..close.end, &format!("{marker}{blank}"));
            if caret == close.start {
                caret = close.end;
                let from = carried.as_ref().map_or(start, |carried| carried.start.min(start));
                let to = carried.as_ref().map_or(start + marker.len(), |carried| carried.end.max(start + marker.len()));
                carried = Some(from..to);
            } else if caret > start && caret < close.start {
                caret += marker.len();
            }
        }
        if leading > 0 {
            let (marker, blank) = (text[open.clone()].to_owned(), text[open.end..open.end + leading].to_owned());
            text.replace_range(open.start..open.end + leading, &format!("{blank}{marker}"));
            if caret == open.end {
                caret = open.start;
            } else if caret > open.end && caret <= open.end + leading {
                caret = open.end + leading;
            }
        }
    }
    let carry = carried.filter(|close| close.end <= caret).map(|close| Carry { text: text.clone(), caret, close });
    Typed { carry, ..caret_at(text, caret) }
}

/// True if the caret at `at` has nothing but marks between it and the start of a line that is
/// a list item, a heading or a quote.
fn at_block_start(doc: &Doc<'_>, at: usize) -> bool {
    let line = doc.line_of(at);
    let start = doc.bounds[line].0;
    let marked = start > doc.lines[line].start || doc.underline.get(line + 1) == Some(&true);
    marked && doc.visible(start..at.max(start)).next().is_none()
}

/// Takes away the formatting of the caret's line: its list marker, or else its heading or
/// quote marks.
fn unformat(doc: &Doc<'_>, caret: usize) -> Option<Typed> {
    let line = lines::title_of(&doc.lines, doc.line_of(caret));
    let action = match &doc.lines[line].marker {
        Some(marker) => Action::List(if marker.number.is_some() { ListKind::Numbered } else { ListKind::Bulleted }),
        None => Action::Style(BlockStyle::Paragraph),
    };
    let change = formatting::apply(doc.text, doc.markup, caret..caret, action)?;
    Some(Typed { change: keep_apart(change), carry: None })
}

/// Adds the empty lines that keep what is typed on an empty line out of the blocks around it.
pub fn keep_apart(mut change: Change) -> Change {
    let rows = lines::parse(&change.text);
    let mut caret = change.selection.start;
    let at = lines::at(&rows, caret);
    let row = &rows[at];
    if !change.selection.is_empty() || row.marker.is_some() || row.heading.is_some() || row.content < row.content_end {
        return change;
    }
    let text_line = |line: &Line| line.kind == Kind::Text && line.heading.is_none() && line.quote == row.quote;
    // A list that starts past 1 would be read as more of the paragraph above it.
    let below = rows.get(at + 1).filter(|below| text_line(below));
    if below.and_then(|below| below.marker.as_ref()).and_then(|marker| marker.number).is_some_and(|number| number != 1) {
        change.text.insert(row.end, '\n');
    }
    if at > 0 && text_line(&rows[at - 1]) {
        change.text.insert(row.start, '\n');
        caret += 1;
    }
    change.selection = caret..caret;
    change
}

/// Takes away the fences of a code block whose only line, `line`, is empty.
fn remove_empty_block(doc: &Doc<'_>, line: usize) -> Option<Typed> {
    let (first, last) = fence_lines(doc).find(|&(first, last)| first + 1 == line && line + 1 == last)?;
    let own = &doc.lines[line];
    if doc.text[own.content..own.content_end.max(own.content)].trim() != "" {
        return None;
    }
    let start = doc.lines[first].start;
    let mut new = doc.text.to_owned();
    new.replace_range(own.end..doc.lines[last].end, "");
    new.replace_range(start..own.start, "");
    Some(caret_at(new, start + own.end - own.start))
}

/// Backspace, or with `word` Ctrl+Backspace. `None` if it does nothing.
pub fn backspace(doc: &Doc<'_>, selection: Range<usize>, word: bool, carry: Option<&Carry>) -> Option<Typed> {
    if !selection.is_empty() {
        return Some(replace(doc, selection, ""));
    }
    let caret = selection.start;
    // Taking back the space after formatting puts the caret back inside it.
    if let Some(carry) = carry.filter(|carry| !word && carry.holds(doc.text, &selection)) {
        let blank = doc.text[carry.close.end..caret].chars().next_back()?;
        let mut new = doc.text.to_owned();
        new.remove(caret - blank.len_utf8());
        let caret = caret - blank.len_utf8();
        if caret == carry.close.end {
            return Some(caret_at(new, carry.close.start));
        }
        let carry = Carry { text: new.clone(), caret, close: carry.close.clone() };
        return Some(Typed { carry: Some(carry), ..caret_at(new, caret) });
    }
    // Only a rule of "*" or "_" just typed has the caret on it. Its last mark comes off.
    let line = doc.line_of(caret);
    if rules(doc).any(|rule| rule == line) && caret > doc.lines[line].start {
        let mut new = doc.text.to_owned();
        new.remove(caret - 1);
        return Some(caret_at(new, caret - 1));
    }
    if let Some(done) = remove_empty_block(doc, line) {
        return Some(done);
    }
    if at_block_start(doc, caret) {
        return unformat(doc, caret);
    }
    let target = if word { doc.word(caret, false) } else { doc.previous(caret) }?;
    Some(replace(doc, target, ""))
}

/// Delete, or with `word` Ctrl+Delete. `None` if it does nothing.
pub fn delete(doc: &Doc<'_>, selection: Range<usize>, word: bool) -> Option<Typed> {
    if !selection.is_empty() {
        return Some(replace(doc, selection, ""));
    }
    let target = if word { doc.word(selection.start, true) } else { doc.next(selection.start) }?;
    let mut done = replace(doc, target, "");
    // The caret stays where it was, if nothing ahead of it went.
    if done.change.text.get(..selection.start) == doc.text.get(..selection.start) && done.carry.is_none() {
        done.change.selection = selection.start..selection.start;
        past_marks(&mut done);
    }
    Some(done)
}

/// Enter over `selection`. In a list it starts the next item, and on an empty item it ends the
/// list, or takes a nested item one level out.
pub fn enter(doc: &Doc<'_>, selection: Range<usize>) -> Typed {
    no_new_underline(doc.markup, broken_at(doc, selection))
}

fn broken_at(doc: &Doc<'_>, selection: Range<usize>) -> Typed {
    if !selection.is_empty() {
        let cleared = replace(doc, selection, "").change;
        let markup = Markup::parse(&cleared.text);
        return broken_at(&Doc::new(&cleared.text, &markup), cleared.selection);
    }
    let (text, mut at) = (doc.text, selection.start);
    let line = doc.line_of(at);
    let row = &doc.lines[line];
    let (start, end) = doc.bounds[line];
    let item = row.marker.as_ref().filter(|_| row.kind == Kind::Text && start > row.start);
    let empty = doc.visible(start..end.max(start)).next().is_none();
    if item.is_some() && empty {
        let outdented = (row.indent > 0).then(|| formatting::apply(text, doc.markup, at..at, Action::Outdent)).flatten();
        if let Some(done) = outdented.map(|change| Typed { change, carry: None }).or_else(|| unformat(doc, at)) {
            return done;
        }
    }

    // The break goes outside the markers it is next to, and never inside a cue.
    let spans = spans(doc.markup);
    while let Some((_, _, close)) = spans.iter().find(|(_, _, close)| close.start == at && !close.is_empty()) {
        at = close.end;
    }
    while let Some((_, open, _)) = spans.iter().find(|(_, open, _)| open.end == at) {
        at = open.start;
    }
    if let Some((_, _, close)) = spans.iter().find(|(held, open, close)| matches!(held, Held::Code | Held::Whole) && open.start < at && at < close.end) {
        at = close.end;
    }

    let underlined = doc.underline.get(line + 1) == Some(&true);
    let heading = row.heading.is_some() && (start > row.start || underlined);
    let broken = |at: usize, put: &str, caret: usize| {
        let mut new = text.to_owned();
        new.insert_str(at, put);
        caret_at(new, caret)
    };
    let quote = &text[row.start..row.quote_end];
    let next_prefix = item.map(|marker| {
        let word = match marker.number {
            Some(number) => format!("{}{}", number.saturating_add(1), &text[marker.digits.end..marker.digits.end + 1]),
            None => text[row.indent_end..row.indent_end + marker.width].to_owned(),
        };
        format!("{}{word} ", &text[row.start..row.indent_end])
    });
    if (item.is_some() || heading) && !empty && doc.visible(start..at.max(start)).next().is_none() {
        // At the start of its text, the new line goes above, and the text keeps its marks.
        let above = match item {
            Some(marker) => {
                let end = row.indent_end + marker.width;
                format!("{} \n", &text[row.start..end])
            }
            None => format!("{}\n", quote.trim_end()),
        };
        return broken(row.start, &above, at + above.len());
    }
    if heading && doc.visible(at..doc.lines[line].end.max(at)).next().is_none() {
        // After the whole heading, with its closing marks or underline.
        let end = if underlined { doc.lines[line + 1].end } else { doc.lines[line].end };
        let put = format!("\n{quote}");
        return broken(end, &put, end + put.len());
    }
    let prefix = match next_prefix {
        Some(prefix) => prefix,
        None if heading && !underlined => quote.to_owned(),
        // In a paragraph, formatting goes on across the break.
        None => return broken(at, "\n", at + 1),
    };
    // Each line gets its own pair of the markers around the break.
    let blanks = |c: char| c == ' ' || c == '\t';
    let head = text[..at].trim_end_matches(blanks).len().max(start.min(at));
    let tail = at + text[at..].len() - text[at..].trim_start_matches(blanks).len();
    let mut around: Vec<_> = spans.iter().filter(|(kind, open, close)| matches!(kind, Held::Format(_)) && open.end < at && at < close.start).collect();
    around.sort_by_key(|(_, open, _)| open.start);
    let closes: String = around.iter().rev().map(|(_, _, close)| &text[close.clone()]).collect();
    let opens: String = around.iter().map(|(_, open, _)| &text[open.clone()]).collect();
    let lead = format!("{}{closes}\n{prefix}{opens}", &text[..head]);
    let caret = lead.len();
    // A heading's closing marks would show on the line broken off it.
    let rest = if heading && end < row.end { format!("{}{}", &text[tail..end.max(tail)], &text[row.end..]) } else { text[tail..].to_owned() };
    caret_at(lead + &rest, caret)
}

#[cfg(test)]
mod tests;
