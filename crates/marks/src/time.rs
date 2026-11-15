//! Time math for the marks sidecar: session time, timecode text and frame snapping.

use crate::model::{Mark, Sidecar};

const MS_PER_SECOND: u64 = 1000;
const MS_PER_MINUTE: u64 = 60 * MS_PER_SECOND;
const MS_PER_HOUR: u64 = 60 * MS_PER_MINUTE;

/// Where a media file starts on the session timeline, or 0 if the index is out of range.
pub(crate) fn media_offset_ms(s: &Sidecar, media: usize) -> u64 {
    s.media.get(media).map_or(0, |entry| entry.offset_ms)
}

/// Session time of a mark's start: the offset of its media file plus `t_ms`.
///
/// Saturates instead of overflowing. An out-of-range media index counts as offset 0.
pub fn session_ms(s: &Sidecar, m: &Mark) -> u64 {
    media_offset_ms(s, m.media).saturating_add(m.t_ms)
}

/// Formats milliseconds as `HH:MM:SS.mmm`. Hours have at least two digits and may exceed 99.
pub fn format_tc(ms: u64) -> String {
    let hours = ms / MS_PER_HOUR;
    let minutes = (ms % MS_PER_HOUR) / MS_PER_MINUTE;
    let seconds = (ms % MS_PER_MINUTE) / MS_PER_SECOND;
    let millis = ms % MS_PER_SECOND;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

/// Parses the text `format_tc` writes back into milliseconds.
///
/// The input must be exactly `HH:MM:SS.mmm`: at least two digits of hours, then two digits of
/// minutes and seconds (each below 60) and three digits of milliseconds. Anything else, including
/// signs, spaces and a value that does not fit in a `u64`, returns `None`.
pub fn parse_tc(s: &str) -> Option<u64> {
    let (clock, millis_text) = s.split_once('.')?;
    let mut parts = clock.split(':');
    let hours_text = parts.next()?;
    let minutes_text = parts.next()?;
    let seconds_text = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if hours_text.len() < 2 || minutes_text.len() != 2 || seconds_text.len() != 2 || millis_text.len() != 3 {
        return None;
    }
    let hours = digits(hours_text)?;
    let minutes = digits(minutes_text)?;
    let seconds = digits(seconds_text)?;
    let millis = digits(millis_text)?;
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    hours.checked_mul(MS_PER_HOUR)?.checked_add(minutes * MS_PER_MINUTE)?.checked_add(seconds * MS_PER_SECOND)?.checked_add(millis)
}

/// Parses a non-empty run of ASCII digits, rejecting signs and anything else `u64::from_str` allows.
fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Rounds `ms` to the nearest frame at `fps_num / fps_den` frames per second, half up,
/// and returns that frame's start time in milliseconds (also rounded half up).
///
/// A zero numerator or denominator cannot describe a frame rate, so `ms` comes back unchanged.
pub fn snap_to_frame(ms: u64, fps_num: u32, fps_den: u32) -> u64 {
    if fps_num == 0 || fps_den == 0 {
        return ms;
    }
    let num = u128::from(fps_num);
    // One frame lasts `den / num` seconds, which is `den * 1000 / num` milliseconds.
    let den_ms = u128::from(fps_den) * u128::from(MS_PER_SECOND);
    // frame = round_half_up(ms * num / den_ms)
    let frame = (2 * u128::from(ms) * num + den_ms) / (2 * den_ms);
    // snapped = round_half_up(frame * den_ms / num)
    let snapped = (2 * frame * den_ms + num) / (2 * num);
    u64::try_from(snapped).unwrap_or(u64::MAX)
}
