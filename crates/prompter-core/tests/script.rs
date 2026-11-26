use prompter_core::script::{Block, BlockKind, Cue, MarkCategory, Section, Span, parse, sections};
use serde::Deserialize;
use serde::de::value::{Error, StrDeserializer};

fn blocks(md: &str) -> Vec<Block> {
    parse(md).blocks
}

/// Each block as its kind and plain text.
fn outline(md: &str) -> Vec<(BlockKind, String)> {
    blocks(md).into_iter().map(|b| (b.kind.clone(), b.text())).collect()
}

fn span(text: &str, bold: bool, italic: bool) -> Span {
    Span { text: text.to_owned(), bold, italic, underline: false, strike: false }
}

fn underlined(text: &str) -> Span {
    Span { underline: true, ..span(text, false, false) }
}

fn struck(text: &str) -> Span {
    Span { strike: true, ..span(text, false, false) }
}

/// The spans of each block.
fn spans(md: &str) -> Vec<Vec<Span>> {
    blocks(md).into_iter().map(|block| block.spans).collect()
}

#[test]
fn underline_and_strikethrough() {
    let plain = |text| span(text, false, false);
    assert_eq!(spans("a <u>b</u> ~~c~~ d"), [vec![plain("a "), underlined("b"), plain(" "), struck("c"), plain(" d")]]);
    // With other styles, inside words, over a line break and in any block.
    let all = Span { bold: true, italic: true, underline: true, strike: true, ..plain("x") };
    assert_eq!(spans("<u>~~***x***~~</u>"), [vec![all]]);
    assert_eq!(spans("a<u>b</u>c~~d~~e"), [vec![plain("a"), underlined("b"), plain("c"), struck("d"), plain("e")]]);
    assert_eq!(spans("<u>one\\\ntwo</u>"), [vec![underlined("one\ntwo")]]);
    assert_eq!(spans("# <u>a</u>\n\n- ~~b~~\n\n> <u>c</u>"), [vec![underlined("a")], vec![struck("b")], vec![underlined("c")]]);
    // The tags may be in capitals or hold space before the '>'.
    assert_eq!(spans("<U>a</u >"), [vec![underlined("a")]]);
    assert_eq!(spans("<u><u>a</u>b</u>c"), [vec![underlined("ab"), plain("c")]]);
}

#[test]
fn an_underline_ends_with_its_block() {
    let plain = |text| span(text, false, false);
    assert_eq!(spans("a <u>b\n\nc"), [vec![plain("a "), underlined("b")], vec![plain("c")]]);
    assert_eq!(spans("- <u>a\n  - b\n- c"), [vec![underlined("a")], vec![plain("b")], vec![plain("c")]]);
    // A closing tag alone does nothing.
    assert_eq!(spans("a</u> <u>b</u>"), [vec![plain("a "), underlined("b")]]);
}

#[test]
fn a_cue_splits_underlined_and_struck_text() {
    assert_eq!(spans("<u>a [[pause]] b</u>"), [vec![underlined("a")], vec![], vec![underlined("b")]]);
    assert_eq!(spans("~~a [[pause]] b~~"), [vec![struck("a")], vec![], vec![struck("b")]]);
}

#[test]
fn other_inline_html_is_dropped_and_other_tildes_are_text() {
    let plain = |text| span(text, false, false);
    for md in ["a <b>b</b> c", "a <s>b</s> c", "a <del>b</del> c", "a <ul>b</ul> c", "a <u class=\"x\">b c", "a <u/>b c", "a <!-- <u> -->b c"] {
        assert_eq!(spans(md), [vec![plain("a b c")]], "{md:?}");
    }
    for text in ["a ~b~ c", "H~2~O", "~5 to ~10", "a ~~ b ~~ c", "~**a**~", "~~a~"] {
        let read: String = blocks(text).iter().map(Block::text).collect();
        assert_eq!(read, text.replace("**", ""), "{text:?}");
        assert!(blocks(text).iter().flat_map(|block| &block.spans).all(|span| !span.strike), "{text:?}");
    }
    assert_eq!(spans("~a ~~b~~ c~"), [vec![plain("~a "), struck("b"), plain(" c~")]]);
    // Escapes and code keep the markers as text.
    assert_eq!(spans(r"\~\~a\~\~ \<u>b\</u>"), [vec![plain("~~a~~ <u>b</u>")]]);
    assert_eq!(spans("`~~a~~ <u>b</u>`"), [vec![plain("~~a~~ <u>b</u>")]]);
}

fn text(kind: BlockKind, text: &str) -> (BlockKind, String) {
    (kind, text.to_owned())
}

fn cue(cue: Cue) -> (BlockKind, String) {
    (BlockKind::Cue(cue), String::new())
}

fn mark(label: &str, category: MarkCategory) -> Cue {
    Cue::Mark { label: label.to_owned(), category }
}

fn item(ordered: bool, depth: u8) -> BlockKind {
    BlockKind::ListItem { ordered, depth, continues: false, start: None }
}

/// The first item of an ordered list.
fn first_item(depth: u8, start: u64) -> BlockKind {
    BlockKind::ListItem { ordered: true, depth, continues: false, start: Some(start) }
}

/// The rest of a list item that a cue split.
fn rest(ordered: bool, depth: u8) -> BlockKind {
    BlockKind::ListItem { ordered, depth, continues: true, start: None }
}

#[test]
fn headings_and_paragraphs() {
    assert_eq!(
        outline("# One\n\nFirst line\nsecond line.\n\n###### Six\n\nSetext\n---\n"),
        [
            text(BlockKind::Heading(1), "One"),
            text(BlockKind::Paragraph, "First line second line."),
            text(BlockKind::Heading(6), "Six"),
            text(BlockKind::Heading(2), "Setext"),
        ]
    );
}

#[test]
fn hard_break_is_a_newline() {
    assert_eq!(outline("one\\\ntwo"), [text(BlockKind::Paragraph, "one\ntwo")]);
}

#[test]
fn tight_nested_lists() {
    assert_eq!(
        outline("- a\n  1. b\n  2. c\n- d\n"),
        [text(item(false, 0), "a"), text(first_item(1, 1), "b"), text(item(true, 1), "c"), text(item(false, 0), "d"),]
    );
}

#[test]
fn loose_list_and_text_after_it() {
    assert_eq!(outline("1. a\n\n2. b\n\nafter\n"), [text(first_item(0, 1), "a"), text(item(true, 0), "b"), text(BlockKind::Paragraph, "after"),]);
}

#[test]
fn ordered_lists_keep_their_start_number() {
    assert_eq!(
        outline("7. a\n8. b\n\ntext\n\n0. c\n"),
        [text(first_item(0, 7), "a"), text(item(true, 0), "b"), text(BlockKind::Paragraph, "text"), text(first_item(0, 0), "c"),]
    );
}

#[test]
fn an_item_without_text_uses_up_its_number() {
    assert_eq!(outline("3. [[pause]]\n4. four\n5. five\n"), [cue(Cue::Pause), text(first_item(0, 4), "four"), text(item(true, 0), "five"),]);
}

#[test]
fn split_list_item_continues() {
    assert_eq!(
        outline("4. a [[pause]] b [[end]] c\n5. d\n"),
        [text(first_item(0, 4), "a"), cue(Cue::Pause), text(rest(true, 0), "b"), cue(Cue::End), text(rest(true, 0), "c"), text(item(true, 0), "d"),]
    );
}

#[test]
fn cue_before_an_item_does_not_split_it() {
    assert_eq!(outline("2. [[pause]] a [[end]] b\n"), [cue(Cue::Pause), text(first_item(0, 2), "a"), cue(Cue::End), text(rest(true, 0), "b"),]);
}

#[test]
fn quotes_and_rules() {
    assert_eq!(
        outline("> one\n>\n> two\n\n---\n\nout\n"),
        [text(BlockKind::Quote, "one"), text(BlockKind::Quote, "two"), text(BlockKind::Rule, ""), text(BlockKind::Paragraph, "out"),]
    );
}

#[test]
fn ids_and_source_lines() {
    let found: Vec<(u32, u32)> =
        blocks("# T\r\n\r\nalpha\r\n[[pause]]\r\nbeta\r\n\r\n- x\r\n- y\r\n\r\n---\r\n").iter().map(|b| (b.id, b.source_line)).collect();
    assert_eq!(found, [(0, 1), (1, 3), (2, 4), (3, 5), (4, 7), (5, 8), (6, 10)]);
}

#[test]
fn bold_and_italic_spans() {
    let found = blocks("a **b** *c* ***d*** e");
    assert_eq!(
        found[0].spans,
        [
            span("a ", false, false),
            span("b", true, false),
            span(" ", false, false),
            span("c", false, true),
            span(" ", false, false),
            span("d", true, true),
            span(" e", false, false),
        ]
    );
}

#[test]
fn links_and_images_are_plain_text() {
    let found = blocks("See [the **docs**](http://x.test) and ![a chart](c.png).");
    assert_eq!(found[0].text(), "See the docs and a chart.");
    assert_eq!(found[0].spans[1], span("docs", true, false));
}

#[test]
fn brackets_stay_one_span() {
    assert_eq!(blocks("an [aside] here")[0].spans, [span("an [aside] here", false, false)]);
}

#[test]
fn html_is_dropped() {
    assert_eq!(outline("<div>\nhidden\n</div>\n\na <b>word</b><!-- [[pause]] --> here\n"), [text(BlockKind::Paragraph, "a word here")]);
}

#[test]
fn each_cue_on_its_own_line() {
    assert_eq!(
        outline("[[note: show the settings menu]]\n\n[[pause]]\n\n[[mark: intro]]\n\n[[end]]\n"),
        [cue(Cue::Note("show the settings menu".to_owned())), cue(Cue::Pause), cue(mark("intro", MarkCategory::Note)), cue(Cue::End),]
    );
}

#[test]
fn keywords_ignore_case_and_labels_are_trimmed() {
    assert_eq!(
        outline("[[ NOTE :   Keep My Case  ]] [[Pause]] [[MaRk:  x ]] [[ END ]]"),
        [cue(Cue::Note("Keep My Case".to_owned())), cue(Cue::Pause), cue(mark("x", MarkCategory::Note)), cue(Cue::End),]
    );
}

#[test]
fn inline_cue_splits_a_paragraph() {
    assert_eq!(outline("before [[pause]] after"), [text(BlockKind::Paragraph, "before"), cue(Cue::Pause), text(BlockKind::Paragraph, "after"),]);
}

#[test]
fn inline_cue_keeps_styles_on_both_sides() {
    let found = blocks("**one [[pause]] two**");
    assert_eq!(found[0].spans, [span("one", true, false)]);
    assert_eq!(found[2].spans, [span("two", true, false)]);
}

#[test]
fn inline_cue_splits_list_items_and_quotes() {
    assert_eq!(
        outline("- a [[pause]] b\n\n> c [[end]]\n"),
        [text(item(false, 0), "a"), cue(Cue::Pause), text(rest(false, 0), "b"), text(BlockKind::Quote, "c"), cue(Cue::End),]
    );
}

#[test]
fn cue_text_may_be_formatted_or_wrapped() {
    assert_eq!(outline("[[note: *show* the\nmenu]]"), [cue(Cue::Note("show the menu".to_owned()))]);
}

#[test]
fn unknown_cues_stay_as_text() {
    for md in ["[[wave]]", "[[note]]", "[[pause: 2]]", "[[mark]]", "[[end", "a [[b]] c"] {
        assert_eq!(outline(md), [text(BlockKind::Paragraph, md)], "{md}");
    }
}

#[test]
fn cue_after_unknown_brackets_is_found() {
    assert_eq!(outline("[[x [[pause]] [[[end]]"), [text(BlockKind::Paragraph, "[[x"), cue(Cue::Pause), text(BlockKind::Paragraph, "["), cue(Cue::End),]);
}

#[test]
fn unclosed_cue_does_not_swallow_the_next_one() {
    assert_eq!(outline("[[note: oops [[pause]]"), [text(BlockKind::Paragraph, "[[note: oops"), cue(Cue::Pause),]);
}

#[test]
fn inline_code_is_never_a_cue() {
    for (md, plain) in [("type `[[pause]]` here", "type [[pause]] here"), ("`[[`pause]]", "[[pause]]"), ("[[pause`]]`", "[[pause]]")] {
        assert_eq!(outline(md), [text(BlockKind::Paragraph, plain)], "{md}");
    }
    assert_eq!(blocks("a `[[end]]` b")[0].spans, [span("a [[end]] b", false, false)]);
}

#[test]
fn code_blocks_are_never_cues() {
    for md in ["```\n[[pause]]\n```\n", "~~~text\n[[pause]]\n~~~\n", "    [[pause]]\n"] {
        assert_eq!(outline(md), [text(BlockKind::Paragraph, "[[pause]]")], "{md}");
    }
    assert_eq!(outline("```\ncode\n```\n\n[[pause]]\n"), [text(BlockKind::Paragraph, "code"), cue(Cue::Pause)]);
}

#[test]
fn a_cue_may_hold_code() {
    assert_eq!(outline("[[note: type `a]] [[b` now]]"), [cue(Cue::Note("type a]] [[b now".to_owned()))]);
}

#[test]
fn a_cue_ends_at_the_last_closing_brackets() {
    assert_eq!(outline("[[note: click [Save]]]"), [cue(Cue::Note("click [Save]".to_owned()))]);
    assert_eq!(outline("[[mark: [a]]] then [[pause]]"), [cue(mark("[a]", MarkCategory::Note)), text(BlockKind::Paragraph, "then"), cue(Cue::Pause),]);
}

#[test]
fn mark_type_and_colon_are_taken_off_the_label() {
    for (written, label, category) in [
        ("Highlight: best take", "best take", MarkCategory::Highlight),
        ("cut: tangent", "tangent", MarkCategory::Cut),
        ("RETAKE:", "", MarkCategory::Retake),
        ("Chapter : Setup", "Setup", MarkCategory::Chapter),
        ("Note:lights", "lights", MarkCategory::Note),
        ("B-roll: desk", "desk", MarkCategory::BRoll),
        ("b_roll:   desk: left", "desk: left", MarkCategory::BRoll),
    ] {
        assert_eq!(outline(&format!("[[mark: {written}]]")), [cue(mark(label, category))], "{written}");
    }
}

#[test]
fn other_mark_labels_are_kept_as_written() {
    for (label, category) in [
        ("B-roll here", MarkCategory::BRoll),
        ("cut the intro", MarkCategory::Cut),
        ("cut the intro: all of it", MarkCategory::Cut),
        ("chapter", MarkCategory::Chapter),
        ("Cutaway: desk", MarkCategory::Note),
        ("desk: B-roll", MarkCategory::Note),
        ("system: restart", MarkCategory::Note),
        ("", MarkCategory::Note),
    ] {
        assert_eq!(outline(&format!("[[mark: {label}]]")), [cue(mark(label, category))], "{label}");
    }
}

#[test]
fn categories_are_spelled_as_in_the_marks_crate() {
    for (name, category) in [
        ("highlight", MarkCategory::Highlight),
        ("cut", MarkCategory::Cut),
        ("retake", MarkCategory::Retake),
        ("chapter", MarkCategory::Chapter),
        ("note", MarkCategory::Note),
        ("b_roll", MarkCategory::BRoll),
    ] {
        assert_eq!(MarkCategory::deserialize(StrDeserializer::<Error>::new(name)), Ok(category), "{name}");
    }
}

#[test]
fn headings_are_sections() {
    let script = parse("# Intro\n\ntext\n\n## The **big** [part](u)\n\n- x\n");
    assert_eq!(
        sections(&script),
        [Section { block_id: 0, level: 1, title: "Intro".to_owned() }, Section { block_id: 2, level: 2, title: "The big part".to_owned() },]
    );
}

#[test]
fn cues_in_a_heading_leave_one_section() {
    let md = "## [[pause]] One [[mark: Chapter: one]] two [[end]]";
    assert_eq!(outline(md), [cue(Cue::Pause), text(BlockKind::Heading(2), "One two"), cue(mark("one", MarkCategory::Chapter)), cue(Cue::End),]);
    assert_eq!(sections(&parse(md)).len(), 1);
}

#[test]
fn empty_script() {
    assert!(blocks("").is_empty());
    assert!(blocks("\n\n<!-- only a comment -->\n").is_empty());
}
