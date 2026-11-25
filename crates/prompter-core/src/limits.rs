//! The script size limit (docs/suite/prompter.md, "Limits").

use pulldown_cmark::{Event, Parser, Tag};

/// The most words one script may hold.
pub const MAX_WORDS: usize = 50_000;

/// Splits Markdown into parts of at most `max_words` whitespace-separated words.
///
/// Each cut is made at the last heading that keeps the part within the limit. Where no heading
/// does, the cut is made after the last blank line, or failing that between two words. The
/// parts joined together are the input, and input within the limit comes back as one part.
/// A `max_words` of 0 is treated as 1.
pub fn split_at_heading(md: &str, max_words: usize) -> Vec<String> {
    let max_words = max_words.max(1);
    let words = word_starts(md);
    if words.len() <= max_words {
        return vec![md.to_owned()];
    }
    let headings = heading_starts(md);
    let paragraphs = paragraph_starts(md);

    let mut parts = Vec::new();
    let mut start = 0;
    let mut first_word = 0;
    while words.len() - first_word > max_words {
        // The first word that does not fit in this part.
        let limit = words[first_word + max_words];
        let after = words[first_word];
        let cut = last_cut(&headings, after, limit).or_else(|| last_cut(&paragraphs, after, limit)).unwrap_or(limit);
        parts.push(md[start..cut].to_owned());
        start = cut;
        first_word = words.partition_point(|&at| at < cut);
    }
    parts.push(md[start..].to_owned());
    parts
}

/// The last of the sorted `cuts` that is past `after` and no later than `limit`.
fn last_cut(cuts: &[usize], after: usize, limit: usize) -> Option<usize> {
    let count = cuts.partition_point(|&at| at <= limit);
    cuts[..count].last().copied().filter(|&at| at > after)
}

fn word_starts(md: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut in_word = false;
    for (at, c) in md.char_indices() {
        if !c.is_whitespace() && !in_word {
            starts.push(at);
        }
        in_word = !c.is_whitespace();
    }
    starts
}

/// The start of the line of each heading that is not inside a list or a quote.
fn heading_starts(md: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut depth = 0_usize;
    for (event, range) in Parser::new(md).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if depth == 0 && matches!(tag, Tag::Heading { .. }) {
                    starts.push(md[..range.start].rfind('\n').map_or(0, |at| at + 1));
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    starts
}

/// The start of each line that follows a blank line.
fn paragraph_starts(md: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut at = 0;
    for line in md.split_inclusive('\n') {
        at += line.len();
        if line.trim().is_empty() {
            starts.push(at);
        }
    }
    starts
}
