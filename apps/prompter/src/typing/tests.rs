use super::*;

/// A script and a selection written as text with `|` at the caret, or at each end of the
/// selection.
fn marked(text: &str) -> (String, Range<usize>) {
    let start = text.find('|').unwrap_or(text.len());
    let rest = text.replacen('|', "", 1);
    let end = rest.find('|').unwrap_or(start);
    (rest.replacen('|', "", 1), start..end)
}

fn shown(typed: &Typed) -> String {
    let mut text = typed.change.text.clone();
    let selection = typed.change.selection.clone();
    if !selection.is_empty() {
        text.insert(selection.end, '|');
    }
    text.insert(selection.start, '|');
    text
}

/// What `key` does to a marked script.
fn after(text: &str, key: impl FnOnce(&Doc<'_>, Range<usize>) -> Option<Typed>) -> String {
    let (text, selection) = marked(text);
    let markup = Markup::parse(&text);
    key(&Doc::new(&text, &markup), selection).map_or_else(|| "nothing".to_owned(), |typed| shown(&typed))
}

/// A script being typed in, which keeps what carries from one key to the next.
struct Page {
    typed: Typed,
}

impl Page {
    fn new(text: &str) -> Self {
        let (text, selection) = marked(text);
        Self { typed: Typed { change: Change { text, selection }, carry: None } }
    }

    fn key(&mut self, key: impl FnOnce(&Doc<'_>, Range<usize>, Option<&Carry>) -> Option<Typed>) -> String {
        let markup = Markup::parse(&self.typed.change.text);
        let doc = Doc::new(&self.typed.change.text, &markup);
        if let Some(typed) = key(&doc, self.typed.change.selection.clone(), self.typed.carry.as_ref()) {
            self.typed = typed;
        }
        shown(&self.typed)
    }

    fn type_text(&mut self, text: &str) -> String {
        for c in text.chars() {
            self.key(|doc, selection, carry| Some(type_text(doc, selection, &c.to_string(), carry)));
        }
        shown(&self.typed)
    }

    fn backspace(&mut self) -> String {
        self.key(|doc, selection, carry| backspace(doc, selection, false, carry))
    }
}

#[test]
fn typing_inside_formatting_goes_on_with_it() {
    for (open, close) in [("**", "**"), ("*", "*"), ("<u>", "</u>"), ("~~", "~~")] {
        let mut page = Page::new(&format!("a {open}bo|ld{close} z"));
        assert_eq!(page.type_text("x"), format!("a {open}box|ld{close} z"));
        let mut page = Page::new(&format!("a {open}bold|{close} z"));
        assert_eq!(page.type_text("er"), format!("a {open}bolder|{close} z"));
        // After the closing marker, it does not.
        let mut page = Page::new(&format!("a {open}bold{close}| z"));
        assert_eq!(page.type_text("er"), format!("a {open}bold{close}er| z"));
    }
}

#[test]
fn words_typed_at_the_end_of_formatting_stay_in_it() {
    for (open, close) in [("**", "**"), ("*", "*"), ("<u>", "</u>"), ("~~", "~~")] {
        let mut page = Page::new(&format!("{open}one|{close}"));
        assert_eq!(page.type_text(" two three"), format!("{open}one two three|{close}"));
    }
    // The space waits outside, so that Markdown still reads the bold.
    let mut page = Page::new("**one|**");
    assert_eq!(page.type_text(" "), "**one** |");
    assert_eq!(page.type_text(" "), "**one**  |");
    assert_eq!(page.backspace(), "**one** |");
    assert_eq!(page.backspace(), "**one|**");
    assert_eq!(page.type_text("s"), "**ones|**");
    // Both kinds at once.
    let mut page = Page::new("***one|***");
    assert_eq!(page.type_text(" two"), "***one two|***");
}

#[test]
fn a_space_typed_at_the_start_of_formatting_goes_ahead_of_it() {
    let mut page = Page::new("a**|bold**");
    assert_eq!(page.type_text(" "), "a **|bold**");
}

#[test]
fn markdown_typed_a_character_at_a_time_comes_out_as_typed() {
    for text in [
        "**bold** text",
        "*slanted* and ~~gone~~ so",
        "<u>under</u> it",
        "# Heading one",
        "- item one",
        "1. first",
        "> quoted words",
        "[[pause]] then [[mark: B-roll: desk]] on",
        "a - b = c * d",
        "***",
    ] {
        let mut page = Page::new("|");
        assert_eq!(page.type_text(text), format!("{text}|"), "{text}");
    }
}

#[test]
fn a_dash_typed_under_text_does_not_turn_that_text_into_a_heading() {
    for mark in ["-", "="] {
        let mut page = Page::new("text\n|");
        assert_eq!(page.type_text(mark), format!("text\n\n{mark}|"));
    }
    let mut page = Page::new("text\n|");
    assert_eq!(page.type_text("- item"), "text\n\n- item|");
    let mut page = Page::new("> text\n> |");
    assert_eq!(page.type_text("-"), "> text\n>\n> -|");
}

#[test]
fn backspace_deletes_what_can_be_seen_and_no_hidden_marker() {
    let key = |text| after(text, |doc, selection| backspace(doc, selection, false, None));
    // After the closing marker, the last letter goes, not an asterisk.
    assert_eq!(key("**bold**| x"), "**bol|** x");
    assert_eq!(key("a **|bold**"), "a|**bold**");
    // The last of what is formatted takes its markers with it.
    assert_eq!(key("a **b|** c"), "a | c");
    assert_eq!(key("a <u>**b|**</u> c"), "a | c");
    assert_eq!(key("x [[pause]] y|"), "x [[pause]] |");
    assert_eq!(key("[[note: ab|]]"), "[[note: a|]]");
    // Two bolds that come to touch become one.
    assert_eq!(key("**a** |**b**"), "**a|b**");
    // A space left at the end of bold goes outside it.
    assert_eq!(key("**ab c|**"), "**ab** |");
    assert_eq!(key("one\n|two"), "one|two");
}

#[test]
fn ctrl_backspace_and_delete_go_by_the_words_that_show() {
    assert_eq!(after("one **two three**|", |doc, selection| backspace(doc, selection, true, None)), "one **two** |");
    assert_eq!(after("|**two** three", |doc, selection| delete(doc, selection, true)), "| three");
}

#[test]
fn delete_takes_the_next_character_that_shows() {
    let key = |text| after(text, |doc, selection| delete(doc, selection, false));
    assert_eq!(key("a|**bold**"), "a|**old**");
    assert_eq!(key("**bold|** x"), "**bold|**x");
    // Joined lines lose the marks that only work at a line's start or end.
    assert_eq!(key("- a|\n- b"), "- a|b");
    assert_eq!(key("a|\n# H"), "a|H");
    assert_eq!(key("# a| ##\nb"), "# a|b");
    // An empty line goes, and the line under it keeps its own.
    assert_eq!(key("|\n- b"), "- |b");
    assert_eq!(key("- |\nplain"), "|plain");
    assert_eq!(key("end|"), "nothing");
}

#[test]
fn a_selection_deletes_what_can_be_seen_of_it() {
    let clear = |text| after(text, |doc, selection| Some(replace(doc, selection, "")));
    assert_eq!(clear("a **bo|ld** c|"), "a **bo|**");
    assert_eq!(clear("a| **bo|ld**"), "a|**ld**");
    assert_eq!(clear("a **|bold|** c"), "a | c");
    assert_eq!(clear("# Ti|tle\n\n- it|em"), "# Ti|em");
    // Select all takes everything.
    assert_eq!(clear("|# Title\n\n- item|"), "|");
    // Whole lines go with their marks.
    assert_eq!(clear("# |Head\n- |item"), "- |item");
    // Typing over a selection inside bold stays bold.
    assert_eq!(after("a **|bold|** c", |doc, selection| Some(replace(doc, selection, "x"))), "a **x|** c");
    // Over bold and its markers, it does not.
    assert_eq!(after("a| **bold** |c", |doc, selection| Some(replace(doc, selection, "x"))), "ax|c");
}

#[test]
fn a_heading_with_an_underline_keeps_its_form_or_takes_the_other_one() {
    let key = |text| after(text, |doc, selection| backspace(doc, selection, false, None));
    assert_eq!(key("Title\n===\n|body"), "# Title|body");
    assert_eq!(key("|Title\n==="), "|Title");
    assert_eq!(after("a\n\n|Title|\n===\nb", |doc, selection| Some(replace(doc, selection, ""))), "a\n\n|\nb");
}

#[test]
fn backspace_at_the_start_of_a_list_item_or_heading_takes_the_formatting_away() {
    for (before, wanted) in [
        ("# |Title", "|Title"),
        ("### |", "|"),
        ("- |item", "|item"),
        ("1. |item", "|item"),
        ("> |quoted", "|quoted"),
        ("- **|bold**", "**|bold**"),
        // An empty line keeps the new paragraph out of the list above it.
        ("- a\n- |b", "- a\n\n|b"),
        ("- a\n- |", "- a\n\n|"),
        ("- a\n  - |b", "- a\n\n|b"),
        ("1. a\n2. |b\n3. c", "1. a\n\n|b\n\n3. c"),
        ("text\n# |Head", "text\n|Head"),
    ] {
        assert_eq!(after(before, |doc, selection| backspace(doc, selection, false, None)), wanted, "{before:?}");
        assert_eq!(after(before, |doc, selection| backspace(doc, selection, true, None)), wanted, "Ctrl, {before:?}");
    }
    // What is typed on the freed line is a paragraph of its own.
    let mut page = Page::new("- a\n- |");
    page.backspace();
    assert_eq!(page.type_text("b"), "- a\n\nb|");
    assert!(Markup::parse(&page.typed.change.text).items.len() == 1);
}

fn entered(text: &str) -> String {
    after(text, |doc, selection| Some(enter(doc, selection)))
}

#[test]
fn enter_in_a_list_starts_the_next_item() {
    for (before, wanted) in [
        ("- a|", "- a\n- |"),
        ("* a|", "* a\n* |"),
        ("3. a|", "3. a\n4. |"),
        ("1) a|", "1) a\n2) |"),
        ("  - a|", "  - a\n  - |"),
        ("> - a|", "> - a\n> - |"),
        ("- ab| cd", "- ab\n- |cd"),
        ("- |ab", "- \n- |ab"),
        ("- **bo|ld**", "- **bo**\n- **|ld**"),
        ("- **one |two**", "- **one**\n- **|two**"),
        ("- **bold|**", "- **bold**\n- |"),
    ] {
        assert_eq!(entered(before), wanted, "{before:?}");
    }
}

#[test]
fn enter_on_an_empty_item_ends_the_list() {
    assert_eq!(entered("- a\n- |"), "- a\n\n|");
    assert_eq!(entered("1. a\n2. |"), "1. a\n\n|");
    assert_eq!(entered("- |"), "|");
    assert_eq!(entered("- a\n- |\n- c"), "- a\n\n|\n- c");
    // A nested one goes out a level first.
    assert_eq!(entered("- a\n  - b\n  - |"), "- a\n  - b\n- |");
    let mut page = Page::new("- a|");
    page.key(|doc, selection, _| Some(enter(doc, selection)));
    page.key(|doc, selection, _| Some(enter(doc, selection)));
    assert_eq!(page.type_text("after"), "- a\n\nafter|");
}

#[test]
fn enter_keeps_headings_and_formatting_whole() {
    for (before, wanted) in [
        ("# Title|", "# Title\n|"),
        ("# |Title", "\n# |Title"),
        ("# Ti|tle", "# Ti\n|tle"),
        ("# a| ##", "# a ##\n|"),
        ("# a|b ##", "# a\n|b"),
        ("Title|\n===", "Title\n===\n|"),
        ("**bold|**", "**bold**\n|"),
        ("a **|bold**", "a \n|**bold**"),
        ("**bo|ld**", "**bo\n|ld**"),
        ("[[pa|use]]", "[[pause]]\n|"),
        ("one|", "one\n|"),
        ("one |two| three", "one \n| three"),
    ] {
        assert_eq!(entered(before), wanted, "{before:?}");
    }
}

#[test]
fn no_key_makes_an_underline_of_the_line_above() {
    // Splitting off an empty nested item right under its parent's text.
    assert_eq!(entered("- a\n  - |b"), "- a\n\n  - \n  - |b");
    assert!(underlines(&Markup::parse("- a\n\n  - \n  - b")).next().is_none());
}
