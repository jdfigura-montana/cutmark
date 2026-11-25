use prompter_core::limits::{MAX_WORDS, split_at_heading};

fn words(md: &str) -> usize {
    md.split_whitespace().count()
}

/// Splits and checks that nothing is lost and no part is over the limit.
fn split(md: &str, max_words: usize) -> Vec<String> {
    let parts = split_at_heading(md, max_words);
    assert_eq!(parts.concat(), md);
    assert!(parts.iter().all(|p| words(p) <= max_words.max(1)), "{parts:?}");
    parts
}

#[test]
fn within_the_limit_is_one_part() {
    assert_eq!(split("# A\n\none two\n", 4), ["# A\n\none two\n"]);
    assert_eq!(split("", 4), [""]);
}

#[test]
fn cuts_at_the_last_heading_that_fits() {
    // Each section is 4 words, the "#" included.
    let md = "# A\n\none two\n\n# B\n\nthree four\n\n# C\n\nfive six\n";
    assert_eq!(split(md, 9), ["# A\n\none two\n\n# B\n\nthree four\n\n", "# C\n\nfive six\n"]);
    assert_eq!(split(md, 7).len(), 3);
}

#[test]
fn a_heading_right_at_the_limit_starts_the_next_part() {
    assert_eq!(split("a b c\n# D\ne\n", 3), ["a b c\n", "# D\ne\n"]);
}

#[test]
fn setext_headings_count_but_code_and_quoted_ones_do_not() {
    assert_eq!(split("a b\n\nTitle\n===\nc d\n", 4), ["a b\n\n", "Title\n===\nc d\n"]);
    assert_eq!(split("a\n\n```\n# not\n```\n\nb c d\n", 6)[0], "a\n\n```\n# not\n```\n\n");
    assert_eq!(split("a b\n\n> # quoted\n\nc d\n", 5), ["a b\n\n> # quoted\n\n", "c d\n"]);
}

#[test]
fn without_a_heading_cuts_after_a_blank_line() {
    assert_eq!(split("a b\r\n\r\nc d e\r\n", 4), ["a b\r\n\r\n", "c d e\r\n"]);
}

#[test]
fn without_any_break_cuts_between_words() {
    assert_eq!(split("é b c d e", 2), ["é b ", "c d ", "e"]);
    assert_eq!(split("a b", 0), ["a ", "b"]);
}

#[test]
fn full_size_script() {
    let section = format!("## Part\n\n{}\n\n", "word ".repeat(998));
    let md = section.repeat(120);
    let parts = split(&md, MAX_WORDS);
    assert_eq!(parts.len(), 3);
    assert_eq!(words(&parts[0]), MAX_WORDS);
    assert!(parts.iter().all(|p| p.starts_with("## Part")));
}
