//! Writes probe_audio.mkv with three AAC 320 kbps tracks and probe_pcm.mkv
//! with one Pcm24 track, all through the mux thread, and prints each summary.

use std::error::Error;
use std::path::Path;

use media_io::{AudioCodec, AudioEncoder, Container, MediaError, MuxSummary, MuxThread, Muxer};

/// The sample rate every track uses.
const SAMPLE_RATE: u32 = 48_000;
/// Seconds of audio per track.
const SECONDS: u32 = 5;
/// Samples per channel per fed chunk.
const CHUNK: usize = 1024;

/// One stereo sample generator, indexed by sample.
type SampleFn = fn(usize) -> (f32, f32);

fn main() -> Result<(), Box<dyn Error>> {
    media_io::init()?;
    let aac_summary = write_aac_file()?;
    println!("probe_audio.mkv: {aac_summary:?}");
    let pcm_summary = write_pcm_file()?;
    println!("probe_pcm.mkv: {pcm_summary:?}");
    Ok(())
}

/// Writes probe_audio.mkv: Mixdown (default), Track 1 and Track 2, each five
/// seconds of AAC 320 kbps.
fn write_aac_file() -> Result<MuxSummary, MediaError> {
    let tracks: [(&str, bool, SampleFn); 3] = [("Mixdown", true, mixdown), ("Track 1", false, track1), ("Track 2", false, track2)];
    let mut muxer = Muxer::create(Path::new("probe_audio.mkv"), Container::Mkv)?;
    let mut encoders = Vec::new();
    for (title, default, _) in tracks {
        let encoder = AudioEncoder::open(AudioCodec::Aac { kbps: 320 }, SAMPLE_RATE, 2)?;
        let stream = muxer.add_audio(encoder.parameters(), encoder.time_base(), title, default)?;
        encoders.push((encoder, stream));
    }
    muxer.start()?;
    let thread = MuxThread::start(muxer)?;
    for ((_, _, sample), (encoder, stream)) in tracks.into_iter().zip(&mut encoders) {
        write_track(encoder, &thread, *stream, sample)?;
    }
    thread.finish()
}

/// Writes probe_pcm.mkv: one five second Pcm24 mixdown track.
fn write_pcm_file() -> Result<MuxSummary, MediaError> {
    let mut muxer = Muxer::create(Path::new("probe_pcm.mkv"), Container::Mkv)?;
    let mut encoder = AudioEncoder::open(AudioCodec::Pcm24, SAMPLE_RATE, 2)?;
    let stream = muxer.add_audio(encoder.parameters(), encoder.time_base(), "Mixdown", true)?;
    muxer.start()?;
    let thread = MuxThread::start(muxer)?;
    write_track(&mut encoder, &thread, stream, mixdown)?;
    thread.finish()
}

/// Feeds one track's samples through the encoder and the mux thread, then finishes it.
fn write_track(encoder: &mut AudioEncoder, thread: &MuxThread, stream: usize, sample: SampleFn) -> Result<(), MediaError> {
    let total = SAMPLE_RATE as usize * SECONDS as usize;
    for start in (0..total).step_by(CHUNK) {
        let count = CHUNK.min(total - start);
        let mut chunk = Vec::with_capacity(count * 2);
        for i in start..start + count {
            let (left, right) = sample(i);
            chunk.push(left);
            chunk.push(right);
        }
        encoder.send(&chunk)?;
        drain(encoder, thread, stream)?;
    }
    let mut send_error: Option<MediaError> = None;
    encoder.finish(&mut |packet| {
        if send_error.is_none() {
            send_error = thread.send(stream, packet).err();
        }
    })?;
    if let Some(error) = send_error {
        return Err(error);
    }
    Ok(())
}

/// Drains ready packets into the mux thread, keeping the first send failure.
fn drain(encoder: &mut AudioEncoder, thread: &MuxThread, stream: usize) -> Result<(), MediaError> {
    let mut send_error: Option<MediaError> = None;
    encoder.receive(&mut |packet| {
        if send_error.is_none() {
            send_error = thread.send(stream, packet).err();
        }
    })?;
    if let Some(error) = send_error {
        return Err(error);
    }
    Ok(())
}

/// Sine at `freq` for sample index `i`.
fn sine(i: usize, freq: f32) -> f32 {
    (std::f32::consts::TAU * freq * i as f32 / SAMPLE_RATE as f32).sin()
}

/// The mixdown: 440 Hz plus 880 Hz at half level, clipped to full scale.
fn mixdown(i: usize) -> (f32, f32) {
    let value = (sine(i, 440.0) + 0.5 * sine(i, 880.0)).clamp(-1.0, 1.0);
    (value, value)
}

/// Track 1: 440 Hz.
fn track1(i: usize) -> (f32, f32) {
    let value = sine(i, 440.0);
    (value, value)
}

/// Track 2: 880 Hz.
fn track2(i: usize) -> (f32, f32) {
    let value = sine(i, 880.0);
    (value, value)
}
