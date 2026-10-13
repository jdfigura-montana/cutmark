//! model: plain project data with serde derive; read by all layers, written only by the editor.

pub mod clip;
pub mod invariants;
pub mod marker;
pub mod media;
pub mod project;
pub mod track;

#[cfg(test)]
pub mod fixtures;

// TEMP: the binary crate has no consumers of these re-exports yet; remove this allow in phase 10.
#[allow(unused_imports)]
pub use clip::Clip;
#[allow(unused_imports)]
pub use marker::Marker;
#[allow(unused_imports)]
pub use media::MediaItem;
#[allow(unused_imports)]
pub use project::{BitDepth, Project, ViewState};
#[allow(unused_imports)]
pub use track::Track;
