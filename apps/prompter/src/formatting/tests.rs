//! Texts are written with the selection between `{` and `}`, or the caret as `^`.

use prompter_core::script::{self, BlockKind};

use super::*;
use Action::{Cue, Indent, List, Outdent, Rule, Style};
use BlockStyle::{Heading, Paragraph, Quote};
use ListKind::{Bulleted, Numbered};

const BOLD: Action = Action::Inline(Inline::Bold);
const ITALIC: Action = Action::Inline(Inline::Italic);
const UNDERLINE: Action = Action::Inline(Inline::Underline);
const STRIKE: Action = Action::Inline(Inline::Strike);

fn read(marked: &str) -> (String, Range<usize>) {
    if let Some(caret) = marked.find('^') {
        return (marked.replacen('^', "", 1), caret..caret);
    }
    let start = marked.find('{').unwrap();
    let end = marked.find('}').unwrap() - 1;
    (marked.replacen('{', "", 1).replacen('}', "", 1), start..end)
}

fn write(text: &str, selection: &Range<usize>) -> String {
    let mut marked = text.to_owned();
    if selection.is_empty() {
        marked.insert(selection.start, '^');
    } else {
        marked.insert(selection.end, '}');
        marked.insert(selection.start, '{');
    }
    marked
}

/// What `action` makes of `marked`, or `None` if it does nothing.
fn run(marked: &str, action: Action) -> Option<String> {
    let (text, selection) = read(marked);
    apply(&text, &Markup::parse(&text), selection, action).map(|change| write(&change.text, &change.selection))
}

/// Checks that `action` turns `off` into `on`, and then back.
fn toggles(off: &str, action: Action, on: &str) {
    assert_eq!(run(off, action).as_deref(), Some(on), "{action:?} on {off:?}");
    assert_eq!(run(on, action).as_deref(), Some(off), "{action:?} off {on:?}");
}

fn becomes(from: &str, action: Action, to: &str) {
    assert_eq!(run(from, action).as_deref(), Some(to), "{action:?} on {from:?}");
}

fn active_at(marked: &str) -> Active {
    let (text, selection) = read(marked);
    active(&text, &Markup::parse(&text), selection.clone(), selection.end)
}

#[test]
fn each_inline_format_toggles_on_a_selection() {
    for (action, open, close) in [(BOLD, "**", "**"), (ITALIC, "*", "*"), (UNDERLINE, "<u>", "</u>"), (STRIKE, "~~", "~~")] {
        toggles("one {two} three", action, &format!("one {open}{{two}}{close} three"));
    }
}

#[test]
fn each_inline_format_toggles_on_the_caret_line_after_its_prefix() {
    for (action, open, close) in [(BOLD, "**", "**"), (ITALIC, "*", "*"), (UNDERLINE, "<u>", "</u>"), (STRIKE, "~~", "~~")] {
        for prefix in ["", "- ", "  12. ", "> ", "## "] {
            toggles(&format!("above\n\n{prefix}two w^ords\n\nbelow"), action, &format!("above\n\n{prefix}{open}two w^ords{close}\n\nbelow"));
        }
    }
    // At the end of the line, typing goes on inside the format.
    becomes("words^", BOLD, "**words^**");
}

#[test]
fn blanks_at_the_ends_of_a_selection_stay_outside_the_markers() {
    becomes("a{ b }c", BOLD, "a{ **b** }c");
    assert_eq!(run("a{ }c", BOLD), None);
}

#[test]
fn a_selection_over_lines_formats_each_lines_text() {
    toggles("- {one\n- two}", BOLD, "- **{one**\n- **two}**");
    // A selection that ends at the start of a line leaves that line alone.
    becomes("{one\n}two", ITALIC, "*{one*\n}two");
}

#[test]
fn part_of_a_formatted_span_can_be_taken_out_of_it() {
    becomes("**one {two} three**", BOLD, "**one** {two} **three**");
    becomes("<u>one {two}</u>", UNDERLINE, "<u>one</u> {two}");
    becomes("__{one}__ two", BOLD, "{one} two");
    becomes("***bo^th***", BOLD, "*bo^th*");
    becomes("***bo^th***", ITALIC, "**bo^th**");
}

#[test]
fn a_format_grows_over_the_spans_it_overlaps_or_touches() {
    becomes("**bo**{ld}", BOLD, "**bo{ld}**");
    becomes("{a **b** c}", BOLD, "**{a b c}**");
    becomes("a ~~{b~~ c}", STRIKE, "a ~~{b c}~~");
}

#[test]
fn a_new_format_never_cuts_across_other_markup() {
    // Inside another span, and around it.
    becomes("**{two}**", ITALIC, "***{two}***");
    becomes("{**two** x}", ITALIC, "*{**two** x}*");
    // Across a span's edge, the whole span comes in.
    becomes("**on{e** two}", ITALIC, "***on{e** two}*");
    // Code, links and cues are taken whole.
    becomes("a `co{de` b}", BOLD, "a **`co{de` b}**");
    becomes("[li{nk](x) y}", BOLD, "**[li{nk](x) y}**");
    becomes("[[note: wa{ve]] z}", BOLD, "**[[note: wa{ve]] z}**");
}

#[test]
fn the_formats_added_read_back_in_the_views() {
    let mut text = "one two three".to_owned();
    for action in [BOLD, ITALIC, UNDERLINE, STRIKE] {
        let change = apply(&text, &Markup::parse(&text), 0..0, action).unwrap();
        text = change.text;
    }
    let spans = script::parse(&text).blocks.remove(0).spans;
    assert_eq!(spans.len(), 1, "{text}");
    assert!(spans[0].bold && spans[0].italic && spans[0].underline && spans[0].strike, "{text}: {:?}", spans[0]);
    assert_eq!(spans[0].text, "one two three");
}

#[test]
fn with_no_text_a_pair_of_markers_goes_around_the_caret_and_comes_off_again() {
    for (action, on) in [(BOLD, "**^**"), (ITALIC, "*^*"), (UNDERLINE, "<u>^</u>"), (STRIKE, "~~^~~")] {
        toggles("^", action, on);
        toggles("- ^", action, &format!("- {on}"));
    }
}

#[test]
fn each_block_style_toggles_on_the_caret_line() {
    for (style, prefix) in [(Heading(1), "# "), (Heading(2), "## "), (Heading(3), "### "), (Quote, "> ")] {
        toggles("wo^rds", Style(style), &format!("{prefix}wo^rds"));
        toggles("above\n\nwo^rds\n\nbelow", Style(style), &format!("above\n\n{prefix}wo^rds\n\nbelow"));
        // An empty line can be given its style before its words.
        toggles("^", Style(style), &format!("{prefix}^"));
    }
}

#[test]
fn each_block_style_toggles_on_a_selection_of_lines() {
    toggles("{one\n\ntwo}", Style(Heading(2)), "## {one\n\n## two}");
    toggles("{one\ntwo}", Style(Quote), "> {one\n> two}");
    // With one line not yet styled, they all take the style.
    becomes("> {one\n\ntwo}", Style(Quote), "> {one\n\n> two}");
}

#[test]
fn the_style_choice_replaces_whatever_the_line_started_with() {
    becomes("# ti^tle #", Style(Heading(3)), "### ti^tle");
    becomes("## ti^tle", Style(Quote), "> ti^tle");
    becomes("> qu^ote", Style(Heading(1)), "# qu^ote");
    becomes("- it^em", Style(Heading(1)), "# it^em");
    becomes("> > de^ep", Style(Paragraph), "de^ep");
    becomes("### ti^tle", Style(Paragraph), "ti^tle");
    assert_eq!(run("pla^in", Style(Paragraph)), None);
}

#[test]
fn a_heading_with_an_underline_loses_it() {
    becomes("Ti^tle\n===\nbody", Style(Paragraph), "Ti^tle\nbody");
    becomes("Ti^tle\n---\nbody", Style(Heading(1)), "# Ti^tle\nbody");
    // From the underline itself, the heading above is changed.
    becomes("Title\n=^==\nbody", Style(Heading(1)), "Title^\nbody");
}

/// The kind of each block the views make of `text`.
fn blocks(text: &str) -> Vec<(BlockKind, String)> {
    script::parse(text).blocks.iter().map(|block| (block.kind.clone(), block.text())).collect()
}

#[test]
fn a_line_that_leaves_a_quote_or_list_is_set_apart_from_it() {
    becomes("> one\n> tw^o", Style(Quote), "> one\n\ntw^o");
    becomes("> one\n> tw^o\n> three", Style(Quote), "> one\n\ntw^o\n> three");
    becomes("- one\n- tw^o\n- three", List(Bulleted), "- one\n\ntw^o\n- three");
    assert_eq!(blocks("- one\n\ntwo\n- three")[1], (BlockKind::Paragraph, "two".to_owned()));
}

#[test]
fn a_new_quote_does_not_take_in_the_line_below() {
    becomes("on^e\ntwo", Style(Quote), "> on^e\n\ntwo");
    becomes("on^e\ntwo", List(Bulleted), "- on^e\n\ntwo");
    assert_eq!(blocks("> one\n\ntwo"), [(BlockKind::Quote, "one".to_owned()), (BlockKind::Paragraph, "two".to_owned())]);
    // An empty item right under a paragraph would make it a heading.
    becomes("one\n^", List(Bulleted), "one\n\n- ^");
}

#[test]
fn each_list_toggles_on_a_line_and_on_a_selection() {
    toggles("wo^rds", List(Bulleted), "- wo^rds");
    toggles("wo^rds", List(Numbered), "1. wo^rds");
    toggles("{one\ntwo\nthree}", List(Bulleted), "- {one\n- two\n- three}");
    toggles("{one\ntwo\nthree}", List(Numbered), "1. {one\n2. two\n3. three}");
    toggles("^", List(Bulleted), "- ^");
}

#[test]
fn one_kind_of_list_turns_into_the_other() {
    becomes("- {one\n- two}", List(Numbered), "1. {one\n2. two}");
    becomes("1. {one\n2. two}", List(Bulleted), "- {one\n- two}");
    // Nested items keep their place and are counted on their own.
    becomes("- {a\n  - b\n  - c\n- d}", List(Numbered), "1. {a\n  1. b\n  2. c\n2. d}");
    becomes("## ti^tle", List(Bulleted), "- ti^tle");
    becomes("> sa^id", List(Bulleted), "> - sa^id");
    // Counting goes on from the item above.
    becomes("3. a\nb^", List(Numbered), "3. a\n4. b^");
}

#[test]
fn indent_and_outdent_nest_a_list_item() {
    toggles_nesting("- a\n- b^", "- a\n  - b^");
    toggles_nesting("1. a\n2. b^", "1. a\n   1. b^");
    toggles_nesting("10. a\n- b^", "10. a\n    - b^");
    toggles_nesting("> - a\n> - b^", "> - a\n>   - b^");
}

/// Checks that Indent turns `out` into `inside`, and Outdent turns it back.
fn toggles_nesting(out: &str, inside: &str) {
    becomes(out, Indent, inside);
    becomes(inside, Outdent, out);
}

#[test]
fn the_items_under_an_item_move_with_it() {
    toggles_nesting("- a\n- b^\n  - c\n    more\n- d", "- a\n  - b^\n    - c\n      more\n- d");
}

#[test]
fn selected_items_move_together_and_stay_level() {
    toggles_nesting("- a\n- {b\n- c}", "- a\n  - {b\n  - c}");
    // The first item has nowhere to go. The others go under it.
    becomes("- {a\n- b\n- c}", Indent, "- {a\n  - b\n  - c}");
    toggles_nesting("1. a\n2. {b\n3. c}", "1. a\n   1. {b\n   2. c}");
}

#[test]
fn a_numbered_item_is_numbered_for_its_new_level() {
    becomes("1. a\n   1. b\n   2. c^", Outdent, "1. a\n   1. b\n2. c^");
    // What was after it now starts a list under it, which has to start at 1.
    becomes("1. a\n   1. b^\n   2. c", Outdent, "1. a\n2. b^\n   1. c");
    let nested = script::parse("1. a\n2. b^\n   1. c").blocks.len();
    assert_eq!(nested, 3);
}

#[test]
fn an_item_with_nothing_to_nest_under_cannot_move() {
    for text in ["- a^", "- a\n  - b^", "text\n\n- a^", "- a\n\ntext\n\n- b^", "plain^", "# head^"] {
        assert_eq!(run(text, Indent), None, "{text}");
    }
    for text in ["- a^", "- a\n- b^", "plain^"] {
        assert_eq!(run(text, Outdent), None, "{text}");
    }
    assert!(active_at("- a\n- b^").can_indent && !active_at("- a\n- b^").can_outdent);
    assert!(active_at("- a\n  - b^").can_outdent && !active_at("- a\n  - b^").can_indent);
}

#[test]
fn a_horizontal_rule_goes_after_the_line_and_comes_off_it() {
    becomes("one^\ntwo", Rule, "one\n\n---\n^two");
    becomes("on^e", Rule, "one\n\n---\n^");
    becomes("^", Rule, "---\n^");
    becomes("one\n^", Rule, "one\n\n---\n^");
    becomes("one\n\n--^-\ntwo", Rule, "one\n\n^two");
    // A heading's underline stays under it.
    becomes("Ti^tle\n===\nbody", Rule, "Title\n===\n\n---\n^body");
    let kinds: Vec<BlockKind> = blocks("one\n\n---\ntwo").into_iter().map(|(kind, _)| kind).collect();
    assert_eq!(kinds, [BlockKind::Paragraph, BlockKind::Rule, BlockKind::Paragraph]);
}

#[test]
fn each_cue_is_inserted_at_the_caret_and_comes_off_again() {
    toggles_cue("one^ two", CueKind::Pause, "one [[pause]]^ two", "one ^two");
    toggles_cue("^", CueKind::End, "[[end]]^", "^");
    becomes("one^two", Cue(CueKind::Pause), "one [[pause]]^ two");
    becomes("one ^", Cue(CueKind::Note), "one [[note: ^]]");
    becomes("one ^", Cue(CueKind::Mark), "one [[mark: ^]]");
    becomes("one [[note: ^]]", Cue(CueKind::Note), "one^");
}

fn toggles_cue(off: &str, kind: CueKind, on: &str, back: &str) {
    becomes(off, Cue(kind), on);
    // The caret inside the cue takes it off.
    let inside = on.replacen('^', "", 1).replacen("]]", "^]]", 1);
    becomes(&inside, Cue(kind), back);
}

#[test]
fn a_selection_becomes_the_words_of_a_note_or_mark() {
    toggles("say {this} now", Cue(CueKind::Note), "say [[note: {this}]] now");
    toggles("say {this} now", Cue(CueKind::Mark), "say [[mark: {this}]] now");
    becomes("say {this}now", Cue(CueKind::Pause), "say this [[pause]]^ now");
}

#[test]
fn a_cue_is_never_put_inside_another() {
    becomes("[[note: a^b]]", Cue(CueKind::Pause), "[[note: ab]] [[pause]]^");
}

#[test]
fn every_inserted_cue_is_one_the_views_read() {
    for kind in [CueKind::Note, CueKind::Pause, CueKind::Mark, CueKind::End] {
        let change = apply("", &Markup::parse(""), 0..0, Cue(kind)).unwrap();
        assert_eq!(Markup::parse(&change.text).cues.iter().map(|cue| cue.kind).collect::<Vec<_>>(), [kind], "{}", change.text);
        assert!(matches!(blocks(&change.text)[0].0, BlockKind::Cue(_)), "{}", change.text);
    }
}

#[test]
fn the_active_formats_are_those_around_the_caret() {
    let at = active_at("**bo^ld** *it*");
    assert!(at.bold && !at.italic && !at.underline && !at.strike);
    let at = active_at("<u>~~*b^oth*~~</u>");
    assert!(at.italic && at.underline && at.strike && !at.bold);
    assert!(!active_at("**bold** ^x").bold);
    // The end of the formatted text still counts.
    assert!(active_at("**bold^**").bold);
    assert!(active_at("**{bo}ld**").bold && !active_at("**bo{ld** x}").bold);
}

#[test]
fn the_active_style_and_list_are_the_caret_lines() {
    let styles = [("pla^in", Paragraph), ("## t^wo", Heading(2)), ("> qu^ote", Quote), ("Ti^tle\n===", Heading(1)), ("- it^em", Paragraph)];
    for (text, style) in styles {
        assert_eq!(active_at(text).style, style, "{text}");
        assert!(active_at(text).has(Style(style)));
    }
    assert_eq!(active_at("- it^em").list, Some(Bulleted));
    assert_eq!(active_at("1. it^em").list, Some(Numbered));
    assert_eq!(active_at("- item\n  mo^re").list, Some(Bulleted));
    assert_eq!(active_at("- item\n\nte^xt").list, None);
    assert_eq!(active_at("```\n- co^de\n```").list, None);
}

#[test]
fn text_beyond_ascii_and_windows_line_ends_keep_their_place() {
    toggles("caf\u{e9} {na\u{ef}ve} \u{130}", BOLD, "caf\u{e9} **{na\u{ef}ve}** \u{130}");
    becomes("one\r\ntw^o\r\n", List(Bulleted), "one\r\n- tw^o\r\n");
}

/// No action may panic or cut a character in half, wherever the selection is.
#[test]
fn every_action_takes_any_selection() {
    let text = "# H #\n\nT\n===\n- **a** <u>b\n  1. [[pause]] ~~c~~\n> q `x` [y](z)\n\n---\n```\ncode\n```\n\u{e9} *\u{e9}*\n";
    let markup = Markup::parse(text);
    let ends: Vec<usize> = text.char_indices().map(|(at, _)| at).chain([text.len()]).collect();
    let actions = [
        Style(Paragraph),
        Style(Heading(1)),
        Style(Quote),
        BOLD,
        ITALIC,
        UNDERLINE,
        STRIKE,
        List(Bulleted),
        List(Numbered),
        Indent,
        Outdent,
        Rule,
        Cue(CueKind::Note),
        Cue(CueKind::Pause),
    ];
    for (from, &start) in ends.iter().enumerate() {
        for &end in &ends[from..] {
            active(text, &markup, start..end, end);
            for action in actions {
                if let Some(change) = apply(text, &markup, start..end, action) {
                    let Range { start, end } = change.selection;
                    assert!(start <= end && change.text.get(start..end).is_some(), "{action:?} on {start}..{end}: {:?}", change.text);
                }
            }
        }
    }
}
