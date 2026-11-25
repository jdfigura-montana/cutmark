//! The spoken forms of one typed word.

use std::mem;

/// The most forms one word gets. Only long runs such as "2024-2025-2026-2027" reach it.
const MAX_FORMS: usize = 8;

/// The largest number read as a number. A longer one is read digit by digit.
const MAX_NUMBER: u32 = 9_999_999;

/// Contractions and what they stand for, keyed with a straight apostrophe. "cannot" has none
/// but goes both ways too.
const CONTRACTIONS: &[(&str, &[&str])] = &[
    ("aren't", &["are not"]),
    ("can't", &["cannot", "can not"]),
    ("cannot", &["can not", "cant"]),
    ("couldn't", &["could not"]),
    ("didn't", &["did not"]),
    ("doesn't", &["does not"]),
    ("don't", &["do not"]),
    ("hadn't", &["had not"]),
    ("hasn't", &["has not"]),
    ("haven't", &["have not"]),
    ("isn't", &["is not"]),
    ("mightn't", &["might not"]),
    ("mustn't", &["must not"]),
    ("needn't", &["need not"]),
    ("shan't", &["shall not"]),
    ("shouldn't", &["should not"]),
    ("wasn't", &["was not"]),
    ("weren't", &["were not"]),
    ("won't", &["will not"]),
    ("wouldn't", &["would not"]),
    ("i'm", &["i am"]),
    ("you're", &["you are"]),
    ("we're", &["we are"]),
    ("they're", &["they are"]),
    ("who're", &["who are"]),
    ("what're", &["what are"]),
    ("i've", &["i have"]),
    ("you've", &["you have"]),
    ("we've", &["we have"]),
    ("they've", &["they have"]),
    ("who've", &["who have"]),
    ("could've", &["could have"]),
    ("should've", &["should have"]),
    ("would've", &["would have"]),
    ("might've", &["might have"]),
    ("must've", &["must have"]),
    ("i'll", &["i will"]),
    ("you'll", &["you will"]),
    ("he'll", &["he will"]),
    ("she'll", &["she will"]),
    ("it'll", &["it will"]),
    ("we'll", &["we will"]),
    ("they'll", &["they will"]),
    ("that'll", &["that will"]),
    ("there'll", &["there will"]),
    ("who'll", &["who will"]),
    ("i'd", &["i would", "i had"]),
    ("you'd", &["you would", "you had"]),
    ("he'd", &["he would", "he had"]),
    ("she'd", &["she would", "she had"]),
    ("it'd", &["it would", "it had"]),
    ("we'd", &["we would", "we had"]),
    ("they'd", &["they would", "they had"]),
    ("that'd", &["that would", "that had"]),
    ("there'd", &["there would", "there had"]),
    ("who'd", &["who would", "who had"]),
    ("it's", &["it is", "it has"]),
    ("he's", &["he is", "he has"]),
    ("she's", &["she is", "she has"]),
    ("that's", &["that is", "that has"]),
    ("there's", &["there is", "there has"]),
    ("here's", &["here is"]),
    ("what's", &["what is", "what has"]),
    ("who's", &["who is", "who has"]),
    ("where's", &["where is", "where has"]),
    ("how's", &["how is", "how has"]),
    ("let's", &["let us"]),
];

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

/// From twenty up.
const TENS: [&str; 8] = ["twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// Ordinals that are not their number plus "th".
const IRREGULAR_ORDINALS: [(&str, &str); 7] =
    [("one", "first"), ("two", "second"), ("three", "third"), ("five", "fifth"), ("eight", "eighth"), ("nine", "ninth"), ("twelve", "twelfth")];

/// One piece of a word: a run of letters, a number or a symbol.
struct Atom {
    readings: Vec<String>,
    letters: bool,
}

/// Every way to say `word`, which holds no whitespace. Empty if there is nothing to say.
///
/// A dash, slash or ellipsis separates words. A hyphen does too, but the words it links may
/// also be heard as one.
pub(super) fn forms(word: &str) -> Vec<String> {
    let text = word.to_lowercase().replace(['\u{2018}', '\u{2019}', '\u{02bc}'], "'").replace("--", "\u{2014}").replace("...", "\u{2026}");
    let mut forms = vec![String::new()];
    for segment in text.split(['\u{2013}', '\u{2014}', '\u{2026}', '/']) {
        let readings = segment_forms(segment);
        if !readings.is_empty() {
            combine(&mut forms, &readings);
        }
    }
    forms.retain(|form| !form.is_empty());
    forms
}

/// The forms of text whose only separators are hyphens.
///
/// "follow-up" gives "followup" and "follow up". The joined form is left out when a part is not
/// a plain run of letters, as in "10-20".
fn segment_forms(segment: &str) -> Vec<String> {
    let parts: Vec<Vec<Atom>> = segment.split(['-', '\u{2010}', '\u{2011}']).map(atoms).filter(|atoms| !atoms.is_empty()).collect();
    if parts.is_empty() {
        return Vec::new();
    }
    let mut split = vec![String::new()];
    for atom in parts.iter().flatten() {
        combine(&mut split, &atom.readings);
    }
    let joinable = parts.len() > 1 && parts.iter().all(|part| matches!(part.as_slice(), [atom] if atom.letters));
    if joinable {
        let joined = parts.iter().flatten().filter_map(|atom| atom.readings.first()).map(String::as_str).collect();
        split.insert(0, joined);
        split.truncate(MAX_FORMS);
    }
    split
}

/// Replaces `forms` with each of them followed by each of `next`.
fn combine(forms: &mut Vec<String>, next: &[String]) {
    let heads = mem::take(forms);
    let pairs = heads.iter().flat_map(|head| next.iter().map(move |tail| (head, tail)));
    for (head, tail) in pairs {
        let form = if head.is_empty() { tail.clone() } else { format!("{head} {tail}") };
        if !forms.contains(&form) {
            forms.push(form);
        }
        if forms.len() == MAX_FORMS {
            break;
        }
    }
}

/// Cuts lowercase text into atoms. Punctuation is dropped, so "e.g." is the one atom "eg".
fn atoms(part: &str) -> Vec<Atom> {
    let chars: Vec<char> = part.chars().collect();
    let mut atoms = Vec::new();
    let mut letters = String::new();
    // A "$" is waiting for its number.
    let mut dollar = false;
    let mut at = 0;
    while let Some(&c) = chars.get(at) {
        // ".5" starts with its point. The point in "v.5" is punctuation.
        let leading_point = c == '.' && is_digit(&chars, at + 1) && (at == 0 || !chars[at - 1].is_alphanumeric());
        if c.is_ascii_digit() || leading_point {
            push_letters(&mut letters, &mut atoms);
            let (atom, end) = number(&chars, at, mem::take(&mut dollar));
            atoms.push(atom);
            at = end;
            continue;
        }
        if c.is_alphabetic() || c == '\'' {
            dollar = dollar && c == '\'';
            letters.push(c);
        } else if c == '$' {
            push_letters(&mut letters, &mut atoms);
            dollar = true;
        } else if let Some(name) = symbol_name(c) {
            push_letters(&mut letters, &mut atoms);
            atoms.push(Atom { readings: vec![name.to_owned()], letters: false });
        }
        at += 1;
    }
    push_letters(&mut letters, &mut atoms);
    atoms
}

fn symbol_name(c: char) -> Option<&'static str> {
    match c {
        '%' => Some("percent"),
        '&' => Some("and"),
        '+' => Some("plus"),
        '@' => Some("at"),
        '#' => Some("number"),
        _ => None,
    }
}

/// Ends a run of letters. Apostrophes around it are quote marks.
fn push_letters(letters: &mut String, atoms: &mut Vec<Atom>) {
    let typed = letters.trim_matches('\'');
    if !typed.is_empty() {
        let mut readings = vec![typed.replace('\'', "")];
        if let Some((_, long)) = CONTRACTIONS.iter().find(|(short, _)| *short == typed) {
            readings.extend(long.iter().map(|&reading| reading.to_owned()));
        }
        atoms.push(Atom { readings, letters: true });
    }
    letters.clear();
}

fn is_digit(chars: &[char], at: usize) -> bool {
    chars.get(at).is_some_and(char::is_ascii_digit)
}

/// Takes the digits starting at `at`.
fn digit_run(chars: &[char], at: &mut usize) -> String {
    let mut digits = String::new();
    while let Some(&c) = chars.get(*at).filter(|c| c.is_ascii_digit()) {
        digits.push(c);
        *at += 1;
    }
    digits
}

/// Reads the number starting at `at`, with its thousands separators, decimals and ordinal
/// ending. Also returns where it stops.
fn number(chars: &[char], mut at: usize, dollar: bool) -> (Atom, usize) {
    let mut digits = digit_run(chars, &mut at);

    let mut grouped = false;
    if (1..=3).contains(&digits.len()) {
        while chars.get(at) == Some(&',') && (1..=3).all(|n| is_digit(chars, at + n)) && !is_digit(chars, at + 4) {
            at += 1;
            digits.push_str(&digit_run(chars, &mut at));
            grouped = true;
        }
    }

    let mut decimals = String::new();
    while chars.get(at) == Some(&'.') && is_digit(chars, at + 1) {
        at += 1;
        decimals.push_str(" point ");
        decimals.push_str(&digit_by_digit(&digit_run(chars, &mut at)));
    }

    let suffix: String = chars.iter().skip(at).take(2).collect();
    let ordinal = decimals.is_empty() && matches!(suffix.as_str(), "st" | "nd" | "rd" | "th") && !chars.get(at + 2).is_some_and(|c| c.is_alphanumeric());

    // Only four bare digits can be a year: not "2,026", "$2026", "2026.5" or "2026th".
    let year = (!grouped && !dollar && !ordinal && decimals.is_empty() && digits.len() == 4).then(|| digits.parse().ok().and_then(year_words)).flatten();

    let whole = if digits.is_empty() { String::new() } else { integer_words(&digits) };
    let mut plain = format!("{whole}{decimals}").trim_start().to_owned();
    if ordinal {
        plain = ordinal_words(&plain);
        at += 2;
    }
    let readings = if !dollar {
        year.into_iter().chain([plain]).collect()
    } else if plain == "one" {
        vec!["one dollar".to_owned(), "one dollars".to_owned()]
    } else {
        vec![format!("{plain} dollars")]
    };
    (Atom { readings, letters: false }, at)
}

/// A whole number in words. One with a leading zero, or past [`MAX_NUMBER`], is read digit by
/// digit.
fn integer_words(digits: &str) -> String {
    let leading_zero = digits.len() > 1 && digits.starts_with('0');
    match digits.parse::<u32>() {
        Ok(value) if value <= MAX_NUMBER && !leading_zero => cardinal(value),
        _ => digit_by_digit(digits),
    }
}

fn digit_by_digit(digits: &str) -> String {
    let words: Vec<&str> = digits.chars().filter_map(|c| c.to_digit(10)).map(|digit| ONES[digit as usize]).collect();
    words.join(" ")
}

/// A number up to [`MAX_NUMBER`] in words, without "and".
fn cardinal(value: u32) -> String {
    if value == 0 {
        return ONES[0].to_owned();
    }
    let mut words = Vec::new();
    for (size, name) in [(1_000_000, Some("million")), (1_000, Some("thousand")), (1, None)] {
        let group = value / size % 1_000;
        if group > 0 {
            push_under_thousand(group, &mut words);
            words.extend(name);
        }
    }
    words.join(" ")
}

fn push_under_thousand(value: u32, words: &mut Vec<&'static str>) {
    if value >= 100 {
        words.extend([ONES[(value / 100) as usize], "hundred"]);
    }
    let rest = (value % 100) as usize;
    if rest >= 20 {
        words.push(TENS[rest / 10 - 2]);
        if !rest.is_multiple_of(10) {
            words.push(ONES[rest % 10]);
        }
    } else if rest > 0 {
        words.push(ONES[rest]);
    }
}

/// How a year from 1100 to 2099 is said, where that differs from the plain number.
fn year_words(value: u32) -> Option<String> {
    if !(1100..=2099).contains(&value) {
        return None;
    }
    let (century, rest) = (value / 100, value % 100);
    match rest {
        // 2000 is only ever "two thousand".
        0 if century == 20 => None,
        0 => Some(format!("{} hundred", cardinal(century))),
        1..=9 => Some(format!("{} oh {}", cardinal(century), cardinal(rest))),
        _ => Some(format!("{} {}", cardinal(century), cardinal(rest))),
    }
}

/// Makes the last word of a number an ordinal.
fn ordinal_words(cardinal: &str) -> String {
    let start = cardinal.rfind(' ').map_or(0, |space| space + 1);
    let (head, last) = cardinal.split_at(start);
    if let Some((_, ordinal)) = IRREGULAR_ORDINALS.iter().find(|(word, _)| *word == last) {
        format!("{head}{ordinal}")
    } else if let Some(stem) = last.strip_suffix('y') {
        format!("{head}{stem}ieth")
    } else {
        format!("{cardinal}th")
    }
}
