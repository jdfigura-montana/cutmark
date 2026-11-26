//! Every OS call the Cutmark Prompter makes (spec "Platform and tech stack", rule 1: only this
//! crate imports OS-specific crates). Windows backend only in v1.

pub mod clipboard;
pub mod error;
pub mod instance;
pub mod os;
pub mod view;

pub use error::{PlatformError, Result};
