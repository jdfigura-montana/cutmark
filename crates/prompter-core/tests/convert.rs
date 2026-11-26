#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

use prompter_core::convert::html_to_markdown;
use prompter_core::script::{self, Block, BlockKind, Cue, MarkCategory};

fn blocks(html: &str) -> Vec<Block> {
    script::parse(&html_to_markdown(html)).blocks
}

fn item(ordered: bool, depth: u8, start: Option<u64>) -> BlockKind {
    BlockKind::ListItem { ordered, depth, continues: false, start }
}

/// Each block's kind and text.
fn outline(html: &str) -> Vec<(BlockKind, String)> {
    blocks(html).into_iter().map(|block| (block.kind.clone(), block.text())).collect()
}

#[test]
fn google_docs_fragment() {
    let html = concat!(
        "<meta charset='utf-8'>",
        r#"<b style="font-weight:normal;" id="docs-internal-guid-1a2b3c4d-7fff-0000">"#,
        r#"<h1 dir="ltr"><span style="font-size:20pt;font-family:Arial;font-weight:400;">Opening</span></h1>"#,
        r#"<p dir="ltr" style="line-height:1.38;"><span style="color:#000000;font-weight:400;">Plain, </span>"#,
        r#"<span style="font-weight:700;font-style:normal;">bold</span><span style="font-weight:400;"> and </span>"#,
        r#"<span style="font-weight:400;font-style:italic;">italic</span><span>.</span></p><br />"#,
        r#"<ul style="margin-top:0;"><li dir="ltr" style="list-style-type:disc;" aria-level="1">"#,
        r#"<p dir="ltr" role="presentation"><span>first</span></p></li>"#,
        // Google Docs puts a nested list next to the item, not inside it.
        r#"<ul><li dir="ltr" aria-level="2"><p dir="ltr" role="presentation"><span>inner</span></p></li></ul>"#,
        r#"<li dir="ltr" aria-level="1"><p dir="ltr" role="presentation"><span style="font-weight:bold">second</span></p></li></ul>"#,
        "</b><br class=\"Apple-interchange-newline\">",
    );
    assert_eq!(html_to_markdown(html), "# Opening\n\nPlain, **bold** and *italic*.\n\n- first\n  - inner\n- **second**\n");
}

#[test]
fn google_docs_wrapper_without_a_style_is_not_bold() {
    assert_eq!(html_to_markdown(r#"<b id="docs-internal-guid-1"><p>plain</p></b>"#), "plain\n");
}

#[test]
fn word_fragment() {
    let html = r#"<html xmlns:o="urn:schemas-microsoft-com:office:office"><head><meta charset=utf-8>
<!--[if gte mso 9]><xml><o:OfficeDocumentSettings/></xml><![endif]-->
<style><!-- p.MsoNormal {mso-style-parent:""; font-weight:bold;} --></style></head>
<body lang=EN-US><!--StartFragment-->
<h2>Steps<o:p></o:p></h2>
<p class=MsoNormal style='mso-bidi-font-weight:bold;mso-bidi-font-style:italic'><span
style='mso-fareast-font-family:Calibri'>Before the
list<o:p></o:p></span></p>
<p class=MsoNormal><o:p>&nbsp;</o:p></p>
<p class=MsoNormal><o:p>&nbsp;</o:p></p>
<p class=MsoListParagraphCxSpFirst style='text-indent:-.25in;mso-list:l0 level1 lfo1'><![if !supportLists]><span
style='font-family:Symbol'><span style='mso-list:Ignore'>·<span style='font:7.0pt "Times New Roman"'>&nbsp;&nbsp;
</span></span></span><![endif]>Open the app<o:p></o:p></p>
<p class=MsoListParagraphCxSpMiddle style='mso-list:l0 level2 lfo1'><!--[if !supportLists]--><span
style='mso-list:Ignore'>o&nbsp; </span><!--[endif]-->Sign <b>in</b><o:p/></p>
<p class=MsoListParagraphCxSpLast style='mso-list:l1 level1 lfo2'><![if !supportLists]><span
style='mso-list:Ignore'>3.&nbsp;&nbsp; </span><![endif]>Third<o:p></o:p></p>
<p class=MsoListParagraph style='mso-list:l1 level1 lfo2'><![if !supportLists]><span
style='mso-list:Ignore'>4.&nbsp; </span><![endif]>Fourth<o:p></o:p></p>
<!--EndFragment--></body></html>"#;
    assert_eq!(html_to_markdown(html), "## Steps\n\nBefore the list\n\n- Open the app\n  - Sign **in**\n\n3. Third\n4. Fourth\n");
}

#[test]
fn word_list_paragraph_without_a_marker_is_a_bullet() {
    assert_eq!(html_to_markdown(r#"<p class="MsoListParagraph">one</p>"#), "- one\n");
}

#[test]
fn browser_fragment_keeps_a_link_as_text_and_drops_an_image() {
    let html = r#"<script>alert("no")</script><style>p { color: red }</style><!-- a comment -->
<div style="font-family: serif; color: #123456; font-size: 18px"><p>Read <a href="https://example.com/a_b">the
<em>full</em> guide</a> now.<img src="https://example.com/x.png" alt="a chart"></p>
<a href="https://example.com"><img src="y.png"></a><svg><text>drawn</text></svg></div>"#;
    assert_eq!(html_to_markdown(html), "Read the *full* guide now.\n");
}

#[test]
fn nested_lists() {
    let html = "<ol><li>one<ul><li>a<ol><li>deep</li></ol></li><li>b</li></ul></li><li>two</li></ol>\
                <ul><li><p>loose</p></li></ul>";
    assert_eq!(
        outline(html),
        [
            (item(true, 0, Some(1)), "one".to_owned()),
            (item(false, 1, None), "a".to_owned()),
            (item(true, 2, Some(1)), "deep".to_owned()),
            (item(false, 1, None), "b".to_owned()),
            (item(true, 0, None), "two".to_owned()),
            (item(false, 0, None), "loose".to_owned()),
        ]
    );
}

#[test]
fn lists_that_start_past_one_stay_lists() {
    let html = r#"<ul><li>top<ol start="5"><li>five</li><li>six</li></ol></li></ul>"#;
    assert_eq!(
        outline(html),
        [(item(false, 0, None), "top".to_owned()), (item(true, 1, Some(5)), "five".to_owned()), (item(true, 1, None), "six".to_owned()),]
    );
    let tens = r#"<ol start="10"><li>ten<ul><li>under</li></ul></li></ol>"#;
    assert_eq!(html_to_markdown(tens), "10. ten\n    - under\n");
    assert_eq!(blocks(tens)[1].kind, item(false, 1, None));
}

#[test]
fn an_item_with_two_paragraphs_has_one_marker() {
    let html = "<ul><li><p>first</p><p>more</p><br><br>last</li><li><img src=x></li></ul>";
    assert_eq!(html_to_markdown(html), "- first\\\n  more\\\n  last\n");
    assert_eq!(blocks(html).len(), 1);
}

#[test]
fn each_table_cell_gets_its_own_line() {
    let html = "<table><caption>Cast</caption><thead><tr><th>Name</th><th>Role</th></tr></thead>\
                <tbody><tr><td>Ana</td><td><b>Host</b></td></tr><tr><td> </td><td>a | b</td></tr></tbody></table>";
    assert_eq!(html_to_markdown(html), "Cast\n\nName\n\nRole\n\nAna\n\n**Host**\n\na \\| b\n");
}

#[test]
fn quotes_and_line_breaks() {
    let html = "<p>one<br>two<br></p><blockquote><p>first</p><p>second<br>third</p></blockquote>\
                <blockquote>other</blockquote><p>a<br><br><br>b</p>";
    assert_eq!(html_to_markdown(html), "one\\\ntwo\n\n> first\n>\n> second\\\n> third\n\n> other\n\na\n\nb\n");
    let kinds: Vec<_> = outline(html).into_iter().map(|(_, text)| text).collect();
    assert_eq!(kinds, ["one\ntwo", "first", "second\nthird", "other", "a", "b"]);
}

#[test]
fn runs_of_empty_paragraphs_make_one_blank_line() {
    let html = "<p></p><p>a</p><p>&nbsp;</p><p><br></p><div><span> </span></div><br><br><p>b</p><p></p>";
    assert_eq!(html_to_markdown(html), "a\n\nb\n");
}

#[test]
fn cues_stay_cues() {
    // Editors often split a cue between elements and style parts of it.
    let html = "<p>Wait [[pause]] here</p><p><span>[[mark: </span><b>B-roll</b><span> desk]]</span></p>\
                <p><b>bold [[end]] after</b></p><h2>Intro [[pause]]</h2>";
    let markdown = html_to_markdown(html);
    assert_eq!(markdown, "Wait [[pause]] here\n\n[[mark: B-roll desk]]\n\n**bold** [[end]] **after**\n\n## Intro [[pause]]\n");
    let cues: Vec<_> = script::parse(&markdown)
        .blocks
        .into_iter()
        .filter_map(|block| match block.kind {
            BlockKind::Cue(cue) => Some(cue),
            _ => None,
        })
        .collect();
    let mark = Cue::Mark { label: "B-roll desk".to_owned(), category: MarkCategory::BRoll };
    assert_eq!(cues, [Cue::Pause, mark, Cue::End, Cue::Pause]);
}

#[test]
fn plain_text_is_escaped() {
    let lines = [
        "press <Enter> now",
        "*word* _and_ `code` ~~gone~~",
        "1. not a list",
        "# not a heading",
        "> not a quote",
        "- not a list",
        "+ not a list",
        "---",
        "===",
        "[a](b) ![c](d)",
        "&amp; R&D",
        r"C:\new\ ends with \",
        "[[not closed",
    ];
    for line in lines {
        let escaped = line.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let html = format!("<p>{escaped}</p>");
        assert_eq!(outline(&html), [(BlockKind::Paragraph, line.to_owned())], "{line:?}");
        // The same after a break, in a list item and in a quote.
        let inside = format!("<ul><li>a<br>{escaped}</li></ul><blockquote>{escaped}</blockquote>");
        assert_eq!(outline(&inside), [(item(false, 0, None), format!("a\n{line}")), (BlockKind::Quote, line.to_owned())], "{line:?}");
    }
}

#[test]
fn a_heading_keeps_its_closing_hashes() {
    for title in ["Learn C #", "#", "a ##"] {
        let html = format!("<h3>{title}</h3>");
        assert_eq!(outline(&html), [(BlockKind::Heading(3), title.to_owned())], "{title:?}");
    }
    assert_eq!(html_to_markdown("<h1>two<br><div>lines</div></h1>"), "# two lines\n");
}

#[test]
fn css_decides_the_style() {
    let html = r#"<p><b style="font-weight: 400">a</b> <span style="FONT-WEIGHT: 600 !important">b</span>
<i style="font-style:normal">c</i> <span style="font-weight:500">d</span></p>"#;
    assert_eq!(html_to_markdown(html), "a **b** c d\n");
}

#[test]
fn markers_move_off_spaces_and_punctuation() {
    assert_eq!(html_to_markdown("<p>a<b> b </b>c</p>"), "a **b** c\n");
    assert_eq!(html_to_markdown("<p><b>Note:</b>text</p>"), "**Note**:text\n");
    assert_eq!(html_to_markdown("<p>a<i>(b)</i></p>"), "a(*b)*\n");
    assert_eq!(html_to_markdown("<p><b>a</b> <b>b</b></p>"), "**a b**\n");
    assert_eq!(html_to_markdown("<p><b>a <i>b</i></b> c</p>"), "**a *b*** c\n");
}

/// Whatever mix of styles the HTML has, the text must come through whole. A style may be
/// lost where Markdown cannot write it, but only rarely, and none may be added.
#[test]
fn no_mix_of_styles_changes_the_text() {
    const TEXTS: [&str; 8] = ["a", " ", "(", ".", "*", "b c", "[[pause]]", "_"];
    const TAGS: [(&str, &str); 4] = [("", ""), ("<b>", "</b>"), ("<i>", "</i>"), ("<b><i>", "</i></b>")];
    let runs: Vec<(String, &str, usize)> =
        TEXTS.iter().flat_map(|&text| TAGS.iter().enumerate().map(move |(style, (open, close))| (format!("{open}{text}{close}"), text, style))).collect();
    let (mut styled, mut lost) = (0, 0);
    for (first, a, first_style) in &runs {
        for (second, b, second_style) in &runs {
            for (third, c, third_style) in &runs {
                let html = format!("<p>x{first}{second}{third}y</p>");
                let markdown = html_to_markdown(&html);
                let parsed = script::parse(&markdown);
                let read: String = parsed.blocks.iter().map(Block::text).collect();
                let wanted = format!("x{a}{b}{c}y").replace("[[pause]]", "");
                let squeeze = |text: &str| text.chars().filter(|c| !c.is_whitespace()).collect::<String>();
                assert_eq!(squeeze(&read), squeeze(&wanted), "{html} gave {markdown:?}");

                // Letters keep the styles of their run.
                let letters: Vec<(char, bool, bool)> = parsed
                    .blocks
                    .iter()
                    .flat_map(|block| &block.spans)
                    .flat_map(|span| span.text.chars().map(|c| (c, span.bold, span.italic)))
                    .filter(|(c, ..)| c.is_alphabetic())
                    .collect();
                let mut expected = vec![('x', false, false)];
                for (text, style) in [(a, first_style), (b, second_style), (c, third_style)] {
                    if *text != "[[pause]]" {
                        let flags = (style % 2 == 1, *style >= 2);
                        expected.extend(text.chars().filter(|c| c.is_alphabetic()).map(|c| (c, flags.0, flags.1)));
                    }
                }
                expected.push(('y', false, false));
                assert_eq!(letters.len(), expected.len(), "{html} gave {markdown:?}");
                for (got, want) in letters.iter().zip(&expected) {
                    assert!(got.0 == want.0 && (!got.1 || want.1) && (!got.2 || want.2), "{html} gave {markdown:?}");
                    styled += usize::from(want.1) + usize::from(want.2);
                    lost += usize::from(want.1 && !got.1) + usize::from(want.2 && !got.2);
                }
            }
        }
    }
    println!("lost {lost} of {styled} letter styles");
    assert!(lost * 100 < styled, "lost {lost} of {styled} letter styles");
}

#[test]
fn deep_nesting_keeps_the_text() {
    let html = format!("{}<p>bottom</p><script>no</script>{}", "<div>".repeat(3_000), "</div>".repeat(3_000));
    assert_eq!(html_to_markdown(&html), "bottom\n");
}

#[test]
fn nothing_gives_an_empty_string() {
    for html in ["", "   ", "<img src=x>", "<style>p{}</style><!-- c -->", "<p><b> </b></p><ul><li></li></ul>"] {
        assert_eq!(html_to_markdown(html), "", "{html:?}");
    }
}
