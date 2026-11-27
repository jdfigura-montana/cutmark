//! The formatting actions of the toolbar and the Format menu. Each takes the text and a
//! selection and gives back the new text and selection, so that it is one change to the
//! script. Ranges are byte ranges.

use std::ops::Range;

use crate::lines::{self, Kind, Line};
use crate::markup::{CueKind, CueMarkup, Inline, InlineSpan, Markup};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockStyle {
    #[default]
    Paragraph,
    /// Level 1 to 6. The menus offer 1 to 3.
    Heading(u8),
    Quote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    Bulleted,
    Numbered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Style(BlockStyle),
    Inline(Inline),
    List(ListKind),
    Outdent,
    Indent,
    Rule,
    Cue(CueKind),
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Style(BlockStyle::Paragraph) => "Paragraph",
            Self::Style(BlockStyle::Heading(1)) => "Heading 1",
            Self::Style(BlockStyle::Heading(2)) => "Heading 2",
            Self::Style(BlockStyle::Heading(3)) => "Heading 3",
            Self::Style(BlockStyle::Heading(4)) => "Heading 4",
            Self::Style(BlockStyle::Heading(5)) => "Heading 5",
            Self::Style(BlockStyle::Heading(_)) => "Heading 6",
            Self::Style(BlockStyle::Quote) => "Quote",
            Self::Inline(Inline::Bold) => "Bold",
            Self::Inline(Inline::Italic) => "Italic",
            Self::Inline(Inline::Underline) => "Underline",
            Self::Inline(Inline::Strike) => "Strikethrough",
            Self::List(ListKind::Bulleted) => "Bulleted list",
            Self::List(ListKind::Numbered) => "Numbered list",
            Self::Outdent => "Outdent",
            Self::Indent => "Indent",
            Self::Rule => "Horizontal rule",
            Self::Cue(CueKind::Note) => "Insert note cue",
            Self::Cue(CueKind::Pause) => "Insert pause cue",
            Self::Cue(CueKind::Mark) => "Insert mark cue",
            Self::Cue(CueKind::End) => "Insert end cue",
        }
    }

    /// The shortcut shown beside the label, or "" for none.
    pub fn chord(self) -> &'static str {
        match self {
            Self::Style(BlockStyle::Paragraph) => "Ctrl+0",
            Self::Style(BlockStyle::Heading(1)) => "Ctrl+1",
            Self::Style(BlockStyle::Heading(2)) => "Ctrl+2",
            Self::Style(BlockStyle::Heading(3)) => "Ctrl+3",
            Self::Style(BlockStyle::Quote) => "Ctrl+Shift+.",
            Self::Inline(Inline::Bold) => "Ctrl+B",
            Self::Inline(Inline::Italic) => "Ctrl+I",
            Self::Inline(Inline::Underline) => "Ctrl+U",
            Self::Inline(Inline::Strike) => "Ctrl+Shift+X",
            Self::List(ListKind::Bulleted) => "Ctrl+Shift+8",
            Self::List(ListKind::Numbered) => "Ctrl+Shift+7",
            Self::Outdent => "Ctrl+[",
            Self::Indent => "Ctrl+]",
            Self::Style(BlockStyle::Heading(_)) | Self::Rule | Self::Cue(_) => "",
        }
    }
}

/// The formatting the caret or selection is in, and what can be done to it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Active {
    /// The caret's line.
    pub style: BlockStyle,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// The list the caret's line is an item of.
    pub list: Option<ListKind>,
    pub can_indent: bool,
    pub can_outdent: bool,
}

impl Active {
    /// Whether the control for `action` shows as on.
    pub fn has(self, action: Action) -> bool {
        match action {
            Action::Style(style) => self.style == style,
            Action::Inline(Inline::Bold) => self.bold,
            Action::Inline(Inline::Italic) => self.italic,
            Action::Inline(Inline::Underline) => self.underline,
            Action::Inline(Inline::Strike) => self.strike,
            Action::List(kind) => self.list == Some(kind),
            Action::Outdent | Action::Indent | Action::Rule | Action::Cue(_) => false,
        }
    }
}

/// The text after an action, and what is selected in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub text: String,
    pub selection: Range<usize>,
}

/// Replaces `cut` bytes at `at` with `put`.
#[derive(Debug, Clone)]
struct Splice {
    at: usize,
    cut: usize,
    put: String,
}

impl Splice {
    fn new(at: usize, cut: usize, put: impl Into<String>) -> Self {
        Self { at, cut, put: put.into() }
    }

    fn insert(at: usize, put: impl Into<String>) -> Self {
        Self::new(at, 0, put)
    }
}

/// Splices of one text, which do not overlap.
#[derive(Debug, Default)]
struct Splices(Vec<Splice>);

impl Splices {
    fn push(&mut self, splice: Splice) {
        if splice.cut > 0 || !splice.put.is_empty() {
            self.0.push(splice);
        }
    }

    /// Insertions go ahead of a cut at the same place. Otherwise the order given is kept.
    fn sort(&mut self) {
        self.0.sort_by_key(|splice| (splice.at, splice.cut > 0));
    }

    fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut from = 0;
        for splice in &self.0 {
            out.push_str(&text[from..splice.at.max(from)]);
            out.push_str(&splice.put);
            from = (splice.at + splice.cut).max(from);
        }
        out.push_str(&text[from..]);
        out
    }

    /// Where position `at` of the old text is in the new one. With `after`, what is put in at
    /// the position goes before it.
    fn map(&self, at: usize, after: bool) -> usize {
        let (mut added, mut removed) = (0, 0);
        for splice in &self.0 {
            if at < splice.at {
                break;
            }
            if splice.cut == 0 {
                if at > splice.at || after {
                    added += splice.put.len();
                }
            } else if at >= splice.at + splice.cut {
                added += splice.put.len();
                removed += splice.cut;
            } else {
                // Inside what is cut.
                let start = splice.at + added - removed;
                return if after { start + splice.put.len() } else { start };
            }
        }
        at + added - removed
    }

    /// The change to `text`, with `selection` carried over. A selection grows inwards from
    /// what is added at its ends. `None` if nothing changes.
    fn change(mut self, text: &str, selection: &Range<usize>) -> Option<Change> {
        self.sort();
        let new = self.apply(text);
        if new == text {
            return None;
        }
        let start = self.map(selection.start, true);
        let end = if selection.is_empty() { start } else { self.map(selection.end, false).max(start) };
        Some(Change { text: new, selection: start..end })
    }
}

/// What `action` does to `text`, which `markup` is the parse of, with `selection` selected.
/// `None` if it does nothing.
pub fn apply(text: &str, markup: &Markup, selection: Range<usize>, action: Action) -> Option<Change> {
    let lines = lines::parse(text);
    match action {
        Action::Style(style) => set_style(text, &lines, selection, style),
        Action::List(kind) => toggle_list(text, &lines, selection, kind),
        Action::Inline(kind) => toggle_inline(text, markup, &lines, selection, kind),
        Action::Indent => nest(text, &lines, &selection, true),
        Action::Outdent => nest(text, &lines, &selection, false),
        Action::Rule => toggle_rule(text, &lines, &selection),
        Action::Cue(kind) => toggle_cue(text, markup, selection, kind),
    }
}

/// The formatting at `selection`, whose cursor is at `caret`.
pub fn active(text: &str, markup: &Markup, selection: Range<usize>, caret: usize) -> Active {
    let lines = lines::parse(text);
    let line = lines::at(&lines, caret);
    let titled = lines::title_of(&lines, line);
    let segments = segments(text, &lines, &selection);
    let on = |kind: Inline| {
        if selection.is_empty() {
            markup.spans.iter().any(|span| span.kind == kind && (span.open.end..=span.close.start).contains(&caret))
        } else {
            formatted(text, markup, &segments, kind)
        }
    };
    Active {
        style: style_of(&lines[titled]),
        bold: on(Inline::Bold),
        italic: on(Inline::Italic),
        underline: on(Inline::Underline),
        strike: on(Inline::Strike),
        list: lines::item_of(&lines, line).and_then(|item| lines[item].marker.as_ref()).map(|marker| kind_of(marker.number)),
        can_indent: nest(text, &lines, &selection, true).is_some(),
        can_outdent: nest(text, &lines, &selection, false).is_some(),
    }
}

fn kind_of(number: Option<u64>) -> ListKind {
    if number.is_some() { ListKind::Numbered } else { ListKind::Bulleted }
}

fn style_of(line: &Line) -> BlockStyle {
    match line.heading {
        Some(level) => BlockStyle::Heading(level),
        None if line.quote > 0 => BlockStyle::Quote,
        None => BlockStyle::Paragraph,
    }
}

/// The lines a selection is on. One that ends at the start of a line is not on it.
fn selected_lines(lines: &[Line], selection: &Range<usize>) -> Range<usize> {
    let first = lines::at(lines, selection.start);
    let mut last = lines::at(lines, selection.end);
    if !selection.is_empty() && last > first && selection.end == lines[last].start {
        last -= 1;
    }
    first..last + 1
}

/// The selected lines whose block can change: those with text, or the one line the caret is
/// on if it is empty.
fn block_targets(lines: &[Line], selection: &Range<usize>) -> Vec<usize> {
    let selected = selected_lines(lines, selection);
    if selected.len() == 1 {
        let line = lines::title_of(lines, selected.start);
        return if matches!(lines[line].kind, Kind::Text | Kind::Blank) { vec![line] } else { Vec::new() };
    }
    selected.filter(|&line| lines[line].kind == Kind::Text).collect()
}

/// A line as a block change leaves it, as far as the lines around it care.
#[derive(Debug, Clone, Copy)]
struct Shape {
    kind: Kind,
    quote: usize,
    indent: usize,
    heading: bool,
    /// `Some` for a list item, with its number if it has one.
    item: Option<Option<u64>>,
    empty: bool,
}

impl Shape {
    fn of(line: &Line) -> Self {
        Self {
            kind: line.kind,
            quote: line.quote,
            indent: line.indent,
            heading: line.heading.is_some(),
            item: line.marker.as_ref().map(|marker| marker.number),
            empty: line.content >= line.content_end,
        }
    }

    /// True if `next` needs an empty line between it and this one, or Markdown would read it
    /// as part of this line's block.
    fn swallows(self, next: Self) -> bool {
        if self.kind != Kind::Text || self.heading {
            return false;
        }
        let paragraph = self.quote == 0 && self.item.is_none();
        match next.kind {
            // "---" under a paragraph makes it a heading.
            Kind::Rule => paragraph,
            Kind::Text if next.heading => false,
            Kind::Text => match next.item {
                // An empty item, or a list that starts past 1, cannot cut a paragraph short.
                Some(number) => paragraph && (next.empty || number.is_some_and(|number| number != 1)),
                // A plain line carries on a quote or a list item, unless it is indented under
                // the item on purpose.
                None => next.indent == 0 && (self.quote > next.quote || (self.quote == next.quote && self.item.is_some())),
            },
            Kind::Blank | Kind::Underline | Kind::Fence => false,
        }
    }
}

/// A line's new start.
struct Rewrite {
    line: usize,
    prefix: String,
    shape: Shape,
}

/// Puts each rewritten line's new prefix in place of everything before its text, and adds the
/// empty lines that keep it a block of its own.
fn rewrite(text: &str, lines: &[Line], selection: &Range<usize>, rewrites: Vec<Rewrite>) -> Option<Change> {
    let mut shapes: Vec<Shape> = lines.iter().map(Shape::of).collect();
    let mut changed = vec![false; lines.len()];
    let mut splices = Splices::default();
    let mut gone = vec![false; lines.len()];
    for Rewrite { line, prefix, shape } in rewrites {
        let old = &lines[line];
        let underlined = lines.get(line + 1).is_some_and(|under| under.kind == Kind::Underline);
        if text[old.start..old.content] == prefix && !underlined {
            continue;
        }
        changed[line] = true;
        shapes[line] = shape;
        splices.push(Splice::new(old.start, old.content - old.start, prefix));
        // A heading's closing "#" run, or the underline of a heading of the other form.
        if old.heading.is_some() {
            splices.push(Splice::new(old.content_end, old.end - old.content_end, ""));
            if let Some(under) = lines.get(line + 1).filter(|under| under.kind == Kind::Underline) {
                splices.push(Splice::new(old.end, under.end - old.end, ""));
                gone[line + 1] = true;
            }
        }
    }
    let kept: Vec<usize> = (0..lines.len()).filter(|&line| !gone[line]).collect();
    let mut gaps = Vec::new();
    for pair in kept.windows(2) {
        let (before, after) = (pair[0], pair[1]);
        if (changed[before] || changed[after]) && shapes[before].swallows(shapes[after]) {
            // Inside a quote, the empty line is part of it.
            let quoted = if shapes[before].quote == shapes[after].quote { shapes[after].quote } else { 0 };
            gaps.push(Splice::insert(lines[after].start, format!("{}\n", ">".repeat(quoted))));
        }
    }
    // Ahead of a new prefix that goes in at the same place.
    splices.0.splice(0..0, gaps);
    splices.change(text, selection)
}

/// Sets the block style of the selected lines. A style they all have already is taken off.
fn set_style(text: &str, lines: &[Line], selection: Range<usize>, style: BlockStyle) -> Option<Change> {
    let targets = block_targets(lines, &selection);
    let all_have = targets.iter().all(|&line| style_of(&lines[line]) == style);
    let style = if all_have { BlockStyle::Paragraph } else { style };
    let rewrites = targets
        .into_iter()
        .filter(|&line| style != BlockStyle::Paragraph || style_of(&lines[line]) != style || lines[line].marker.is_some())
        .map(|line| {
            let (prefix, quote, heading) = match style {
                BlockStyle::Paragraph => (String::new(), 0, false),
                BlockStyle::Heading(level) => (format!("{} ", "#".repeat(usize::from(level))), 0, true),
                BlockStyle::Quote => ("> ".to_owned(), 1, false),
            };
            let empty = lines[line].content >= lines[line].content_end;
            Rewrite { line, prefix, shape: Shape { kind: Kind::Text, quote, indent: 0, heading, item: None, empty } }
        })
        .collect();
    rewrite(text, lines, &selection, rewrites)
}

/// Makes the selected lines items of a `kind` list. If they all are, they become text.
fn toggle_list(text: &str, lines: &[Line], selection: Range<usize>, kind: ListKind) -> Option<Change> {
    let targets = block_targets(lines, &selection);
    let is_kind = |line: usize| lines[line].marker.as_ref().is_some_and(|marker| kind_of(marker.number) == kind);
    let remove = targets.iter().all(|&line| is_kind(line));
    // The last number given at each indent.
    let mut numbers: Vec<(usize, u64)> = Vec::new();
    let rewrites = targets
        .iter()
        .map(|&line| {
            let old = &lines[line];
            let quote = &text[old.start..old.quote_end];
            let indent = if !remove && old.marker.is_some() { old.indent } else { 0 };
            let number = (!remove && kind == ListKind::Numbered).then(|| {
                numbers.retain(|&(at, _)| at <= indent);
                match numbers.last_mut().filter(|(at, _)| *at == indent) {
                    Some((_, last)) => {
                        *last += 1;
                        *last
                    }
                    None => {
                        // Counting goes on from a numbered item just above.
                        let above = lines::previous_item(lines, line, indent).and_then(|item| {
                            let marker = lines[item].marker.as_ref().filter(|_| lines[item].indent == indent)?;
                            marker.number
                        });
                        let number = above.map_or(1, |above| above.saturating_add(1));
                        numbers.push((indent, number));
                        number
                    }
                }
            });
            let marker = match number {
                _ if remove => String::new(),
                Some(number) => format!("{}{number}. ", " ".repeat(indent)),
                None => format!("{}- ", " ".repeat(indent)),
            };
            let empty = old.content >= old.content_end;
            let item = (!remove).then_some(number);
            let shape = Shape { kind: if remove && empty { Kind::Blank } else { Kind::Text }, quote: old.quote, indent, heading: false, item, empty };
            Rewrite { line, prefix: format!("{quote}{marker}"), shape }
        })
        .collect();
    rewrite(text, lines, &selection, rewrites)
}

/// Moves the selected list items, with the items under them, one level in or out.
fn nest(text: &str, lines: &[Line], selection: &Range<usize>, inward: bool) -> Option<Change> {
    let mut indents: Vec<usize> = lines.iter().map(|line| line.indent).collect();
    let mut moved = vec![false; lines.len()];
    // Each moved item that was selected itself, or heads the selected ones under it: its old
    // indent and how far it went.
    let mut roots: Vec<(usize, usize, isize)> = Vec::new();
    let selected = selected_lines(lines, selection);
    let first = selected.start;
    for line in selected {
        // A line that carries on an item stands for it, if the selection starts there.
        let item = if line == first { lines::item_of(lines, line) } else { lines[line].marker.is_some().then_some(line) };
        let Some(item) = item.filter(|&item| !moved[item]) else {
            continue;
        };
        let indent = lines[item].indent;
        let delta = if inward {
            lines::previous_item(lines, item, indent).and_then(|above| {
                if lines[above].indent != indent {
                    return None;
                }
                // The item above went in too, so this one goes as far.
                let went = roots.iter().find(|&&(root, old, _)| root == above && old == indent).map(|&(_, _, delta)| delta);
                went.or_else(|| {
                    let marker = lines[above].marker.as_ref()?;
                    // Under where the text of the item above starts.
                    isize::try_from(marker.width + 1).ok()
                })
            })
        } else if indent == 0 {
            None
        } else {
            let parent = indent.checked_sub(1).and_then(|less| lines::previous_item(lines, item, less));
            let target = parent.map_or(0, |parent| lines[parent].indent);
            isize::try_from(indent - target).ok().map(|back| -back)
        };
        let Some(delta) = delta else {
            continue;
        };
        roots.push((item, indent, delta));
        for under in lines::subtree(lines, item) {
            if lines[under].kind != Kind::Blank && !moved[under] {
                moved[under] = true;
                indents[under] = indents[under].saturating_add_signed(delta);
            }
        }
    }
    if roots.is_empty() {
        return None;
    }

    // Numbered items are numbered for their new place. A list that starts under another
    // item's text has to start at 1.
    let mut renumbered = lines.to_vec();
    for (line, indent) in renumbered.iter_mut().zip(&indents) {
        line.indent = *indent;
    }
    let mut splices = Splices::default();
    let start = roots[0].0;
    let end = lines::list_end(lines, start);
    for line in start..end {
        let Some(number) = renumbered[line].marker.as_ref().and_then(|marker| marker.number) else {
            continue;
        };
        let indent = renumbered[line].indent;
        let sibling = lines::previous_item(&renumbered, line, indent).filter(|&above| renumbered[above].indent == indent);
        let is_root = roots.iter().any(|&(root, _, _)| root == line);
        let new = match sibling {
            Some(above) if is_root => renumbered[above].marker.as_ref().and_then(|marker| marker.number).map_or(1, |above| above.saturating_add(1)),
            Some(_) => number,
            None => 1,
        };
        if let Some(marker) = renumbered[line].marker.as_mut().filter(|_| new != number) {
            marker.number = Some(new);
            let digits = marker.digits.clone();
            splices.push(Splice::new(digits.start, digits.len(), new.to_string()));
        }
    }
    for (line, old) in lines.iter().enumerate() {
        if moved[line] {
            splices.push(Splice::new(old.quote_end, old.indent_end - old.quote_end, " ".repeat(indents[line])));
        }
    }
    splices.change(text, selection)
}

/// Puts a horizontal rule after the selection's last line, or in place of it if it is empty.
/// On a rule, removes it.
fn toggle_rule(text: &str, lines: &[Line], selection: &Range<usize>) -> Option<Change> {
    let mut at = selected_lines(lines, selection).end - 1;
    let line = &lines[at];
    if line.kind == Kind::Rule {
        let next = lines.get(at + 1).map_or(line.end, |next| next.start);
        let mut new = text.to_owned();
        new.replace_range(line.start..next, "");
        return Some(Change { selection: line.start..line.start, text: new });
    }
    let (from, to, insert) = if line.kind == Kind::Blank {
        // Right under text, "---" would make that text a heading.
        let under_text = at > 0 && lines[at - 1].kind == Kind::Text;
        (line.start, line.end, if under_text { "\n---" } else { "---" })
    } else {
        // A heading's underline stays with it.
        if lines.get(at + 1).is_some_and(|under| under.kind == Kind::Underline) {
            at += 1;
        }
        (lines[at].end, lines[at].end, "\n\n---")
    };
    let mut new = text.to_owned();
    new.replace_range(from..to, insert);
    let rule_end = from + insert.len();
    // The caret goes to the line after the rule, which is made if there is none.
    let caret = match new[rule_end..].find('\n') {
        Some(newline) => rule_end + newline + 1,
        None => {
            new.push('\n');
            new.len()
        }
    };
    Some(Change { text: new, selection: caret..caret })
}

/// The part of each selected line that formatting can go around, without the blanks at its
/// ends. With nothing selected, that is the whole of the caret's line.
fn segments(text: &str, lines: &[Line], selection: &Range<usize>) -> Vec<Range<usize>> {
    selected_lines(lines, selection)
        .filter(|&line| lines[line].kind == Kind::Text)
        .filter_map(|line| {
            let Line { content, content_end, .. } = lines[line];
            let part = if selection.is_empty() { content..content_end } else { content.max(selection.start)..content_end.min(selection.end) };
            trimmed(text, part)
        })
        .collect()
}

/// `range` without the blanks at its ends, or `None` if nothing is left.
fn trimmed(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let part = text.get(range.clone())?;
    let start = range.start + (part.len() - part.trim_start().len());
    let end = range.start + part.trim_end().len();
    (start < end).then_some(start..end)
}

/// True if all the text of `segments` has the format.
fn formatted(text: &str, markup: &Markup, segments: &[Range<usize>], kind: Inline) -> bool {
    let mut any = false;
    for segment in segments {
        let first = markup.runs.partition_point(|run| run.range.end <= segment.start);
        for run in markup.runs[first..].iter().take_while(|run| run.range.start < segment.end) {
            let part = run.range.start.max(segment.start)..run.range.end.min(segment.end);
            if run.style.marker || text[part].trim().is_empty() {
                continue;
            }
            let has = match kind {
                Inline::Bold => run.style.bold,
                Inline::Italic => run.style.italic,
                Inline::Underline => run.style.underline,
                Inline::Strike => run.style.strike,
            };
            if !has {
                return false;
            }
            any = true;
        }
    }
    any
}

/// The markers that formatting is added with.
fn markers(kind: Inline) -> (&'static str, &'static str) {
    match kind {
        Inline::Bold => ("**", "**"),
        Inline::Italic => ("*", "*"),
        Inline::Underline => ("<u>", "</u>"),
        Inline::Strike => ("~~", "~~"),
    }
}

fn toggle_inline(text: &str, markup: &Markup, lines: &[Line], selection: Range<usize>, kind: Inline) -> Option<Change> {
    if selection.is_empty()
        && let Some(change) = remove_empty_pair(text, markup, selection.start, kind)
    {
        return Some(change);
    }
    let segments = segments(text, lines, &selection);
    if segments.is_empty() {
        return selection.is_empty().then(|| insert_pair(text, selection.start, kind));
    }
    let spans = markup.spans.iter().filter(|span| span.kind == kind);
    let (open, close) = markers(kind);
    let mut splices = Splices::default();
    let remove = |splices: &mut Splices, span: &InlineSpan| {
        splices.push(Splice::new(span.open.start, span.open.len(), ""));
        splices.push(Splice::new(span.close.start, span.close.len(), ""));
    };
    if formatted(text, markup, &segments, kind) {
        for span in spans {
            let content = span.open.end..span.close.start;
            if !segments.iter().any(|segment| segment.start < content.end && content.start < segment.end) {
                continue;
            }
            remove(&mut splices, span);
            // What the span had outside the selection keeps the format.
            let mut from = content.start;
            for segment in segments.iter().chain(std::iter::once(&(content.end..content.end))) {
                let to = segment.start.clamp(from, content.end);
                if let Some(rest) = trimmed(text, from..to) {
                    splices.push(Splice::insert(rest.start, open));
                    splices.push(Splice::insert(rest.end, close));
                }
                from = segment.end.clamp(from, content.end);
            }
        }
    } else {
        for wrap in wraps(markup, segments, kind) {
            for span in spans.clone().filter(|span| wrap.start <= span.open.start && span.close.end <= wrap.end) {
                remove(&mut splices, span);
            }
            splices.push(Splice::insert(wrap.start, open));
            splices.push(Splice::insert(wrap.end, close));
        }
    }
    // With a caret at the end of the formatted text, typing goes on inside it.
    if selection.is_empty() {
        splices.sort();
        let new = splices.apply(text);
        let line = &lines[lines::at(lines, selection.start)];
        let caret = splices.map(selection.start, selection.start < line.content_end);
        return (new != text).then_some(Change { text: new, selection: caret..caret });
    }
    splices.change(text, &selection)
}

/// Takes away the pair of markers with nothing in it that the caret is between.
fn remove_empty_pair(text: &str, markup: &Markup, caret: usize, kind: Inline) -> Option<Change> {
    let (open, close) = markers(kind);
    let (before, after) = (&text[..caret], &text[caret..]);
    // Two asterisks a side are bold alone.
    let bold_alone = kind == Inline::Italic && (before.len() - before.trim_end_matches('*').len()) % 2 == 0;
    // Markers of formatted text are not an empty pair.
    let in_use = |at: usize| markup.spans.iter().any(|span| span.open.end < span.close.start && (span.open.contains(&at) || span.close.contains(&at)));
    if !before.ends_with(open) || !after.starts_with(close) || bold_alone || in_use(caret - 1) || in_use(caret) {
        return None;
    }
    let start = caret - open.len();
    let mut new = text.to_owned();
    new.replace_range(start..caret + close.len(), "");
    Some(Change { text: new, selection: start..start })
}

/// With no text to format, a pair of markers around the caret, to type in.
fn insert_pair(text: &str, caret: usize, kind: Inline) -> Change {
    let (open, close) = markers(kind);
    let mut new = text.to_owned();
    new.insert_str(caret, &format!("{open}{close}"));
    let caret = caret + open.len();
    Change { text: new, selection: caret..caret }
}

/// What to put a new pair of markers around, for `segments`. Each grows to take in the spans
/// of the same kind it overlaps or touches, and whatever it would otherwise cut across.
fn wraps(markup: &Markup, mut segments: Vec<Range<usize>>, kind: Inline) -> Vec<Range<usize>> {
    let solid: Vec<Range<usize>> = markup.solid.iter().cloned().chain(markup.cues.iter().map(|cue| cue.range.clone())).collect();
    for segment in &mut segments {
        loop {
            let before = segment.clone();
            for span in &markup.spans {
                let full = span.open.start..span.close.end;
                let content = span.open.end..span.close.start;
                if span.kind == kind {
                    if segment.start <= full.end && full.start <= segment.end {
                        cover(segment, &full);
                    }
                    continue;
                }
                let apart = segment.end <= full.start || full.end <= segment.start;
                let inside = content.start <= segment.start && segment.end <= content.end;
                let around = segment.start <= full.start && full.end <= segment.end;
                if apart || inside || around {
                    continue;
                }
                let shared = segment.start.max(content.start)..segment.end.min(content.end);
                if shared == content {
                    cover(segment, &full);
                } else if full.start <= segment.start && segment.end <= full.end {
                    // It starts or ends in the span's markers.
                    *segment = shared;
                } else if shared.is_empty() {
                    // Only markers are shared.
                    if segment.start < full.start {
                        segment.end = full.start;
                    } else {
                        segment.start = full.end;
                    }
                } else {
                    cover(segment, &full);
                }
            }
            for whole in &solid {
                let overlaps = segment.start < whole.end && whole.start < segment.end;
                if overlaps && !(segment.start <= whole.start && whole.end <= segment.end) {
                    cover(segment, whole);
                }
            }
            if *segment == before {
                break;
            }
        }
    }
    segments.retain(|segment| segment.start < segment.end);
    segments.sort_by_key(|segment| segment.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(segments.len());
    for segment in segments {
        match merged.last_mut() {
            Some(last) if segment.start <= last.end => last.end = last.end.max(segment.end),
            _ => merged.push(segment),
        }
    }
    merged
}

fn cover(range: &mut Range<usize>, other: &Range<usize>) {
    range.start = range.start.min(other.start);
    range.end = range.end.max(other.end);
}

fn cue_word(kind: CueKind) -> &'static str {
    match kind {
        CueKind::Note => "note",
        CueKind::Pause => "pause",
        CueKind::Mark => "mark",
        CueKind::End => "end",
    }
}

/// Inserts a cue. With the caret or selection on a cue of the kind, takes that cue away, and
/// leaves a note's or mark's words as text.
fn toggle_cue(text: &str, markup: &Markup, selection: Range<usize>, kind: CueKind) -> Option<Change> {
    let on = |cue: &CueMarkup| {
        if selection.is_empty() {
            cue.range.start < selection.start && selection.start < cue.range.end
        } else {
            cue.range.start < selection.end && selection.start < cue.range.end
        }
    };
    if let Some(cue) = markup.cues.iter().find(|cue| cue.kind == kind && on(cue)) {
        return Some(remove_cue(text, cue));
    }
    let labelled = matches!(kind, CueKind::Note | CueKind::Mark);
    let word = cue_word(kind);
    // A selection of words on one line becomes the label.
    if labelled && !markup.cues.iter().any(on) {
        let label = trimmed(text, selection.clone()).filter(|label| {
            let label = &text[label.clone()];
            !label.contains(['\n', '[', ']'])
        });
        if let Some(label) = label {
            let open = format!("[[{word}: ");
            let mut new = text.to_owned();
            new.insert_str(label.end, "]]");
            new.insert_str(label.start, &open);
            let selection = label.start + open.len()..label.end + open.len();
            return Some(Change { text: new, selection });
        }
    }
    // Cues do not nest.
    let mut at = markup.cues.iter().find(|cue| cue.range.start < selection.end && selection.end < cue.range.end).map_or(selection.end, |cue| cue.range.end);
    while !text.is_char_boundary(at) {
        at += 1;
    }
    let lead = if text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace()) { " " } else { "" };
    let tail = if text[at..].chars().next().is_some_and(|c| !c.is_whitespace()) { " " } else { "" };
    let cue = if labelled { format!("[[{word}: ]]") } else { format!("[[{word}]]") };
    // A note or mark has the caret where its words go.
    let caret = at + lead.len() + cue.len() - if labelled { 2 } else { 0 };
    let mut new = text.to_owned();
    new.insert_str(at, &format!("{lead}{cue}{tail}"));
    Some(Change { text: new, selection: caret..caret })
}

fn remove_cue(text: &str, cue: &CueMarkup) -> Change {
    let inner = &text[cue.range.start + 2..cue.range.end - 2];
    let label = match cue.kind {
        CueKind::Note | CueKind::Mark => inner.split_once(':').map_or("", |(_, label)| label.trim()),
        CueKind::Pause | CueKind::End => "",
    };
    let mut range = cue.range.clone();
    if label.is_empty() {
        // One of the spaces around it goes too.
        let spaced_before = range.start == 0 || text[..range.start].ends_with([' ', '\n']);
        if spaced_before && text[range.end..].starts_with(' ') {
            range.end += 1;
        } else if text[..range.start].ends_with(' ') {
            range.start -= 1;
        }
    }
    let mut new = text.to_owned();
    new.replace_range(range.clone(), label);
    Change { text: new, selection: range.start..range.start + label.len() }
}

#[cfg(test)]
mod tests;
