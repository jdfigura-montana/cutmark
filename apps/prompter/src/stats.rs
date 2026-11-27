//! Word counts and read times, for the title bar and the stats bar. The open script's count
//! comes from its word index, made off the UI thread.

use std::ops::Range;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use prompter_core::index::WordIndex;
use prompter_core::script::{self, Block, BlockKind, Script, Span};

use crate::markup::Markup;

/// The reading pace behind the read time.
const WORDS_PER_MINUTE: usize = 150;
/// The shortest time between the starts of two rebuilds.
const REBUILD_EVERY: Duration = Duration::from_millis(250);

/// Whole minutes to read `words` spoken words, rounded up.
pub fn read_minutes(words: usize) -> usize {
    words.div_ceil(WORDS_PER_MINUTE)
}

/// The whole script's count, as the title bar and the stats bar show it, for example
/// "1,240 words · 9 min".
pub fn caption(words: usize) -> String {
    format!("{} · {} min", word_label(words), read_minutes(words))
}

/// The stats bar's text with text selected, for example "Selection: 86 words · 35 s". Under
/// a minute the time is in seconds, and from there in minutes, both rounded up.
pub fn selection_caption(words: usize) -> String {
    let seconds = words.saturating_mul(60).div_ceil(WORDS_PER_MINUTE);
    if seconds < 60 { format!("Selection: {} · {seconds} s", word_label(words)) } else { format!("Selection: {}", caption(words)) }
}

/// The spoken words in the bytes `selection` of the text `markup` was parsed from. A word
/// that is partly selected counts. Notes and other cues do not, even selected from inside.
pub fn selected_words(markup: &Markup, selection: Range<usize>) -> usize {
    let mut blocks: Vec<Block> = Vec::new();
    let mut last = None;
    for said in &markup.said {
        // A character the source writes as several bytes, as "&amp;" does, is taken to be at
        // the first of them.
        let within = |at: usize| selection.contains(&(said.at + at));
        let text: String = said.text.char_indices().filter(|&(at, _)| within(at)).map(|(_, c)| c).collect();
        if text.is_empty() {
            continue;
        }
        let span = Span { text, bold: false, italic: false, underline: false, strike: false };
        match blocks.last_mut() {
            Some(block) if last == Some(said.block) => block.spans.push(span),
            _ => {
                let id = u32::try_from(blocks.len()).unwrap_or(u32::MAX);
                blocks.push(Block { id, kind: BlockKind::Paragraph, spans: vec![span], source_line: 1 });
                last = Some(said.block);
            }
        }
    }
    WordIndex::build(&Script { blocks, source_hash: 0 }).spoken_words()
}

/// A word count as written out, for example "1,240 words".
pub fn word_label(words: usize) -> String {
    let digits = words.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let noun = if words == 1 { "word" } else { "words" };
    format!("{grouped} {noun}")
}

pub fn spoken_words(markdown: &str) -> usize {
    WordIndex::build(&script::parse(markdown)).spoken_words()
}

/// The spoken word count of a text that keeps changing. One rebuild runs at a time.
pub struct WordCount {
    /// `None` until the first count is in.
    words: Option<usize>,
    /// The text changed after the last rebuild started.
    stale: bool,
    started: Option<Instant>,
    worker: Option<Receiver<usize>>,
}

impl WordCount {
    /// A count that still has to be made.
    pub fn new() -> Self {
        Self { words: None, stale: true, started: None, worker: None }
    }

    pub fn words(&self) -> Option<usize> {
        self.words
    }

    pub fn text_changed(&mut self) {
        self.stale = true;
    }

    /// Takes in a finished count, and starts the next one when it is due.
    pub fn tick(&mut self, ctx: &egui::Context, text: &str, now: Instant) {
        if let Some(worker) = &self.worker {
            match worker.try_recv() {
                Ok(words) => self.words = Some(words),
                // The worker asks for a repaint when it is done.
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => log::warn!("the word count stopped early"),
            }
            self.worker = None;
        }
        if !self.stale {
            return;
        }
        let wait = self.started.map_or(Duration::ZERO, |started| REBUILD_EVERY.saturating_sub(now.saturating_duration_since(started)));
        if !wait.is_zero() {
            ctx.request_repaint_after(wait);
            return;
        }
        self.stale = false;
        self.started = Some(now);
        let (send, receive) = mpsc::channel();
        self.worker = Some(receive);
        let (text, ctx) = (text.to_owned(), ctx.clone());
        std::thread::spawn(move || {
            // The script may have been closed since, and then nothing is listening.
            let _ = send.send(spoken_words(&text));
            ctx.request_repaint();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_time_rounds_up_to_whole_minutes() {
        let minutes = [0, 1, 149, 150, 151, 300, 301].map(read_minutes);
        assert_eq!(minutes, [0, 1, 1, 1, 2, 2, 3]);
    }

    #[test]
    fn the_caption_groups_thousands() {
        assert_eq!(caption(0), "0 words · 0 min");
        assert_eq!(caption(1), "1 word · 1 min");
        assert_eq!(caption(999), "999 words · 7 min");
        assert_eq!(caption(1_240), "1,240 words · 9 min");
        assert_eq!(caption(1_000_000), "1,000,000 words · 6667 min");
    }

    /// The count for the first `wanted` in `text`, selected.
    fn count(text: &str, wanted: &str) -> usize {
        let start = text.find(wanted).unwrap();
        selected_words(&Markup::parse(text), start..start + wanted.len())
    }

    #[test]
    fn a_selection_gives_its_time_in_seconds_and_from_a_minute_in_minutes() {
        assert_eq!(selection_caption(0), "Selection: 0 words · 0 s");
        assert_eq!(selection_caption(1), "Selection: 1 word · 1 s");
        assert_eq!(selection_caption(86), "Selection: 86 words · 35 s");
        assert_eq!(selection_caption(147), "Selection: 147 words · 59 s");
        assert_eq!(selection_caption(150), "Selection: 150 words · 1 min");
        assert_eq!(selection_caption(1_240), "Selection: 1,240 words · 9 min");
    }

    #[test]
    fn selecting_everything_counts_what_the_whole_script_does() {
        for text in [
            "# 1. Intro\n\n- **bold**, and *more*\n- 25% off in 2026\n\n> quoted <u>under</u> ~~gone~~",
            "one [[note: do not say this]] two[[pause]]three [[mark: B-roll: desk]]\n\n[[end]]\n\n---",
            "a [link text](https://example.com/some/long/path) and `code [[pause]]` &amp; \\*stars\\*",
            "```rust\nlet x = 1;\n```\n\n<div>\nraw html\n</div>\n\nsoft\nbreak  \nhard",
            "caf\u{e9} don't **wo**rld, [[not a cue]] [[pause]]]\r\nnext\r\n",
            "",
        ] {
            assert_eq!(selected_words(&Markup::parse(text), 0..text.len()), spoken_words(text), "{text:?}");
        }
    }

    #[test]
    fn a_selection_counts_only_the_spoken_words_in_it() {
        let text = "one two [[note: show the menu]] three [[pause]] four";
        assert_eq!(count(text, "two [[note: show the menu]] three"), 2);
        // Inside a note, and across its end.
        assert_eq!(count(text, "show the menu"), 0);
        assert_eq!(count(text, "menu]] three"), 1);
        assert_eq!(count(text, "[[pause]]"), 0);
        // Part of a word is a word, and formatting does not split one.
        assert_eq!(count(text, "hre"), 1);
        assert_eq!(count("**wo**rld again", "wo**rl"), 1);
        assert_eq!(count("`ab` cd", "b` c"), 2);
        assert_eq!(count("# Title\n\nbody", "# Title"), 1);
        assert_eq!(count("I have 25", "25"), 2);
    }

    #[test]
    fn cues_are_not_counted_and_numbers_count_as_spoken() {
        assert_eq!(spoken_words("Hello [[pause]] there"), 2);
        assert_eq!(spoken_words("I have 25"), 4);
    }
}
