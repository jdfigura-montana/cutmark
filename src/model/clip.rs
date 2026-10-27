//! model::clip: Clip, a reference into a media item with in/out points and fades.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A placed reference to a range of source media.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Unique clip id.
    pub id: Uuid,
    /// Id of the media item this clip plays.
    pub media_id: Uuid,
    /// Timeline start position in samples.
    pub start: u64,
    /// Offset into the conformed media in samples.
    pub src_in: u64,
    /// Exclusive end offset into the conformed media in samples.
    pub src_out: u64,
    /// Clip gain, -60.0 to +24.0 dB, default 0.0.
    pub gain_db: f32,
    /// Fade-in length in samples.
    pub fade_in: u64,
    /// Fade-out length in samples.
    pub fade_out: u64,
}

impl Clip {
    /// Clip length in samples: `src_out - src_in`, 0 when `src_out < src_in`
    /// (a project file can hold such a clip; it must never wrap to a huge length).
    pub fn len(&self) -> u64 {
        self.src_out.saturating_sub(self.src_in)
    }

    /// Exclusive timeline end: `start + len()`, saturating at `u64::MAX`.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.len())
    }

    /// Shrinks the fades so `fade_in + fade_out <= len`, trimming fade_out first.
    pub fn clamp_fades(&mut self) {
        self.fade_out = self.fade_out.min(self.len());
        self.fade_in = self.fade_in.min(self.len() - self.fade_out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip with the given placement and source range, no gain or fades.
    fn clip(start: u64, src_in: u64, src_out: u64) -> Clip {
        Clip {
            id: Uuid::new_v4(),
            media_id: Uuid::new_v4(),
            start,
            src_in,
            src_out,
            gain_db: 0.0,
            fade_in: 0,
            fade_out: 0,
        }
    }

    #[test]
    fn len_and_end_of_a_normal_clip() {
        let c = clip(100, 10, 60);
        assert_eq!(c.len(), 50);
        assert_eq!(c.end(), 150);
    }

    #[test]
    fn len_is_zero_when_src_out_is_before_src_in() {
        let c = clip(100, 60, 10);
        assert_eq!(c.len(), 0);
        assert_eq!(c.end(), 100);
    }

    #[test]
    fn end_saturates_instead_of_overflowing() {
        let c = clip(u64::MAX - 5, 0, 100);
        assert_eq!(c.end(), u64::MAX);
    }
}
