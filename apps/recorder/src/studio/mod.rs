//! Studio (spec docs/suite/recorder-studio.md): turning a take with an input track into a finished
//! video. This module is pure logic, with no UI and no rendering:
//!
//! - [`settings`]: the `{stem}.studio.json` file beside the take.
//! - [`autozoom`]: the zoom segments made from the take's clicks, shortcuts and cursor dwells.
//! - [`path`]: the zoom view and the smoothed cursor at any time, computed ahead per take.
//!
//! Positions here are source pixels: pixels of the take's video frame. The input track holds desktop
//! pixels, so the functions that read one take a `to_source` function from the caller. It gets a
//! time in microseconds and a desktop position, and answers None when that position was not in the
//! captured frame.

pub mod autozoom;
pub mod path;
pub mod settings;
