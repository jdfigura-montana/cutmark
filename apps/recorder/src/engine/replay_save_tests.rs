use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use chrono::Local;
use media_io::AudioCodec;

use super::*;
use crate::config::{Container, Profile, WebcamSettings};
use crate::engine::session::TrackSpec;

/// The ticks per second the merge tests count in.
const FREQUENCY: i64 = 10_000;

/// How long a test waits for the writer thread before it gives up.
const WAIT: Duration = Duration::from_secs(10);

#[test]
fn two_saves_under_two_seconds_apart_count_as_one() {
    let mut merge = SaveMerge::default();
    assert!(!merge.merges(0, FREQUENCY), "the first save always counts");
    merge.saved(50_000);
    assert!(merge.merges(50_000, FREQUENCY));
    assert!(merge.merges(69_999, FREQUENCY));
    assert!(!merge.merges(70_000, FREQUENCY), "two seconds apart are two saves");
    // The window runs from the last save that was queued, so presses that were merged do not stretch it.
    assert!(merge.merges(65_000, FREQUENCY));
    assert!(!merge.merges(75_000, FREQUENCY));
}

#[test]
fn a_save_while_another_is_writing_waits_its_turn() -> Result<(), Box<dyn std::error::Error>> {
    let (started_tx, started) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let queue = SaveQueue::start(move |job: u32| {
        let _ = started_tx.send(job);
        let _ = released.recv_timeout(WAIT);
        job * 10
    })?;
    assert!(queue.push(1));
    assert_eq!(started.recv_timeout(WAIT)?, 1);
    // Pushing never waits for the job that is being written.
    assert!(queue.push(2));
    assert!(queue.push(3));
    assert_eq!(queue.try_result(), None);
    assert!(started.try_recv().is_err(), "the second job started before the first was done");
    for _ in 0..3 {
        release.send(())?;
    }
    assert_eq!(queue.finish(), [10, 20, 30]);
    assert_eq!(started.try_iter().collect::<Vec<_>>(), [2, 3]);
    Ok(())
}

#[test]
fn takes_picked_at_the_same_time_get_their_own_numbers() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::temp_dir().join(format!("cutmark_replay_numbers_{}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    let settings = Settings { root_folder: Some(root.clone()), ..Settings::default() };
    let now = Local::now();
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let settings = settings.clone();
            std::thread::spawn(move || takes::next_take_for(&settings, now, Container::Mkv).map(|take| take.folder))
        })
        .collect();
    let mut folders = BTreeSet::new();
    for thread in threads {
        folders.insert(thread.join().map_err(|_| "a thread panicked")??);
    }
    assert_eq!(folders.len(), 8);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}

/// A capture config with two tracks at 8 Mbps of video, no camera and 2 s between keyframes.
fn config() -> SessionConfig {
    let mut profile = Profile::tutorial_av1_1440p60();
    profile.rate_control = RateControl::Cbr { mbps: 8.0 };
    profile.keyframe_secs = 2.0;
    profile.webcam = None;
    let track = |name: &str| TrackSpec {
        name: name.to_owned(),
        device: None,
        device_id: String::new(),
        device_name: String::new(),
        gain_db: 0.0,
        muted: false,
        offset_ms: 0,
        in_mixdown: true,
        app: None,
    };
    SessionConfig {
        adapter: 0,
        monitor: 0,
        cursor: false,
        profile,
        output_file: PathBuf::new(),
        tracks: vec![track("Desktop"), track("Mic")],
        audio_codec: AudioCodec::Aac { kbps: 320 },
        mixdown: true,
        track_streams: true,
        wav_sidecars: false,
        take_folder: PathBuf::new(),
        base_name: String::new(),
    }
}

#[test]
fn the_estimate_counts_every_stream_a_take_writes() {
    // 32 s: the 30 s length and one keyframe interval.
    let estimate = |cfg: &SessionConfig| ram_estimate(cfg, Duration::from_secs(30));
    let mut cfg = config();
    // 1 MB/s of video, and the mixdown and two tracks at 40 kB/s each.
    assert_eq!(estimate(&cfg), 32 * (1_000_000 + 3 * 40_000));
    cfg.mixdown = false;
    assert_eq!(estimate(&cfg), 32 * (1_000_000 + 2 * 40_000));
    cfg.track_streams = false;
    assert_eq!(estimate(&cfg), 32 * 1_000_000);
    // WAV sidecars keep 48 kHz stereo float PCM for every track.
    cfg.wav_sidecars = true;
    assert_eq!(estimate(&cfg), 32 * (1_000_000 + 2 * 384_000));
    // A PCM stream is 24-bit.
    (cfg.wav_sidecars, cfg.mixdown, cfg.audio_codec) = (false, true, AudioCodec::Pcm24);
    assert_eq!(estimate(&cfg), 32 * (1_000_000 + 288_000));
}

#[test]
fn the_estimate_counts_the_camera_only_when_it_gets_a_file() {
    let mut cfg = config();
    cfg.tracks.clear();
    let mut webcam = WebcamSettings { iso: false, iso_rc: RateControl::VbrPeak { target_mbps: 16.0, max_mbps: 30.0 }, ..WebcamSettings::default() };
    cfg.profile.webcam = Some(webcam.clone());
    assert_eq!(ram_estimate(&cfg, Duration::from_secs(60)), 62 * 1_000_000);
    webcam.iso = true;
    cfg.profile.webcam = Some(webcam);
    assert_eq!(ram_estimate(&cfg, Duration::from_secs(60)), 62 * 3_000_000);
}

#[test]
fn each_rate_control_counts_at_its_target_or_its_ceiling() {
    assert_eq!(video_bytes_per_s(RateControl::Cbr { mbps: 8.0 }), 1_000_000.0);
    assert_eq!(video_bytes_per_s(RateControl::VbrPeak { target_mbps: 16.0, max_mbps: 40.0 }), 2_000_000.0);
    assert_eq!(video_bytes_per_s(RateControl::ConstantQuality { quality: 70, max_mbps: 40.0 }), 5_000_000.0);
    assert_eq!(video_bytes_per_s(RateControl::ConstantQp { qp_i: 20, qp_p: 22 }), 5_000_000.0);
}

#[test]
fn only_more_than_a_quarter_of_the_free_memory_is_over_the_limit() {
    assert!(!over_ram_limit(1_000, 4_000));
    assert!(over_ram_limit(1_001, 4_000));
    assert!(over_ram_limit(1, 0));
    assert!(!over_ram_limit(u64::MAX / 4, u64::MAX));
    assert!(over_ram_limit(u64::MAX, u64::MAX));
}

#[test]
fn windows_says_how_much_memory_is_free() {
    assert!(free_physical_memory().is_some_and(|free| free > 0));
}
