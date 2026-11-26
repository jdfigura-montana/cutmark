use std::io::Write;
use std::time::{Duration, Instant};

use prompter_core::index::{Token, WordIndex};
use prompter_core::script::parse;
use serde::Serialize;
use serde::de::DeserializeOwned;

fn index(md: &str) -> WordIndex {
    WordIndex::build(&parse(md))
}

/// The forms of one typed word.
fn forms(word: &str) -> Vec<String> {
    // The word goes after another one, so that "#" or "1100." is not read as Markdown.
    let index = index(&format!("x {word}"));
    assert_eq!(index.token_count(), 2, "{word:?}");
    index.tokens()[1].forms.clone()
}

fn check(cases: &[(&str, &[&str])]) {
    for (word, expected) in cases {
        assert_eq!(forms(word), *expected, "{word:?}");
    }
}

fn displays(tokens: &[Token]) -> Vec<&str> {
    tokens.iter().map(|token| token.display.as_str()).collect()
}

#[test]
fn contractions() {
    check(&[
        ("don't", &["dont", "do not"]),
        ("Don\u{2019}t", &["dont", "do not"]),
        ("it's", &["its", "it is", "it has"]),
        ("we're", &["were", "we are"]),
        ("I'm", &["im", "i am"]),
        ("can't", &["cant", "cannot", "can not"]),
        ("cannot", &["cannot", "can not", "cant"]),
        ("won't", &["wont", "will not"]),
        ("they'd", &["theyd", "they would", "they had"]),
        ("let\u{2019}s", &["lets", "let us"]),
        ("'don't!'", &["dont", "do not"]),
    ]);
}

#[test]
fn words_that_only_look_like_contractions() {
    check(&[("its", &["its"]), ("wont", &["wont"]), ("Ana's", &["anas"])]);
}

#[test]
fn years() {
    check(&[
        ("2026", &["twenty twenty six", "two thousand twenty six"]),
        ("1999", &["nineteen ninety nine", "one thousand nine hundred ninety nine"]),
        ("1905", &["nineteen oh five", "one thousand nine hundred five"]),
        ("1900", &["nineteen hundred", "one thousand nine hundred"]),
        ("2000", &["two thousand"]),
        ("1100.", &["eleven hundred", "one thousand one hundred"]),
        ("2099", &["twenty ninety nine", "two thousand ninety nine"]),
        ("1099", &["one thousand ninety nine"]),
        ("2100", &["two thousand one hundred"]),
        ("2,026", &["two thousand twenty six"]),
    ]);
}

#[test]
fn plain_numbers() {
    check(&[
        ("0", &["zero"]),
        ("7", &["seven"]),
        ("13", &["thirteen"]),
        ("40", &["forty"]),
        ("105", &["one hundred five"]),
        ("10,000", &["ten thousand"]),
        ("1000001", &["one million one"]),
        ("9,999,999", &["nine million nine hundred ninety nine thousand nine hundred ninety nine"]),
        ("10000000", &["one zero zero zero zero zero zero zero"]),
        ("007", &["zero zero seven"]),
        ("1,2", &["one two"]),
        ("3:30", &["three thirty"]),
    ]);
}

#[test]
fn decimals() {
    check(&[
        ("3.14", &["three point one four"]),
        ("0.5", &["zero point five"]),
        (".5", &["point five"]),
        ("12.05.", &["twelve point zero five"]),
        ("1.2.3", &["one point two point three"]),
    ]);
}

#[test]
fn ordinals() {
    check(&[
        ("1st", &["first"]),
        ("2nd", &["second"]),
        ("23rd,", &["twenty third"]),
        ("4th", &["fourth"]),
        ("12th", &["twelfth"]),
        ("20th", &["twentieth"]),
        ("100th", &["one hundredth"]),
        ("1,000th", &["one thousandth"]),
    ]);
}

#[test]
fn money() {
    check(&[
        ("$5", &["five dollars"]),
        ("$1", &["one dollar", "one dollars"]),
        ("$1,250", &["one thousand two hundred fifty dollars"]),
        ("$3.50", &["three point five zero dollars"]),
        ("($2026)", &["two thousand twenty six dollars"]),
    ]);
}

#[test]
fn symbols() {
    check(&[
        ("%", &["percent"]),
        ("&", &["and"]),
        ("+", &["plus"]),
        ("@", &["at"]),
        ("#", &["number"]),
        ("50%", &["fifty percent"]),
        ("#1", &["number one"]),
        ("R&D", &["r and d"]),
        ("@home", &["at home"]),
        ("100+", &["one hundred plus"]),
        ("MP3", &["mp three"]),
    ]);
}

#[test]
fn hyphens_and_other_joiners() {
    check(&[
        ("follow-up", &["followup", "follow up"]),
        ("state-of-the-art,", &["stateoftheart", "state of the art"]),
        ("pre-", &["pre"]),
        ("10-20", &["ten twenty"]),
        ("COVID-19", &["covid nineteen"]),
        ("here\u{2014}now", &["here now"]),
        ("here--now", &["here now"]),
        ("wait...what", &["wait what"]),
        ("and/or", &["and or"]),
    ]);
}

#[test]
fn forms_of_a_long_run_are_capped() {
    assert_eq!(forms("2024/2025/2026/2027").len(), 8);
}

#[test]
fn punctuation_stays_in_display_only() {
    let index = index("\"Hello,\" world\u{2026} (e.g.) done?!");

    assert_eq!(displays(index.tokens()), ["\"Hello,\"", "world\u{2026}", "(e.g.)", "done?!"]);
    let forms: Vec<&str> = index.tokens().iter().map(|token| token.forms[0].as_str()).collect();
    assert_eq!(forms, ["hello", "world", "eg", "done"]);
}

#[test]
fn punctuation_alone_gives_no_token() {
    let index = index("one \u{2014} two ... !");

    assert_eq!(displays(index.tokens()), ["one", "two"]);
}

#[test]
fn a_style_change_does_not_end_a_word() {
    let script = parse("Say **hello**, then **un**done");
    let index = WordIndex::build(&script);

    assert_eq!(displays(index.tokens()), ["Say", "hello,", "then", "undone"]);
    let places: Vec<(u16, u32, u32)> = index.tokens().iter().map(|t| (t.span, t.char_range.start, t.char_range.end)).collect();
    // The spans are "Say ", "hello", ", then ", "un" and "done".
    assert_eq!(places, [(0, 0, 3), (1, 0, 6), (2, 2, 6), (3, 0, 6)]);
}

#[test]
fn char_ranges_count_chars_not_bytes() {
    let index = index("caf\u{e9} \u{201c}ol\u{e9}\u{201d}");

    assert_eq!(index.tokens()[0].char_range, 0..4);
    assert_eq!(index.tokens()[1].char_range, 5..10);
}

#[test]
fn cues_notes_and_rules_give_no_tokens() {
    let index = index("[[note: show the menu]]\n\n[[pause]]\n\n---\n\n[[mark: B-roll: desk]]\n\n[[end]]");

    assert_eq!(index.token_count(), 0);
    assert_eq!(index.spoken_words(), 0);
}

#[test]
fn cues_inside_text_give_no_tokens() {
    let index = index("# Intro [[note: smile]]\n\nOne [[pause]] two [[note: three]]");

    assert_eq!(displays(index.tokens()), ["Intro", "One", "two"]);
    assert!(index.tokens().iter().enumerate().all(|(at, token)| token.index as usize == at));
}

#[test]
fn spoken_words_counts_the_first_form() {
    let index = index("In 2026 we don't follow-up & pay $5.");

    assert_eq!(index.token_count(), 8);
    // "twenty twenty six" is three words and "five dollars" two.
    assert_eq!(index.spoken_words(), 11);
}

#[test]
fn tokens_map_back_to_blocks_and_lines() {
    let md = "# Title here\n\nFirst line\nstill first.\n\n[[pause]]\n\n- item one\n- item two\n";
    let script = parse(md);
    let index = WordIndex::build(&script);

    assert_eq!(displays(index.tokens_in_block(0)), ["Title", "here"]);
    assert_eq!(displays(index.tokens_in_block(1)), ["First", "line", "still", "first."]);
    assert!(index.tokens_in_block(2).is_empty());
    assert_eq!(displays(index.tokens_in_block(4)), ["item", "two"]);
    assert!(index.tokens_in_block(99).is_empty());

    // "still" is typed on line 4, in the paragraph that starts on line 3.
    assert_eq!(index.block_of(4), Some(1));
    assert_eq!(index.source_line_of(4), Some(3));
    assert_eq!(index.block_of(9), Some(4));
    assert_eq!(index.source_line_of(9), Some(9));
    assert_eq!(index.block_of(10), None);
    assert_eq!(index.source_line_of(10), None);
}

#[test]
fn a_cache_is_checked_against_the_source() {
    let index = index("Some words.");

    assert!(index.is_current(&parse("Some words.")));
    assert!(!index.is_current(&parse("Some other words.")));
    // The blocks match, but their lines moved.
    assert!(!index.is_current(&parse("\nSome words.")));
}

#[test]
fn the_index_can_be_cached() {
    fn cacheable<T: Serialize + DeserializeOwned>() {}
    cacheable::<WordIndex>();
}

/// A script of at least `words` words with headings, lists, cues and numbers.
fn long_script(words: usize) -> String {
    let mut md = String::new();
    let mut count = 0;
    let mut section = 0;
    while count < words {
        section += 1;
        let part = format!(
            "## Section {section}\n\n\
             We're **really** glad you're here, and we don't plan to waste a minute of it. \
             In 2026 the follow-up cost $1,250 & saved 37.5% on the 23rd try. [[pause]] \
             That's the part nobody can't repeat often enough, so listen closely.\n\n\
             - First, open the settings menu\n\
             - Then pick option #{section} [[mark: B-roll: menu]]\n\
             - It's the *last* one on the list\n\n\
             [[note: look at the camera]]\n\n"
        );
        count += part.split_whitespace().count();
        md.push_str(&part);
    }
    md
}

#[test]
#[ignore = "timing; run with --release"]
fn index_10k() {
    let md = long_script(10_000);

    let started = Instant::now();
    let script = parse(&md);
    let index = WordIndex::build(&script);
    let elapsed = started.elapsed();

    // Written straight to stderr, so the time shows without --nocapture.
    let report = writeln!(
        std::io::stderr(),
        "index_10k: {} typed words, {} tokens, parse and index took {elapsed:?}",
        md.split_whitespace().count(),
        index.token_count()
    );
    assert!(report.is_ok());
    assert!(index.token_count() > 8_000);
    assert!(elapsed < Duration::from_millis(50), "{elapsed:?}");
}

#[test]
fn underline_and_strikethrough_do_not_change_the_spoken_words() {
    let spoken = |md| index(md).tokens().iter().map(|token| (token.display.clone(), token.forms.clone())).collect::<Vec<_>>();
    // Struck words are still spoken, and a style that changes partway does not end a word.
    let plain = spoken("Don't open the settings, in 2026.");
    assert_eq!(spoken("<u>Don't</u> ~~open the~~ set<u>ti</u>ngs, in ~~20~~26."), plain);
    assert_eq!(plain.len(), 6);
}
