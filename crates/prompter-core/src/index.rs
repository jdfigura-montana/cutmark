//! The word index Voice Follow tracks (docs/suite/prompter.md, "Word index").

mod normalize;

use std::mem;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::script::{Block, Script};

/// Bump this when the cached layout or the parsing or normalizing rules change, so that old
/// caches are rebuilt.
pub const FORMAT_VERSION: u32 = 1;

/// One spoken word, as typed between two stretches of whitespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// The token's position in [`WordIndex::tokens`].
    pub index: u32,
    /// The word as typed, with its punctuation.
    pub display: String,
    pub block_id: u32,
    /// Index of the block's span where the word starts.
    pub span: u16,
    /// Where `display` is, in chars from the start of that span's text. A word whose style
    /// changes partway ("**world**,") runs on into the next spans, so the range then ends past
    /// the end of the first one.
    pub char_range: Range<u32>,
    /// The ways the word may be spoken: lowercase, without punctuation, never empty. A form of
    /// several words separates them with spaces. The first form follows the typed text most
    /// closely, except that a year comes before the plain number.
    pub forms: Vec<String>,
}

/// Every spoken word of a script, in reading order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordIndex {
    format_version: u32,
    source_hash: u64,
    tokens: Vec<Token>,
    /// Each block's source line, by block id.
    block_lines: Vec<u32>,
}

impl WordIndex {
    /// Indexes the text of every block. Cues, notes included, and rules have no text, so they
    /// give no tokens.
    pub fn build(script: &Script) -> Self {
        let mut tokens = Vec::new();
        for block in &script.blocks {
            push_tokens(block, &mut tokens);
        }
        let block_lines = script.blocks.iter().map(|block| block.source_line).collect();
        Self { format_version: FORMAT_VERSION, source_hash: script.source_hash, tokens, block_lines }
    }

    /// True if this index, usually one read from a cache, was built by this version from the
    /// same Markdown as `script`.
    pub fn is_current(&self, script: &Script) -> bool {
        self.format_version == FORMAT_VERSION && self.source_hash == script.source_hash
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    /// The words it takes to say the script, going by each token's first form.
    pub fn spoken_words(&self) -> usize {
        self.tokens.iter().filter_map(|token| token.forms.first()).map(|form| form.split_whitespace().count()).sum()
    }

    /// The block's tokens. Empty if it has none or there is no such block.
    pub fn tokens_in_block(&self, id: u32) -> &[Token] {
        let start = self.tokens.partition_point(|token| token.block_id < id);
        let end = self.tokens.partition_point(|token| token.block_id <= id);
        &self.tokens[start..end]
    }

    /// The id of the block that holds a token.
    pub fn block_of(&self, token: u32) -> Option<u32> {
        self.tokens.get(usize::try_from(token).ok()?).map(|token| token.block_id)
    }

    /// The 1-based Markdown line where the token's block starts. A block that spans several
    /// lines gives its first line for all of its tokens.
    pub fn source_line_of(&self, token: u32) -> Option<u32> {
        let block = usize::try_from(self.block_of(token)?).ok()?;
        self.block_lines.get(block).copied()
    }
}

/// Splits a block's text at whitespace. A change of style does not end a word.
fn push_tokens(block: &Block, tokens: &mut Vec<Token>) {
    let mut display = String::new();
    // The span and char where `display` starts.
    let mut origin = (0_u16, 0_u32);
    for (span, text) in block.spans.iter().enumerate() {
        let span = u16::try_from(span).unwrap_or(u16::MAX);
        let mut at = 0_u32;
        for c in text.text.chars() {
            if c.is_whitespace() {
                push_token(block.id, origin, mem::take(&mut display), tokens);
            } else {
                if display.is_empty() {
                    origin = (span, at);
                }
                display.push(c);
            }
            at = at.saturating_add(1);
        }
    }
    push_token(block.id, origin, display, tokens);
}

/// Adds a token for `display`, unless there is nothing in it to say (a lone dash, say).
fn push_token(block_id: u32, (span, start): (u16, u32), display: String, tokens: &mut Vec<Token>) {
    let forms = normalize::forms(&display);
    if forms.is_empty() {
        return;
    }
    let index = u32::try_from(tokens.len()).unwrap_or(u32::MAX);
    let length = u32::try_from(display.chars().count()).unwrap_or(u32::MAX);
    let char_range = start..start.saturating_add(length);
    tokens.push(Token { index, display, block_id, span, char_range, forms });
}
