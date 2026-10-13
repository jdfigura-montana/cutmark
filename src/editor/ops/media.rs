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
