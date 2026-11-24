//! Review playback: file probing and threaded decoding into RGBA frames.
//!
//! [`probe`] reads the headers of a media file in one pass, and
//! [`video::VideoDecoder`] decodes a file on its own thread into scaled RGBA
//! frames that the caller pulls with its own clock.

pub mod probe;
pub mod video;
