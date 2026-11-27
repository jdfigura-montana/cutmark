//! A fake encoder drives the capture output the way the stream threads do, into the fake sink.

use std::collections::VecDeque;
use std::sync::Arc;

use ffmpeg_next::Packet;
use ffmpeg_next::packet::Flags;
use media_io::EncodedPacket;

use super::fake::{FakeSink, Output};
use super::*;

/// A QPC clock that makes one 60 fps frame 10 ticks long, so frame `n` is due at `START + 10 n`.
const FREQ: i64 = 600;
const START: i64 = 1_000;
const FPS: u32 = 60;

/// The fake encoder makes a regular keyframe this often, and holds this many packets back.
const GOP: u64 = 30;
const DELAY: usize = 2;

/// The QPC time of a press that falls exactly on frame `n`.
fn press(frame: u64) -> i64 {
    START + frame as i64 * 10
}

fn packet(pts: i64, key: bool) -> EncodedPacket {
    let mut packet = Packet::copy(&[0; 4]);
    packet.set_pts(Some(pts));
    packet.set_dts(Some(pts));
    if key {
        packet.set_flags(Flags::KEY);
    }
    EncodedPacket::from_raw_packet(packet)
}

fn capture(camera_rate: Option<(u32, u32)>) -> CaptureOutput {
    let stats = Arc::new(Stats::new(&[]));
    CaptureOutput::new(stats, 1, TakeGate::new(START, FREQ, FPS, camera_rate))
}

/// A running video encoder.
#[derive(Default)]
struct FakeEncoder {
    next: u64,
    held: VecDeque<(i64, bool)>,
    /// The frames a keyframe was forced on.
    forced: Vec<u64>,
}

impl FakeEncoder {
    /// Encodes frames up to, not including, `end`.
    fn run(&mut self, output: &CaptureOutput, end: u64) -> Result<(), MediaError> {
        while self.next < end {
            let plan = output.video_frame(self.next);
            if plan.force_keyframe {
                self.forced.push(self.next);
            }
            self.held.push_back((self.next as i64, plan.force_keyframe || self.next.is_multiple_of(GOP)));
            self.next += 1;
            if self.held.len() > DELAY
                && let Some((pts, key)) = self.held.pop_front()
            {
                output.video(packet(pts, key))?;
            }
        }
        Ok(())
    }

    /// Hands out the held packets, as the encoder's finish does.
    fn drain(&mut self, output: &CaptureOutput) -> Result<(), MediaError> {
        while let Some((pts, key)) = self.held.pop_front() {
            output.video(packet(pts, key))?;
        }
        Ok(())
    }
}

fn frames(range: std::ops::Range<i64>) -> Vec<i64> {
    range.collect()
}

#[test]
fn a_take_on_running_encoders_begins_exactly_on_its_forced_keyframe() -> Result<(), EngineError> {
    let (output, sink) = (capture(None), Arc::new(FakeSink::default()));
    let mut encoder = FakeEncoder::default();
    encoder.run(&output, 45)?;
    // Pressed on frame 50, which the encoder has not seen yet.
    assert_eq!(output.start_take(sink.clone(), press(50))?, press(50));
    encoder.run(&output, 80)?;
    encoder.drain(&output)?;
    assert_eq!(encoder.forced, [50]);
    // Frame 60's regular keyframe is just another packet of the running take.
    assert_eq!(sink.stamps(Output::Video), frames(0..30));
    Ok(())
}

#[test]
fn a_press_on_a_frame_already_encoded_takes_the_next_one() -> Result<(), EngineError> {
    let (output, sink) = (capture(None), Arc::new(FakeSink::default()));
    let mut encoder = FakeEncoder::default();
    encoder.run(&output, 45)?;
    // Frames 43 and 44 are still inside the encoder; they are not the take's.
    assert_eq!(output.start_take(sink.clone(), press(43))?, press(45));
    encoder.run(&output, 55)?;
    encoder.drain(&output)?;
    assert_eq!(encoder.forced, [45]);
    assert_eq!(sink.stamps(Output::Video), frames(0..10));
    Ok(())
}

#[test]
fn a_regular_keyframe_that_arrives_first_is_skipped() -> Result<(), MediaError> {
    let mut gate = TakeGate::new(START, FREQ, FPS, None);
    assert_eq!(gate.open(press(30)), 30);
    // Frame 30 fails to encode, so the keyframe moves to frame 31.
    assert!(gate.video_frame(30).force_keyframe);
    gate.video_failed(30);
    assert!(gate.video_frame(31).force_keyframe);
    assert!(!gate.video_frame(32).force_keyframe);
    // A regular keyframe inside the span, but not the forced frame's, opens nothing.
    assert_eq!(gate.video_packet(30, true), None);
    assert_eq!(gate.video_packet(32, true), None);
    assert_eq!(gate.video_packet(31, false), None);
    assert_eq!(gate.video_packet(31, true), Some(30));
    assert_eq!(gate.video_packet(32, false), Some(30));
    Ok(())
}

#[test]
fn resume_begins_on_its_forced_keyframe_and_leaves_no_gap() -> Result<(), EngineError> {
    let (output, sink) = (capture(None), Arc::new(FakeSink::default()));
    output.begin_take(sink.clone());
    let mut encoder = FakeEncoder::default();
    encoder.run(&output, 20)?;
    output.pause(press(20));
    // The pause covers the regular keyframes on frames 30 and 60.
    encoder.run(&output, 70)?;
    output.resume(press(70));
    encoder.run(&output, 95)?;
    encoder.drain(&output)?;
    assert_eq!(encoder.forced, [70]);
    // Frames 18 and 19 leave the encoder after the pause and still belong to the take.
    assert_eq!(sink.stamps(Output::Video), frames(0..45));
    Ok(())
}

#[test]
fn a_pause_before_the_keyframe_leaves_the_resume_to_force_its_own() -> Result<(), EngineError> {
    let (output, sink) = (capture(None), Arc::new(FakeSink::default()));
    let mut encoder = FakeEncoder::default();
    encoder.run(&output, 10)?;
    output.start_take(sink.clone(), press(10))?;
    output.pause(press(10));
    encoder.run(&output, 40)?;
    output.resume(press(40));
    encoder.run(&output, 50)?;
    encoder.drain(&output)?;
    assert_eq!(encoder.forced, [40]);
    assert_eq!(sink.stamps(Output::Video), frames(0..10));
    Ok(())
}

#[test]
fn audio_of_a_take_on_running_encoders_starts_at_the_forced_frame() -> Result<(), EngineError> {
    let (output, sink) = (capture(None), Arc::new(FakeSink::default()));
    output.open_pcm()?;
    // AAC packets from the capture's priming packet on, and 10 ms PCM blocks.
    let audio = |from: i64, to: i64| (from..to).try_for_each(|k| output.audio(0, packet(k * 1024 - 1024, true)));
    let pcm = |from: u64, to: u64| (from..to).try_for_each(|k| output.pcm(0, k * 480, &[0.0; 960]).map(|_| ()));
    audio(0, 40)?;
    pcm(0, 90)?;
    // Frame 60 is capture sample 48000.
    output.start_take(sink.clone(), press(60))?;
    audio(40, 60)?;
    pcm(90, 120)?;
    let packets = sink.stamps(Output::Audio(0));
    // The first packet is the one at capture pts 47 * 1024 - 1024 = 47104, which primes the decoder.
    assert_eq!(packets.first(), Some(&(47_104 - 48_000)));
    assert!(packets.windows(2).all(|pair| pair[1] - pair[0] == 1024));
    // The PCM starts at take sample 0, cut inside the capture's block, and stays unbroken.
    assert_eq!(sink.stamps(Output::Pcm(0)).first(), Some(&0));
    assert_eq!(sink.received().get(&Output::Pcm(0)), Some(&(120 * 480 - 48_000)));
    Ok(())
}

#[test]
fn audio_stays_on_one_grid_across_a_pause() {
    let mut gate = TakeGate::new(START, FREQ, FPS, None);
    gate.begin_with_capture();
    gate.close(press(60));
    gate.open(press(125));
    let outputs: Vec<(i64, i64)> = (0..200).filter_map(|k| gate.audio_packet(0, k * 1024 - 1024).map(|(out, _)| (k * 1024 - 1024, out))).collect();
    assert!(outputs.windows(2).all(|pair| pair[1].1 - pair[0].1 == 1024));
    // After the resume no packet is more than half a frame from its true place: 65 video frames are 52000 samples.
    let resumed = outputs.iter().filter(|(capture, _)| *capture >= 99_000);
    assert!(resumed.clone().count() > 50);
    assert!(resumed.map(|(capture, out)| capture - 52_000 - out).all(|error| error.abs() <= 512));
    // PCM: exactly the samples of the two spans, back to back.
    assert_eq!(gate.pcm(47_900, 200), [(0..100, 47_900)]);
    assert_eq!(gate.pcm(99_900, 200), [(100..200, 48_000)]);
    assert!(gate.pcm(60_000, 480).is_empty());
    // App events: a moment inside the pause counts as the moment of the pause.
    assert_eq!((gate.take_ms(5_000_000), gate.take_ms(15_000_000), gate.take_ms(30_000_000)), (500, 1_000, 1_917));
}

#[test]
fn the_camera_of_a_take_on_running_encoders_waits_for_its_own_forced_keyframe() {
    let mut gate = TakeGate::new(START, FREQ, FPS, Some((30, 1)));
    gate.open(press(60));
    // Frame 60 is camera tick 30, but the camera has already encoded through tick 31.
    assert_eq!(gate.camera_frame(31), FramePlan { force_keyframe: true, in_take: true });
    assert_eq!(gate.camera_frame(32), FramePlan { force_keyframe: false, in_take: true });
    assert_eq!(gate.camera_frame(29), FramePlan::default());
    assert_eq!(gate.camera_packet(30, true), None, "a regular keyframe first is skipped");
    assert_eq!(gate.camera_packet(31, true), Some(30));
    assert_eq!(gate.camera_packet(32, false), Some(30));
}

#[test]
fn a_camera_that_ran_past_the_pause_keeps_its_stamps_in_order() {
    let mut gate = TakeGate::new(START, FREQ, FPS, Some((30, 1)));
    gate.begin_with_capture();
    // A sync offset puts the camera ahead: it has written ticks 0 to 34 when the take pauses at tick 30.
    let before: Vec<i64> = (0..35).filter_map(|tick| gate.camera_packet(tick, tick == 0).map(|offset| tick - offset)).collect();
    assert_eq!(before, frames(0..35));
    gate.close(press(60));
    gate.open(press(180));
    // Resumed at camera tick 90, which would be take tick 30 again.
    assert!(gate.camera_frame(90).force_keyframe);
    assert_eq!(gate.camera_packet(90, true), Some(55));
    assert_eq!(gate.camera_packet(91, false), Some(55));
}

#[test]
fn a_normal_take_passes_every_output_through_unchanged() -> Result<(), EngineError> {
    let (output, sink) = (capture(Some((60, 1))), Arc::new(FakeSink::default()));
    output.begin_take(sink.clone());
    output.open_pcm()?;
    let mut encoder = FakeEncoder::default();
    encoder.run(&output, 90)?;
    encoder.drain(&output)?;
    for tick in 0..90 {
        let plan = output.camera_frame(tick);
        assert_eq!(plan, FramePlan { force_keyframe: false, in_take: true });
        output.camera(packet(tick as i64, tick == 0))?;
    }
    for k in 0..70 {
        output.audio(0, packet(k * 1024 - 1024, true))?;
        output.pcm(0, k as u64 * 480, &[0.0; 960])?;
    }
    assert!(encoder.forced.is_empty());
    assert_eq!(sink.stamps(Output::Video), frames(0..90));
    assert_eq!(sink.stamps(Output::Camera), frames(0..90));
    assert_eq!(sink.stamps(Output::Audio(0)), (0..70).map(|k| k * 1024 - 1024).collect::<Vec<_>>());
    assert_eq!(sink.stamps(Output::Pcm(0)), (0..70).map(|k| k * 480).collect::<Vec<_>>());
    Ok(())
}
