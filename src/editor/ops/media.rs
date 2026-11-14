//! editor::ops::media: pure media bin edits: import, remove, rename and
//! recorded takes.

use uuid::Uuid;

use super::OpError;
use super::clip::place_clip;
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

/// The bin name for the next recorded take: `Take <n>`, where `n` is the
/// highest existing `Take <k>` plus one (1 when the bin has none). Only names
/// that are exactly "Take " and digits count.
pub fn next_take_name(project: &Project) -> String {
    let highest = project
        .media
        .iter()
        .filter_map(|item| item.display_name.strip_prefix("Take "))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|digits| digits.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("Take {}", highest.saturating_add(1))
}

/// Adds a recorded take: the bin entry (renamed `Take <n>`, see
/// [`next_take_name`]) and one clip of it on `track_id` at `start`, playing the
/// take from `src_in` (the hidden latency) to its end. The clip is placed with
/// the overwrite rules of [`place_clip`].
///
/// The take must be longer than `src_in`. On an error the project is unchanged.
pub fn record_take(
    project: &mut Project,
    mut item: MediaItem,
    track_id: Uuid,
    start: u64,
    src_in: u64,
) -> Result<(), OpError> {
    if !project.tracks.iter().any(|track| track.id == track_id) {
        return Err(OpError::NotFound(track_id));
    }
    if src_in >= item.length {
        return Err(OpError::Invalid(
            "the take is not longer than its latency".to_string(),
        ));
    }
    item.display_name = next_take_name(project);
    let (media_id, src_out) = (item.id, item.length);
    import_media(project, item)?;
    place_clip(project, track_id, media_id, start, src_in, src_out, false)?;
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

    /// A recorded take of `length` frames (the display name is replaced by the op).
    fn take_item(length: u64) -> MediaItem {
        MediaItem {
            length,
            ..item("rec-20261114-101500")
        }
    }

    /// A project whose bin holds items with these display names.
    fn project_with_names(names: &[&str]) -> Project {
        let mut project = Project::new("P".to_string(), 48_000, BitDepth::Int24);
        project.media = names.iter().map(|name| item(name)).collect();
        project
    }

    #[test]
    fn next_take_name_counts_up_from_the_highest_take() {
        assert_eq!(next_take_name(&project_with_names(&[])), "Take 1");
        assert_eq!(next_take_name(&project_with_names(&["Take 1"])), "Take 2");
        assert_eq!(
            next_take_name(&project_with_names(&["Take 1", "Take 3", "Take 2"])),
            "Take 4",
            "the highest wins, not the count"
        );
        assert_eq!(
            next_take_name(&project_with_names(&["Take 9", "Take 10"])),
            "Take 11",
            "numeric, not alphabetic"
        );
    }

    #[test]
    fn next_take_name_ignores_names_that_only_look_like_takes() {
        let names = [
            "Take 2 copy",
            "Takes 9",
            "take 8",
            "Take +9",
            "Take -4",
            "Take ",
            "Take",
            "My Take 7",
            "Take 1.5",
        ];
        assert_eq!(next_take_name(&project_with_names(&names)), "Take 1");
        let mut with_one = names.to_vec();
        with_one.push("Take 1");
        assert_eq!(next_take_name(&project_with_names(&with_one)), "Take 2");
    }

    #[test]
    fn record_take_adds_the_bin_item_and_one_clip_with_the_latency_hidden() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[], &[]]);
        let track = project.tracks[1].id;
        let take = take_item(5_000);
        let id = take.id;

        record_take(&mut project, take, track, 1_000, 480).unwrap();

        assert_eq!(project.media.len(), 2, "the fixture's item plus the take");
        let added = project.media.iter().find(|m| m.id == id).unwrap();
        assert_eq!(added.display_name, "Take 1");
        assert_eq!(added.length, 5_000);
        assert!(project.tracks[0].clips.is_empty(), "only the armed track");
        let clips = &project.tracks[1].clips;
        assert_eq!(clips.len(), 1);
        let clip = &clips[0];
        assert_eq!(
            (clip.media_id, clip.start, clip.src_in, clip.src_out),
            (id, 1_000, 480, 5_000)
        );
        assert_eq!((clip.gain_db, clip.fade_in, clip.fade_out), (0.0, 0, 0));
        assert!(crate::model::invariants::check(&project).is_ok());
    }

    #[test]
    fn a_second_take_is_named_take_2() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[]]);
        let track = project.tracks[0].id;
        record_take(&mut project, take_item(2_000), track, 0, 0).unwrap();
        let second = take_item(2_000);
        let second_id = second.id;
        record_take(&mut project, second, track, 5_000, 0).unwrap();
        let name = |id| {
            project
                .media
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .display_name
                .clone()
        };
        assert_eq!(name(second_id), "Take 2");
    }

    #[test]
    fn record_take_overwrites_what_it_lands_on() {
        // An existing clip 0..1,000 on the track; the take's clip covers 400..700.
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 1_000)]]);
        let track = project.tracks[0].id;
        record_take(&mut project, take_item(400), track, 400, 100).unwrap();
        assert_eq!(
            crate::model::fixtures::spans(&project, 0),
            [(0, 400), (400, 700), (700, 1_000)],
            "the old clip is split around the take, like any overwrite"
        );
        assert_eq!(
            crate::model::fixtures::srcs(&project, 0),
            [(0, 400), (100, 400), (700, 1_000)]
        );
        assert!(crate::model::invariants::check(&project).is_ok());
    }

    #[test]
    fn record_take_errors_change_nothing() {
        let mut project = crate::model::fixtures::project_with_clips(&[&[(0, 100)]]);
        let track = project.tracks[0].id;
        let before = project.clone();

        // Unknown track.
        assert!(matches!(
            record_take(&mut project, take_item(1_000), Uuid::new_v4(), 0, 0),
            Err(OpError::NotFound(_))
        ));
        // No audible frames after the latency: equal and longer latency.
        for src_in in [1_000, 1_001] {
            assert!(matches!(
                record_take(&mut project, take_item(1_000), track, 0, src_in),
                Err(OpError::Invalid(_))
            ));
        }
        // A media id that is already in the bin.
        let mut duplicate = take_item(1_000);
        duplicate.id = project.media[0].id;
        assert!(matches!(
            record_take(&mut project, duplicate, track, 0, 0),
            Err(OpError::Invalid(_))
        ));
        assert_eq!(project, before);
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
