//! editor::ops::media: pure media bin edits: import, remove and rename.

use uuid::Uuid;

use super::OpError;
use crate::model::media::MediaItem;
use crate::model::project::Project;

/// Appends a bin entry to the project's media bin.
///
/// The audio was already conformed to the project rate by `io::import`; this
/// only records the bin entry. `id` must not already be in the bin.
pub fn import_media(project: &mut Project, item: MediaItem) -> Result<(), OpError> {
    if project.media.iter().any(|m| m.id == item.id) {
        return Err(OpError::Invalid(format!("duplicate media id {}", item.id)));
    }
    project.media.push(item);
    Ok(())
}

/// Removes the bin entry and every clip that uses it.
pub fn remove_media(project: &mut Project, id: Uuid) -> Result<(), OpError> {
    if !project.media.iter().any(|m| m.id == id) {
        return Err(OpError::NotFound(id));
    }
    project.media.retain(|m| m.id != id);
    for track in &mut project.tracks {
        track.clips.retain(|c| c.media_id != id);
    }
    Ok(())
}

/// Replaces an offline bin entry's file, rates and length after a relink.
///
/// The audio was already re-imported by `io::import::relink_file`; this only
/// records the new file name and stats. Every clip using the item must still
/// fit inside the replacement's length.
pub fn relink_media(
    project: &mut Project,
    id: Uuid,
    file_name: String,
    original_rate: u32,
    channels: u16,
    length: u64,
) -> Result<(), OpError> {
    if !project.media.iter().any(|m| m.id == id) {
        return Err(OpError::NotFound(id));
    }
    if let Some(highest) = project
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| c.media_id == id)
        .map(|c| c.src_out)
        .max()
        .filter(|src_out| *src_out > length)
    {
        let need = crate::util::time::samples_to_timecode(highest, project.sample_rate);
        return Err(OpError::Invalid(format!(
            "The replacement is too short. The clips using this item need at least {need}."
        )));
    }
    let item = project
        .media
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or(OpError::NotFound(id))?;
    item.file_name = file_name;
    item.original_rate = original_rate;
    item.channels = channels;
    item.length = length;
    Ok(())
}

/// Renames a bin entry's display name (the file in media/ keeps its name).
///
/// Empty or whitespace-only names are invalid.
pub fn rename_media(project: &mut Project, id: Uuid, name: &str) -> Result<(), OpError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(OpError::Invalid("media name is empty".to_string()));
    }
    let item = project
        .media
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or(OpError::NotFound(id))?;
    item.display_name = trimmed.to_string();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::project::BitDepth;

    /// A small bin entry with a unique id.
    fn item(name: &str) -> MediaItem {
        MediaItem {
            id: Uuid::new_v4(),
            display_name: name.to_string(),
            file_name: format!("{name}.wav"),
            original_rate: 48_000,
            channels: 2,
            length: 100,
        }
    }

    #[test]
    fn import_media_appends_and_rejects_duplicates() {
        let mut project = Project::new("P".to_string(), 48_000, BitDepth::Int24);
        let first = item("a");
        import_media(&mut project, first.clone()).unwrap();
        assert_eq!(project.media.len(), 1);
        assert!(matches!(
            import_media(&mut project, first),
            Err(OpError::Invalid(_))
        ));
    }

    #[test]
    fn remove_media_removes_item_and_clips() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 100)]]);
        let id = project.media[0].id;
        remove_media(&mut project, id).unwrap();
        assert!(project.media.is_empty());
        assert!(project.tracks.iter().all(|t| t.clips.is_empty()));
        assert_eq!(remove_media(&mut project, id), Err(OpError::NotFound(id)));
    }

    #[test]
    fn relink_media_updates_the_item() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 100)]]);
        let id = project.media[0].id;
        relink_media(&mut project, id, "new.mp3".to_string(), 44_100, 1, 500_000).unwrap();
        let item = project.media.iter().find(|m| m.id == id).unwrap();
        assert_eq!(item.file_name, "new.mp3");
        assert_eq!(item.original_rate, 44_100);
        assert_eq!(item.channels, 1);
        assert_eq!(item.length, 500_000);
        assert_eq!(item.display_name, "Test Media");
        assert!(matches!(
            relink_media(&mut project, Uuid::new_v4(), "x".to_string(), 48_000, 1, 1),
            Err(OpError::NotFound(_))
        ));
    }

    #[test]
    fn relink_media_too_short_errors_and_changes_nothing() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 100)]]);
        let id = project.media[0].id;
        let before = project.clone();
        // The clip's src_out is 100 samples = 2 ms at 48 kHz.
        let result = relink_media(&mut project, id, "short.wav".to_string(), 48_000, 2, 99);
        match result {
            Err(OpError::Invalid(msg)) => {
                assert!(msg.contains("The replacement is too short"), "{msg}");
                assert!(msg.contains("00:00:00.002"), "{msg}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert_eq!(project, before, "a too-short relink changes nothing");
        // Exactly long enough: allowed.
        relink_media(&mut project, id, "short.wav".to_string(), 48_000, 2, 100).unwrap();
    }

    #[test]
    fn rename_media_sets_name_and_rejects_empty() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[]]);
        let id = project.media[0].id;
        rename_media(&mut project, id, "  Song  ").unwrap();
        assert_eq!(project.media[0].display_name, "Song");
        for bad in ["", "   "] {
            assert!(matches!(
                rename_media(&mut project, id, bad),
                Err(OpError::Invalid(_))
            ));
        }
        assert!(matches!(
            rename_media(&mut project, Uuid::new_v4(), "x"),
            Err(OpError::NotFound(_))
        ));
    }
}
