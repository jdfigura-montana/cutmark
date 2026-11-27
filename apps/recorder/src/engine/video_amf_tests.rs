//! A check against the real AMF encoder, run by hand on a PC with an AMD GPU and an unlocked desktop:
//! `cargo test -p cutmark-recorder -q -- --ignored real_amf --nocapture --test-threads=1`

use media_io::player::video::VideoDecoder;

use super::super::replay::{ReplayRings, TakePaths, write_replay};
use super::*;
use crate::config::{Container, Profile};

/// How long the encoder runs, and how much of it the ring keeps. The difference makes the clip start mid-stream,
/// so nothing the encoder only sent with its first packet is in it.
const CAPTURE: Duration = Duration::from_secs(10);
const KEPT: Duration = Duration::from_secs(4);

fn check(name: &str, mut profile: Profile) -> Result<(), Box<dyn std::error::Error>> {
    media_io::init()?;
    let folder = std::env::temp_dir().join(format!("cutmark_amf_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&folder)?;
    profile.container = Container::Mkv;
    let cfg = SessionConfig {
        adapter: 0,
        monitor: 0,
        cursor: true,
        profile,
        output_file: folder.join("buffer.mkv"),
        tracks: Vec::new(),
        audio_codec: media_io::AudioCodec::Aac { kbps: 192 },
        mixdown: false,
        track_streams: false,
        wav_sidecars: false,
        take_folder: folder.clone(),
        base_name: "buffer".to_owned(),
    };
    let (mut pipeline, facts) = setup(&cfg)?;
    let mut rings = ReplayRings::new(&cfg, facts, None, Vec::new(), KEPT);
    let stats = Stats::new(&cfg.tracks);
    let mut pacer = Pacer::new(qpc_frequency(), cfg.profile.fps, 1, qpc_now());
    let began = Instant::now();
    while began.elapsed() < CAPTURE {
        for n in pacer.due_frames(qpc_now()) {
            let Some(frame) = pipeline.compositor.latest() else { continue };
            let pts = pacer.pts_for(n);
            let mut sink = |packet| {
                rings.push_video(packet);
                Ok(())
            };
            encode_one_frame(&mut pipeline, &mut sink, &stats, &frame, pts)?;
            pacer.record(n, frame.content_seq, true);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let now_ms = u64::try_from(CAPTURE.as_millis())?;
    for container in [Container::Mkv, Container::Mp4] {
        let mut cfg = cfg.clone();
        cfg.profile.container = container;
        let extension = format!("{container:?}").to_lowercase();
        let base_name = format!("clip_{extension}");
        let take = TakePaths { folder: folder.clone(), output_file: folder.join(format!("{base_name}.{extension}")), base_name };
        let snapshot = rings.snapshot(now_ms).ok_or("the ring holds no video")?;
        assert!(snapshot.start_us() > 1_000_000, "the clip must start mid-stream, not at {} us", snapshot.start_us());
        let saved = write_replay(snapshot, &cfg, &take)?;
        let info = media_io::player::probe::probe(&take.output_file)?;
        assert!(info.video.is_some(), "{name} {extension}: no video stream");
        let decoder = VideoDecoder::open(&take.output_file, 640)?;
        decoder.play();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut decoded = false;
        while !decoded && Instant::now() < deadline {
            decoded = decoder.frame_at(500).is_some();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(decoded, "{name} {extension}: no frame decoded from {}", take.output_file.display());
        println!("{name} {extension}: {} frames, {} ms, opens and decodes: {}", saved.summary.frames, info.duration_ms, take.output_file.display());
    }
    Ok(())
}

#[test]
#[ignore = "needs an AMD GPU and an unlocked desktop; run by hand"]
fn real_amf_av1_replay_clip_from_mid_stream_decodes() -> Result<(), Box<dyn std::error::Error>> {
    check("av1_amf", Profile::tutorial_av1_1440p60())
}

#[test]
#[ignore = "needs an AMD GPU and an unlocked desktop; run by hand"]
fn real_amf_hevc_replay_clip_from_mid_stream_decodes() -> Result<(), Box<dyn std::error::Error>> {
    check("hevc_amf", Profile::premiere_hevc_1080p60())
}
