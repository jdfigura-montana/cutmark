//! Where the caret may be while Markdown syntax is hidden, and how it moves. It moves from
//! one visible character to the next and never rests inside hidden syntax, or ahead of a
//! line's list, quote or heading marks. All positions are bytes of the text.

use std::ops::Range;

use egui::text::{CCursor, CharIndex};
use egui::text_selection::text_cursor_state::{ccursor_next_word, ccursor_previous_word};

use crate::lines::{self, Line};
use crate::markup::{Edge, Markup};

/// Which way to leave a place the caret may not be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bias {
    Back,
    Forward,
}

/// A script with what of it is hidden.
pub struct Doc<'a> {
    pub text: &'a str,
    pub markup: &'a Markup,
    pub lines: Vec<Line>,
    /// One flag for each byte.
    pub hidden: Vec<bool>,
    /// For each line, the first and last place the caret may be on it.
    pub bounds: Vec<(usize, usize)>,
    /// The lines that are the underline of a heading.
    pub underline: Vec<bool>,
    /// The lines the caret is never on: those, horizontal rules and code fences.
    pub skipped: Vec<bool>,
}

impl<'a> Doc<'a> {
    /// `markup` is the parse of `text`.
    pub fn new(text: &'a str, markup: &'a Markup) -> Self {
        let lines = lines::parse(text);
        let mut hidden = vec![false; text.len()];
        for run in markup.runs.iter().filter(|run| run.style.marker) {
            hidden[run.range.clone()].fill(true);
        }
        for (at, _) in text.match_indices("\r\n") {
            hidden[at] = true;
        }
        let mut bounds: Vec<(usize, usize)> = lines.iter().map(|line| (line.start, line.end)).collect();
        let mut underline = vec![false; lines.len()];
        let mut skipped = vec![false; lines.len()];
        for (range, edge) in &markup.edges {
            let line = lines::at(&lines, range.start);
            match edge {
                // One line break shows between the lines around a skipped one.
                Edge::Rule => {
                    skipped[line] = true;
                    // The break after the rule is the one that shows. On the last line, it
                    // is the one ahead.
                    if line > 0 && line + 1 < lines.len() {
                        hidden[lines[line - 1].end..lines[line].start].fill(true);
                    }
                }
                // As with an underline, the break ahead of a fence goes unseen. A first
                // line has none, so the one after it does.
                Edge::FenceOpen | Edge::FenceClose => {
                    skipped[line] = true;
                    let unseen = match line.checked_sub(1) {
                        Some(above) => lines[above].end..lines[line].end,
                        None => lines[line].start..lines.get(1).map_or(lines[line].end, |next| next.start),
                    };
                    hidden[unseen].fill(true);
                }
                Edge::Prefix => bounds[line].0 = bounds[line].0.max(range.end),
                Edge::Suffix => bounds[line].1 = bounds[line].1.min(range.start),
                Edge::Underline if line > 0 => {
                    (underline[line], skipped[line]) = (true, true);
                    // The line break ahead of it goes unseen too, so that one break shows.
                    hidden[lines[line - 1].end..lines[line].end].fill(true);
                }
                Edge::Underline => {}
            }
        }
        for bound in &mut bounds {
            bound.1 = bound.1.max(bound.0);
        }
        Self { text, markup, lines, hidden, bounds, underline, skipped }
    }

    pub fn line_of(&self, at: usize) -> usize {
        lines::at(&self.lines, at)
    }

    /// The nearest place to `at` that the caret may be.
    pub fn rest(&self, at: usize, bias: Bias) -> usize {
        let line = self.line_of(at);
        if self.skipped[line] {
            let before = (0..line).rev().find(|&line| !self.skipped[line]);
            let after = (line + 1..self.lines.len()).find(|&line| !self.skipped[line]);
            return match (bias, before, after) {
                (Bias::Forward, _, Some(after)) | (Bias::Back, None, Some(after)) => self.rest(self.bounds[after].0, bias),
                (_, Some(before), _) => self.rest(self.bounds[before].1, Bias::Back),
                // There is no other line to be on.
                (_, None, None) => self.bounds[line].1,
            };
        }
        let (start, end) = self.bounds[line];
        let mut at = at.clamp(start, end);
        while !self.text.is_char_boundary(at) {
            at -= 1;
        }
        let (first, last) = self.cluster(at);
        match bias {
            _ if at == first || at == last => at,
            Bias::Back => first,
            Bias::Forward => last,
        }
    }

    /// The run of hidden syntax around `at`, which is on a line the caret may be on: the
    /// places before and after it. They look the same on screen. With no hidden syntax
    /// there, both are `at`.
    pub fn cluster(&self, at: usize) -> (usize, usize) {
        let (start, end) = self.bounds[self.line_of(at)];
        let (mut first, mut last) = (at, at);
        while first > start && self.hidden[first - 1] {
            first -= 1;
        }
        while last < end && self.hidden[last] {
            last += 1;
        }
        (first, last)
    }

    /// The visible characters that start in `range`.
    pub fn visible(&self, range: Range<usize>) -> impl DoubleEndedIterator<Item = Range<usize>> + '_ {
        let from = range.start;
        self.text[range].char_indices().map(move |(at, c)| from + at..from + at + c.len_utf8()).filter(|found| !self.hidden[found.start])
    }

    pub fn previous(&self, at: usize) -> Option<Range<usize>> {
        self.visible(0..at).next_back()
    }

    pub fn next(&self, at: usize) -> Option<Range<usize>> {
        self.visible(at..self.text.len()).next()
    }

    /// One visible character back. The caret ends up just ahead of the character it passed,
    /// inside whatever formatting that one has.
    pub fn left(&self, at: usize) -> usize {
        match self.previous(at) {
            Some(passed) => self.rest(passed.start, Bias::Back),
            None => self.rest(0, Bias::Forward),
        }
    }

    /// One visible character on, to just after the character passed.
    pub fn right(&self, at: usize) -> usize {
        match self.next(at) {
            Some(passed) => self.rest(passed.end, Bias::Back),
            None => self.rest(self.text.len(), Bias::Back),
        }
    }

    /// The visible characters from `at` to where the next word ends, or from where the
    /// previous one starts up to `at`. `None` at the end of the text that way.
    pub fn word(&self, at: usize, forward: bool) -> Option<Range<usize>> {
        let all: Vec<Range<usize>> = self.visible(0..self.text.len()).collect();
        let shown: String = all.iter().map(|found| &self.text[found.clone()]).collect();
        let here = all.partition_point(|found| found.start < at);
        let cursor = CCursor::new(CharIndex(here));
        let there = if forward { ccursor_next_word(&shown, cursor) } else { ccursor_previous_word(&shown, cursor) }.index.0.min(all.len());
        let (first, last) = if forward { (here, there) } else { (there, here) };
        (first < last).then(|| all[first].start..all[last - 1].end)
    }

    pub fn word_left(&self, at: usize) -> usize {
        match self.word(at, false) {
            Some(passed) => self.rest(passed.start, Bias::Back),
            None => self.rest(0, Bias::Forward),
        }
    }

    pub fn word_right(&self, at: usize) -> usize {
        match self.word(at, true) {
            Some(passed) => self.rest(passed.end, Bias::Back),
            None => self.rest(self.text.len(), Bias::Back),
        }
    }
}

#[cfg(test)]
mod tests;
