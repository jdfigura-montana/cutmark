//! The replay buffer: everything a take would write, held in RAM rings, and the writer that saves a snapshot of the
//! rings as a normal take.
//!
//! Every time is on the buffer's media clock, where 0 is the instant all streams started from. Packets are stamped in
//! their encoder's own time base, PCM positions count 48 kHz frames, and mark times are in ms.
//! See the replay buffer spec: Under the hood, The buffer and Save.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ::marks::{Clock, MarkType, NewMark, Source, SystemClock};
use chrono::{DateTime, TimeDelta, Utc};
use ffmpeg_next::packet::{Flags, Packet};
use media_io::mux::{MuxThread, SendParameters};
use media_io::{AudioCodec, EncodedPacket, MediaError};

use super::audio::{OUT_CHANNELS, OUT_RATE, SidecarWav};
use super::cam::{self, CamSummary};
use super::clock::MediaClock;
use super::composite::CompositeBlock;
use super::marks::{MarksWarning, TakeMarks};
use super::session::{AudioStreamSpec, EngineError, SessionConfig, SessionFile, Stats, Summary, VideoSetup, build_muxer, file_name, finish_marks};
use super::sink::{CamFileEnd, PacketSink};

const US_PER_S: i64 = 1_000_000;

/// The samples of one AAC frame, which is also the encoder's priming.
const AAC_FRAME_SAMPLES: i64 = 1024;

/// The label of the Highlight that covers a whole saved replay.
const REPLAY_LABEL: &str = "Replay";

/// How many frames of silence are written at a time where the PCM ring has a gap.
const SILENCE_FRAMES: u64 = 4_800;

/// Seconds per tick, as (numerator, denominator).
type TimeBase = (i32, i32);

/// `value * mul / div`, rounded to the nearest whole number.
fn scale(value: i64, mul: i64, div: i64) -> i64 {
    let (value, mul, div) = (i128::from(value), i128::from(mul), i128::from(div.max(1)));
    let rounded = (2 * value * mul + div).div_euclid(2 * div);
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

fn ticks_to_us(ticks: i64, time_base: TimeBase) -> i64 {
    scale(ticks, i64::from(time_base.0) * US_PER_S, i64::from(time_base.1))
}

fn us_to_ticks(us: i64, time_base: TimeBase) -> i64 {
    scale(us, i64::from(time_base.1), i64::from(time_base.0) * US_PER_S)
}

/// The 48 kHz frame at a time on the buffer's clock.
fn pcm_frame(us: i64) -> u64 {
    u64::try_from(scale(us, i64::from(OUT_RATE), US_PER_S)).unwrap_or(0)
}

fn whole_ms(us: i64) -> u64 {
    u64::try_from(us / 1000).unwrap_or(0)
}

/// One buffered packet. Cloning it shares the payload.
#[derive(Clone)]
struct RingPacket {
    packet: Arc<EncodedPacket>,
    /// Presentation and decode times in the stream's time base.
    pts: i64,
    dts: i64,
    /// The presentation time on the buffer's clock.
    time_us: i64,
    key: bool,
}

/// The buffered packets of one stream, in the order the encoder gave them.
pub(super) struct PacketRing {
    time_base: TimeBase,
    packets: VecDeque<RingPacket>,
}

impl PacketRing {
    fn new(time_base: TimeBase) -> Self {
        Self { time_base, packets: VecDeque::new() }
    }

    /// Keeps one packet and returns its presentation time; a packet with no time stamp is not kept.
    fn push(&mut self, packet: EncodedPacket) -> Option<i64> {
        let pts = packet.pts()?;
        let dts = packet.dts().unwrap_or(pts);
        let time_us = ticks_to_us(pts, self.time_base);
        let key = packet.is_key();
        self.packets.push_back(RingPacket { packet: Arc::new(packet), pts, dts, time_us, key });
        Some(time_us)
    }

    /// Drops packets from the front for as long as `old` says so.
    fn drop_front_while(&mut self, old: impl Fn(&RingPacket) -> bool) {
        while self.packets.front().is_some_and(&old) {
            self.packets.pop_front();
        }
    }

    /// Drops whole groups of pictures from the front for as long as the next keyframe is at or before `keep_from_us`,
    /// so a ring that starts on a keyframe still does and still reaches back that far. True when anything went.
    fn drop_old_gops(&mut self, keep_from_us: i64) -> bool {
        let mut dropped = false;
        while let Some(next_key) = self.packets.iter().skip(1).position(|packet| packet.key).map(|at| at + 1) {
            if self.packets.get(next_key).is_none_or(|packet| packet.time_us > keep_from_us) {
                break;
            }
            self.packets.drain(..next_key);
            dropped = true;
        }
        dropped
    }
}

/// The main video's ring, which decides where every other ring starts.
struct VideoRing {
    params: SendParameters,
    ring: PacketRing,
    /// The latest presentation time seen.
    newest_us: i64,
}

/// What the camera ISO stream of a take is made of.
pub(super) struct CamStream {
    pub(super) params: SendParameters,
    pub(super) time_base: TimeBase,
    /// The camera file's frame rate, as (numerator, denominator).
    pub(super) fps: (u32, u32),
    /// The camera picture's width and height, for the composite block.
    pub(super) size: (u32, u32),
}

struct CamRing {
    stream: CamStream,
    ring: PacketRing,
}

impl CamRing {
    /// Starts the ring on its newest keyframe at or before the video's start, or on its first one while it has none
    /// that old. The camera's keyframes need not fall on the main video's.
    fn trim(&mut self, start_us: i64) {
        self.ring.drop_front_while(|packet| !packet.key);
        self.ring.drop_old_gops(start_us);
    }
}

struct AudioRing {
    spec: AudioStreamSpec,
    /// How far before the clip's start a packet is still kept, in ticks: one AAC frame, which primes the decoder as
    /// the first packet of a take does.
    lead: i64,
    ring: PacketRing,
}

impl AudioRing {
    /// The earliest presentation time, in ticks, of a packet that belongs to a clip starting at `start_us`.
    fn cut(&self, start_us: i64) -> i64 {
        us_to_ticks(start_us, self.ring.time_base) - self.lead
    }

    fn trim(&mut self, start_us: i64) {
        let cut = self.cut(start_us);
        self.ring.drop_front_while(|packet| packet.pts < cut);
    }
}

/// A run of interleaved stereo samples. Cloning it shares the samples.
#[derive(Clone)]
struct PcmBlock {
    /// The 48 kHz frame of the first sample.
    start: u64,
    samples: Arc<[f32]>,
}

impl PcmBlock {
    fn end(&self) -> u64 {
        self.start + (self.samples.len() / usize::from(OUT_CHANNELS)) as u64
    }
}

/// The buffered PCM of one track, kept for its WAV sidecar: 48 kHz, interleaved, with the sidecar's channel count.
#[derive(Default)]
pub(super) struct PcmRing {
    blocks: VecDeque<PcmBlock>,
}

impl PcmRing {
    fn push(&mut self, start: u64, samples: &[f32]) {
        self.blocks.push_back(PcmBlock { start, samples: samples.into() });
    }

    /// Drops the blocks that end at or before `frame`.
    fn trim(&mut self, frame: u64) {
        while self.blocks.front().is_some_and(|block| block.end() <= frame) {
            self.blocks.pop_front();
        }
    }

    /// The blocks that overlap `from..to`.
    fn clip(&self, from: u64, to: u64) -> Vec<PcmBlock> {
        self.blocks.iter().filter(|block| block.end() > from && block.start < to).cloned().collect()
    }
}

/// The time a mark was made: the end of a range, which is when its key was pressed, or the time of a point.
fn pressed_ms(mark: &NewMark) -> u64 {
    mark.end_ms.map_or(mark.t_ms, |end| end.max(mark.t_ms))
}

/// The marks made while buffering, with their times on the buffer's clock.
#[derive(Default)]
pub(super) struct MarkRing {
    marks: VecDeque<NewMark>,
}

impl MarkRing {
    fn push(&mut self, mark: NewMark) {
        self.marks.push_back(mark);
    }

    /// Drops the marks that ended before `start_ms`.
    fn trim(&mut self, start_ms: u64) {
        self.marks.retain(|mark| pressed_ms(mark) >= start_ms);
    }

    /// The marks in `start_ms..=now_ms`, in the order they were made, timed from `start_ms`. A range that began
    /// earlier starts at 0.
    fn clip(&self, start_ms: u64, now_ms: u64) -> Vec<NewMark> {
        let mut kept: Vec<NewMark> = self.marks.iter().filter(|mark| mark.t_ms <= now_ms && pressed_ms(mark) >= start_ms).cloned().collect();
        kept.sort_by_key(pressed_ms);
        for mark in &mut kept {
            mark.t_ms = mark.t_ms.saturating_sub(start_ms);
            mark.end_ms = mark.end_ms.map(|end| end.min(now_ms).saturating_sub(start_ms));
        }
        kept
    }
}

/// The rings of one recording setup: one for every stream a take of it would write.
pub(super) struct ReplayRings {
    /// The replay length setting.
    length: Duration,
    fps: u32,
    video: VideoRing,
    camera: Option<CamRing>,
    /// In the take's stream order: the mixdown first when there is one, then one per track stream.
    audio: Vec<AudioRing>,
    /// One per track in track order when WAV sidecars are on, otherwise none.
    pcm: Vec<PcmRing>,
    marks: MarkRing,
}

impl ReplayRings {
    /// Empty rings for the streams a take of `cfg` would write, as its video and audio setups describe them.
    pub(super) fn new(cfg: &SessionConfig, video: VideoSetup, camera: Option<CamStream>, audio: Vec<AudioStreamSpec>, length: Duration) -> Self {
        let priming = matches!(cfg.audio_codec, AudioCodec::Aac { .. });
        let audio = audio
            .into_iter()
            .map(|spec| {
                let time_base = spec.time_base;
                let frame = scale(AAC_FRAME_SAMPLES, i64::from(time_base.1), i64::from(time_base.0) * i64::from(OUT_RATE));
                AudioRing { spec, lead: if priming { frame } else { 0 }, ring: PacketRing::new(time_base) }
            })
            .collect();
        let wav_tracks = if cfg.wav_sidecars { cfg.tracks.len() } else { 0 };
        Self {
            length,
            fps: cfg.profile.fps.max(1),
            video: VideoRing { params: video.params, ring: PacketRing::new(video.time_base), newest_us: i64::MIN },
            camera: camera.map(|stream| CamRing { ring: PacketRing::new(stream.time_base), stream }),
            audio,
            pcm: std::iter::repeat_with(PcmRing::default).take(wav_tracks).collect(),
            marks: MarkRing::default(),
        }
    }

    /// The video ring's start on the buffer's clock, once it holds a keyframe.
    fn start_us(&self) -> Option<i64> {
        self.video.ring.packets.front().map(|packet| packet.time_us)
    }

    fn length_us(&self) -> i64 {
        i64::try_from(self.length.as_micros()).unwrap_or(i64::MAX)
    }

    /// Adds a main video packet and drops what is older than the ring now has to hold, in every ring.
    pub(super) fn push_video(&mut self, packet: EncodedPacket) {
        let first = self.video.ring.packets.is_empty();
        // Nothing before the first keyframe can be decoded.
        if first && !packet.is_key() {
            return;
        }
        let Some(time_us) = self.video.ring.push(packet) else { return };
        self.video.newest_us = self.video.newest_us.max(time_us);
        let frame_us = US_PER_S / i64::from(self.fps);
        let keep_from_us = self.video.newest_us.saturating_add(frame_us).saturating_sub(self.length_us());
        if self.video.ring.drop_old_gops(keep_from_us) || first {
            self.trim_to_video();
        }
    }

    /// Cuts every other ring to the video ring's start.
    fn trim_to_video(&mut self) {
        let Some(start_us) = self.start_us() else { return };
        if let Some(camera) = &mut self.camera {
            camera.trim(start_us);
        }
        for audio in &mut self.audio {
            audio.trim(start_us);
        }
        for pcm in &mut self.pcm {
            pcm.trim(pcm_frame(start_us));
        }
        self.marks.trim(whole_ms(start_us));
    }

    /// Adds a camera ISO packet; ignored when the setup has no camera file.
    pub(super) fn push_camera(&mut self, packet: EncodedPacket) {
        let start_us = self.start_us();
        let Some(camera) = &mut self.camera else { return };
        camera.ring.push(packet);
        match start_us {
            Some(start_us) => camera.trim(start_us),
            None => camera.ring.drop_front_while(|packet| !packet.key),
        }
    }

    /// Adds a packet of the audio stream in `slot`, counted in the order the streams were given.
    pub(super) fn push_audio(&mut self, slot: usize, packet: EncodedPacket) {
        let start_us = self.start_us();
        let Some(audio) = self.audio.get_mut(slot) else { return };
        audio.ring.push(packet);
        if let Some(start_us) = start_us {
            audio.trim(start_us);
        }
    }

    /// Adds interleaved stereo samples of a track that start at the 48 kHz frame `start`; ignored when WAV sidecars
    /// are off.
    pub(super) fn push_pcm(&mut self, track: usize, start: u64, samples: &[f32]) {
        let start_us = self.start_us();
        let Some(pcm) = self.pcm.get_mut(track) else { return };
        pcm.push(start, samples);
        if let Some(start_us) = start_us {
            pcm.trim(pcm_frame(start_us));
        }
    }

    /// Keeps a mark whose times are in ms on the buffer's clock.
    pub(super) fn push_mark(&mut self, mark: NewMark) {
        self.marks.push(mark);
    }

    /// A copy of the rings for a save pressed at `now_ms`, which shares every payload. The clip starts at the newest
    /// video keyframe at or before `now_ms` minus the length, or at the oldest one while the buffer is still shorter
    /// than that. None while there is no video to save.
    pub(super) fn snapshot(&self, now_ms: u64) -> Option<ReplaySnapshot> {
        let now_us = i64::try_from(now_ms).unwrap_or(i64::MAX).saturating_mul(1000);
        let target_us = now_us.saturating_sub(self.length_us());
        let packets = &self.video.ring.packets;
        let mut first = None;
        for (at, packet) in packets.iter().enumerate() {
            if packet.key && packet.time_us < now_us && (first.is_none() || packet.time_us <= target_us) {
                first = Some(at);
            }
        }
        let first = first?;
        let last = packets.iter().rposition(|packet| packet.time_us < now_us)?;
        let video: Vec<RingPacket> = packets.range(first..=last).cloned().collect();
        let start = video.first()?;
        let (start_us, start_pts) = (start.time_us, start.pts);
        let end_pts = video.iter().map(|packet| packet.pts).max()?.saturating_add(1);
        let end_us = ticks_to_us(end_pts, self.video.ring.time_base);

        // The other streams are cut to the same start, and end where the video does. Cutting the end after the last
        // packet that belongs keeps every packet an earlier one depends on.
        let until_end = |packets: Vec<RingPacket>| {
            let keep = packets.iter().rposition(|packet| packet.time_us < end_us).map_or(0, |at| at + 1);
            let mut packets = packets;
            packets.truncate(keep);
            packets
        };
        let camera = self.camera.as_ref().map(|camera| {
            let packets = &camera.ring.packets;
            let first = packets.iter().rposition(|packet| packet.key && packet.time_us <= start_us).or_else(|| packets.iter().position(|packet| packet.key));
            let packets = first.map_or_else(Vec::new, |first| packets.range(first..).cloned().collect());
            CamClip {
                params: SendParameters(camera.stream.params.0.clone()),
                time_base: camera.ring.time_base,
                fps: camera.stream.fps,
                size: camera.stream.size,
                packets: until_end(packets),
            }
        });
        let audio = self
            .audio
            .iter()
            .map(|audio| {
                let cut = audio.cut(start_us);
                let packets = audio.ring.packets.iter().filter(|packet| packet.pts >= cut).cloned().collect();
                AudioClip {
                    params: SendParameters(audio.spec.params.0.clone()),
                    time_base: audio.ring.time_base,
                    title: audio.spec.title.clone(),
                    default: audio.spec.default,
                    packets: until_end(packets),
                }
            })
            .collect();
        let (pcm_from, pcm_to) = (pcm_frame(start_us), pcm_frame(end_us));
        let start_ms = whole_ms(start_us);
        Some(ReplaySnapshot {
            length: self.length,
            saved_at: SystemClock.now(),
            age_us: now_us.saturating_sub(start_us),
            start_us,
            duration_ms: whole_ms(end_us - start_us),
            video_params: SendParameters(self.video.params.0.clone()),
            video_time_base: self.video.ring.time_base,
            video_start_pts: start_pts,
            video,
            camera,
            audio,
            pcm_from,
            pcm_to,
            pcm: self.pcm.iter().map(|pcm| pcm.clip(pcm_from, pcm_to)).collect(),
            marks: self.marks.clip(start_ms, now_ms),
        })
    }
}

/// The rings as a listener of a running capture: every packet and PCM block goes into them. Keeping one never fails.
pub(super) struct RingSink(Mutex<ReplayRings>);

impl RingSink {
    pub(super) fn new(rings: ReplayRings) -> Self {
        Self(Mutex::new(rings))
    }

    /// The rings, locked; keep it short, because the stream threads wait for it.
    pub(super) fn rings(&self) -> MutexGuard<'_, ReplayRings> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PacketSink for RingSink {
    fn video(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        self.rings().push_video(packet);
        Ok(())
    }

    fn camera(&self, packet: EncodedPacket) -> Result<(), MediaError> {
        self.rings().push_camera(packet);
        Ok(())
    }

    fn close_camera(&self) -> Option<CamFileEnd> {
        None
    }

    fn camera_bytes(&self) -> u64 {
        0
    }

    fn audio(&self, slot: usize, packet: EncodedPacket) -> Result<(), EngineError> {
        self.rings().push_audio(slot, packet);
        Ok(())
    }

    fn open_pcm(&self) -> Result<(), EngineError> {
        Ok(())
    }

    fn pcm(&self, track: usize, start: u64, samples: &[f32]) -> Result<u64, EngineError> {
        self.rings().push_pcm(track, start, samples);
        Ok(0)
    }

    fn flush_pcm(&self) {}

    fn close_pcm(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

struct CamClip {
    params: SendParameters,
    time_base: TimeBase,
    fps: (u32, u32),
    size: (u32, u32),
    packets: Vec<RingPacket>,
}

struct AudioClip {
    params: SendParameters,
    time_base: TimeBase,
    title: String,
    default: bool,
    packets: Vec<RingPacket>,
}

/// What one save writes. It owns its share of the buffer, so the rings keep filling while a writer thread works on it.
pub(super) struct ReplaySnapshot {
    length: Duration,
    /// The wall-clock time of the save.
    saved_at: DateTime<Utc>,
    /// How long before the save the clip starts.
    age_us: i64,
    /// The clip's start on the buffer's clock.
    start_us: i64,
    /// The length of the clip's video.
    duration_ms: u64,
    video_params: SendParameters,
    video_time_base: TimeBase,
    video_start_pts: i64,
    video: Vec<RingPacket>,
    camera: Option<CamClip>,
    audio: Vec<AudioClip>,
    /// The 48 kHz frames the WAV sidecars cover.
    pcm_from: u64,
    pcm_to: u64,
    pcm: Vec<Vec<PcmBlock>>,
    /// The kept marks, timed from the clip's start.
    marks: Vec<NewMark>,
}

impl ReplaySnapshot {
    /// Where the clip starts on the buffer's clock.
    #[cfg(test)]
    pub(super) fn start_us(&self) -> i64 {
        self.start_us
    }
}

/// Where a saved replay goes, named like any take.
pub(super) struct TakePaths {
    /// The take folder, which the writer creates.
    pub(super) folder: PathBuf,
    /// The base name of the take's files.
    pub(super) base_name: String,
    /// The main video file.
    pub(super) output_file: PathBuf,
}

/// A saved replay.
pub(super) struct ReplayTake {
    /// The totals of the take.
    pub(super) summary: Summary,
    /// The totals of the camera ISO file, when one was written.
    pub(super) cam: Option<CamSummary>,
    /// Why the camera file is missing although the setup has a camera.
    pub(super) cam_notice: Option<String>,
    /// One line per MP4 or MOV file that could not be finalized; such a file stays fragmented and playable.
    pub(super) finalize_warnings: Vec<String>,
    /// The last marks problem, which never fails the save.
    pub(super) marks_warning: Option<String>,
}

/// Writes a snapshot as a take of the setup `cfg`: the main video and the WAV sidecars are timed from the clip's start,
/// and the camera file from its own first frame. As after stopping a recording, MP4 and MOV files are then finalized.
pub(super) fn write_replay(mut snapshot: ReplaySnapshot, cfg: &SessionConfig, take: &TakePaths) -> Result<ReplayTake, EngineError> {
    media_io::init()?;
    std::fs::create_dir_all(&take.folder).map_err(|err| EngineError::Failed(format!("could not create {}: {err}", take.folder.display())))?;
    let mut cfg = cfg.clone();
    cfg.output_file.clone_from(&take.output_file);
    cfg.take_folder.clone_from(&take.folder);
    cfg.base_name.clone_from(&take.base_name);
    let started_at = snapshot.saved_at - TimeDelta::microseconds(snapshot.age_us);

    // As in a recording, a camera file that cannot be written never fails the take.
    let cam_size = snapshot.camera.as_ref().map(|camera| camera.size);
    let (cam, cam_notice) = match snapshot.camera.take().map(|camera| write_camera(camera, snapshot.start_us, &cfg)) {
        Some(Ok(written)) => (Some(written), None),
        Some(Err(reason)) => {
            let notice = format!("The camera file was skipped: {reason}");
            log::warn!("{notice}");
            (None, Some(notice))
        }
        None => (None, None),
    };
    let (cam, cam_start_offset_ms) = cam.map_or((None, None), |(summary, offset)| (Some(summary), Some(offset)));

    let mut session_file = SessionFile::begin(&cfg);
    session_file.set_replay(started_at.with_timezone(&chrono::Local), snapshot.length);
    session_file.set_cam(cam.as_ref().map(|cam| cam.file.as_path()), cam_notice.as_deref());
    let cam_name = cam.as_ref().map(|cam| file_name(&cam.file));
    session_file.set_composite(&CompositeBlock::new(&cfg.profile, cam_size, &file_name(&cfg.output_file), cam_name.as_deref()));
    if let Some((name, offset_ms)) = cam_name.as_deref().zip(cam_start_offset_ms) {
        session_file.set_camera_start_offset(name, offset_ms);
    }
    if let Err(err) = session_file.write() {
        log::warn!("could not write the session file: {err}");
    }

    let warning = MarksWarning::default();
    let outcome = write_main(snapshot, &cfg, started_at, cam.as_ref(), &warning);
    session_file.finish(&outcome, &Stats::new(&cfg.tracks).snapshot(), cam.as_ref());
    if let Err(err) = session_file.write() {
        log::warn!("could not update the session file: {err}");
    }
    let summary = outcome?;
    let files = std::iter::once(summary.file.as_path()).chain(cam.as_ref().map(|cam| cam.file.as_path()));
    let finalize_warnings = files.filter_map(finalize_file).collect();
    Ok(ReplayTake { summary, cam, cam_notice, marks_warning: warning.take(), finalize_warnings })
}

/// Rewrites an MP4 or MOV file as a standard file, as after a recording; the warning when that fails. Other files are
/// final as written.
fn finalize_file(file: &Path) -> Option<String> {
    let container = file.extension().and_then(|extension| extension.to_str()).and_then(media_io::Container::from_extension)?;
    if !container.needs_finalize() {
        return None;
    }
    let err = media_io::mux::finalize(file, container, &mut |_| {}).err()?;
    log::warn!("could not finalize {}: {err}", file.display());
    Some(format!("{} could not be finished and stays in its fragmented form, which still plays: {err}", file_name(file)))
}

/// One stream's packets on their way into the main file.
struct Feed {
    stream: usize,
    time_base: TimeBase,
    /// The clip's start in the stream's ticks, taken off every time stamp.
    offset: i64,
    packets: Vec<RingPacket>,
}

/// A copy of a buffered packet with new time stamps; None for a packet with no payload.
fn restamped(source: &RingPacket, pts: i64, dts: i64) -> Option<EncodedPacket> {
    let mut packet = Packet::copy(source.packet.data()?);
    packet.set_pts(Some(pts));
    packet.set_dts(Some(dts));
    if source.key {
        packet.set_flags(Flags::KEY);
    }
    Some(EncodedPacket::from_raw_packet(packet))
}

/// Queues every stream's packets in decode time order, so the file is interleaved as a recording is.
fn send_interleaved(mux: &MuxThread, feeds: Vec<Feed>) -> Result<(), MediaError> {
    let mut feeds: Vec<(Feed, usize)> = feeds.into_iter().map(|feed| (feed, 0)).collect();
    loop {
        let earliest = feeds
            .iter_mut()
            .filter_map(|(feed, next)| {
                let packet = feed.packets.get(*next)?;
                Some((ticks_to_us(packet.dts - feed.offset, feed.time_base), feed, next))
            })
            .min_by_key(|(time_us, ..)| *time_us);
        let Some((_, feed, next)) = earliest else { return Ok(()) };
        if let Some(source) = feed.packets.get(*next)
            && let Some(packet) = restamped(source, source.pts - feed.offset, source.dts - feed.offset)
        {
            mux.send(feed.stream, packet)?;
        }
        *next += 1;
    }
}

/// Writes the camera ISO file of a clip that starts at `start_us`. The file starts on the camera's own keyframe, so it
/// also returns how many ms before the clip's start that is: negative when the camera starts later.
fn write_camera(clip: CamClip, start_us: i64, cfg: &SessionConfig) -> Result<(CamSummary, i64), String> {
    let Some(end_pts) = clip.packets.iter().map(|packet| packet.pts).max().map(|pts| pts.saturating_add(1)) else {
        return Err("the buffer holds no camera keyframe in the clip.".to_owned());
    };
    let (offset, first_us) = clip.packets.first().map_or((0, start_us), |packet| (packet.pts, packet.time_us));
    let file = cam::file_path(&cfg.take_folder, &cfg.base_name, cfg.profile.container);
    let fps = (clip.fps.0 as i32, clip.fps.1 as i32);
    let mux = cam::open_muxer(&file, cfg.profile.container, clip.params.0, clip.time_base, fps)?;
    let sent =
        clip.packets.iter().try_for_each(|source| restamped(source, source.pts - offset, source.dts - offset).map_or(Ok(()), |packet| mux.send(0, packet)));
    // The mux thread's own result names the cause when a send failed.
    let finished = mux.finish().map_err(|err| err.to_string())?;
    sent.map_err(|err| err.to_string())?;
    let start_offset_ms = scale(start_us - first_us, 1, 1000);
    let summary = CamSummary {
        file,
        frames: clip.packets.len() as u64,
        duplicated: 0,
        dropped: 0,
        bytes_written: finished.bytes_written,
        duration_ms: whole_ms(ticks_to_us(end_pts - offset, clip.time_base)),
        fps: f64::from(clip.fps.0) / f64::from(clip.fps.1.max(1)),
        encode_avg_ms: 0.0,
        encode_p95_ms: 0.0,
        lost: false,
    };
    Ok((summary, start_offset_ms))
}

/// Writes the WAV sidecar of the track at `index`, as long as the clip, with silence wherever the ring has no samples.
fn write_wav(cfg: &SessionConfig, index: usize, blocks: &[PcmBlock], from: u64, to: u64) -> Result<PathBuf, EngineError> {
    let name = cfg.tracks.get(index).map_or("", |track| track.name.as_str());
    let mut wav = SidecarWav::create(&cfg.take_folder, &cfg.base_name, index, name)?;
    let channels = usize::from(OUT_CHANNELS);
    let silence = vec![0.0f32; SILENCE_FRAMES as usize * channels];
    let pad = |wav: &mut SidecarWav, mut frames: u64| -> Result<(), EngineError> {
        while frames > 0 {
            let step = frames.min(SILENCE_FRAMES);
            wav.write(&silence[..step as usize * channels])?;
            frames -= step;
        }
        Ok(())
    };
    let mut cursor = from;
    for block in blocks {
        let (first, end) = (cursor.max(block.start), block.end().min(to));
        if end <= first {
            continue;
        }
        pad(&mut wav, first - cursor)?;
        let range = (first - block.start) as usize * channels..(end - block.start) as usize * channels;
        wav.write(block.samples.get(range).unwrap_or_default())?;
        cursor = end;
    }
    pad(&mut wav, to.saturating_sub(cursor))?;
    wav.finalize()
}

/// The wall clock of a take written after the fact: the writer sets it to the time of whatever it adds next, so the
/// sidecar's times are those of the clip and a repeated key is judged by when it was pressed.
struct ReplayClock(Arc<Mutex<DateTime<Utc>>>);

impl Clock for ReplayClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn new_id(&self) -> uuid::Uuid {
        SystemClock.new_id()
    }
}

/// Writes the main video file, the WAV sidecars and the marks files.
fn write_main(
    snapshot: ReplaySnapshot,
    cfg: &SessionConfig,
    started_at: DateTime<Utc>,
    cam: Option<&CamSummary>,
    warning: &MarksWarning,
) -> Result<Summary, EngineError> {
    let mut wav_files = Vec::with_capacity(snapshot.pcm.len());
    for (index, blocks) in snapshot.pcm.iter().enumerate() {
        wav_files.push(write_wav(cfg, index, blocks, snapshot.pcm_from, snapshot.pcm_to)?);
    }

    let (frames, duration_ms) = (snapshot.video.len() as u64, snapshot.duration_ms);
    let mut specs = Vec::with_capacity(snapshot.audio.len());
    let mut audio_packets = Vec::with_capacity(snapshot.audio.len());
    for clip in snapshot.audio {
        let offset = us_to_ticks(snapshot.start_us, clip.time_base);
        audio_packets.push((clip.time_base, offset, clip.packets));
        specs.push(AudioStreamSpec { params: clip.params, time_base: clip.time_base, title: clip.title, default: clip.default });
    }
    let video_setup = VideoSetup { params: snapshot.video_params, time_base: snapshot.video_time_base };
    let (mux, indexes) = build_muxer(&cfg.output_file, cfg.profile.container, cfg.profile.fps, video_setup, specs)?;
    // The video stream is the muxer's first.
    let mut feeds = vec![Feed { stream: 0, time_base: snapshot.video_time_base, offset: snapshot.video_start_pts, packets: snapshot.video }];
    feeds.extend(indexes.into_iter().zip(audio_packets).map(|(stream, (time_base, offset, packets))| Feed { stream, time_base, offset, packets }));
    if let Err(err) = send_interleaved(&mux, feeds) {
        // The mux thread's own result names the cause when it has one.
        mux.finish()?;
        return Err(err.into());
    }

    let wall = Arc::new(Mutex::new(started_at));
    let set_wall = |at: DateTime<Utc>| *wall.lock().unwrap_or_else(PoisonError::into_inner) = at;
    // Mark times arrive already on the take's clock, so the media clock is never asked.
    let mut marks = TakeMarks::begin_with_wall(
        &cfg.take_folder,
        &cfg.base_name,
        &cfg.output_file,
        cfg.profile.fps,
        MediaClock::new(0, 1000),
        warning.clone(),
        Box::new(ReplayClock(Arc::clone(&wall))),
    );
    set_wall(snapshot.saved_at);
    let mut whole = NewMark { end_ms: Some(duration_ms), ..NewMark::point(MarkType::Highlight, 0, Source::Auto) };
    whole.label = Some(REPLAY_LABEL.to_owned());
    marks.keep(whole);
    for mut mark in snapshot.marks {
        // A press can be a few ms past the last frame the encoder had delivered.
        mark.t_ms = mark.t_ms.min(duration_ms);
        mark.end_ms = mark.end_ms.map(|end| end.min(duration_ms));
        set_wall(started_at + TimeDelta::milliseconds(i64::try_from(pressed_ms(&mark)).unwrap_or(0)));
        marks.keep(mark);
    }
    set_wall(snapshot.saved_at);
    mux.set_chapters(finish_marks(marks, &wav_files, duration_ms, cam));

    let finished = mux.finish()?;
    let seconds = duration_ms as f64 / 1000.0;
    Ok(Summary {
        file: cfg.output_file.clone(),
        frames,
        duplicated: 0,
        dropped: 0,
        bytes_written: finished.bytes_written,
        packets: finished.packets,
        duration_ms,
        average_mbps: if seconds > 0.0 { finished.bytes_written as f64 * 8.0 / seconds / 1_000_000.0 } else { 0.0 },
        wav_files,
        encode_avg_ms: 0.0,
        encode_p95_ms: 0.0,
        track_peaks: Vec::new(),
    })
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
