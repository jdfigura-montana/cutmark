use super::*;

/// What `with` makes of the parse of `text`.
fn doc<R>(text: &str, with: impl FnOnce(&Doc<'_>) -> R) -> R {
    let markup = Markup::parse(text);
    with(&Doc::new(text, &markup))
}

/// Each place the caret gets to from `from`, a step at a time, until it stays put.
fn walk(text: &str, from: usize, step: impl Fn(&Doc<'_>, usize) -> usize) -> Vec<usize> {
    doc(text, |doc| {
        let mut places = Vec::new();
        let mut at = from;
        loop {
            let next = step(doc, at);
            if next == at || places.len() > text.len() {
                return places;
            }
            places.push(next);
            at = next;
        }
    })
}

#[test]
fn right_steps_over_hidden_markers_and_stays_with_what_it_passed() {
    // 8 is inside the bold, after the "d", and the step from there is over the space.
    assert_eq!(walk("a **bold** c", 0, |doc, at| doc.right(at)), [1, 2, 5, 6, 7, 8, 11, 12]);
}

#[test]
fn left_steps_over_hidden_markers_and_stays_with_what_it_passed() {
    // 10 is outside the bold, ahead of the space, and 4 is inside it, ahead of the "b".
    assert_eq!(walk("a **bold** c", 12, |doc, at| doc.left(at)), [11, 10, 7, 6, 5, 4, 1, 0]);
}

#[test]
fn left_and_right_reach_outside_formatting_at_either_end_of_the_script() {
    let text = "**ab**";
    doc(text, |doc| {
        assert_eq!(doc.left(2), 0);
        assert_eq!(doc.right(4), 6);
        assert_eq!(doc.right(0), 3);
        assert_eq!(doc.left(6), 3);
    });
}

#[test]
fn the_caret_goes_from_a_lines_text_to_the_text_of_the_next() {
    // A heading, an empty line and a list item.
    let text = "# T\n\n- i";
    assert_eq!(walk(text, 2, |doc, at| doc.right(at)), [3, 4, 7, 8]);
    assert_eq!(walk(text, 8, |doc, at| doc.left(at)), [7, 4, 3, 2]);
    // A heading's closing marks.
    assert_eq!(walk("## a ##\nb", 3, |doc, at| doc.right(at)), [4, 8, 9]);
    assert_eq!(walk("## a ##\nb", 9, |doc, at| doc.left(at)), [8, 4, 3]);
}

#[test]
fn a_rule_is_stepped_over_with_one_line_break() {
    assert_eq!(walk("a\n***", 1, |doc, at| doc.right(at)), [] as [usize; 0]);
    assert_eq!(walk("a\n***\nb", 1, |doc, at| doc.right(at)), [6, 7]);
    assert_eq!(walk("a\n***\nb", 7, |doc, at| doc.left(at)), [6, 1, 0]);
    // First and last in the script.
    assert_eq!(walk("---\nb", 5, |doc, at| doc.left(at)), [4]);
    assert_eq!(walk("a\n\n---", 1, |doc, at| doc.right(at)), [2]);
    doc("a\n\n***\n\nb", |doc| assert_eq!((doc.rest(4, Bias::Back), doc.rest(4, Bias::Forward)), (2, 7)));
    // Two together, and a script that is only a rule.
    assert_eq!(walk("a\n***\n***\nb", 1, |doc, at| doc.right(at)), [10, 11]);
    doc("***", |doc| assert_eq!(doc.rest(0, Bias::Back), 3));
}

#[test]
fn code_fences_links_and_backticks_are_stepped_over() {
    let text = "a\n```\nb\n```\nc";
    assert_eq!(walk(text, 1, |doc, at| doc.right(at)), [6, 7, 12, 13]);
    assert_eq!(walk(text, 13, |doc, at| doc.left(at)), [12, 7, 6, 1, 0]);
    assert_eq!(walk("[a](url) `b`", 0, |doc, at| doc.right(at)), [2, 9, 11, 12]);
}

#[test]
fn a_headings_underline_is_stepped_over_with_one_line_break() {
    let text = "Title\n===\nbody";
    assert_eq!(walk(text, 4, |doc, at| doc.right(at)), [5, 10, 11, 12, 13, 14]);
    assert_eq!(walk(text, 11, |doc, at| doc.left(at)), [10, 5, 4, 3, 2, 1, 0]);
    doc(text, |doc| {
        assert_eq!((doc.rest(8, Bias::Back), doc.rest(8, Bias::Forward)), (5, 10));
    });
}

#[test]
fn a_place_inside_hidden_syntax_or_ahead_of_a_lines_marks_is_left() {
    for (text, at, back, forward) in [
        ("a **bold**", 3, 2, 4),
        ("# Title", 0, 2, 2),
        ("# Title", 1, 2, 2),
        ("> > q", 1, 4, 4),
        ("- **b**", 0, 2, 2),
        ("- **b**", 3, 2, 4),
        ("## a ##", 6, 4, 4),
        ("x\r\ny", 2, 1, 1),
    ] {
        doc(text, |doc| assert_eq!((doc.rest(at, Bias::Back), doc.rest(at, Bias::Forward)), (back, forward), "{text:?} at {at}"));
    }
    // Both sides of a marker are places to be.
    doc("a **bold**", |doc| {
        for at in [2, 4, 8, 10] {
            assert_eq!(doc.rest(at, Bias::Back), at);
            assert_eq!(doc.rest(at, Bias::Forward), at);
        }
    });
}

#[test]
fn ctrl_arrows_go_by_the_words_that_show() {
    let text = "**bold** word [[pause]]";
    assert_eq!(walk(text, 0, |doc, at| doc.word_right(at)), [6, 13, 21, 23]);
    assert_eq!(walk(text, 23, |doc, at| doc.word_left(at)), [16, 9, 2, 0]);
}

#[test]
fn every_step_through_every_form_ends_where_the_caret_may_be() {
    let text = "# H ##\r\nSetext\n---\n\n- **a**\n  1. *b* [[note: c]]\n> <u>~~d~~</u>\n> > e\n***f***[[end]]\n# \n- \n";
    type Step = fn(&Doc<'_>, usize) -> usize;
    let forward: [Step; 2] = [|doc, at| doc.right(at), |doc, at| doc.word_right(at)];
    let back: [Step; 2] = [|doc, at| doc.left(at), |doc, at| doc.word_left(at)];
    for (steps, from, onward) in [(forward, 0, true), (back, text.len(), false)] {
        for step in steps {
            let start = doc(text, |doc| doc.rest(from, Bias::Back));
            let places = walk(text, start, step);
            assert!(places.len() > 8);
            let mut last = start;
            for at in places {
                assert!(if onward { at > last } else { at < last }, "from {last} to {at}");
                doc(text, |doc| assert_eq!((doc.rest(at, Bias::Back), doc.rest(at, Bias::Forward)), (at, at), "{at}"));
                last = at;
            }
        }
    }
}
