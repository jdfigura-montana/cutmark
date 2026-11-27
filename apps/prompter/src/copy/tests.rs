use super::*;
use crate::markup::Markup;

/// What is copied from a script written with `|` at each end of the selection.
fn copied(marked: &str) -> String {
    let start = marked.find('|').unwrap();
    let rest = marked.replacen('|', "", 1);
    let end = rest.find('|').unwrap();
    let text = rest.replacen('|', "", 1);
    let markup = Markup::parse(&text);
    markdown(&Doc::new(&text, &markup), start..end)
}

#[track_caller]
fn all(cases: &[(&str, &str)]) {
    for (marked, wanted) in cases {
        assert_eq!(copied(marked), *wanted, "{marked:?}");
    }
}

#[test]
fn formatting_cut_through_is_closed_around_what_is_selected() {
    all(&[
        ("**w|ord|**", "**ord**"),
        ("**|wor|d**", "**wor**"),
        ("**w|or|d**", "**or**"),
        ("a **bo|ld** c|", "**ld** c"),
        ("|a **bo|ld** c", "a **bo**"),
        ("**ab|c** d *e|f*", "**c** d *e*"),
        ("***bo|th|***", "***th***"),
        ("*a **b|c|** d*", "***c***"),
        ("~~st|ri|ke~~", "~~ri~~"),
        ("<u>un|de|r</u>", "<u>de</u>"),
        ("<u>le|ft| open", "<u>ft</u>"),
        ("## **He|ad|** ##", "**ad**"),
    ]);
}

#[test]
fn blanks_stay_outside_the_markers_and_unused_markers_stay_behind() {
    all(&[
        ("**bold |text|**", "**text**"),
        ("**|bold |text**", "**bold** "),
        // Only the end of the bold's markers is selected, with none of its text.
        ("**bold|** and|", " and"),
        ("|and **|bold**", "and "),
    ]);
}

#[test]
fn code_links_and_cues_keep_their_shape() {
    all(&[
        ("`co|de|`", "`de`"),
        ("[li|nk|](http://x)", "[nk](http://x)"),
        ("```\nco|de|\n```", "```\nde\n```"),
        ("```rust\none\nt|wo\n```\n\naf|ter", "```rust\nwo\n```\n\naf"),
        ("[[note: wa|ve|]] x", "[[note: ve]]"),
        ("[[mark: B-roll: |desk|]]", "[[mark: desk]]"),
        ("[[|pause|]]", "[[pause]]"),
        // Part of a cue's keyword is not a cue.
        ("[[pau|se|]]", "se"),
    ]);
}

#[test]
fn all_of_a_lines_text_takes_the_lines_marks() {
    all(&[
        ("# |Title|", "# Title"),
        ("- |item|", "- item"),
        ("> |said|", "> said"),
        ("# Ti|tle|", "tle"),
        ("- |it|em", "it"),
        ("# |Title\n\nbo|dy", "# Title\n\nbo"),
        ("- |one\n- two|", "- one\n- two"),
        ("- a\n  - |nested|", "- nested"),
        // Up to the start of the next line's text, its marks stay.
        ("- |one\n- |two", "- one\n"),
        ("|Title|\n===\nbody", "Title\n==="),
        ("# |**bo|ld** x", "**bo**"),
        ("# |**bold**|", "# **bold**"),
    ]);
}

#[test]
fn everything_is_copied_as_it_is_written() {
    let text = "# T ##\n\n**a** [b](c)\n";
    let markup = Markup::parse(text);
    assert_eq!(markdown(&Doc::new(text, &markup), 0..text.len()), text);
}

#[test]
fn the_html_keeps_formatting_headings_and_lists() {
    let text = "# Head\n\n**b** *i* <u>u</u> ~~s~~\n\n- one\n  1. two\n\n3. three\n\n> said\n\n---";
    let wanted = "<h1>Head</h1><p><strong>b</strong> <em>i</em> <u>u</u> <s>s</s></p>\
        <ul><li>one<ol><li>two</li></ol></li></ul><ol start=\"3\"><li>three</li></ol>\
        <blockquote><p>said</p></blockquote><hr>";
    assert_eq!(html(text), wanted);
}

#[test]
fn the_html_has_text_for_links_and_cues_and_escapes_the_rest() {
    assert_eq!(html("[a <b> & \"c\"](http://x) [[pause]] `<i>` ~d~"), "a &lt;b&gt; &amp; &quot;c&quot; [[pause]] &lt;i&gt; ~d~");
    assert_eq!(html("one\ntwo\\\nthree"), "one two<br>three");
    // An underline left open ends with its block, and a stray end is dropped.
    assert_eq!(html("<u>a\n\nb</u>"), "<p><u>a</u></p><p>b</p>");
}

/// No copy or key may panic or cut a character in half, wherever the selection is.
#[test]
fn copy_and_the_keys_take_any_selection() {
    for text in [
        "# H ##\r\nT\n---\n***\n- **a** <u>b\n  1. [[note: c]] ~~d~~\n> `e` [f](g) <http://h>\n\n```\ncode\n```\n---\n\u{e9} *\u{e9}*\n\n___",
        "```\n\n```\n---\n---\n> ```\n> a\n> ```\n> ***\n- ```\n  b\n  ```",
    ] {
        let markup = Markup::parse(text);
        let doc = Doc::new(text, &markup);
        let ends: Vec<usize> = text.char_indices().map(|(at, _)| at).chain([text.len()]).collect();
        for (from, &start) in ends.iter().enumerate() {
            for &end in &ends[from..] {
                html(&markdown(&doc, start..end));
                let done = [
                    Some(typing::replace(&doc, start..end, "")),
                    Some(typing::type_text(&doc, start..end, "-", None)),
                    Some(typing::type_text(&doc, start..end, "`", None)),
                    Some(typing::enter(&doc, start..end)),
                    typing::backspace(&doc, start..end, false, None),
                    typing::delete(&doc, start..end, false),
                ];
                for typed in done.into_iter().flatten() {
                    let selection = typed.change.selection;
                    assert!(typed.change.text.get(selection.clone()).is_some(), "{start}..{end}: {selection:?} in {:?}", typed.change.text);
                }
            }
        }
    }
}

/// With the caret anywhere it may be, one Backspace or Delete brings no hidden rule, fence,
/// link target or backtick into view.
#[test]
fn one_delete_key_exposes_no_hidden_syntax() {
    let text = "a\n\n---\n\nb\n***\nc\n\n```\nd\n```\n\ne [f](g) `h`\n```\ni\n```\n---";
    let shown = |text: &str| {
        let markup = Markup::parse(text);
        let doc = Doc::new(text, &markup);
        doc.visible(0..text.len()).map(|found| &text[found]).collect::<String>()
    };
    let markup = Markup::parse(text);
    let doc = Doc::new(text, &markup);
    for at in 0..=text.len() {
        if doc.rest(at, crate::caret::Bias::Back) != at {
            continue;
        }
        for (key, done) in [("Backspace", typing::backspace(&doc, at..at, false, None)), ("Delete", typing::delete(&doc, at..at, false))] {
            let Some(done) = done else { continue };
            let left = shown(&done.change.text);
            assert!(!left.contains(['-', '*', '`', '(', '[']), "{key} at {at}: {:?} shows {left:?}", done.change.text);
        }
    }
}
