//! io::import: copy originals into media/, conform, cache and build peaks.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;

use super::IoError;
use super::decode::{self, Decoded};
use super::paths::{ProjectPaths, conformed_path, peaks_path, unique_media_name};
use super::peaks;
use super::resample::StreamResampler;
use super::wav;
use crate::model::media::{MediaItem, Peaks};

/// Frames handed to the resampler per call; sets how often progress is reported.
const CONFORM_BLOCK_FRAMES: usize = 16_384;

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
    let Decoded {
        samples,
        channels,
        rate,
    } = decode_file_mapped(src, progress, 0.0, 0.6)?;

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

    let loaded = match cache_conformed(
        paths,
        media_id,
        Decoded {
            samples,
            channels,
            rate,
        },
        project_rate,
        progress,
    ) {
        Ok(loaded) => loaded,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };

    let item = MediaItem {
        id: media_id,
        display_name,
        file_name: media_name,
        original_rate: rate,
        channels,
        length: loaded.samples.len() as u64 / u64::from(channels),
    };
    Ok((item, loaded))
}

/// Builds the cache for a file that is already in media/, without copying it:
/// decodes `file_name`, conforms it to `project_rate`, writes the conformed
/// WAV and peaks into cache/ and returns the bin entry plus the loaded audio.
/// Used for recorded takes, which the record writer creates in media/.
///
/// `progress` reports 0.0..=1.0. If a step fails, the cache files written so
/// far are deleted; the file in media/ is left alone.
pub fn conform_existing(
    paths: &ProjectPaths,
    file_name: &str,
    media_id: Uuid,
    display_name: String,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<(MediaItem, Loaded), IoError> {
    let media_path = paths.media_file(file_name)?;
    progress(0.0);
    let decoded = decode_file_mapped(&media_path, progress, 0.0, 0.6)?;
    let (channels, rate) = (decoded.channels, decoded.rate);
    match cache_conformed(paths, media_id, decoded, project_rate, progress) {
        Ok(loaded) => {
            let item = MediaItem {
                id: media_id,
                display_name,
                file_name: file_name.to_string(),
                original_rate: rate,
                channels,
                length: loaded.samples.len() as u64 / u64::from(channels),
            };
            Ok((item, loaded))
        }
        Err(e) => {
            let _ = fs::remove_file(conformed_path(paths, media_id));
            let _ = fs::remove_file(peaks_path(paths, media_id));
            Err(e)
        }
    }
}

/// Conforms decoded audio to `project_rate`, writes the conformed WAV and the
/// peaks into cache/ and returns the loaded audio. Progress covers 0.6..=1.0
/// (decoding, 0.0..0.6, is the caller's). Cleaning up after an error is the
/// caller's job.
fn cache_conformed(
    paths: &ProjectPaths,
    media_id: Uuid,
    decoded: Decoded,
    project_rate: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<Loaded, IoError> {
    let Decoded {
        samples,
        channels,
        rate,
    } = decoded;
    let resampled = conform(samples, channels, rate, project_rate, &mut |p| {
        progress(0.6 + p * 0.25)
    })?;
    wav::write_f32(
        &conformed_path(paths, media_id),
        &resampled,
        channels,
        project_rate,
    )?;
    progress(0.95);
    let built = peaks::build(&resampled, channels);
    peaks::write(&peaks_path(paths, media_id), &built)?;
    progress(1.0);
    Ok(Loaded {
        samples: resampled.into(),
        peaks: built.into(),
    })
}

/// Loads previously imported media, rebuilding the cache if needed.
///
/// If the original file in media/ is missing, returns `IoError::MediaOffline`.
/// If the conformed WAV and peaks cache are intact and match the item, they
/// are loaded directly; otherwise they are rebuilt from the media file. A peaks
/// file that fails to read, or whose channels or frame count differ from the
/// conformed WAV, is deleted and rebuilt the same way.
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
    // frame count, and the peaks file must read cleanly and describe the same
    // audio. A peaks file that does not is deleted and rebuilt below.
    if let Ok(decoded) = wav::read_f32(&conformed) {
        let frames = decoded.samples.len() as u64 / u64::from(decoded.channels.max(1));
        if decoded.channels == item.channels && frames == item.length {
            match peaks::read(&peaks_file) {
                Ok(loaded) if loaded.channels == decoded.channels && loaded.frames == frames => {
                    progress(1.0);
                    return Ok(Loaded {
                        samples: decoded.samples.into(),
                        peaks: loaded.into(),
                    });
                }
                _ => {
                    let _ = fs::remove_file(&peaks_file);
                }
            }
        }
    }

    // Slow path: rebuild from the original in media/.
    let decoded = decode_file_mapped(&media_path, progress, 0.0, 0.6)?;
    cache_conformed(paths, item.id, decoded, project_rate, progress)
}

/// Conforms decoded interleaved samples from `from` Hz to `to` Hz.
///
/// The audio goes through a `StreamResampler` in blocks, appending output as it
/// is produced, so memory is the input plus exactly the output (no worst-case
/// preallocation scaled by `to / from`). The result has exactly
/// `round(frames * to / from)` frames. Equal rates hand the samples back
/// untouched. `progress` reports 0.0..=1.0.
fn conform(
    samples: Vec<f32>,
    channels: u16,
    from: u32,
    to: u32,
    progress: &mut dyn FnMut(f32),
) -> Result<Vec<f32>, IoError> {
    if from == to {
        progress(1.0);
        return Ok(samples);
    }
    let mut resampler = StreamResampler::new(channels, from, to)?;
    let ch = usize::from(channels);
    let in_frames = (samples.len() / ch) as u64;
    let target = (in_frames * u64::from(to) + u64::from(from) / 2) / u64::from(from);
    // The output size is known exactly, so reserve it once. A request the
    // machine cannot meet is an import error, not a process abort.
    let mut out: Vec<f32> = Vec::new();
    usize::try_from(target)
        .ok()
        .and_then(|frames| frames.checked_mul(ch))
        .and_then(|len| out.try_reserve_exact(len).ok())
        .ok_or_else(|| IoError::Decode("File is too large to load".to_string()))?;

    let total = samples.len();
    let mut done = 0usize;
    for block in samples.chunks(CONFORM_BLOCK_FRAMES * ch) {
        resampler.push(block, &mut out)?;
        done += block.len();
        progress(done as f32 / total as f32);
    }
    resampler.finish(&mut out)?;
    progress(1.0);
    Ok(out)
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

    /// Writes a 24-bit mono WAV of `frames` frames at `rate`; every sample is
    /// `value / 8_388_607`.
    fn write_mono24(path: &Path, rate: u32, frames: usize, value: i32) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: rate,
            bits_per_sample: 24,
            sample_format: SampleFormat::Int,
        };
        let mut writer = WavWriter::create(path, spec).unwrap();
        for _ in 0..frames {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn conform_existing_builds_the_cache_without_copying() {
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        write_mono24(&paths.media().join("take.wav"), 48_000, 4_800, 4_194_304);
        let id = Uuid::new_v4();

        let (item, loaded) = conform_existing(
            &paths,
            "take.wav",
            id,
            "Take".to_string(),
            48_000,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(item.id, id);
        assert_eq!(item.display_name, "Take");
        assert_eq!(item.file_name, "take.wav");
        assert_eq!(
            (item.original_rate, item.channels, item.length),
            (48_000, 1, 4_800)
        );
        assert_eq!(loaded.samples.len(), 4_800);
        assert!((loaded.samples[0] - 0.5).abs() < 1e-5, "24-bit half scale");
        assert_eq!((loaded.peaks.channels, loaded.peaks.frames), (1, 4_800));
        assert!(conformed_path(&paths, id).is_file());
        assert!(peaks_path(&paths, id).is_file());
        // Nothing was copied: media/ still holds only the take.
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 1);
    }

    #[test]
    fn conform_existing_resamples_to_the_project_rate() {
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        write_mono24(&paths.media().join("take.wav"), 44_100, 44_100, 1_000_000);
        let (item, loaded) = conform_existing(
            &paths,
            "take.wav",
            Uuid::new_v4(),
            "Take".to_string(),
            48_000,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(item.original_rate, 44_100);
        assert_eq!(item.length, 48_000);
        assert_eq!(loaded.samples.len(), 48_000);
    }

    #[test]
    fn conform_existing_failures_leave_no_cache_and_keep_the_file() {
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let id = Uuid::new_v4();

        // A file that is not audio: decode fails, the file stays.
        fs::write(paths.media().join("junk.wav"), b"not a wav").unwrap();
        let result = conform_existing(&paths, "junk.wav", id, "x".to_string(), 48_000, &mut |_| {});
        assert!(result.is_err());
        assert!(paths.media().join("junk.wav").is_file());
        assert_eq!(cache_file_count(&paths), 0);

        // A missing file and an unsafe name are errors too.
        assert!(
            conform_existing(&paths, "gone.wav", id, "x".to_string(), 48_000, &mut |_| {}).is_err()
        );
        assert!(matches!(
            conform_existing(
                &paths,
                "..\\x.wav",
                id,
                "x".to_string(),
                48_000,
                &mut |_| {}
            ),
            Err(IoError::UnsafePath(_))
        ));
        assert_eq!(cache_file_count(&paths), 0);
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

    /// A 440 Hz sine at 0.5 amplitude, `frames` long, `channels` interleaved
    /// channels (the second channel is the first inverted).
    fn sine(rate: u32, frames: usize, channels: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let phase = std::f64::consts::TAU * 440.0 * i as f64 / f64::from(rate);
                let s = (0.5 * phase.sin()) as f32;
                (0..channels).map(move |c| if c == 0 { s } else { -s })
            })
            .collect()
    }

    fn rounded_frames(frames: usize, from: u32, to: u32) -> usize {
        ((frames as u64 * u64::from(to) + u64::from(from) / 2) / u64::from(from)) as usize
    }

    #[test]
    fn conform_with_equal_rates_hands_the_samples_back() {
        let input = sine(48_000, 777, 2);
        let mut last = 0.0;
        let out = conform(input.clone(), 2, 48_000, 48_000, &mut |p| last = p).unwrap();
        assert_eq!(out, input);
        assert_eq!(last, 1.0);
    }

    #[test]
    fn conform_matches_the_old_batch_output_for_a_sine() {
        for channels in [1usize, 2] {
            let input = sine(44_100, 44_100, channels);
            let old =
                crate::io::resample::resample(&input, channels as u16, 44_100, 48_000, &mut |_| {})
                    .unwrap();
            let new = conform(input, channels as u16, 44_100, 48_000, &mut |_| {}).unwrap();
            assert_eq!(new.len(), 48_000 * channels, "same length as before");
            assert_eq!(new.len(), old.len());
            // Same filter and chunking. They differ only in the last ~400
            // frames, where the old code padded zeros instead of flushing.
            let compare = old.len() - 400 * channels;
            for i in 0..compare {
                assert!(
                    (new[i] - old[i]).abs() < 1e-5,
                    "{channels} ch, sample {i}: new {} vs old {}",
                    new[i],
                    old[i]
                );
            }
        }
    }

    #[test]
    fn conform_length_is_round_frames_times_to_over_from() {
        // (from, to, frames, channels). The common rates cover empty, tiny and
        // block-straddling sizes (the conform feeds 16,384 frames at a time);
        // the extreme ratios stay small because debug builds resample slowly.
        let mut cases: Vec<(u32, u32, usize, usize)> = Vec::new();
        for (from, to) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            for frames in [0, 1, 1_023, 16_385] {
                for channels in [1, 2] {
                    cases.push((from, to, frames, channels));
                }
            }
        }
        for frames in [16_383, 16_384, 40_000] {
            cases.push((44_100, 48_000, frames, 1));
        }
        for frames in [0, 1, 17] {
            cases.push((8_000, 192_000, frames, 1));
        }
        for frames in [1, 1_023, 16_385] {
            cases.push((384_000, 8_000, frames, 1));
        }
        for (from, to, frames, channels) in cases {
            let input = vec![0.25_f32; frames * channels];
            let out = conform(input, channels as u16, from, to, &mut |_| {}).unwrap();
            assert_eq!(
                out.len(),
                rounded_frames(frames, from, to) * channels,
                "{from}->{to}, {frames} frames, {channels} ch"
            );
        }
    }

    #[test]
    fn conform_reports_steady_progress_ending_at_one() {
        let input = sine(44_100, 50_000, 2);
        let mut seen: Vec<f32> = Vec::new();
        conform(input, 2, 44_100, 48_000, &mut |p| seen.push(p)).unwrap();
        assert!(seen.len() > 3, "reported {} times", seen.len());
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
        assert_eq!(seen.last(), Some(&1.0));
    }

    #[test]
    fn conform_rejects_a_partial_frame_and_zero_channels() {
        assert!(conform(vec![0.0; 5], 2, 44_100, 48_000, &mut |_| {}).is_err());
        assert!(conform(vec![0.0; 5], 0, 44_100, 48_000, &mut |_| {}).is_err());
    }

    #[test]
    fn one_second_round_trips_have_exact_lengths() {
        let src_dir = tempdir().unwrap();
        for (rate, project) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            let src = src_dir.path().join(format!("one_{rate}.wav"));
            write_stereo_wav(&src, rate, rate as usize);
            let proj_dir = tempdir().unwrap();
            let paths = create_layout(proj_dir.path(), "Proj").unwrap();
            let (item, loaded) =
                import_file(&src, &paths, Uuid::new_v4(), project, &mut |_| {}).unwrap();
            assert_eq!(item.length, u64::from(project), "{rate} -> {project}");
            assert_eq!(loaded.samples.len(), project as usize * 2);
            assert_eq!(loaded.peaks.frames, u64::from(project));
            // The cache on disk agrees, so reopening takes the fast path.
            let reopened = load_or_rebuild(&paths, &item, project, &mut |_| {}).unwrap();
            assert_eq!(reopened.samples.len(), loaded.samples.len());
        }
    }

    #[test]
    fn import_of_a_1_hz_wav_is_refused_and_leaves_nothing_behind() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("one_hz.wav");
        write_stereo_wav(&src, 1, 100);
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();

        let err = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap_err();
        assert!(matches!(err, IoError::UnsupportedAudio(_)), "{err:?}");
        assert!(
            err.to_string().contains("Unsupported sample rate: 1 Hz"),
            "{err}"
        );
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 0);
        assert_eq!(cache_file_count(&paths), 0);
    }

    #[test]
    fn relink_to_a_1_hz_wav_is_refused_and_leaves_nothing_behind() {
        let src_dir = tempdir().unwrap();
        let original = src_dir.path().join("gone.wav");
        write_stereo_wav(&original, 48_000, 500);
        let bad = src_dir.path().join("one_hz.wav");
        write_stereo_wav(&bad, 1, 100);
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) =
            import_file(&original, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        fs::remove_file(paths.media().join(&item.file_name)).unwrap();

        let err = relink_file(&bad, &paths, &item, 48_000, &mut |_| {}).unwrap_err();
        assert!(
            err.to_string().contains("Unsupported sample rate: 1 Hz"),
            "{err}"
        );
        assert_eq!(fs::read_dir(paths.media()).unwrap().count(), 0);
        assert!(!conformed_path(&paths, item.id).exists());
        assert!(!peaks_path(&paths, item.id).exists());
    }

    #[test]
    fn load_or_rebuild_slow_path_conforms_to_the_project_rate() {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("cd.wav");
        write_stereo_wav(&src, 44_100, 44_100);
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, first) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        fs::remove_file(conformed_path(&paths, item.id)).unwrap();
        fs::remove_file(peaks_path(&paths, item.id)).unwrap();

        let rebuilt = load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
        assert_eq!(rebuilt.samples.len(), 48_000 * 2);
        assert_eq!(rebuilt.samples, first.samples, "same audio as the import");
        assert_eq!(rebuilt.peaks, first.peaks);
        assert!(conformed_path(&paths, item.id).is_file());
        assert_eq!(
            peaks::read(&peaks_path(&paths, item.id)).unwrap(),
            *first.peaks
        );
    }

    /// A project with one imported 500 frame stereo item and its cache.
    fn imported_item() -> (tempfile::TempDir, ProjectPaths, MediaItem) {
        let src_dir = tempdir().unwrap();
        let src = src_dir.path().join("keep.wav");
        write_stereo_wav(&src, 48_000, 500);
        let proj_dir = tempdir().unwrap();
        let paths = create_layout(proj_dir.path(), "Proj").unwrap();
        let (item, _) = import_file(&src, &paths, Uuid::new_v4(), 48_000, &mut |_| {}).unwrap();
        (proj_dir, paths, item)
    }

    /// The audit's crafted peaks file: a header and one level that claims
    /// `blocks` blocks without carrying them.
    fn crafted_peaks(channels: u16, block_size: u32, blocks: u64, levels: u32) -> Vec<u8> {
        let mut bytes = b"PEAK".to_vec();
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&levels.to_le_bytes());
        bytes.extend_from_slice(&block_size.to_le_bytes());
        bytes.extend_from_slice(&blocks.to_le_bytes());
        bytes
    }

    #[test]
    fn a_bad_peaks_file_is_rebuilt_on_load() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("2^40 blocks", crafted_peaks(1, 256, 1 << 40, 1)),
            ("2^62 blocks", crafted_peaks(1, 256, 1 << 62, 1)),
            ("block size 0", crafted_peaks(1, 0, 0, 1)),
            ("3 channels", crafted_peaks(3, 256, 0, 1)),
            ("no levels", crafted_peaks(1, 256, 0, 0)),
            ("garbage", b"not a peaks file at all".to_vec()),
            ("empty file", Vec::new()),
        ];
        for (name, bytes) in cases {
            let (_dir, paths, item) = imported_item();
            let peaks_file = peaks_path(&paths, item.id);
            let good = peaks::read(&peaks_file).unwrap();
            fs::write(&peaks_file, &bytes).unwrap();

            let loaded = load_or_rebuild(&paths, &item, 48_000, &mut |_| {})
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(*loaded.peaks, good, "{name}: rebuilt peaks");
            assert_eq!(loaded.samples.len(), 500 * 2, "{name}");
            // The new file on disk is the rebuilt one and reads back fine.
            assert_eq!(peaks::read(&peaks_file).unwrap(), good, "{name}: on disk");
        }
    }

    #[test]
    fn a_truncated_peaks_file_is_rebuilt_on_load() {
        let (_dir, paths, item) = imported_item();
        let peaks_file = peaks_path(&paths, item.id);
        let good_bytes = fs::read(&peaks_file).unwrap();
        fs::write(&peaks_file, &good_bytes[..good_bytes.len() / 2]).unwrap();

        let loaded = load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
        assert_eq!(fs::read(&peaks_file).unwrap(), good_bytes);
        assert_eq!(loaded.peaks.frames, 500);
    }

    #[test]
    fn a_peaks_file_for_other_audio_is_rebuilt_on_load() {
        // Valid files that do not describe the conformed WAV (item: 2 ch, 500 frames).
        let other: Vec<(&str, Peaks)> = vec![
            ("one channel", peaks::build(&[0.5; 500], 1)),
            ("fewer frames", peaks::build(&[0.5; 2 * 100], 2)),
            ("more frames", peaks::build(&[0.5; 2 * 900], 2)),
        ];
        for (name, wrong) in other {
            let (_dir, paths, item) = imported_item();
            let peaks_file = peaks_path(&paths, item.id);
            let good = peaks::read(&peaks_file).unwrap();
            peaks::write(&peaks_file, &wrong).unwrap();

            let loaded = load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
            assert_eq!(*loaded.peaks, good, "{name}: rebuilt peaks");
            assert_eq!(peaks::read(&peaks_file).unwrap(), good, "{name}: on disk");
        }
    }

    #[test]
    fn an_intact_peaks_file_is_not_touched() {
        let (_dir, paths, item) = imported_item();
        let peaks_file = peaks_path(&paths, item.id);
        let before = fs::metadata(&peaks_file).unwrap().modified().unwrap();
        load_or_rebuild(&paths, &item, 48_000, &mut |_| {}).unwrap();
        let after = fs::metadata(&peaks_file).unwrap().modified().unwrap();
        assert_eq!(before, after);
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
