//! The offline help viewer (spec: S1 Help "Help window", "Markdown subset",
//! "Topic files"). An app supplies its own Markdown topics; this module parses
//! them, searches them and draws the window. No files are read at runtime and
//! no crate is added: the topics are embedded with `include_str!`.

pub mod context;
pub mod markdown;
pub mod search;
pub mod topic;
pub mod window;

/// The id of the generated "Keyboard shortcuts" page, so apps route it through
/// [`help_window`]'s `extra` callback (spec: S1 Help "Shortcut sheet").
pub const KEYBOARD_PAGE_ID: &str = "keyboard-shortcuts";

pub use context::{context, handle_f1};
pub use markdown::{Block, Span};
pub use search::{Hit, search};
pub use topic::{HelpBook, HelpError, Section, Topic};
pub use window::{HelpState, help_window, open};
