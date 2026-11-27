//! Tests of the replay rings and of writing a snapshot as a take. The video is fake, so no hardware encoder is needed.

use std::path::Path;

use ffmpeg_next::codec::Parameters;
use media_io::AudioEncoder;

use super::super::marks::wav_length_ms;
use super::super::session::TrackSpec;
use super::*;
use crate::config::{Container, Profile};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const FPS: u32 = 30;
/// Audio samples per video frame at 48 kHz and 30 fps.
const SAMPLES_PER_FRAME: i64 = 1600;

fn scratch(name: &str) -> std::io::Result<PathBuf> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("cutmark_replay_{name}_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn config(folder: &Path, wav_sidecars: bool) -> SessionConfig {
    let mut profile = Profile::tutorial_av1_1440p60();
    (profile.width, profile.height, profile.fps, profile.container) = (16, 16, FPS, Container::Mkv);
    SessionConfig {
        adapter: 0,
        monitor: 0,
        cursor: false,
        profile,
        output_file: folder.join("buffer.mkv"),
        tracks: vec![TrackSpec {
            name: "Mic".to_owned(),
            device: None,
            device_id: String::new(),
            device_name: String::new(),
            gain_db: 0.0,
            muted: false,
            offset_ms: 0,
            in_mixdown: true,
            app: None,
        }],
        audio_codec: AudioCodec::Aac { kbps: 128 },
        mixdown: true,
        track_streams: true,
        wav_sidecars,
        take_folder: folder.to_path_buf(),
        base_name: "buffer".to_owned(),
    }
}

/// Codec parameters for a tiny MJPEG video stream, which needs no encoder to describe.
fn mjpeg_params() -> Parameters {
    let mut params = Parameters::new();
    // SAFETY: params owns a fresh AVCodecParameters, so writing its plain fields is sound.
    #[allow(unsafe_code)]
    unsafe {
        let raw = params.as_mut_ptr();
        (*raw).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*raw).codec_id = ffmpeg_next::ffi::AVCodecID::AV_CODEC_ID_MJPEG;
        (*raw).width = 16;
        (*raw).height = 16;
    }
    params
}

fn fake_packet(pts: i64, key: bool) -> EncodedPacket {
    let mut packet = Packet::copy(&[0xFF, 0xD8, 0xFF, 0xD9]);
    packet.set_pts(Some(pts));
    packet.set_dts(Some(pts));
    if key {
        packet.set_flags(Flags::KEY);
    }
    EncodedPacket::from_raw_packet(packet)
}

fn video_setup() -> VideoSetup {
    VideoSetup { params: SendParameters(mjpeg_params()), time_base: (1, FPS as i32) }
}

fn cam_stream() -> CamStream {
    CamStream { params: SendParameters(mjpeg_params()), time_base: (1, FPS as i32), fps: (FPS, 1), size: (16, 16) }
}

fn aac_spec(encoder: &AudioEncoder, title: &str, default: bool) -> AudioStreamSpec {
    AudioStreamSpec { params: SendParameters(encoder.parameters()), time_base: encoder.time_base(), title: title.to_owned(), default }
}

/// Rings with fake video and one AAC stream whose packets the test makes itself.
fn fake_rings(camera: bool, wav: bool, seconds: u64) -> Result<ReplayRings, MediaError> {
    let encoder = AudioEncoder::open(AudioCodec::Aac { kbps: 128 }, OUT_RATE, OUT_CHANNELS)?;
    let cfg = config(Path::new("unused"), wav);
    let audio = vec![aac_spec(&encoder, "Mixdown", true)];
    Ok(ReplayRings::new(&cfg, video_setup(), camera.then(cam_stream), audio, Duration::from_secs(seconds)))
}

fn front_pts(ring: &PacketRing) -> Option<i64> {
    ring.packets.front().map(|packet| packet.pts)
}

#[test]
fn trimming_keeps_a_keyframe_first_and_at_least_the_length() -> TestResult {
    let mut rings = fake_rings(false, false, 5)?;
    rings.push_video(fake_packet(0, false));
    assert!(rings.video.ring.packets.is_empty(), "nothing before the first keyframe is kept");
    // Two second groups of pictures for 20 s.
    for pts in 0..600i64 {
        rings.push_video(fake_packet(pts, pts % 60 == 0));
        let front = rings.video.ring.packets.front().ok_or("empty ring")?;
        assert!(front.key, "frame {pts} left a non-keyframe first");
        let span = pts + 1 - front.pts;
        if pts + 1 >= 150 {
            assert!((150..150 + 60).contains(&span), "frame {pts}: the ring spans {span} frames");
        } else {
            assert_eq!(front.pts, 0);
        }
    }
    Ok(())
}

#[test]
fn a_snapshot_starts_on_the_newest_keyframe_at_or_before_now_minus_the_length() -> TestResult {
    let mut rings = fake_rings(false, false, 5)?;
    assert!(rings.snapshot(1000).is_none(), "no video, nothing to save");
    // Keyframes with uneven gaps; 150 frames are 5 s, so nothing is trimmed.
    for pts in 0..150i64 {
        rings.push_video(fake_packet(pts, [0, 10, 20, 100, 130, 140].contains(&pts)));
    }
    for (now_ms, first) in [(5000, 0), (5333, 0), (5334, 10), (5700, 20), (8400, 100), (9400, 130), (60_000, 140)] {
        let snapshot = rings.snapshot(now_ms).ok_or("no snapshot")?;
        assert_eq!(snapshot.video_start_pts, first, "now {now_ms} ms");
        assert!(snapshot.video.first().is_some_and(|packet| packet.key));
    }
    // A buffer still shorter than the length starts at its oldest keyframe and ends at the press.
    let early = rings.snapshot(2000).ok_or("no snapshot")?;
    assert_eq!((early.video_start_pts, early.video.len(), early.duration_ms), (0, 60, 2000));
    Ok(())
}

#[test]
fn audio_camera_pcm_and_marks_are_cut_to_the_video_start() -> TestResult {
    let mut rings = fake_rings(true, true, 5)?;
    rings.push_mark(NewMark::point(MarkType::Note, 5999, Source::Hotkey));
    rings.push_mark(NewMark::point(MarkType::Note, 6000, Source::Hotkey));
    let mut audio_pts = -AAC_FRAME_SAMPLES;
    for frame in 0..360i64 {
        rings.push_video(fake_packet(frame, frame % 60 == 0));
        // The camera's keyframes sit one frame after the main video's.
        rings.push_camera(fake_packet(frame, frame % 60 == 1));
        while audio_pts < (frame + 1) * SAMPLES_PER_FRAME {
            rings.push_audio(0, fake_packet(audio_pts, true));
            audio_pts += AAC_FRAME_SAMPLES;
        }
        rings.push_pcm(0, (frame * SAMPLES_PER_FRAME) as u64, &[0.0; 2 * SAMPLES_PER_FRAME as usize]);
    }
    // 12 s buffered with 5 s to keep: the video starts on the keyframe at 6 s.
    let start_sample = 6 * i64::from(OUT_RATE);
    assert_eq!(front_pts(&rings.video.ring), Some(180));
    // The camera keeps its own newest keyframe at or before that, from 4 s.
    assert_eq!(rings.camera.as_ref().and_then(|camera| front_pts(&camera.ring)), Some(121));
    let audio_front = rings.audio.first().and_then(|audio| front_pts(&audio.ring)).ok_or("no audio")?;
    assert!((start_sample - AAC_FRAME_SAMPLES..start_sample).contains(&audio_front), "{audio_front}");
    assert_eq!(rings.pcm.first().and_then(|pcm| pcm.blocks.front()).map(|block| block.start), Some(start_sample as u64));
    assert_eq!(rings.marks.marks.len(), 1);

    // A press at 11.5 s still starts at 6 s, and every stream ends at the press.
    let snapshot = rings.snapshot(11_500).ok_or("no snapshot")?;
    assert_eq!((snapshot.start_us, snapshot.duration_ms), (6_000_000, 5500));
    let camera = snapshot.camera.as_ref().ok_or("no camera")?;
    assert_eq!((camera.packets.first().map(|p| p.pts), camera.packets.last().map(|p| p.pts)), (Some(121), Some(344)));
    let audio = &snapshot.audio.first().ok_or("no audio")?.packets;
    assert_eq!(audio.first().map(|p| p.pts), Some(audio_front));
    assert!(audio.last().is_some_and(|p| p.time_us < 11_500_000 && p.time_us > 11_470_000));
    assert_eq!((snapshot.pcm_from, snapshot.pcm_to), (288_000, 552_000));
    assert_eq!(snapshot.marks.iter().map(|mark| mark.t_ms).collect::<Vec<_>>(), vec![0]);
    Ok(())
}

/// The presentation times in ms of a file's packets, per stream.
fn packet_times(path: &Path) -> Result<Vec<Vec<i64>>, Box<dyn std::error::Error>> {
    let mut input = ffmpeg_next::format::input(path)?;
    let mut times = vec![Vec::new(); input.streams().count()];
    for (stream, packet) in input.packets() {
        let base = stream.time_base();
        if let (Some(pts), Some(list)) = (packet.pts(), times.get_mut(stream.index())) {
            list.push(scale(pts, 1000 * i64::from(base.numerator()), i64::from(base.denominator())));
        }
    }
    Ok(times)
}

#[test]
fn a_saved_replay_is_a_take_with_its_streams_marks_wav_and_session_file() -> TestResult {
    fn assert_send<T: Send>() {}
    assert_send::<ReplaySnapshot>();
    media_io::init()?;
    // The writer creates the take folder.
    let dir = scratch("take")?.join("take07");
    let mut cfg = config(&dir, true);
    cfg.profile.layout = crate::config::LayoutKind::Pip;
    cfg.profile.webcam = Some(crate::config::WebcamSettings::default());
    let mut encoders = Vec::new();
    for _ in 0..2 {
        encoders.push(AudioEncoder::open(cfg.audio_codec, OUT_RATE, OUT_CHANNELS)?);
    }
    let specs = encoders.iter().zip(["Mixdown", "Mic"]).enumerate().map(|(slot, (encoder, title))| aac_spec(encoder, title, slot == 0)).collect();
    let mut rings = ReplayRings::new(&cfg, video_setup(), Some(cam_stream()), specs, Duration::from_secs(5));

    // A Note older than the clip, two presses of one hotkey 2 s apart, a range that began before the clip and a
    // System mark.
    rings.push_mark(NewMark::point(MarkType::Note, 1000, Source::Hotkey));
    rings.push_mark(NewMark { end_ms: Some(9000), ..NewMark::point(MarkType::Highlight, 5000, Source::Hotkey) });
    rings.push_mark(NewMark::point(MarkType::Note, 8000, Source::Hotkey));
    rings.push_mark(NewMark::point(MarkType::Note, 10_000, Source::Hotkey));
    rings.push_mark(NewMark { label: Some("Paused".to_owned()), ..NewMark::point(MarkType::System, 11_000, Source::Auto) });

    let block = vec![0.25f32; SAMPLES_PER_FRAME as usize * 2];
    // 12 s with a keyframe every second; the camera's keyframes sit two frames later.
    for frame in 0..360i64 {
        rings.push_video(fake_packet(frame, frame % 30 == 0));
        rings.push_camera(fake_packet(frame, frame % 30 == 2));
        for (slot, encoder) in encoders.iter_mut().enumerate() {
            encoder.send(&block)?;
            encoder.receive(&mut |packet| rings.push_audio(slot, packet))?;
        }
        rings.push_pcm(0, (frame * SAMPLES_PER_FRAME) as u64, &block);
    }
    let snapshot = rings.snapshot(12_000).ok_or("no snapshot")?;
    let held = rings.video.ring.packets.len();

    let take = TakePaths { folder: dir.clone(), base_name: "take07".to_owned(), output_file: dir.join("take07.mkv") };
    let saved = write_replay(snapshot, &cfg, &take)?;
    assert_eq!((saved.cam_notice, saved.marks_warning), (None, None));
    assert_eq!(rings.video.ring.packets.len(), held, "saving leaves the buffer alone");
    assert_eq!((saved.summary.frames, saved.summary.duration_ms), (150, 5000));

    let info = media_io::player::probe::probe(&take.output_file)?;
    assert!(info.video.is_some());
    assert_eq!(info.audio.iter().map(|audio| audio.title.clone()).collect::<Vec<_>>(), ["Mixdown", "Mic"].map(|t| Some(t.to_owned())));
    assert!((4900..=6000).contains(&info.duration_ms), "{}", info.duration_ms);
    let times = packet_times(&take.output_file)?;
    let first = |stream: usize| times.get(stream).and_then(|list| list.first().copied()).ok_or("empty stream");
    // Matroska shifts the whole file by the AAC priming, as it does for a recording.
    assert!((0..=22).contains(&first(0)?), "{}", first(0)?);
    for stream in [1, 2] {
        assert!((first(0)? - first(stream)?).abs() <= 33, "stream {stream} starts at {}", first(stream)?);
    }

    let cam = saved.cam.ok_or("no camera file")?;
    assert_eq!(cam.file, dir.join("take07_cam.mkv"));
    let cam_times = packet_times(&cam.file)?;
    // The clip starts at frame 210; the camera file starts on its keyframe at frame 182, with every frame at its own time.
    assert_eq!(cam_times.first().map(|list| list.iter().take(2).copied().collect::<Vec<_>>()), Some(vec![0, 33]));
    assert_eq!(cam.duration_ms, 5933);

    assert_eq!(saved.summary.wav_files, vec![dir.join("take07_t1_Mic.wav")]);
    assert_eq!(saved.summary.wav_files.first().and_then(|wav| wav_length_ms(wav)), Some(5000));

    let (sidecar, _) = ::marks::load(&dir.join("take07.marks.json"))?;
    let marks: Vec<(MarkType, u64, Option<u64>, Option<&str>)> =
        sidecar.marks.iter().map(|mark| (mark.mark_type, mark.t_ms, mark.end_ms, mark.label.as_deref())).collect();
    assert_eq!(marks.len(), 5, "{marks:?}");
    assert!(marks.contains(&(MarkType::Highlight, 0, Some(5000), Some("Replay"))), "{marks:?}");
    assert!(marks.contains(&(MarkType::Highlight, 0, Some(2000), None)), "{marks:?}");
    assert!(marks.contains(&(MarkType::Note, 1000, None, None)) && marks.contains(&(MarkType::Note, 3000, None, None)), "{marks:?}");
    assert!(marks.contains(&(MarkType::System, 4000, None, Some("Paused"))), "{marks:?}");
    assert_eq!(sidecar.media.len(), 3);
    assert_eq!(sidecar.media.iter().filter(|media| media.duration_ms == 5000).count(), 2);
    let made = |t_ms: u64| sidecar.marks.iter().find(|mark| mark.mark_type == MarkType::Note && mark.t_ms == t_ms).map(|mark| mark.created_at);
    assert_eq!(made(3000).zip(made(1000)).map(|(late, early)| (late - early).num_milliseconds()), Some(2000));
    assert!(dir.join("take07_marks.md").exists() && dir.join("take07_marks.csv").exists());

    let session: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("take07_session.json"))?)?;
    assert_eq!(session["capture"], "replay");
    assert_eq!(session["replay_length_s"], 5);
    assert_eq!(session["take"], "take07");
    assert_eq!(session["video_file"], "take07.mkv");
    assert_eq!(session["cam_file"], "take07_cam.mkv");
    assert_eq!(session["result"]["finished"], true);
    assert_eq!(session["composite"]["layers"][0]["start_offset_ms"], 933);
    assert!(saved.finalize_warnings.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
