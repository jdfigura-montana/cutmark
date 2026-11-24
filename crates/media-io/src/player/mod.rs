//! Review playback: file probing and threaded decoding into RGBA frames and audio samples.
//!
//! [`probe`] reads the headers of a media file in one pass, [`video::VideoDecoder`] decodes
//! a file on its own thread into scaled RGBA frames that the caller pulls with its own
//! clock, and [`audio::AudioDecoder`] decodes the mixdown stream into a lock-free ring of
//! stereo samples an output callback drains.

pub mod audio;
pub mod probe;
pub mod video;
