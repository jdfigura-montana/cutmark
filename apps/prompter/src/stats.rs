//! The open script's word count and read time, counted from its word index off the UI thread.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use prompter_core::index::WordIndex;
use prompter_core::script;

/// The reading pace behind the read time.
const WORDS_PER_MINUTE: usize = 150;
/// The shortest time between the starts of two rebuilds.
const REBUILD_EVERY: Duration = Duration::from_millis(250);

/// Whole minutes to read `words` spoken words, rounded up.
pub fn read_minutes(words: usize) -> usize {
    words.div_ceil(WORDS_PER_MINUTE)
}

/// The title bar's caption, for example "1,240 words · 9 min".
pub fn caption(words: usize) -> String {
    format!("{} · {} min", word_label(words), read_minutes(words))
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

    #[test]
    fn cues_are_not_counted_and_numbers_count_as_spoken() {
        assert_eq!(spoken_words("Hello [[pause]] there"), 2);
        assert_eq!(spoken_words("I have 25"), 4);
    }
}
