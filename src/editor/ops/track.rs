//! editor::ops::track: add, delete, reorder and configure tracks.

use uuid::Uuid;

use crate::editor::ops::OpError;
use crate::model::project::Project;
use crate::model::track::Track;

/// Minimum track or master volume in dB.
pub const MIN_LEVEL_DB: f32 = -60.0;
/// Maximum track or master volume in dB.
pub const MAX_LEVEL_DB: f32 = 12.0;
/// Minimum pan (full left).
pub const MIN_PAN: f32 = -1.0;
/// Maximum pan (full right).
pub const MAX_PAN: f32 = 1.0;

/// Clamps a value into the inclusive range `lo..=hi`.
fn clamp(value: f32, lo: f32, hi: f32) -> f32 {
    value.max(lo).min(hi)
}

/// Returns a mutable reference to the track with the given id, or NotFound.
fn track_mut(project: &mut Project, id: Uuid) -> Result<&mut Track, OpError> {
    project
        .tracks
        .iter_mut()
        .find(|t| t.id == id)
        .ok_or(OpError::NotFound(id))
}

/// Adds a track; `None` index appends, `None` name gives "Track N" (N = count + 1).
pub fn add_track(
    project: &mut Project,
    index: Option<usize>,
    name: Option<String>,
) -> Result<(), OpError> {
    let count = project.tracks.len();
    if let Some(i) = index
        && i > count
    {
        return Err(OpError::Overflow);
    }
    let name = match name {
        Some(n) if n.trim().is_empty() => return Err(OpError::Invalid("empty track name".into())),
        Some(n) => n,
        None => format!("Track {}", count + 1),
    };
    let track = Track::new(name);
    match index {
        Some(i) if i < count => project.tracks.insert(i, track),
        _ => project.tracks.push(track),
    }
    Ok(())
}

/// Deletes the track with the given id.
pub fn delete_track(project: &mut Project, id: Uuid) -> Result<(), OpError> {
    let index = project
        .tracks
        .iter()
        .position(|t| t.id == id)
        .ok_or(OpError::NotFound(id))?;
    project.tracks.remove(index);
    Ok(())
}

/// Renames the track with the given id.
pub fn rename_track(project: &mut Project, id: Uuid, name: String) -> Result<(), OpError> {
    if name.trim().is_empty() {
        return Err(OpError::Invalid("empty track name".into()));
    }
    track_mut(project, id)?.name = name;
    Ok(())
}

/// Moves the track with the given id to a new zero-based index.
pub fn move_track(project: &mut Project, id: Uuid, new_index: usize) -> Result<(), OpError> {
    let old_index = project
        .tracks
        .iter()
        .position(|t| t.id == id)
        .ok_or(OpError::NotFound(id))?;
    if new_index >= project.tracks.len() {
        return Err(OpError::Overflow);
    }
    let track = project.tracks.remove(old_index);
    project.tracks.insert(new_index, track);
    Ok(())
}

/// Sets track volume in dB, clamped to -60..+12.
pub fn set_track_volume(project: &mut Project, id: Uuid, db: f32) -> Result<(), OpError> {
    track_mut(project, id)?.volume_db = clamp(db, MIN_LEVEL_DB, MAX_LEVEL_DB);
    Ok(())
}

/// Sets track pan, clamped to -1..+1.
pub fn set_track_pan(project: &mut Project, id: Uuid, pan: f32) -> Result<(), OpError> {
    track_mut(project, id)?.pan = clamp(pan, MIN_PAN, MAX_PAN);
    Ok(())
}

/// Sets the master volume in dB, clamped to -60..+12.
pub fn set_master_volume(project: &mut Project, db: f32) -> Result<(), OpError> {
    project.master_volume_db = clamp(db, MIN_LEVEL_DB, MAX_LEVEL_DB);
    Ok(())
}

/// Sets the track mute flag.
pub fn set_track_mute(project: &mut Project, id: Uuid, mute: bool) -> Result<(), OpError> {
    track_mut(project, id)?.mute = mute;
    Ok(())
}

/// Sets the track solo flag.
pub fn set_track_solo(project: &mut Project, id: Uuid, solo: bool) -> Result<(), OpError> {
    track_mut(project, id)?.solo = solo;
    Ok(())
}

/// Solos the track with the given id and unsolos every other track.
pub fn solo_exclusive(project: &mut Project, id: Uuid) -> Result<(), OpError> {
    track_mut(project, id)?;
    for track in &mut project.tracks {
        track.solo = track.id == id;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::project_with_clips;

    #[test]
    fn add_track_none_index_appends_and_names_track_n() {
        let mut project = project_with_clips(&[&[], &[]]);
        add_track(&mut project, None, None).unwrap();
        assert_eq!(project.tracks.len(), 3);
        assert_eq!(project.tracks[2].name, "Track 3");
        add_track(&mut project, Some(0), Some("Top".into())).unwrap();
        assert_eq!(project.tracks[0].name, "Top");
    }

    #[test]
    fn index_out_of_range_is_overflow() {
        let mut project = project_with_clips(&[&[]]);
        let id = project.tracks[0].id;
        assert_eq!(
            add_track(&mut project, Some(2), None),
            Err(OpError::Overflow)
        );
        assert_eq!(move_track(&mut project, id, 5), Err(OpError::Overflow));
    }

    #[test]
    fn solo_exclusive_unsolos_others() {
        let mut project = project_with_clips(&[&[], &[], &[]]);
        let a = project.tracks[0].id;
        let b = project.tracks[1].id;
        let c = project.tracks[2].id;
        set_track_solo(&mut project, a, true).unwrap();
        set_track_solo(&mut project, b, true).unwrap();
        solo_exclusive(&mut project, c).unwrap();
        assert!(!project.tracks[0].solo && !project.tracks[1].solo);
        assert!(project.tracks[2].solo);
        let missing = Uuid::new_v4();
        assert_eq!(
            solo_exclusive(&mut project, missing),
            Err(OpError::NotFound(missing))
        );
    }
}
