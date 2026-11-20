//! The design tokens: one source of truth for every fixed design value,
//! generated from `design/tokens.json` by `build.rs` at compile time.
//!
//! The generated Rust lives in the submodules of this module (`color`,
//! `accent`, `typography`, `space`, `size`, `radius`, `motion`). [`TOKENS_CSS`]
//! holds the same values as CSS custom properties for design tooling.

// Generated content: modules `color`, `accent`, `typography` (the JSON group
// "type"), `space`, `size`, `radius` and `motion`.
include!(concat!(env!("OUT_DIR"), "/tokens.rs"));

/// The generated CSS custom properties, kept in sync with the Rust tokens.
pub const TOKENS_CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/tokens.css"));
