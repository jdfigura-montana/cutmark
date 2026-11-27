//! A fake capture around the real capture output, fed the way the stream threads feed it. The rings and the takes are
//! fake sinks.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use ffmpeg_next::Packet;
use ffmpeg_next::packet::Flags;
use media_io::EncodedPacket;

use super::super::session::Stats;
use super::super::sink::fake::{FakeSink, Output};
use super::super::sink::{CaptureOutput, PacketSink};
use super::super::take_gate::TakeGate;
use super::*;

/// A QPC clock that makes one 60 fps frame 10 ticks long.
const FREQ: i64 = 600;
const FPS: u32 = 60;
const LENGTH: Duration = Duration::from_secs(60);

/// What the fake captures did, kept outside them because closing one consumes it.
#[derive(Default)]
struct Log {
    opened: Cell<u32>,
    closed: Cell<u32>,
    dead: Cell<bool>,
    /// The rings of every buffer made, oldest first.
    rings: RefCell<Vec<Arc<FakeSink>>>,
    /// The running capture's output and its next frame.
    output: RefCell<Option<Arc<CaptureOutput>>>,
    frame: Cell<i64>,
}

impl Log {
    /// Encodes the next `count` frames, every one of them a keyframe, so a forced one is too.
    fn run(&self, count: i64) {
        let Some(output) = self.output.borrow().clone() else { return };
        for _ in 0..count {
            let frame = self.frame.get();
            self.frame.set(frame + 1);
            output.video_frame(frame as u64);
            let mut packet = Packet::copy(&[0; 4]);
            packet.set_pts(Some(frame));
            packet.set_dts(Some(frame));
            packet.set_flags(Flags::KEY);
            assert!(output.video(EncodedPacket::from_raw_packet(packet)).is_ok());
        }
    }

    fn ring_frames(&self, index: usize) -> u64 {
        self.rings.borrow().get(index).and_then(|rings| rings.received().get(&Output::Video).copied()).unwrap_or(0)
    }
}

#[derive(Clone)]
struct Config(Rc<Log>);

struct Fake {
    log: Rc<Log>,
    output: Arc<CaptureOutput>,
}

impl Fake {
    fn open(log: Rc<Log>) -> Self {
        log.opened.set(log.opened.get() + 1);
        log.frame.set(0);
        let output = Arc::new(CaptureOutput::new(Arc::new(Stats::new(&[])), 0, TakeGate::new(0, FREQ, FPS, None)));
        log.output.replace(Some(Arc::clone(&output)));
        Self { log, output }
    }

    /// The press time of the next frame.
    fn now(&self) -> i64 {
        self.log.frame.get() * 10
    }
}

impl SharedCapture for Fake {
    type Config = Config;
    type Take = Arc<FakeSink>;
    /// The frames the finished take received.
    type Stopped = u64;

    fn open_for_take(config: Config) -> Result<(Self, Self::Take), EngineError> {
        let capture = Self::open(config.0);
        let take = Arc::new(FakeSink::default());
        capture.output.begin_take(Arc::clone(&take) as Arc<dyn PacketSink>);
        Ok((capture, take))
    }

    fn open_for_replay(config: Config, length: Duration) -> Result<Self, EngineError> {
        let mut capture = Self::open(config.0);
        capture.add_rings(length)?;
        Ok(capture)
    }

    fn join_take(&mut self, _config: Config, press_qpc: i64) -> Result<Self::Take, EngineError> {
        let take = Arc::new(FakeSink::default());
        self.output.start_take(Arc::clone(&take) as Arc<dyn PacketSink>, press_qpc)?;
        Ok(take)
    }

    fn leave_take(&mut self, take: Self::Take, press_qpc: i64) -> u64 {
        self.output.pause(press_qpc);
        drop(self.output.leave_take());
        frames(&take)
    }

    fn close(self, take: Option<Self::Take>) -> Option<u64> {
        self.log.closed.set(self.log.closed.get() + 1);
        self.log.output.replace(None);
        drop(self.output.leave_take());
        take.map(|take| frames(&take))
    }

    fn add_rings(&mut self, _length: Duration) -> Result<(), EngineError> {
        let rings = Arc::new(FakeSink::default());
        self.log.rings.borrow_mut().push(Arc::clone(&rings));
        self.output.set_rings(Some(rings));
        Ok(())
    }

    fn drop_rings(&mut self) {
        self.output.set_rings(None);
    }

    fn is_dead(&self) -> bool {
        self.log.dead.get()
    }
}

fn frames(take: &FakeSink) -> u64 {
    take.received().get(&Output::Video).copied().unwrap_or(0)
}

fn setup() -> (Rc<Log>, Sharing<Fake>) {
    (Rc::new(Log::default()), Sharing::default())
}

fn press(sharing: &Sharing<Fake>) -> i64 {
    sharing.capture().map_or(0, Fake::now)
}

#[test]
fn idle_then_replay_then_two_takes_then_replay_off_use_one_capture() -> Result<(), EngineError> {
    let (log, mut sharing) = setup();
    sharing.replay_start(|| Config(Rc::clone(&log)), LENGTH)?;
    log.run(30);
    for length in [20, 40] {
        sharing.start_take(Config(Rc::clone(&log)), press(&sharing))?;
        log.run(length);
        assert_eq!(sharing.stop_take(press(&sharing)), Some(length as u64));
        log.run(10);
    }
    assert!(sharing.replay_on() && sharing.take().is_none());
    assert_eq!((log.opened.get(), log.closed.get()), (1, 0));
    // The one buffer saw every frame, before, during, between and after the takes, each on the capture's own clock.
    assert_eq!(log.rings.borrow().len(), 1);
    assert_eq!(log.rings.borrow()[0].stamps(Output::Video), (0..110).collect::<Vec<_>>());
    sharing.replay_stop();
    assert!(!sharing.replay_on() && sharing.capture().is_none());
    assert_eq!((log.opened.get(), log.closed.get()), (1, 1));
    Ok(())
}

#[test]
fn the_rings_keep_filling_while_the_take_is_paused() -> Result<(), EngineError> {
    let (log, mut sharing) = setup();
    sharing.replay_start(|| Config(Rc::clone(&log)), LENGTH)?;
    log.run(5);
    sharing.start_take(Config(Rc::clone(&log)), press(&sharing))?;
    log.run(10);
    let output = log.output.borrow().clone();
    if let Some(output) = &output {
        output.pause(press(&sharing));
    }
    log.run(10);
    if let Some(output) = &output {
        output.resume(press(&sharing));
    }
    log.run(10);
    // The take's frames follow on from each other, with the pause removed.
    assert_eq!(sharing.take().map(|take| take.stamps(Output::Video)), Some((0..20).collect()));
    assert_eq!(log.ring_frames(0), 35);
    Ok(())
}

#[test]
fn replay_off_during_a_take_only_drops_the_rings() -> Result<(), EngineError> {
    let (log, mut sharing) = setup();
    sharing.replay_start(|| Config(Rc::clone(&log)), LENGTH)?;
    sharing.start_take(Config(Rc::clone(&log)), press(&sharing))?;
    log.run(10);
    sharing.replay_stop();
    log.run(10);
    assert!(!sharing.replay_on() && sharing.take().is_some());
    assert_eq!((log.opened.get(), log.closed.get()), (1, 0));
    assert_eq!(log.ring_frames(0), 10);
    assert_eq!(sharing.stop_take(press(&sharing)), Some(20));
    assert!(sharing.capture().is_none());
    assert_eq!((log.opened.get(), log.closed.get()), (1, 1));
    Ok(())
}

#[test]
fn replay_turned_on_during_a_take_joins_its_capture_and_outlives_it() -> Result<(), EngineError> {
    let (log, mut sharing) = setup();
    sharing.start_take(Config(Rc::clone(&log)), 0)?;
    log.run(10);
    sharing.replay_start(|| Config(Rc::clone(&log)), LENGTH)?;
    log.run(10);
    assert_eq!(sharing.stop_take(press(&sharing)), Some(20));
    log.run(5);
    assert_eq!((log.opened.get(), log.closed.get()), (1, 0));
    assert_eq!(log.ring_frames(0), 15);
    Ok(())
}

#[test]
fn a_dead_capture_is_closed_with_the_buffer() -> Result<(), EngineError> {
    let (log, mut sharing) = setup();
    sharing.replay_start(|| Config(Rc::clone(&log)), LENGTH)?;
    log.dead.set(true);
    assert!(sharing.buffer_died());
    sharing.start_take(Config(Rc::clone(&log)), 0)?;
    assert!(!sharing.buffer_died());
    assert!(sharing.stop_take(0).is_some());
    assert!(!sharing.replay_on() && sharing.capture().is_none());
    assert_eq!(log.closed.get(), 1);
    Ok(())
}
