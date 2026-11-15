//! editor::ops::paste: insert and overwrite paste operations.

use std::sync::Arc;

use uuid::Uuid;

use crate::editor::clipboard::Clipboard;
use crate::editor::ops::OpError;
use crate::editor::ops::clip;
use crate::model::clip::Clip;
use crate::model::project::Project;
use crate::model::track::Track;

/// Returns the mutable receiving track, appending "Track N" tracks as needed.
///
/// Clipboard track `k` lands on project track `target_track + k`.
fn receiving_track(project: &mut Project, target_track: usize, k: usize) -> &mut Track {
    let index = target_track + k;
    while project.tracks.len() <= index {
        let name = format!("Track {}", project.tracks.len() + 1);
        project.tracks.push(Track::new(name));
    }
    &mut project.tracks[index]
}

/// Builds a pasted copy of a clipboard clip at the given timeline position.
fn pasted_clip(source: &Clip, pos: u64) -> Clip {
    Clip {
        id: Uuid::new_v4(),
        media_id: source.media_id,
        start: pos + source.start,
        src_in: source.src_in,
        src_out: source.src_out,
        gain_db: source.gain_db,
        fade_in: source.fade_in,
        fade_out: source.fade_out,
    }
}

/// Inserts the clipboard at `pos` on the receiving tracks, always shifting later clips.
///
/// Clips under `pos` split; later clips on each receiving track shift right by the
/// clipboard length, whatever the Ripple toggle says.
pub fn paste_insert(
    project: &mut Project,
    pos: u64,
    target_track: usize,
    clipboard: &Arc<Clipboard>,
) -> Result<(), OpError> {
    let length = clipboard.length.min(i64::MAX as u64) as i64;
    for (k, clips) in clipboard.tracks.iter().enumerate() {
        let track = receiving_track(project, target_track, k);
        let _ = clip::split_clip_at(track, pos);
        clip::shift_from(track, pos, length)?;
        for source in clips {
            track.clips.push(pasted_clip(source, pos));
        }
        track.sort_clips();
    }
    Ok(())
}

/// Replaces the range `s..e` on the receiving tracks with the clipboard.
///
/// Ripple on: later clips shift by the clipboard-length difference. Ripple off:
/// the clipboard overwrites from `s`, possibly past `e`.
pub fn paste_replace(
    project: &mut Project,
    s: u64,
    e: u64,
    target_track: usize,
    clipboard: &Arc<Clipboard>,
    ripple: bool,
) -> Result<(), OpError> {
    let length = clipboard.length;
    for (k, clips) in clipboard.tracks.iter().enumerate() {
        let track = receiving_track(project, target_track, k);
        if ripple {
            clip::clear_range(track, s, e);
            let old = e.saturating_sub(s).min(i64::MAX as u64) as i64;
            clip::shift_from(track, e, length.min(i64::MAX as u64) as i64 - old)?;
            for source in clips {
                track.clips.push(pasted_clip(source, s));
            }
        } else {
            clip::clear_range(track, s, e.max(s + length));
            for source in clips {
                track.clips.push(pasted_clip(source, s));
            }
        }
        track.sort_clips();
    }
    Ok(())
}
