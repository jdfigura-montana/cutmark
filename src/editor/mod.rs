//! editor: the only code that changes a Project; the Editor gateway between ui, audio and io.

pub mod action;
pub mod clipboard;
#[allow(clippy::module_inception)] // SPEC names src/editor/editor.rs
pub mod editor;
pub mod history;
pub mod jobs;
pub mod ops;
pub mod selection;
pub mod snap;
pub mod transport;

pub use self::editor::{Editor, recent_projects, remove_recent};
pub use crate::io::recent::RecentEntry;
