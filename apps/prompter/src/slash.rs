//! The "/" command menu. Typing "/" at the start of a line or after a space opens it at the
//! caret. What is typed after the "/" filters it, and picking a command takes the "/" and
//! what was typed back out of the script.

use std::ops::Range;

use egui::{Order, Pos2, Rect, vec2};
use ui_kit::tokens::{radius, space};

use crate::caret::Doc;
use crate::formatting::{self, Action, BlockStyle, Change, ListKind};
use crate::lines::Kind;
use crate::markup::{CueKind, Markup};
use crate::typing;

/// The commands, as they are listed.
pub const COMMANDS: [(&str, Action); 12] = [
    ("Paragraph", Action::Style(BlockStyle::Paragraph)),
    ("Heading 1", Action::Style(BlockStyle::Heading(1))),
    ("Heading 2", Action::Style(BlockStyle::Heading(2))),
    ("Heading 3", Action::Style(BlockStyle::Heading(3))),
    ("Bulleted list", Action::List(ListKind::Bulleted)),
    ("Numbered list", Action::List(ListKind::Numbered)),
    ("Quote", Action::Style(BlockStyle::Quote)),
    ("Horizontal rule", Action::Rule),
    ("Note", Action::Cue(CueKind::Note)),
    ("Pause", Action::Cue(CueKind::Pause)),
    ("Mark", Action::Cue(CueKind::Mark)),
    ("End", Action::Cue(CueKind::End)),
];

const WIDTH: f32 = 220.0;
/// The room between the caret's line and the menu.
const GAP: f32 = space::SCALE_4;

/// An open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Menu {
    /// The byte the "/" is at.
    pub at: usize,
    /// The highlighted one of the commands listed.
    pub selected: usize,
    /// Scroll to it: the keys moved it.
    pub reveal: bool,
}

impl Menu {
    pub fn new(at: usize) -> Self {
        Self { at, selected: 0, reveal: true }
    }

    /// Moves the highlight one step among `count` commands, and around at either end.
    pub fn step(&mut self, down: bool, count: usize) {
        let count = count.max(1);
        self.selected = (self.selected + if down { 1 } else { count - 1 }) % count;
        self.reveal = true;
    }

    /// What has been typed after the "/", or `None` if the menu has to close: the "/" is
    /// gone, the caret has left what follows it, or no command matches.
    pub fn query<'a>(&self, text: &'a str, selection: &Range<usize>) -> Option<&'a str> {
        if !selection.is_empty() || text.as_bytes().get(self.at) != Some(&b'/') {
            return None;
        }
        let query = text.get(self.at + 1..selection.start)?;
        let ends = query.starts_with(char::is_whitespace) || query.contains(['\n', '/']);
        (!ends && !matching(query).is_empty()).then_some(query)
    }
}

/// The commands whose name, or a word of it, starts with `query`. Case does not matter.
pub fn matching(query: &str) -> Vec<(&'static str, Action)> {
    let query = query.to_lowercase();
    let starts = |label: &str| {
        let label = label.to_lowercase();
        std::iter::once(0).chain(label.match_indices(' ').map(|(at, _)| at + 1)).any(|at| label[at..].starts_with(&query))
    };
    COMMANDS.into_iter().filter(|(label, _)| starts(label)).collect()
}

/// True if a "/" typed at byte `at` opens the menu: it starts its line's text or follows a
/// space, and is not in a code block.
pub fn opens(doc: &Doc<'_>, at: usize) -> bool {
    let line = doc.line_of(at);
    let start = doc.bounds[line].0;
    let before = doc.visible(start.min(at)..at).next_back();
    doc.lines[line].kind != Kind::Fence && before.is_none_or(|before| doc.text[before].chars().all(char::is_whitespace))
}

/// Takes `typed`, the "/" and the query, out of the text and does the command where it was.
pub fn run(doc: &Doc<'_>, typed: Range<usize>, action: Action) -> Change {
    let removed = typing::replace(doc, typed, "").change;
    let markup = Markup::parse(&removed.text);
    let (text, caret) = (removed.text.as_str(), removed.selection.start);
    let active = formatting::active(text, &markup, caret..caret, caret);
    let in_cue = |kind| markup.cues.iter().any(|cue| cue.kind == kind && cue.range.start < caret && caret < cue.range.end);
    // The actions turn off what is on already, which a command never does.
    let done = match action {
        Action::Style(BlockStyle::Paragraph) => formatting::apply(text, &markup, caret..caret, action).map(typing::keep_apart),
        Action::Cue(kind) if in_cue(kind) => None,
        _ if active.has(action) => None,
        _ => formatting::apply(text, &markup, caret..caret, action),
    };
    done.unwrap_or(removed)
}

/// What [`show`] saw of the pointer.
pub struct Shown {
    /// The command clicked, as an index of those listed.
    pub clicked: Option<usize>,
    /// The pointer is on the menu.
    pub pointed: bool,
}

/// Draws the menu under `caret`, the caret's rectangle, or over it with more room there.
pub fn show(ui: &egui::Ui, id: egui::Id, menu: &mut Menu, listed: &[(&str, Action)], caret: Rect) -> Shown {
    let ctx = ui.ctx();
    let screen = ctx.content_rect();
    let row = ui.spacing().interact_size.y;
    let padding = 2.0 * space::SCALE_4;
    let wanted = listed.len() as f32 * row + padding;
    let (below, above) = (screen.bottom() - caret.bottom() - GAP, caret.top() - screen.top() - GAP);
    let under = wanted <= below || below >= above;
    let height = wanted.min(if under { below } else { above }).max(row + padding);
    let at: Pos2 = if under { caret.left_bottom() + vec2(0.0, GAP) } else { caret.left_top() - vec2(0.0, GAP + height) };

    let reveal = std::mem::take(&mut menu.reveal);
    let mut clicked = None;
    let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
    let area = egui::Area::new(id.with("slash-menu")).order(Order::Foreground).fixed_pos(at).constrain(true);
    let response = area
        .show(ctx, |ui| {
            ui_kit::glass_frame(ui.ctx(), radius::GROUP).inner_margin(space::SCALE_4).show(ui, |ui| {
                ui.set_width(WIDTH);
                ui.spacing_mut().item_spacing.y = 0.0;
                // Without the least height, the list would not grow again after a filter
                // had shortened it.
                let rows = egui::ScrollArea::vertical().max_height(height - padding).min_scrolled_height(height - padding);
                rows.show(ui, |ui| {
                    for (index, (label, _)) in listed.iter().enumerate() {
                        let button = egui::Button::selectable(index == menu.selected, *label).min_size(vec2(WIDTH, row));
                        let response = ui.add(button);
                        if reveal && index == menu.selected {
                            response.scroll_to_me(None);
                        }
                        if moved && response.hovered() {
                            menu.selected = index;
                        }
                        if response.clicked() {
                            clicked = Some(index);
                        }
                    }
                });
            });
        })
        .response;
    let pointed = ctx.input(|i| i.pointer.interact_pos()).is_some_and(|pointer| response.rect.contains(pointer));
    Shown { clicked, pointed }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str) -> Vec<&'static str> {
        matching(query).into_iter().map(|(label, _)| label).collect()
    }

    #[test]
    fn typing_filters_by_the_start_of_the_name_or_of_a_word_in_it() {
        let all =
            ["Paragraph", "Heading 1", "Heading 2", "Heading 3", "Bulleted list", "Numbered list", "Quote", "Horizontal rule", "Note", "Pause", "Mark", "End"];
        assert_eq!(names(""), all);
        assert_eq!(names("h"), ["Heading 1", "Heading 2", "Heading 3", "Horizontal rule"]);
        assert_eq!(names("HEADING 2"), ["Heading 2"]);
        assert_eq!(names("list"), ["Bulleted list", "Numbered list"]);
        assert_eq!(names("pa"), ["Paragraph", "Pause"]);
        assert!(names("graph").is_empty());
    }

    #[test]
    fn the_query_ends_when_the_slash_or_the_caret_leaves_or_nothing_matches() {
        let menu = Menu::new(2);
        assert_eq!(menu.query("a /he", &(5..5)), Some("he"));
        assert_eq!(menu.query("a /heading 2", &(12..12)), Some("heading 2"));
        assert_eq!(menu.query("a /", &(3..3)), Some(""));
        for (text, selection) in [("a ", 2..2), ("a /he", 2..2), ("a /he", 3..5), ("a / ", 4..4), ("a /xyz", 6..6), ("a /\n", 4..4)] {
            assert_eq!(menu.query(text, &selection), None, "{text:?} {selection:?}");
        }
    }

    #[test]
    fn the_menu_opens_at_a_lines_start_or_after_a_space() {
        let at = |text: &str| {
            let markup = Markup::parse(text);
            opens(&Doc::new(text, &markup), text.find('/').unwrap_or(0))
        };
        for text in ["/", "one /", "one\n/", "- /", "# /", "> /", "**/**", "a **/b**"] {
            assert!(at(text), "{text:?}");
        }
        for text in ["one/", "1/2", "**a**/", "```\n/\n```"] {
            assert!(!at(text), "{text:?}");
        }
    }

    /// Does the command where `|` is, after the "/" and query ahead of it.
    fn ran(marked: &str, label: &str) -> String {
        let caret = marked.find('|').unwrap_or(marked.len());
        let text = marked.replacen('|', "", 1);
        let slash = text[..caret].rfind('/').unwrap_or(0);
        let markup = Markup::parse(&text);
        let action = COMMANDS.into_iter().find(|(name, _)| *name == label).map(|(_, action)| action);
        let Some(action) = action else {
            return format!("no command {label}");
        };
        let mut change = run(&Doc::new(&text, &markup), slash..caret, action);
        change.text.insert(change.selection.start, '|');
        change.text
    }

    #[test]
    fn each_command_takes_out_what_was_typed_and_formats() {
        for (marked, label, wanted) in [
            ("/h|", "Heading 1", "# |"),
            ("one /|", "Heading 2", "## one |"),
            ("text\n/|", "Heading 3", "text\n### |"),
            ("/|", "Bulleted list", "- |"),
            ("text\n/num|", "Numbered list", "text\n\n1. |"),
            ("/|", "Quote", "> |"),
            ("## two /p|", "Paragraph", "two|"),
            ("- a\n- /|", "Paragraph", "- a\n\n|"),
            ("words\n\n/|", "Horizontal rule", "words\n\n---\n|"),
            ("say /note|", "Note", "say [[note: |]]"),
            ("say /|", "Pause", "say [[pause]]|"),
            ("/mark|", "Mark", "[[mark: |]]"),
            ("# /end|", "End", "# [[end]]|"),
        ] {
            assert_eq!(ran(marked, label), wanted, "{marked:?} {label}");
        }
    }

    #[test]
    fn a_command_never_turns_off_what_is_on() {
        assert_eq!(ran("# a /|", "Heading 1"), "# a |");
        assert_eq!(ran("- a /|", "Bulleted list"), "- a |");
        assert_eq!(ran("> a /|", "Quote"), "> a |");
        assert_eq!(ran("[[note: a /|]]", "Note"), "[[note: a |]]");
    }
}
