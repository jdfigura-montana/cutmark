//! io::import: copy originals into media/, conform, cache and build peaks.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;

use super::IoError;
use super::decode::{self, Decoded};
use super::paths::{ProjectPaths, conformed_path, peaks_path, unique_media_name};
use super::peaks;
use super::resample::resample;
use super::wav;
use crate::model::media::{MediaItem, Peaks};

/// Fully loaded media: interleaved f32 samples at the project rate, plus peaks.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// Interleaved f32 samples at the project sample rate.
    pub samples: Arc<[f32]>,
    /// Waveform peaks built from those samples.
    pub peaks: Arc<Peaks>,
}

/// Imports a supported audio file into the project.
///
/// Decodes `src`, copies it unmodified into media/, resamples it to
/// `project_rate`, writes the conformed WAV and peaks into cache/, and returns
/// the bin entry plus the loaded audio. `progress` reports 0.0..=1.0. If any
/// step after the copy fails, the copy and any cache files written are deleted.
pub fn import_file(
    src: &Path,
    paths: &ProjectPaths,
    media_id: Uuid,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<(MediaItem, Loaded), IoError> {
    import_core(
        src,
        paths,
        media_id,
        display_stem(src),
        project_rate,
        progress,
    )
}

/// Re-links an offline media item to a replacement file.
///
/// Runs the same pipeline as `import_file`, but keeps the item's id and
/// display name, deletes the item's old cache files first (they belong to the
/// missing original), and copies `src` into media/ under a unique name. If any
/// step fails, the copy and any cache files written are deleted.
pub fn relink_file(
    src: &Path,
    paths: &ProjectPaths,
    item: &MediaItem,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<(MediaItem, Loaded), IoError> {
    let _ = fs::remove_file(conformed_path(paths, item.id));
    let _ = fs::remove_file(peaks_path(paths, item.id));
    import_core(
        src,
        paths,
        item.id,
        item.display_name.clone(),
        project_rate,
        progress,
    )
}

/// Shared import pipeline: decode, copy into media/, conform, cache and peaks.
///
/// `display_name` becomes the bin entry's display name; `media_id` names the
/// cache files. If any step after the copy fails, the copy and any cache
/// files written are deleted.
fn import_core(
    src: &Path,
    paths: &ProjectPaths,
    media_id: Uuid,
    display_name: String,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<(MediaItem, Loaded), IoError> {
    if !decode::is_supported(src) {
        return Err(IoError::UnsupportedAudio(src.display().to_string()));
    }
    progress(0.0);
    let decoded = decode_file_mapped(src, progress, 0.0, 0.6)?;

    // Copy the original into media/. Everything after this point cleans up.
    let src_name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| IoError::UnsupportedAudio(src.display().to_string()))?;
    let media_name = unique_media_name(&paths.media(), &src_name);
    // A name the loader would refuse (e.g. a device name made with a `\\?\`
    // path) must never be written into a project: it would not open again.
    let media_path = paths.media_file(&media_name)?;
    fs::copy(src, &media_path)?;

    let cleanup = || {
        let _ = fs::remove_file(&media_path);
        let _ = fs::remove_file(conformed_path(paths, media_id));
        let _ = fs::remove_file(peaks_path(paths, media_id));
    };

    let resampled = match resample(
        &decoded.samples,
        decoded.channels,
        decoded.rate,
        project_rate,
        &mut |p| progress(0.6 + p * 0.25),
    ) {
        Ok(r) => r,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };

    let channels = decoded.channels;
    let frames = resampled.len() as u64 / u64::from(channels);

    let conformed = conformed_path(paths, media_id);
    if let Err(e) = wav::write_f32(&conformed, &resampled, channels, project_rate) {
        cleanup();
        return Err(e);
    }
    progress(0.95);

    let built = peaks::build(&resampled, channels);
    if let Err(e) = peaks::write(&peaks_path(paths, media_id), &built) {
        cleanup();
        return Err(e);
    }
    progress(1.0);

    let item = MediaItem {
        id: media_id,
        display_name,
        file_name: media_name,
        original_rate: decoded.rate,
        channels,
        length: frames,
    };
    Ok((
        item,
        Loaded {
            samples: resampled.into(),
            peaks: built.into(),
        },
    ))
}

/// Loads previously imported media, rebuilding the cache if needed.
///
/// If the original file in media/ is missing, returns `IoError::MediaOffline`.
/// If the conformed WAV and peaks cache are intact and match the item, they
/// are loaded directly; otherwise they are rebuilt from the media file.
/// `progress` reports 0.0..=1.0.
pub fn load_or_rebuild(
    paths: &ProjectPaths,
    item: &MediaItem,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<Loaded, IoError> {
    // `file_name` comes from project.json: an unsafe name (UNC, absolute, `..`
    // ...) is an error here and is never joined, so nothing outside media/ is
    // touched (not even a network path probed).
    let media_path = paths.media_file(&item.file_name)?;
    if !media_path.is_file() {
        return Err(IoError::MediaOffline(item.id));
    }
    let conformed = conformed_path(paths, item.id);
    let peaks_file = peaks_path(paths, item.id);

    // Fast path: intact cache. The WAV must have the item's channel count and
    // frame count, and the peaks file must read cleanly.
    if let Ok(decoded) = wav::read_f32(&conformed) {
        let frames = decoded.samples.len() as u64 / u64::from(decoded.channels.max(1));
        if decoded.channels == item.channels
            && frames == item.length
            && let Ok(loaded) = peaks::read(&peaks_file)
        {
            progress(1.0);
            return Ok(Loaded {
                samples: decoded.samples.into(),
                peaks: loaded.into(),
            });
        }
    }

    // Slow path: rebuild from the original in media/.
    let decoded = decode_file_mapped(&media_path, progress, 0.0, 0.6)?;
    let resampled = resample(
        &decoded.samples,
        decoded.channels,
        decoded.rate,
        project_rate,
        &mut |p| progress(0.6 + p * 0.25),
    )?;
    let channels = decoded.channels;
    let conformed = conformed_path(paths, item.id);
    wav::write_f32(&conformed, &resampled, channels, project_rate)?;
    progress(0.95);
    let built = peaks::build(&resampled, channels);
    peaks::write(&peaks_path(paths, item.id), &built)?;
    progress(1.0);
    Ok(Loaded {
        samples: resampled.into(),
        peaks: built.into(),
    })
}

/// Decodes a file while mapping progress into the given sub-range.
fn decode_file_mapped(
    path: &Path,
    progress: &mut dyn FnMut(f32),
    from: f32,
    to: f32,
) -> Result<Decoded, IoError> {
    decode::decode_file(path, &mut |p| progress(from + p * (to - from)))
}

/// File stem of a path, for the bin's display name.
fn display_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::paths::create_layout;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use tempfile::tempdir;

    /// Writes a 16-bit int WAV with `frames` stereo sine frames at `rate`.
    fn write_stereo_wav(path: &Path, rate: u32, frames: usize) {
        let spec = WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for i in 0..frames {
            let s = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin();
            let v = (s * 16_000.0) as i16;
            writer.write_sample(v).unwrap();
            writer.write_sample(-v).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn cache_file_count(paths: &ProjectPaths) -> usize {
        fs::read_dir(paths.cache()).unwrap().count()
    }

    #[test]
    fn import_conforms_44k_stereo_to_48k() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("tone.wav");
        write_stereo_wav(&src, 44_100, 44_100);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, loaded) =
            import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();

        assert_eq!(item.original_rate, 44_100);
        assert_eq!(item.channels, 2);
        assert_eq!(item.length, 48_000);
        assert_eq!(item.display_name, "tone");
        assert!(paths.media().join(&item.file_name).is_file());
        assert!(conformed_path(&paths, item.id).is_file());
        assert!(peaks_path(&paths, item.id).is_file());
        assert_eq!(loaded.samples.len(), 48_000 * 2);
        assert_eq!(loaded.peaks.frames, 48_000);
        assert_eq!(loaded.peaks.channels, 2);
        assert_eq!(cache_file_count(&paths), 2);
    }

    #[test]
    fn import_same_file_twice_gives_second_name() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("x.wav");
        write_stereo_wav(&src, 48_000, 100);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (first, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        let (second, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        assert_eq!(first.file_name, "x.wav");
        assert_eq!(second.file_name, "x (2).wav");
        assert!(paths.media().join("x.wav").is_file());
        assert!(paths.media().join("x (2).wav").is_file());
    }

    #[test]
    fn import_txt_errors_and_leaves_folders_empty() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("notes.txt");
        fs::write(&src, "hello").unwrap();

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let result = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {});
        assert!(matches!(result, Err(IoError::UnsupportedAudio(_))));
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 0);
        assert_eq!(fs::read_dir(paths.cache()).unwrap().count(), 0);
    }

    #[test]
    fn relink_file_keeps_id_and_name_and_replaces_cache() {
        let src_dir = tempdir().unwrap();
        let original = src_dir.path().join("gone.wav");
        write_stereo_wav(&original, 48_000, 500);
        let replacement = src_dir.path().join("new.wav");
        write_stereo_wav(&replacement, 48_000, 1_000);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) =
            import_file(&original, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        let old_conformed = conformed_path(&paths, item.id);
        let before = fs::read(&old_conformed).unwrap();

        let (relinked, loaded) =
            relink_file(&replacement, &paths, &item, 48_000, &mut |_| {}).unwrap();
        assert_eq!(relinked.id, item.id);
        assert_eq!(relinked.display_name, item.display_name);
        assert_ne!(relinked.file_name, item.file_name);
        assert_eq!(relinked.file_name, "new.wav");
        assert_eq!(relinked.length, 1_000);
        assert_eq!(loaded.samples.len(), 1_000 * 2);
        assert!(paths.media().join("new.wav").is_file());
        let after = fs::read(&old_conformed).unwrap();
        assert_ne!(before, after, "cache was rewritten for the replacement");
        assert!(peaks_path(&paths, item.id).is_file());
    }

    #[test]
    fn relink_file_unsupported_leaves_nothing_behind() {
        let src_dir = tempdir().unwrap();
        let original = src_dir.path().join("gone.wav");
        write_stereo_wav(&original, 48_000, 500);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) =
            import_file(&original, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        fs::remove_file(paths.media().join(&item.file_name)).unwrap();

        let notes = src_dir.path().join("notes.txt");
        fs::write(&notes, "hello").unwrap();
        let result = relink_file(&notes, &paths, &item, 48_000, &mut |_| {});
        assert!(matches!(result, Err(IoError::UnsupportedAudio(_))));
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 0);
        assert!(!conformed_path(&paths, item.id).exists());
        assert!(!peaks_path(&paths, item.id).exists());
    }

    #[test]
    fn load_or_rebuild_intact_cache_does_not_rewrite() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("keep.wav");
        write_stereo_wav(&src, 48_000, 500);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        let conformed = conformed_path(&paths, item.id);
        let before = fs::metadata(&conformed).unwrap().modified().unwrap();

        let loaded = load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
        let after = fs::metadata(&conformed).unwrap().modified().unwrap();
        assert_eq!(before, after, "intact cache must not be rewritten");
        assert_eq!(loaded.samples.len(), 500 * 2);
    }

    #[test]
    fn load_or_rebuild_rebuilds_deleted_cache() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("gone.wav");
        write_stereo_wav(&src, 48_000, 500);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        fs::remove_file(conformed_path(&paths, item.id)).unwrap();
        fs::remove_file(peaks_path(&paths, item.id)).unwrap();

        let loaded = load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
        assert_eq!(loaded.samples.len(), 500 * 2);
        assert!(conformed_path(&paths, item.id).is_file());
        assert!(peaks_path(&paths, item.id).is_file());
    }

    #[test]
    fn load_or_rebuild_never_joins_an_unsafe_file_name() {
        // A real, decodable WAV outside the project: an absolute `file_name`
        // used to load it, a UNC name would have probed the network.
        let src_dir = tempdir().unwrap();
        let outside = src_dir.path().join("outside.wav");
        write_stereo_wav(&outside, 48_000, 500);
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();

        let unsafe_names = [
            outside.display().to_string(),
            r"\\attacker.example\share\a.wav".to_string(),
            r"\outside.wav".to_string(),
            r"..\..\outside.wav".to_string(),
            "bad\"name.wav".to_string(),
            "con.wav".to_string(),
        ];
        for name in unsafe_names {
            let item = MediaItem {
                id: Uuid::new_v4(),
                display_name: "x".to_string(),
                file_name: name.clone(),
                original_rate: 48_000,
                channels: 2,
                length: 500,
            };
            let result = load_or_rebuild(&paths, &item, 48_000, &mut |_| {});
            assert!(
                matches!(result, Err(IoError::UnsafePath(_))),
                "{name:?} must be refused, got {result:?}"
            );
        }
        // Nothing was decoded, so nothing was cached.
        assert_eq!(cache_file_count(&paths), 0);
    }

    #[test]
    fn import_refuses_a_name_the_loader_would_reject() {
        // Windows only lets a device name exist through a verbatim path.
        let src_dir = tempdir().unwrap();
        let verbatim = format!(r"\\?\{}\nul.wav", src_dir.path().display());
        let src = Path::new(&verbatim);
        write_stereo_wav(src, 48_000, 100);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let result = import_file(src, &paths, Uuid::new_v4(), 48_000, &mut |_| {});
        fs::remove_file(src).unwrap();
        assert!(
            matches!(result, Err(IoError::UnsafePath(_))),
            "got {result:?}"
        );
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 0);
        assert_eq!(cache_file_count(&paths), 0);
    }

    #[test]
    fn load_or_rebuild_media_offline() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("offline.wav");
        write_stereo_wav(&src, 48_000, 500);

        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        fs::remove_file(paths.media().join(&item.file_name)).unwrap();

        let result = load_or_rebuild(&paths, &item, 48_000, &mut |_| {});
        match result {
            Err(IoError::MediaOffline(id)) => assert_eq!(id, item.id),
            other => panic!("expected MediaOffline, got {other:?}"),
        }
    }
}
