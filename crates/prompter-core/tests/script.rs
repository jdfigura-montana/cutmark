use prompter_core::script::{Block, BlockKind, Cue, MarkCategory, Section, Span, parse, sections};

fn blocks(md: &str) -> Vec<Block> {
    parse(md).blocks
}

/// Each block as its kind and plain text.
fn outline(md: &str) -> Vec<(BlockKind, String)> {
    blocks(md).into_iter().map(|b| (b.kind.clone(), b.text())).collect()
}

fn span(text: &str, bold: bool, italic: bool) -> Span {
    Span { text: text.to_owned(), bold, italic }
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
    BlockKind::ListItem { ordered, depth }
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
        [text(item(false, 0), "a"), text(item(true, 1), "b"), text(item(true, 1), "c"), text(item(false, 0), "d"),]
    );
}

#[test]
fn loose_list_and_text_after_it() {
    assert_eq!(outline("1. a\n\n2. b\n\nafter\n"), [text(item(true, 0), "a"), text(item(true, 0), "b"), text(BlockKind::Paragraph, "after"),]);
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
        [text(item(false, 0), "a"), cue(Cue::Pause), text(item(false, 0), "b"), text(BlockKind::Quote, "c"), cue(Cue::End),]
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
fn mark_categories() {
    for (label, category) in [
        ("Highlight: best take", MarkCategory::Highlight),
        ("cut: tangent", MarkCategory::Cut),
        ("RETAKE:", MarkCategory::Retake),
        ("Chapter: Setup", MarkCategory::Chapter),
        ("Note: lights", MarkCategory::Note),
        ("B-roll: desk", MarkCategory::BRoll),
        ("B-roll here", MarkCategory::BRoll),
        ("chapter", MarkCategory::Chapter),
        ("Cutaway: desk", MarkCategory::Note),
        ("desk: B-roll", MarkCategory::Note),
        ("", MarkCategory::Note),
    ] {
        assert_eq!(outline(&format!("[[mark: {label}]]")), [cue(mark(label, category))], "{label}");
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
    assert_eq!(outline(md), [cue(Cue::Pause), text(BlockKind::Heading(2), "One two"), cue(mark("Chapter: one", MarkCategory::Chapter)), cue(Cue::End),]);
    assert_eq!(sections(&parse(md)).len(), 1);
}

#[test]
fn empty_script() {
    assert!(blocks("").is_empty());
    assert!(blocks("\n\n<!-- only a comment -->\n").is_empty());
}
