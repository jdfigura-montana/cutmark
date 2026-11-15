//! model::validate: checks for a project read from disk (project.json, autosave).
//!
//! `invariants::check` guards edits; a file on disk is untrusted, so on top of
//! it this module bounds every value that later reaches a path join, a
//! division, an allocation or a loop: the sample rate, channel counts, media
//! file names, level ranges, element counts and the 24 hour length limit.

use std::path::{Component, Path};

use thiserror::Error;
use uuid::Uuid;

use crate::model::invariants::{self, InvariantError};
use crate::model::project::Project;
use crate::model::track::{DEFAULT_HEIGHT, MAX_HEIGHT, MIN_HEIGHT};

/// Lowest accepted project sample rate in Hz.
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// Highest accepted project sample rate in Hz. A range, not a fixed set:
/// older projects may be 44,000 or 88,000 Hz.
pub const MAX_SAMPLE_RATE: u32 = 192_000;
/// Most media items a loaded project may hold.
pub const MAX_MEDIA: usize = 10_000;
/// Most tracks a loaded project may hold.
pub const MAX_TRACKS: usize = 1_000;
/// Most clips (over all tracks) a loaded project may hold.
pub const MAX_CLIPS: usize = 100_000;
/// Longest accepted timeline, in seconds (24 hours).
pub const MAX_SECONDS: u64 = 24 * 60 * 60;

/// Longest text shown from a file's contents inside an error message.
const MAX_SHOWN_CHARS: usize = 48;

/// Why a loaded project was rejected; `Display` is the human-readable reason.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LoadError {
    /// The sample rate is outside `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE`.
    #[error("the sample rate {0} Hz is outside the supported range of 8000 to 192000 Hz")]
    SampleRate(u32),
    /// A list is longer than the loader accepts.
    #[error("it has too many {what} ({count}; the limit is {limit})")]
    TooMany {
        /// Name of the list ("media items", "tracks" or "clips").
        what: &'static str,
        /// Length found.
        count: usize,
        /// Length allowed.
        limit: usize,
    },
    /// A media item is neither mono nor stereo.
    #[error("media item {index} has {channels} channels (only 1 or 2 are supported)")]
    Channels {
        /// 1-based position in the media bin.
        index: usize,
        /// Channel count found.
        channels: u16,
    },
    /// A media file name could leave the media folder or name a device.
    #[error("the media file name {name:?} is not allowed: {reason}")]
    FileName {
        /// The name, shortened for display.
        name: String,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A level is not a finite number inside its allowed range.
    #[error("{what} is {value}, outside the allowed range {min} to {max}")]
    OutOfRange {
        /// Which value ("track 2 volume", ...).
        what: String,
        /// Value found.
        value: f32,
        /// Lowest allowed value.
        min: f32,
        /// Highest allowed value.
        max: f32,
    },
    /// A clip ends before it starts in its source media.
    #[error("clip {0} has its source end before its source start")]
    ClipSource(Uuid),
    /// A clip ends after the 24 hour limit.
    #[error("clip {0} ends after the 24 hour limit")]
    ClipTooLong(Uuid),
    /// A marker sits after the 24 hour limit.
    #[error("marker {0} is after the 24 hour limit")]
    MarkerTooLate(Uuid),
    /// The data breaks a model rule (overlap, bounds, fades, duplicate ids...).
    #[error("the project data is inconsistent: {0}")]
    Inconsistent(#[from] InvariantError),
}

/// Text from a file, cut to a length that fits an error message.
fn shorten(text: &str) -> String {
    let mut shown: String = text.chars().take(MAX_SHOWN_CHARS).collect();
    if text.chars().nth(MAX_SHOWN_CHARS).is_some() {
        shown.push_str("...");
    }
    shown
}

/// True for a Windows reserved device name, with or without an extension and
/// in any case: CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM0-9, LPT0-9 (also the
/// superscript digits Windows reserves).
pub fn is_reserved_device_name(name: &str) -> bool {
    // "CON.txt" and "CON .txt" still name the device: look at what precedes
    // the first dot, without trailing spaces.
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let Some(digit) = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
    else {
        return false;
    };
    let mut chars = digit.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(c), None) if c.is_ascii_digit() || matches!(c, '\u{b9}' | '\u{b2}' | '\u{b3}')
    )
}

/// Checks that `name` is one plain file name for the project's media folder.
///
/// Rejected: empty names, control characters, path separators, drive or UNC
/// prefixes, `..` and `.`, `:` (drives and alternate streams), `"` and the
/// other characters Windows forbids in file names (`<>|?*`), and reserved
/// device names. Joining an accepted name onto `media/` cannot leave it.
pub fn check_media_file_name(name: &str) -> Result<(), LoadError> {
    let reject = |reason: &'static str| LoadError::FileName {
        name: shorten(name),
        reason,
    };
    if name.is_empty() {
        return Err(reject("it is empty"));
    }
    if name.chars().any(char::is_control) {
        return Err(reject("it contains a control character"));
    }
    if name.contains(['/', '\\']) {
        return Err(reject("it contains a path separator"));
    }
    if name.contains(':') {
        return Err(reject("it contains a colon (drive or stream marker)"));
    }
    if name.contains(['"', '<', '>', '|', '?', '*']) {
        return Err(reject(
            "it contains a quote or another character Windows forbids",
        ));
    }
    if is_reserved_device_name(name) {
        return Err(reject("it is a reserved Windows device name"));
    }
    // Belt and braces: the whole text must be exactly one normal path component.
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if part == std::ffi::OsStr::new(name) => Ok(()),
        _ => Err(reject("it is not a plain file name")),
    }
}

/// True when `value` is a finite number in `min..=max`.
fn level_ok(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}

/// Builds the error for a level that is not finite or out of range.
fn out_of_range(what: String, value: f32, min: f32, max: f32) -> LoadError {
    LoadError::OutOfRange {
        what,
        value,
        min,
        max,
    }
}

/// Clamps every track height to the lane limits (a non-number becomes the
/// default height). Heights are UI-only, so a bad value is repaired rather
/// than rejected.
pub fn clamp_track_heights(project: &mut Project) {
    for track in &mut project.tracks {
        track.height = if track.height.is_nan() {
            DEFAULT_HEIGHT
        } else {
            track.height.clamp(MIN_HEIGHT, MAX_HEIGHT)
        };
    }
}

/// Validates a project read from disk. Pure: no I/O, never panics, and the
/// cheap count checks run before anything proportional to the data.
///
/// Rules: sample rate in 8,000..=192,000 Hz; at most 10,000 media items,
/// 1,000 tracks and 100,000 clips; media with 1 or 2 channels and a safe file
/// name; finite levels in range (track volume -60..=12 dB, pan -1..=1, clip
/// gain -60..=24 dB, master -60..=12 dB); every clip end and marker within 24
/// hours at the project rate; then `invariants::check`.
pub fn validate_loaded(project: &Project) -> Result<(), LoadError> {
    let rate = project.sample_rate;
    if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&rate) {
        return Err(LoadError::SampleRate(rate));
    }
    let too_many =
        |what: &'static str, count: usize, limit: usize| LoadError::TooMany { what, count, limit };
    if project.media.len() > MAX_MEDIA {
        return Err(too_many("media items", project.media.len(), MAX_MEDIA));
    }
    if project.tracks.len() > MAX_TRACKS {
        return Err(too_many("tracks", project.tracks.len(), MAX_TRACKS));
    }
    let clip_count: usize = project.tracks.iter().map(|t| t.clips.len()).sum();
    if clip_count > MAX_CLIPS {
        return Err(too_many("clips", clip_count, MAX_CLIPS));
    }

    for (index, item) in project.media.iter().enumerate() {
        if !(1..=2).contains(&item.channels) {
            return Err(LoadError::Channels {
                index: index + 1,
                channels: item.channels,
            });
        }
        check_media_file_name(&item.file_name)?;
    }

    if !level_ok(project.master_volume_db, -60.0, 12.0) {
        return Err(out_of_range(
            "the master volume".to_string(),
            project.master_volume_db,
            -60.0,
            12.0,
        ));
    }

    // At most 192,000 * 86,400, far below u64::MAX.
    let limit = MAX_SECONDS * u64::from(rate);
    for (index, track) in project.tracks.iter().enumerate() {
        let number = index + 1;
        if !level_ok(track.volume_db, -60.0, 12.0) {
            return Err(out_of_range(
                format!("track {number} volume"),
                track.volume_db,
                -60.0,
                12.0,
            ));
        }
        if !level_ok(track.pan, -1.0, 1.0) {
            return Err(out_of_range(
                format!("track {number} pan"),
                track.pan,
                -1.0,
                1.0,
            ));
        }
        for clip in &track.clips {
            if clip.src_out < clip.src_in {
                return Err(LoadError::ClipSource(clip.id));
            }
            if clip.end() > limit {
                return Err(LoadError::ClipTooLong(clip.id));
            }
            if !level_ok(clip.gain_db, -60.0, 24.0) {
                return Err(out_of_range(
                    format!("the gain of a clip on track {number}"),
                    clip.gain_db,
                    -60.0,
                    24.0,
                ));
            }
        }
    }
    for marker in &project.markers {
        if marker.position > limit {
            return Err(LoadError::MarkerTooLate(marker.id));
        }
    }

    invariants::check(project)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::clip::Clip;
    use crate::model::fixtures::project_with_clips;
    use crate::model::marker::Marker;
    use crate::model::media::MediaItem;
    use crate::model::track::Track;

    /// A valid project with two clips on one track (media file "test.wav").
    fn valid() -> Project {
        project_with_clips(&[&[(0, 1_000), (2_000, 500)]])
    }

    #[test]
    fn a_valid_project_passes() {
        assert_eq!(validate_loaded(&valid()), Ok(()));
        let empty = Project::new(
            "Empty".to_string(),
            48_000,
            crate::model::project::BitDepth::Int24,
        );
        assert_eq!(validate_loaded(&empty), Ok(()));
    }

    /// The text of the rejection, panicking when the project is accepted.
    fn reason(project: &Project) -> String {
        validate_loaded(project)
            .expect_err("the project should be rejected")
            .to_string()
    }

    #[test]
    fn sample_rate_is_a_range_not_a_set() {
        for ok in [8_000, 44_000, 44_100, 48_000, 88_000, 96_000, 192_000] {
            let mut p = valid();
            p.sample_rate = ok;
            assert_eq!(validate_loaded(&p), Ok(()), "{ok} Hz should load");
        }
        for bad in [0, 1, 7_999, 192_001, u32::MAX] {
            let mut p = valid();
            p.sample_rate = bad;
            assert_eq!(validate_loaded(&p), Err(LoadError::SampleRate(bad)));
        }
    }

    #[test]
    fn channels_must_be_one_or_two() {
        for ok in [1, 2] {
            let mut p = valid();
            p.media[0].channels = ok;
            assert_eq!(validate_loaded(&p), Ok(()));
        }
        for bad in [0, 3, 8, u16::MAX] {
            let mut p = valid();
            p.media[0].channels = bad;
            assert!(matches!(
                validate_loaded(&p),
                Err(LoadError::Channels { index: 1, .. })
            ));
        }
    }

    #[test]
    fn unsafe_file_names_are_rejected() {
        let bad = [
            ("UNC", r"\\attacker.example\share\a.wav"),
            ("UNC with forward slashes", "//attacker.example/share/a.wav"),
            ("absolute", r"C:\Windows\win.ini"),
            ("drive relative", "C:win.ini"),
            ("rooted", r"\Windows\win.ini"),
            ("parent", r"..\a.wav"),
            ("bare parent", ".."),
            ("current dir", "."),
            ("subfolder", "sub/a.wav"),
            ("alternate stream", "a.wav:evil"),
            ("quote", "a\".wav"),
            ("wildcard", "a*.wav"),
            ("control char", "a\u{7}.wav"),
            ("newline", "a\n.wav"),
            ("empty", ""),
            ("CON", "CON"),
            ("lowercase with extension", "con.wav"),
            ("NUL", "NUL.wav"),
            ("AUX", "aux"),
            ("PRN", "Prn.mp3"),
            ("COM1", "COM1.wav"),
            ("COM9 with extra extension", "com9.tar.wav"),
            ("LPT1", "lpt1.wav"),
            ("LPT9", "LPT9"),
            ("superscript COM", "COM\u{b9}.wav"),
            ("trailing space", "CON .wav"),
            ("CONOUT$", "CONOUT$"),
        ];
        for (what, name) in bad {
            let mut p = valid();
            p.media[0].file_name = name.to_string();
            assert!(
                matches!(validate_loaded(&p), Err(LoadError::FileName { .. })),
                "{what} ({name:?}) must be rejected"
            );
            assert!(
                check_media_file_name(name).is_err(),
                "{what} ({name:?}) must be rejected by the helper"
            );
        }
    }

    #[test]
    fn ordinary_file_names_are_accepted() {
        for name in [
            "boom.wav",
            "boom (2).wav",
            "My Song.mp3",
            "consul.wav",
            "COM10.wav",
            "COM.wav",
            "comet.mp3",
            "a.b.c.wav",
            "r\u{e9}sum\u{e9}.wav",
            "lpt.wav",
        ] {
            assert_eq!(check_media_file_name(name), Ok(()), "{name:?}");
        }
    }

    #[test]
    fn a_long_hostile_name_is_shortened_in_the_message() {
        let mut p = valid();
        p.media[0].file_name = format!("{}\\x", "a".repeat(5_000));
        let text = reason(&p);
        assert!(text.len() < 300, "message was {} bytes", text.len());
        assert!(text.contains("path separator"), "{text}");
    }

    #[test]
    fn a_clip_ending_before_it_starts_is_rejected() {
        let mut p = valid();
        p.tracks[0].clips[0].src_in = 500;
        p.tracks[0].clips[0].src_out = 100;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::ClipSource(id)) if id == p.tracks[0].clips[0].id
        ));
    }

    #[test]
    fn a_clip_past_24_hours_is_rejected_but_exactly_24_hours_is_not() {
        let day = MAX_SECONDS * 48_000;
        let mut p = project_with_clips(&[&[(0, 1_000)]]);
        p.media[0].length = day + 10_000;
        p.tracks[0].clips[0].start = day - 1_000;
        assert_eq!(validate_loaded(&p), Ok(()), "ends exactly at 24 h");
        p.tracks[0].clips[0].start = day - 999;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::ClipTooLong(_))
        ));
        // A start that would overflow u64 when the length is added.
        p.tracks[0].clips[0].start = u64::MAX - 10;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::ClipTooLong(_))
        ));
    }

    #[test]
    fn a_marker_past_24_hours_is_rejected() {
        let limit = MAX_SECONDS * 48_000;
        let mut p = valid();
        p.markers.push(Marker {
            id: Uuid::new_v4(),
            position: limit,
        });
        assert_eq!(validate_loaded(&p), Ok(()));
        p.markers.push(Marker {
            id: Uuid::new_v4(),
            position: limit + 1,
        });
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::MarkerTooLate(_))
        ));
    }

    #[test]
    fn non_finite_and_out_of_range_levels_are_rejected() {
        /// Sets one level field of a project.
        type SetLevel = fn(&mut Project, f32);
        let cases: [(&str, SetLevel); 4] = [
            ("track volume", |p, v| p.tracks[0].volume_db = v),
            ("pan", |p, v| p.tracks[0].pan = v),
            ("clip gain", |p, v| p.tracks[0].clips[0].gain_db = v),
            ("master volume", |p, v| p.master_volume_db = v),
        ];
        for (what, set) in cases {
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0e9, -1.0e9] {
                let mut p = valid();
                set(&mut p, bad);
                assert!(
                    matches!(validate_loaded(&p), Err(LoadError::OutOfRange { .. })),
                    "{what} = {bad} must be rejected"
                );
            }
        }
        // Just outside each range.
        for (what, set, bad) in [
            ("track volume", cases[0].1, 12.01),
            ("track volume low", cases[0].1, -60.01),
            ("pan", cases[1].1, 1.01),
            ("clip gain", cases[2].1, 24.01),
            ("master volume", cases[3].1, 12.01),
        ] {
            let mut p = valid();
            set(&mut p, bad);
            assert!(
                matches!(validate_loaded(&p), Err(LoadError::OutOfRange { .. })),
                "{what} = {bad}"
            );
        }
        // The range ends are fine.
        for (set, edge) in [
            (cases[0].1, 12.0),
            (cases[0].1, -60.0),
            (cases[1].1, 1.0),
            (cases[1].1, -1.0),
            (cases[2].1, 24.0),
            (cases[2].1, -60.0),
            (cases[3].1, 12.0),
        ] {
            let mut p = valid();
            set(&mut p, edge);
            assert_eq!(validate_loaded(&p), Ok(()), "edge {edge}");
        }
    }

    #[test]
    fn too_many_tracks_media_or_clips_are_rejected() {
        let mut p = valid();
        p.tracks = (0..=MAX_TRACKS)
            .map(|i| Track::new(format!("T{i}")))
            .collect();
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::TooMany {
                what: "tracks",
                count: 1_001,
                ..
            })
        ));

        let mut p = valid();
        let template = p.media[0].clone();
        p.media = (0..=MAX_MEDIA)
            .map(|_| MediaItem {
                id: Uuid::new_v4(),
                ..template.clone()
            })
            .collect();
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::TooMany {
                what: "media items",
                ..
            })
        ));

        let mut p = valid();
        let template = p.tracks[0].clips[0].clone();
        p.tracks[0].clips = (0..=MAX_CLIPS)
            .map(|_| Clip {
                id: Uuid::new_v4(),
                ..template.clone()
            })
            .collect();
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::TooMany {
                what: "clips",
                count: 100_001,
                ..
            })
        ));
    }

    #[test]
    fn model_rule_violations_are_reported() {
        // Overlapping clips.
        let mut p = valid();
        p.tracks[0].clips[1].start = 500;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::Inconsistent(
                InvariantError::UnsortedOrOverlapping { .. }
            ))
        ));
        // Clip pointing at media that is not in the bin.
        let mut p = valid();
        p.tracks[0].clips[0].media_id = Uuid::new_v4();
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::Inconsistent(InvariantError::MissingMedia { .. }))
        ));
        // Duplicate track id.
        let mut p = project_with_clips(&[&[], &[]]);
        p.tracks[1].id = p.tracks[0].id;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::Inconsistent(InvariantError::DuplicateId { .. }))
        ));
    }

    #[test]
    fn hostile_fade_lengths_do_not_overflow() {
        let mut p = valid();
        p.tracks[0].clips[0].fade_in = u64::MAX;
        p.tracks[0].clips[0].fade_out = 1;
        assert!(matches!(
            validate_loaded(&p),
            Err(LoadError::Inconsistent(InvariantError::FadeSum { .. }))
        ));
    }

    #[test]
    fn a_large_valid_project_validates_quickly() {
        // 1,000 tracks x 100 clips: the id and media lookups must stay linear.
        let placements: Vec<(u64, u64)> = (0..100).map(|i| (i * 2_000, 1_000)).collect();
        let rows: Vec<&[(u64, u64)]> = (0..1_000).map(|_| placements.as_slice()).collect();
        let p = project_with_clips(&rows);
        let started = std::time::Instant::now();
        assert_eq!(validate_loaded(&p), Ok(()));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "validation took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn track_heights_are_clamped_not_rejected() {
        let mut p = project_with_clips(&[&[], &[], &[], &[], &[]]);
        for (track, height) in p
            .tracks
            .iter_mut()
            .zip([5.0, 1.0e9, f32::INFINITY, f32::NAN, 120.0])
        {
            track.height = height;
        }
        clamp_track_heights(&mut p);
        let heights: Vec<f32> = p.tracks.iter().map(|t| t.height).collect();
        assert_eq!(heights, vec![40.0, 400.0, 400.0, 96.0, 120.0]);
        assert_eq!(validate_loaded(&p), Ok(()));
    }

    #[test]
    fn reserved_device_names_are_detected() {
        for name in [
            "con", "PRN", "Aux.txt", "nul", "COM1", "lpt9.wav", "CON .", "COM0", "LPT0",
        ] {
            assert!(is_reserved_device_name(name), "{name}");
        }
        for name in ["", "CONSOLE", "COM10", "LPT", "My Project", "xcon"] {
            assert!(!is_reserved_device_name(name), "{name}");
        }
    }
}
