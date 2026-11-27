//! Bake with cursor on a short clip made the way a take is: AV1 from the AMF encoder and AAC audio in Matroska, with
//! a session.json and a synthetic events file. The tests that need the clip are skipped where it cannot be encoded.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use events::{CaptureSource, CursorSample, CursorShape, EventsWriter, Header, InputEvent, ScreenRect, ShapeFrame, Streams};
use media_io::AudioEncoder;
use media_io::player::probe::AudioInfo;

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

pub(crate) const WIDTH: u32 = 640;
/// AMF rejects heights that are not a multiple of 16.
pub(crate) const HEIGHT: u32 = 368;
const FPS: u32 = 60;
const FRAMES: u32 = 90;
/// The picture of every frame of the clip: a blue that BT.601 and BT.709 turn into visibly different pixels.
pub(crate) const BACKGROUND: [u8; 3] = [40, 90, 160];
/// The cursor: a 16 by 16 green square with its hotspot at the top left.
const CURSOR_COLOR: [u8; 3] = [0, 255, 0];
const CURSOR_SIDE: u32 = 16;
/// Where the cursor is up to frame 44, and from frame 45 on.
const FIRST_PLACE: (i32, i32) = (100, 100);
const SECOND_PLACE: (i32, i32) = (400, 200);
/// The first frame with the cursor at its second place; its time is 750 ms.
pub(crate) const MOVE_FRAME: u32 = 45;
/// The move is logged 5 ms before that frame, so a time that is off by more than that shows.
const MOVE_US: i64 = 745_000;

/// How many frames in the video of a late clip starts: 100 ms, far more than the 5 ms the move is checked to.
pub(crate) const LATE_FRAMES: u32 = 6;

/// Encodes the clip, with its video starting `late` frames after its audio's start.
fn encode_clip(path: &Path, late: u32) -> TestResult {
    media_io::init()?;
    let gpu = Gpu::create(None)?;
    let hw_device = D3d11Device::from_device(gpu.device())?;
    let pool = D3d11FramePool::new(&hw_device, WIDTH, HEIGHT, 8)?;
    let config = EncoderConfig {
        codec: Codec::Av1,
        width: WIDTH,
        height: HEIGHT,
        fps_num: FPS,
        fps_den: 1,
        rate_control: RateControl::ConstantQp { qp_i: 28, qp_p: 30 },
        preset: Preset::Speed,
        keyframe_secs: 1.0,
    };
    let mut video = AmfEncoder::open(&config, &pool)?;
    let mut audio = AudioEncoder::open(AudioCodec::Aac { kbps: 192 }, 48_000, 2)?;
    let mut muxer = Muxer::create(path, Container::Mkv)?;
    muxer.add_video(video.parameters(), video.time_base(), (FPS as i32, 1))?;
    muxer.add_audio(audio.parameters(), audio.time_base(), "Mixdown", true)?;
    muxer.start()?;

    // As long as the video: 800 stereo samples of silence per video frame.
    let mut sound = Vec::new();
    for chunk in vec![0.0f32; FRAMES as usize * 800 * 2].chunks(2_048) {
        audio.send(chunk)?;
        audio.receive(&mut |packet| sound.push(packet))?;
    }
    audio.finish(&mut |packet| sound.push(packet))?;

    let mut converter = Nv12Converter::new(&gpu, WIDTH, HEIGHT, WIDTH, HEIGHT, FPS)?;
    let nv12 = converter.create_output()?;
    let texture = upload_texture(&gpu, WIDTH, HEIGHT)?;
    let [red, green, blue] = BACKGROUND;
    let mut picture = RgbaFrame { width: WIDTH, height: HEIGHT, pts_ms: 0, data: [red, green, blue, 255].repeat((WIDTH * HEIGHT) as usize) };
    upload_picture(&gpu, &texture, (WIDTH, HEIGHT), &mut picture)?;
    converter.convert(&texture, &nv12)?;
    let mut pictures = Vec::new();
    for index in 0..FRAMES {
        let mut frame = HwFrame::from_pool(&pool)?;
        frame.copy_from(gpu.context(), &nv12)?;
        frame.set_pts(i64::from(late + index));
        video.send(frame)?;
        video.receive(&mut |packet| pictures.push(packet))?;
    }
    video.finish(&mut |packet| pictures.push(packet))?;
    for packet in pictures {
        muxer.write(0, &packet)?;
    }
    for packet in sound {
        muxer.write(1, &packet)?;
    }
    muxer.finish()?;
    Ok(())
}

/// A clip's bytes, each of the two encoded once per process.
fn clip_bytes(late: u32) -> Result<&'static [u8], String> {
    static ON_TIME: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    static LATE: OnceLock<Result<Vec<u8>, String>> = OnceLock::new();
    let clip = if late == 0 { &ON_TIME } else { &LATE };
    clip.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("cutmark_bake_clip_{}_{late}.mkv", std::process::id()));
        let made = encode_clip(&path, late).and_then(|()| Ok(fs::read(&path)?)).map_err(|err| err.to_string());
        let _ = fs::remove_file(path);
        made
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}

fn write_events(path: &Path) -> TestResult {
    let header = Header {
        qpc_frequency: 1_000_000,
        first_frame_qpc: 0,
        source: CaptureSource::Monitor(ScreenRect { x: 0, y: 0, width: WIDTH as i32, height: HEIGHT as i32 }),
        displays: Vec::new(),
        cursor_size: 32,
        streams: Streams { cursor: true, clicks: true, keys: false, apps: false },
    };
    let [red, green, blue] = CURSOR_COLOR;
    let shape = CursorShape {
        id: 1,
        width: CURSOR_SIDE,
        height: CURSOR_SIDE,
        hotspot_x: 0,
        hotspot_y: 0,
        frames: vec![ShapeFrame { duration_us: 0, bgra: [blue, green, red, 255].repeat((CURSOR_SIDE * CURSOR_SIDE) as usize) }],
    };
    let sample = |qpc, (x, y): (i32, i32)| InputEvent::Sample(CursorSample { qpc, x, y, shape: Some(1), visible: true });
    let mut writer = EventsWriter::create(path, &header)?;
    for event in [InputEvent::Shape(shape), sample(0, FIRST_PLACE), sample(MOVE_US, SECOND_PLACE)] {
        writer.push(event)?;
    }
    writer.finish(1_500_000)?;
    Ok(())
}

/// A take made of the clip in a folder of its own.
pub(crate) struct TestTake {
    pub(crate) folder: PathBuf,
    pub(crate) video: PathBuf,
    events: PathBuf,
}

impl TestTake {
    /// Err with the reason when the clip cannot be encoded here.
    pub(crate) fn new(name: &str) -> Result<Result<Self, String>, Box<dyn std::error::Error>> {
        Self::starting(name, 0)
    }

    /// A take whose video starts `late` frames into the file; only 0 and [`LATE_FRAMES`] are made.
    pub(crate) fn starting(name: &str, late: u32) -> Result<Result<Self, String>, Box<dyn std::error::Error>> {
        let clip = match clip_bytes(late) {
            Ok(clip) => clip,
            Err(reason) => return Ok(Err(reason)),
        };
        let folder = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-bake-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder)?;
        let take = Self { video: folder.join("take01.mkv"), events: folder.join("take01.events"), folder };
        fs::write(&take.video, clip)?;
        write_events(&take.events)?;
        let mut profile = Profile::tutorial_av1_1440p60();
        (profile.width, profile.height, profile.fps) = (WIDTH, HEIGHT, FPS);
        profile.rate_control = RateControl::ConstantQp { qp_i: 28, qp_p: 30 };
        fs::write(take.folder.join("take01_session.json"), serde_json::to_vec(&serde_json::json!({ "profile": profile }))?)?;
        Ok(Ok(take))
    }

    fn request(&self) -> BakeRequest {
        BakeRequest { video: self.video.clone(), events: self.events.clone(), style: CursorStyle::default() }
    }

    fn file_names(&self) -> Result<Vec<String>, std::io::Error> {
        let mut names =
            fs::read_dir(&self.folder)?.map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned())).collect::<Result<Vec<_>, _>>()?;
        names.sort();
        Ok(names)
    }
}

/// How many packets each stream of a file has, and the first time stamp of each in microseconds.
type PacketFacts = (BTreeMap<usize, u64>, BTreeMap<usize, i64>);

/// What a file's packets say.
fn packet_facts(path: &Path) -> Result<PacketFacts, Box<dyn std::error::Error>> {
    let mut input = ffmpeg_next::format::input(path)?;
    let (mut counts, mut firsts) = (BTreeMap::new(), BTreeMap::new());
    let mut packet = Packet::empty();
    loop {
        match packet.read(&mut input) {
            Ok(()) => {}
            Err(ffmpeg_next::Error::Eof) => return Ok((counts, firsts)),
            Err(err) => return Err(err.into()),
        }
        let time_base = input.stream(packet.stream()).ok_or("a packet without a stream")?.time_base();
        let at = packet.pts().ok_or("a packet without a time stamp")?.rescale(time_base, Rational::new(1, 1_000_000));
        *counts.entry(packet.stream()).or_insert(0u64) += 1;
        let first = firsts.entry(packet.stream()).or_insert(at);
        *first = (*first).min(at);
    }
}

/// Every decoded video frame as RGBA.
fn decode_all(path: &Path) -> Result<Vec<frame::Video>, Box<dyn std::error::Error>> {
    let mut input = ffmpeg_next::format::input(path)?;
    let (index, parameters) = {
        let stream = input.streams().best(Type::Video).ok_or("no video")?;
        (stream.index(), stream.parameters())
    };
    let codec = decoder::find_by_name("libdav1d").ok_or("no libdav1d")?;
    let mut decoder = ffmpeg_next::codec::Context::from_parameters(parameters)?.decoder().open_as(codec)?.video()?;
    let mut scaler = None;
    let mut frames = Vec::new();
    let mut take = |decoder: &mut decoder::Video| -> TestResult {
        let mut decoded = frame::Video::empty();
        while decoder.receive_frame(&mut decoded).is_ok() {
            if scaler.is_none() {
                let (format, width, height) = (decoded.format(), decoded.width(), decoded.height());
                let flags = scaling::Flags::BILINEAR | scaling::Flags::ACCURATE_RND | scaling::Flags::FULL_CHR_H_INT;
                let mut made = scaling::Context::get(format, width, height, Pixel::RGBA, width, height, flags)?;
                use_bt709(&mut made)?;
                scaler = Some(made);
            }
            let mut rgba = frame::Video::empty();
            scaler.as_mut().ok_or("no scaler")?.run(&decoded, &mut rgba)?;
            frames.push(rgba);
        }
        Ok(())
    };
    let mut packet = Packet::empty();
    while packet.read(&mut input).is_ok() {
        if packet.stream() == index {
            decoder.send_packet(&packet)?;
            take(&mut decoder)?;
        }
    }
    decoder.send_eof()?;
    take(&mut decoder)?;
    Ok(frames)
}

/// Whether a pixel is the colour, allowing for what encoding loses.
pub(crate) fn is_near(pixel: &[u8], color: [u8; 3]) -> bool {
    pixel.iter().zip(color).all(|(&found, wanted)| found.abs_diff(wanted) <= 8)
}

/// The colour in the middle of the cursor when it is at `place`, and the background elsewhere.
fn assert_cursor_at(frame: &frame::Video, place: (i32, i32), empty: (i32, i32), what: &str) {
    let pixel = |(x, y): (i32, i32)| {
        let at = (y + CURSOR_SIDE as i32 / 2) as usize * frame.stride(0) + (x + CURSOR_SIDE as i32 / 2) as usize * 4;
        frame.data(0)[at..at + 3].to_vec()
    };
    assert!(is_near(&pixel(place), CURSOR_COLOR), "{what}: no cursor at {place:?}, but {:?}", pixel(place));
    assert!(is_near(&pixel(empty), BACKGROUND), "{what}: not the background at {empty:?}, but {:?}", pixel(empty));
}

/// Bakes the clip whose video starts `late` frames in, and checks the copy against it.
fn bake_and_check(name: &str, late: u32) -> TestResult {
    let take = match TestTake::starting(name, late)? {
        Ok(take) => take,
        Err(reason) => {
            eprintln!("skipped: {reason}");
            return Ok(());
        }
    };
    let late_us = i64::from(late) * 1_000_000 / i64::from(FPS);
    let start = video_start_us(&take.video)?;
    assert!((start - late_us).abs() <= 1_000, "the take's video starts at {start} us");

    let mut shares = Vec::new();
    let baked = bake(&take.request(), &AtomicBool::new(false), &mut |share| shares.push(share))?;
    assert_eq!(baked, Baked { file: take.folder.join("take01.baked.mp4"), frames: u64::from(FRAMES), note: None });
    assert_eq!(take.file_names()?, ["take01.baked.mp4", "take01.events", "take01.mkv", "take01_session.json"]);
    assert!(shares.windows(2).all(|pair| pair[0] <= pair[1]) && shares.last() == Some(&1.0), "{shares:?}");

    let (source, copy) = (probe(&take.video)?, probe(&baked.file)?);
    let frame_ms = u64::from(1000 / FPS) + 1;
    // The copy's video starts at zero. Matroska's length also counts the 22 ms of AAC priming, which MP4 cuts away.
    let source_ms = source.duration_ms - u64::try_from(late_us / 1000)?;
    assert!(copy.duration_ms.abs_diff(source_ms) <= frame_ms + 22, "{} against {source_ms}", copy.duration_ms);
    assert!(copy.duration_ms.abs_diff(u64::from(FRAMES * 1000 / FPS)) <= frame_ms, "{}", copy.duration_ms);
    let video = copy.video.ok_or("no video")?;
    assert_eq!((video.codec.as_str(), video.width, video.height), ("av1", WIDTH, HEIGHT));
    let sound = |audio: &[AudioInfo]| audio.iter().map(|a| (a.codec.clone(), a.channels, a.rate)).collect::<Vec<_>>();
    assert_eq!(sound(&copy.audio), sound(&source.audio));
    assert_eq!(copy.audio.len(), 1);

    let (source_counts, source_firsts) = packet_facts(&take.video)?;
    let (counts, firsts) = packet_facts(&baked.file)?;
    assert_eq!(counts, source_counts, "every frame and every audio packet is in the copy");
    assert_eq!(counts.get(&0), Some(&u64::from(FRAMES)));
    assert_eq!(firsts[&0], 0);
    // The audio keeps its place against the video, within the milliseconds Matroska rounds to.
    let (before, after) = (source_firsts[&1] - source_firsts[&0], firsts[&1] - firsts[&0]);
    assert!((before - after).abs() <= 2_000, "the audio moved against the video: {before} us to {after} us");

    let frames = decode_all(&baked.file)?;
    assert_eq!(frames.len(), FRAMES as usize);
    assert_cursor_at(&frames[0], FIRST_PLACE, SECOND_PLACE, "the first frame");
    assert_cursor_at(&frames[MOVE_FRAME as usize - 1], FIRST_PLACE, SECOND_PLACE, "the frame before the move");
    assert_cursor_at(&frames[MOVE_FRAME as usize], SECOND_PLACE, FIRST_PLACE, "the frame of the move");
    assert_cursor_at(&frames[FRAMES as usize - 1], SECOND_PLACE, FIRST_PLACE, "the last frame");
    fs::remove_dir_all(&take.folder)?;
    Ok(())
}

#[test]
fn a_baked_copy_has_the_takes_frames_length_and_audio_with_the_cursor_drawn() -> TestResult {
    bake_and_check("whole", 0)
}

/// The track counts from the first video frame, so a video that starts late must not shift the cursor or the audio.
#[test]
fn a_video_that_starts_late_keeps_its_cursor_and_audio_in_step() -> TestResult {
    bake_and_check("late", LATE_FRAMES)
}

#[test]
fn a_cancelled_bake_leaves_no_partial_file() -> TestResult {
    let take = match TestTake::new("cancel")? {
        Ok(take) => take,
        Err(reason) => {
            eprintln!("skipped: {reason}");
            return Ok(());
        }
    };
    // An earlier baked copy must come through untouched.
    fs::write(take.folder.join("take01.baked.mp4"), b"earlier")?;
    let before = take.file_names()?;

    let cancel = AtomicBool::new(false);
    let mut temp_seen = false;
    let outcome = bake(&take.request(), &cancel, &mut |_| {
        temp_seen |= take.folder.join("take01.baked.mp4.tmp").exists();
        cancel.store(true, Ordering::Relaxed);
    });
    assert!(matches!(outcome, Err(BakeError::Cancelled)), "{outcome:?}");
    assert!(temp_seen, "the bake was under way when it was cancelled");
    assert_eq!(take.file_names()?, before);
    assert_eq!(fs::read(take.folder.join("take01.baked.mp4"))?, b"earlier");

    // A cancel that arrives after the last packet still keeps the copy from being put in place.
    let cancel = AtomicBool::new(false);
    let outcome = bake(&take.request(), &cancel, &mut |share| cancel.store(share >= 1.0, Ordering::Relaxed));
    assert!(matches!(outcome, Err(BakeError::Cancelled)), "{outcome:?}");
    assert_eq!(take.file_names()?, before);

    // Closing the app drops the job, which cancels it and waits.
    fs::remove_file(take.folder.join("take01.baked.mp4"))?;
    let job = BakeJob::start("take01".to_owned(), take.request(), || {})?;
    drop(job);
    let names = take.file_names()?;
    assert!(!names.iter().any(|name| name.ends_with(".tmp")), "{names:?}");
    fs::remove_dir_all(&take.folder)?;
    Ok(())
}

#[test]
fn a_bake_that_fails_leaves_no_file() -> TestResult {
    let folder = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-bake-broken", std::process::id()));
    fs::create_dir_all(&folder)?;
    let video = folder.join("take01.mkv");
    fs::write(&video, b"not a video")?;
    let request = BakeRequest { video, events: folder.join("take01.events"), style: CursorStyle::default() };
    assert!(bake(&request, &AtomicBool::new(false), &mut |_| {}).is_err());
    assert_eq!(fs::read_dir(&folder)?.count(), 1);
    fs::remove_dir_all(&folder)?;
    Ok(())
}

#[test]
fn the_copy_is_named_after_the_take() {
    let output = output_path(Path::new("takes/my.take.mkv"));
    assert_eq!(output, Path::new("takes/my.take.baked.mp4"));
    assert_eq!(temp_path(&output), Path::new("takes/my.take.baked.mp4.tmp"));
}

fn media(audio_codec: &str) -> MediaInfo {
    let audio = AudioInfo { index: 1, codec: audio_codec.to_owned(), channels: 2, rate: 48_000, title: None };
    MediaInfo { duration_ms: 1_000, video: None, audio: vec![audio] }
}

#[test]
fn av1_that_mp4_cannot_take_becomes_hevc_and_pcm_is_refused() -> TestResult {
    let constant_qp = RateControl::ConstantQp { qp_i: 100, qp_p: 255 };
    let at = |height| EncoderConfig { codec: Codec::Av1, height, rate_control: constant_qp, ..EncoderConfig::default() };
    let mut fine = at(1440);
    assert_eq!(fit_mp4(&mut fine, &media("aac"))?, None);
    assert_eq!(fine, at(1440));
    // 900 is no multiple of 16, and no GPU is asked about it.
    let mut odd = at(900);
    let note = fit_mp4(&mut odd, &media("aac"))?.ok_or("no note")?;
    assert!(note.contains("HEVC") && note.contains("900p"), "{note}");
    assert_eq!((odd.codec, odd.rate_control), (Codec::Hevc, RateControl::ConstantQp { qp_i: 20, qp_p: 51 }));
    for height in [900, 1440] {
        assert!(matches!(fit_mp4(&mut at(height), &media("pcm_s24le")), Err(BakeError::PcmAudio)), "{height}");
    }
    Ok(())
}

#[test]
fn the_encoder_settings_come_from_the_file_and_its_session() -> TestResult {
    let folder = std::env::temp_dir().join(format!("cutmark-recorder-test-{}-bake-settings", std::process::id()));
    fs::create_dir_all(&folder)?;
    let video = folder.join("take01.mkv");
    let stream = VideoInfo { codec: "hevc".to_owned(), width: 1280, height: 720, fps: 59.94 };
    // Without a session.json: the file's codec, size and frame rate, and a new encoder's defaults.
    let config = take_config(&video, &stream)?;
    assert_eq!(config, EncoderConfig { codec: Codec::Hevc, width: 1280, height: 720, fps_num: 60, ..EncoderConfig::default() });
    assert!(shows_screen(&video));

    let mut profile = Profile::tutorial_av1_1440p60();
    (profile.fps, profile.keyframe_secs, profile.preset) = (30, 4.0, Preset::Quality);
    profile.rate_control = RateControl::Cbr { mbps: 12.0 };
    profile.layout = LayoutKind::CamOnly;
    fs::write(folder.join("take01_session.json"), serde_json::to_vec(&serde_json::json!({ "profile": profile }))?)?;
    let config = take_config(&video, &stream)?;
    assert_eq!((config.codec, config.width, config.fps_num), (Codec::Hevc, 1280, 30), "the codec and size stay the file's");
    assert_eq!((config.rate_control, config.preset, config.keyframe_secs), (RateControl::Cbr { mbps: 12.0 }, Preset::Quality, 4.0));
    assert!(!shows_screen(&video), "a Cam only take has no screen");
    assert!(take_config(&video, &VideoInfo { codec: "mjpeg".to_owned(), ..stream }).is_err());
    fs::remove_dir_all(&folder)?;
    Ok(())
}
