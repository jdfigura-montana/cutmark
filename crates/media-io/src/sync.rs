//! Camera sync measurement: finds the screen flashes of a sync test in the main file and in the camera file and
//! reports how far the camera runs behind the screen, in milliseconds and in frames.
//!
//! Each file is decoded in software and reduced to one number per frame, the mean luma. A flash is a frame whose luma
//! rises sharply against the previous ten frames. The threshold is relative to the file's own swing, so a bright room,
//! where the camera sees only a small change, still works.

use std::path::{Path, PathBuf};

use ffmpeg_next::format::Pixel;
use ffmpeg_next::frame::Video;
use ffmpeg_next::media::Type;
use thiserror::Error;

/// How many flash onsets each file needs before a measurement is trusted.
pub const MIN_ONSETS: usize = 3;

/// How many frames before a frame make up the level it is compared with.
const BASELINE_FRAMES: usize = 10;

/// After an onset no other onset is accepted for this long, so one flash is counted once.
const REFRACTORY_MS: f64 = 1000.0;

/// The furthest apart two onsets may be and still be the same flash, in milliseconds.
const MAX_PAIR_GAP_MS: f64 = 1000.0;

/// The share of the file's swing a rise must reach to count as an onset.
const RELATIVE_THRESHOLD: f64 = 0.4;

/// How many times the typical frame-to-frame change a rise must exceed.
const NOISE_MULTIPLE: f64 = 4.0;

/// The smallest rise that counts, in luma levels out of 255.
const MIN_RISE: f64 = 0.5;

/// Only every this many rows and columns of the luma plane are read; the mean of a sample is as good as the mean of all.
const SAMPLE_STEP: usize = 4;

/// Why a sync measurement failed.
#[derive(Debug, Error)]
pub enum SyncError {
    /// FFmpeg could not be started.
    #[error("FFmpeg could not be started: {0}")]
    Init(String),
    /// A file could not be opened or decoded.
    #[error("could not read {path}: {source}")]
    Decode {
        /// The file that failed.
        path: PathBuf,
        /// What FFmpeg said.
        source: ffmpeg_next::Error,
    },
    /// A file has no video stream.
    #[error("{0} has no video stream")]
    NoVideo(PathBuf),
    /// A file shows too few flashes.
    #[error("found {found} screen flashes in the {file} file, and the test needs at least {MIN_ONSETS}")]
    TooFewFlashes {
        /// Which file: "screen" or "camera".
        file: &'static str,
        /// How many flashes were found.
        found: usize,
    },
    /// The decoder produced pixels in a format the measurement cannot read.
    #[error("the video uses the pixel format {0}, which the sync test cannot read")]
    PixelFormat(String),
    /// The flashes of the two files could not be matched up.
    #[error("only {found} flashes could be matched between the screen and the camera, and the test needs at least {MIN_ONSETS}")]
    TooFewPairs {
        /// How many pairs were found.
        found: usize,
    },
}

/// The result type of this module.
pub type Result<T> = std::result::Result<T, SyncError>;

/// The mean luma of one decoded frame, and when the frame is shown.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// The frame's presentation time in milliseconds.
    pub ms: f64,
    /// The mean luma of the frame, from 0 to 255.
    pub luma: f64,
}

/// What a sync test measured.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    /// The camera's onset time minus the screen's, for each matched flash, in milliseconds.
    pub offsets_ms: Vec<f64>,
    /// The median of the offsets in milliseconds; positive means the camera frames are late.
    pub median_ms: f64,
    /// The largest offset minus the smallest, in milliseconds.
    pub spread_ms: f64,
    /// The median offset in frames of the main file.
    pub median_frames: f64,
    /// The frame rate of the main file.
    pub main_fps: f64,
    /// How many flashes were found in the main file.
    pub main_onsets: usize,
    /// How many flashes were found in the camera file.
    pub cam_onsets: usize,
}

impl SyncReport {
    /// Whether the median offset is no more than one frame either way.
    pub fn within_one_frame(&self) -> bool {
        self.median_frames.abs() <= 1.0
    }

    /// The median offset as a whole number of frames, without its sign.
    pub fn frames_off(&self) -> u32 {
        self.median_frames.abs().round() as u32
    }
}

/// Measures how far the camera file runs behind the main file, from the flashes of a sync test.
pub fn measure_offset(main: &Path, cam: &Path) -> Result<SyncReport> {
    crate::init().map_err(|err| SyncError::Init(err.to_string()))?;
    let (main_series, main_fps) = luma_series(main)?;
    let (cam_series, _) = luma_series(cam)?;
    analyse(&main_series, &cam_series, main_fps)
}

/// Finds the flashes in both series and builds the report.
pub fn analyse(main: &[Sample], cam: &[Sample], main_fps: f64) -> Result<SyncReport> {
    let main_onsets = find_onsets(main);
    let cam_onsets = find_onsets(cam);
    if main_onsets.len() < MIN_ONSETS {
        return Err(SyncError::TooFewFlashes { file: "screen", found: main_onsets.len() });
    }
    if cam_onsets.len() < MIN_ONSETS {
        return Err(SyncError::TooFewFlashes { file: "camera", found: cam_onsets.len() });
    }
    let offsets_ms = pair_offsets(&main_onsets, &cam_onsets);
    if offsets_ms.len() < MIN_ONSETS {
        return Err(SyncError::TooFewPairs { found: offsets_ms.len() });
    }
    let median_ms = median(&offsets_ms);
    let smallest = offsets_ms.iter().copied().fold(f64::INFINITY, f64::min);
    let largest = offsets_ms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok(SyncReport {
        median_frames: median_ms * main_fps / 1000.0,
        spread_ms: largest - smallest,
        median_ms,
        main_fps,
        main_onsets: main_onsets.len(),
        cam_onsets: cam_onsets.len(),
        offsets_ms,
    })
}

/// The times of the flash onsets in a luma series, in milliseconds.
pub fn find_onsets(series: &[Sample]) -> Vec<f64> {
    if series.len() < 2 {
        return Vec::new();
    }
    let threshold = onset_threshold(series);
    let mut onsets = Vec::new();
    let mut last: Option<f64> = None;
    for (index, sample) in series.iter().enumerate().skip(1) {
        if last.is_some_and(|at| sample.ms - at < REFRACTORY_MS) {
            continue;
        }
        let window = &series[index.saturating_sub(BASELINE_FRAMES)..index];
        let baseline = window.iter().map(|s| s.luma).sum::<f64>() / window.len() as f64;
        if sample.luma - baseline >= threshold {
            onsets.push(sample.ms);
            last = Some(sample.ms);
        }
    }
    onsets
}

/// How far a frame must rise above its baseline to be an onset: a share of the file's swing, but never inside the noise.
fn onset_threshold(series: &[Sample]) -> f64 {
    let lumas: Vec<f64> = series.iter().map(|s| s.luma).collect();
    let swing = lumas.iter().copied().fold(f64::NEG_INFINITY, f64::max) - median(&lumas);
    let steps: Vec<f64> = series.windows(2).map(|pair| (pair[1].luma - pair[0].luma).abs()).collect();
    let noise = median(&steps);
    (RELATIVE_THRESHOLD * swing).max(NOISE_MULTIPLE * noise).max(MIN_RISE)
}

/// Matches the onsets in order and returns camera minus main for each match.
fn pair_offsets(main: &[f64], cam: &[f64]) -> Vec<f64> {
    let mut offsets = Vec::new();
    let mut next = 0;
    for &at in main {
        while next < cam.len() && cam[next] < at - MAX_PAIR_GAP_MS {
            next += 1;
        }
        if let Some(&other) = cam.get(next)
            && (other - at).abs() <= MAX_PAIR_GAP_MS
        {
            offsets.push(other - at);
            next += 1;
        }
    }
    offsets
}

/// The median of a list; zero for an empty one.
fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 { sorted[mid] } else { (sorted[mid - 1] + sorted[mid]) / 2.0 }
}

/// A rational as a float; zero when its denominator is zero.
fn rational(value: ffmpeg_next::Rational) -> f64 {
    if value.denominator() == 0 { 0.0 } else { f64::from(value.numerator()) / f64::from(value.denominator()) }
}

/// Decodes a file's video and returns the mean luma of every frame, with the file's frame rate.
fn luma_series(path: &Path) -> Result<(Vec<Sample>, f64)> {
    let decode = |source| SyncError::Decode { path: path.to_path_buf(), source };
    let mut input = ffmpeg_next::format::input(path).map_err(decode)?;
    let (index, time_base, mut fps, parameters) = {
        let stream = input.streams().best(Type::Video).ok_or_else(|| SyncError::NoVideo(path.to_path_buf()))?;
        (stream.index(), rational(stream.time_base()), rational(stream.avg_frame_rate()), stream.parameters())
    };
    let context = ffmpeg_next::codec::context::Context::from_parameters(parameters).map_err(decode)?;
    let mut decoder = context.decoder().video().map_err(decode)?;
    let mut reader = FrameReader::default();
    for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        decoder.send_packet(&packet).map_err(decode)?;
        reader.drain(&mut decoder, time_base)?;
    }
    decoder.send_eof().map_err(decode)?;
    reader.drain(&mut decoder, time_base)?;
    if fps <= 0.0 {
        fps = estimate_fps(&reader.samples);
    }
    Ok((reader.samples, fps))
}

/// The frame rate implied by the first and last frame times.
fn estimate_fps(samples: &[Sample]) -> f64 {
    match (samples.first(), samples.last()) {
        (Some(first), Some(last)) if last.ms > first.ms => (samples.len() - 1) as f64 * 1000.0 / (last.ms - first.ms),
        _ => 0.0,
    }
}

/// Turns decoded frames into luma samples.
#[derive(Default)]
struct FrameReader {
    samples: Vec<Sample>,
}

impl FrameReader {
    /// Takes every frame the decoder has ready and appends its luma sample.
    fn drain(&mut self, decoder: &mut ffmpeg_next::decoder::Video, time_base: f64) -> Result<()> {
        let mut frame = Video::empty();
        while decoder.receive_frame(&mut frame).is_ok() {
            let luma = mean_luma(&frame)?;
            let ms = match frame.timestamp().or_else(|| frame.pts()) {
                Some(pts) => pts as f64 * time_base * 1000.0,
                None => self.samples.last().map_or(0.0, |last| last.ms),
            };
            self.samples.push(Sample { ms, luma });
        }
        Ok(())
    }
}

/// How the luma plane of a pixel format is stored.
#[derive(Clone, Copy)]
enum LumaPlane {
    /// One byte per sample, 0 to 255.
    Bits8,
    /// Two bytes per sample, little endian, with the value in the low 10 bits.
    Low10,
    /// Two bytes per sample, little endian, with the value in the high 10 bits.
    High10,
}

/// Where the luma is for the pixel formats a recording can leave in a file, or none for any other.
fn luma_plane(format: Pixel) -> Option<LumaPlane> {
    match format {
        Pixel::YUV420P | Pixel::YUVJ420P | Pixel::YUV422P | Pixel::YUVJ422P | Pixel::YUV444P | Pixel::YUVJ444P | Pixel::NV12 | Pixel::NV21 | Pixel::GRAY8 => {
            Some(LumaPlane::Bits8)
        }
        Pixel::YUV420P10LE | Pixel::YUV422P10LE | Pixel::YUV444P10LE => Some(LumaPlane::Low10),
        Pixel::P010LE => Some(LumaPlane::High10),
        _ => None,
    }
}

/// The mean luma of a frame in 0 to 255, read from a sparse grid of its first plane.
fn mean_luma(frame: &Video) -> Result<f64> {
    let layout = luma_plane(frame.format()).ok_or_else(|| SyncError::PixelFormat(format!("{:?}", frame.format())))?;
    let (width, height, stride, data) = (frame.width() as usize, frame.height() as usize, frame.stride(0), frame.data(0));
    let mut sum = 0u64;
    let mut count = 0u64;
    for row in (0..height).step_by(SAMPLE_STEP) {
        let Some(line) = data.get(row * stride..) else { break };
        for column in (0..width).step_by(SAMPLE_STEP) {
            let level = match layout {
                LumaPlane::Bits8 => line.get(column).map(|&byte| u64::from(byte)),
                LumaPlane::Low10 => line.get(column * 2..column * 2 + 2).map(|pair| u64::from(u16::from_le_bytes([pair[0], pair[1]]) >> 2)),
                LumaPlane::High10 => line.get(column * 2..column * 2 + 2).map(|pair| u64::from(u16::from_le_bytes([pair[0], pair[1]]) >> 8)),
            };
            if let Some(level) = level {
                sum += level;
                count += 1;
            }
        }
    }
    Ok(if count == 0 { 0.0 } else { sum as f64 / count as f64 })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f64 = 30.0;

    /// A tiny deterministic noise source, from -1 to 1.
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            f64::from(self.0) / f64::from(u32::MAX) * 2.0 - 1.0
        }
    }

    /// Twelve seconds of frames at 30 fps: `base` luma, plus `amplitude` for 100 ms at each flash time, plus noise.
    fn series(base: f64, amplitude: f64, flashes: &[f64], noise: f64, seed: u32) -> Vec<Sample> {
        let mut rng = Noise(seed);
        (0..360)
            .map(|frame| {
                let ms = f64::from(frame) * 1000.0 / FPS;
                let lit = flashes.iter().any(|&at| ms >= at && ms < at + 100.0);
                Sample { ms, luma: base + if lit { amplitude } else { 0.0 } + rng.next() * noise }
            })
            .collect()
    }

    const FLASHES: [f64; 5] = [2000.0, 4000.0, 6000.0, 8000.0, 10000.0];

    fn shifted(by: f64) -> Vec<f64> {
        FLASHES.iter().map(|at| at + by).collect()
    }

    #[test]
    fn clean_flashes_give_the_shift_between_the_files() {
        let main = series(40.0, 180.0, &FLASHES, 0.2, 7);
        let cam = series(60.0, 120.0, &shifted(100.0), 0.2, 11);
        let report = analyse(&main, &cam, FPS).expect("clean flashes are measurable");
        assert_eq!(report.main_onsets, 5);
        assert_eq!(report.cam_onsets, 5);
        assert_eq!(report.offsets_ms.len(), 5);
        assert!((report.median_ms - 100.0).abs() < 1000.0 / FPS, "median {}", report.median_ms);
        assert!((report.median_frames - 3.0).abs() < 1.1, "frames {}", report.median_frames);
        assert!(report.spread_ms < 2.0 * 1000.0 / FPS, "spread {}", report.spread_ms);
        assert!(!report.within_one_frame());
    }

    #[test]
    fn a_camera_that_runs_early_gives_a_negative_offset() {
        let main = series(40.0, 180.0, &FLASHES, 0.2, 3);
        let cam = series(60.0, 120.0, &shifted(-200.0), 0.2, 5);
        let report = analyse(&main, &cam, FPS).expect("measurable");
        assert!(report.median_ms < -150.0 && report.median_ms > -250.0, "median {}", report.median_ms);
    }

    #[test]
    fn a_bright_noisy_room_with_a_small_change_still_works() {
        let main = series(120.0, 90.0, &FLASHES, 0.3, 21);
        let cam = series(235.0, 5.0, &shifted(70.0), 0.4, 33);
        let report = analyse(&main, &cam, FPS).expect("a small change is still a flash");
        assert_eq!(report.cam_onsets, 5);
        assert!((report.median_ms - 70.0).abs() < 2.0 * 1000.0 / FPS, "median {}", report.median_ms);
    }

    #[test]
    fn noise_alone_has_no_flashes() {
        let quiet = series(200.0, 0.0, &[], 0.6, 99);
        assert!(find_onsets(&quiet).is_empty());
    }

    #[test]
    fn one_missing_flash_is_skipped_without_misaligning_the_rest() {
        let main = series(40.0, 180.0, &FLASHES, 0.2, 1);
        let cam_flashes: Vec<f64> = shifted(60.0).into_iter().filter(|&at| (at - 6060.0).abs() > 1.0).collect();
        let cam = series(60.0, 120.0, &cam_flashes, 0.2, 2);
        let report = analyse(&main, &cam, FPS).expect("four of five is enough");
        assert_eq!(report.cam_onsets, 4);
        assert_eq!(report.offsets_ms.len(), 4);
        assert!(report.offsets_ms.iter().all(|offset| (offset - 60.0).abs() < 1000.0 / FPS), "offsets {:?}", report.offsets_ms);
    }

    #[test]
    fn fewer_than_three_flashes_in_the_camera_is_an_error() {
        let main = series(40.0, 180.0, &FLASHES, 0.2, 4);
        let cam = series(60.0, 120.0, &[2060.0, 4060.0], 0.2, 6);
        let err = analyse(&main, &cam, FPS).expect_err("two flashes are not enough");
        assert!(matches!(err, SyncError::TooFewFlashes { file: "camera", found: 2 }), "{err}");
        assert!(err.to_string().contains("camera"));
    }

    #[test]
    fn fewer_than_three_flashes_in_the_screen_file_is_an_error() {
        let main = series(40.0, 180.0, &[2000.0], 0.2, 8);
        let cam = series(60.0, 120.0, &shifted(60.0), 0.2, 9);
        let err = analyse(&main, &cam, FPS).expect_err("one flash is not enough");
        assert!(matches!(err, SyncError::TooFewFlashes { file: "screen", found: 1 }), "{err}");
    }

    #[test]
    fn flashes_that_do_not_line_up_are_not_paired() {
        let main = series(40.0, 180.0, &[2000.0, 4000.0, 6000.0], 0.2, 12);
        let cam = series(60.0, 120.0, &[9000.0, 10000.0, 11000.0], 0.2, 13);
        let err = analyse(&main, &cam, FPS).expect_err("nothing matches");
        assert!(matches!(err, SyncError::TooFewPairs { .. }), "{err}");
    }

    #[test]
    fn the_median_of_an_even_list_is_the_mean_of_the_middle_two() {
        assert!((median(&[4.0, 1.0, 3.0, 2.0]) - 2.5).abs() < f64::EPSILON);
        assert!((median(&[5.0, 1.0, 3.0]) - 3.0).abs() < f64::EPSILON);
        assert!(median(&[]).abs() < f64::EPSILON);
    }

    #[test]
    fn within_one_frame_and_frames_off_follow_the_median() {
        let mut report =
            SyncReport { offsets_ms: vec![10.0; 3], median_ms: 10.0, spread_ms: 0.0, median_frames: 0.3, main_fps: 30.0, main_onsets: 3, cam_onsets: 3 };
        assert!(report.within_one_frame());
        report.median_frames = -2.6;
        assert!(!report.within_one_frame());
        assert_eq!(report.frames_off(), 3);
    }
}
