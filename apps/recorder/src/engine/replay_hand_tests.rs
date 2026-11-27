//! A temporary hand test of the replay buffer with the real screen, audio devices and encoder. It runs only when asked
//! for.

use std::error::Error;
use std::time::{Duration, Instant};

use ffmpeg_next::media::Type;

use super::{audio_codec, track_specs};
use crate::config::{Profile, Settings, default_tracks};
use crate::engine::replay_save::{ReplayWriter, SaveJob, write_job};
use crate::engine::session::{RecordingSession, SessionConfig};

/// How long the buffer runs before the save.
const RUN: Duration = Duration::from_secs(20);

#[test]
#[ignore = "hand test: records the real screen and audio devices for 20 s"]
fn real_replay_buffer_saves_a_playable_take() -> Result<(), Box<dyn Error>> {
    media_io::init()?;
    // Everything is written under a temp folder, so the user's takes and settings are not touched.
    let root = std::env::temp_dir().join(format!("cutmark_replay_hand_{}", std::process::id()));
    let mut settings = Settings { root_folder: Some(root), replay_length_s: 30, ..Settings::default() };
    settings.audio.tracks = default_tracks(&capture::list_audio_devices()?);
    let profile = Profile::tutorial_av1_1440p60();
    let (tracks, missing) = track_specs(&settings);
    assert!(missing.is_empty(), "missing audio devices: {missing:?}");
    let (mixdown, track_streams, keyframe_secs) = (profile.mixdown, profile.aac_tracks, f64::from(profile.keyframe_secs));
    let audio_streams = usize::from(mixdown && !tracks.is_empty()) + if track_streams { tracks.len() } else { 0 };
    let cfg = SessionConfig {
        adapter: 0,
        monitor: 0,
        cursor: true,
        profile,
        output_file: Default::default(),
        tracks,
        audio_codec: audio_codec(settings.audio.codec),
        mixdown,
        track_streams,
        wav_sidecars: settings.audio.wav_sidecars,
        take_folder: Default::default(),
        base_name: String::new(),
    };

    let mut session = RecordingSession::default();
    let length = settings.replay_length();
    session.replay_start(move || cfg, length)?;
    let began = Instant::now();
    while began.elapsed() < RUN {
        std::thread::sleep(Duration::from_millis(100));
        session.apply_app_events();
        assert!(!session.buffer_died(), "the capture ended: {:?}", session.stats().last_error);
    }
    let (snapshot, cfg) = session.replay_snapshot(capture::qpc_now()).ok_or("the buffer holds no video")?;
    let writer = ReplayWriter::start(write_job)?;
    assert!(writer.push(SaveJob { snapshot, cfg, settings, pressed_at: chrono::Local::now() }));
    let saved = writer.finish().pop().ok_or("the writer gave no result")?;
    let dropped = session.stats().dropped;
    session.replay_stop();
    let take = saved?;

    let file = &take.summary.file;
    let input = ffmpeg_next::format::input(file)?;
    let kinds: Vec<Type> = input.streams().map(|stream| stream.parameters().medium()).collect();
    let seconds = input.duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE);
    println!("saved {} with streams {kinds:?}, {seconds:.2} s, {dropped} dropped frames", file.display());
    println!("finalize warnings: {:?}, marks warning: {:?}", take.finalize_warnings, take.marks_warning);
    assert_eq!(kinds.first(), Some(&Type::Video));
    assert_eq!(kinds.iter().filter(|kind| **kind == Type::Video).count(), 1);
    assert_eq!(kinds.iter().filter(|kind| **kind == Type::Audio).count(), audio_streams);
    assert!(take.finalize_warnings.is_empty());
    // The clip is the whole run, or the set length and up to one keyframe interval more.
    let expected = RUN.min(length).as_secs_f64();
    assert!(seconds > expected - 1.5 && seconds < expected + keyframe_secs + 1.0, "the clip is {seconds:.2} s, expected about {expected} s");
    Ok(())
}
