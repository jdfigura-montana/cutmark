//! The platform independent half of per-app audio: the shared types, picking
//! the top-level processes of an app, and mixing their streams by QPC time.
//! The Windows half is in `app_loopback` and `app_list`; other targets get the
//! stubs at the end of this file.
//!
//! See the per-app audio spec: Capture.

// Only the Windows half and the tests call the logic in this file.
#![cfg_attr(not(windows), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The sample rate of every per-app capture, in Hz.
pub(crate) const APP_SAMPLE_RATE: u32 = 48_000;
/// The channel count of every per-app capture.
pub(crate) const APP_CHANNELS: u16 = 2;
/// 100 nanosecond units per second.
const UNITS_PER_SECOND: i128 = 10_000_000;
/// A packet this close to where its stream's previous one ended continues it
/// without a gap or overlap, so timestamp jitter does not click: 10 ms.
const SNAP_FRAMES: i64 = 480;
/// How long the mixer waits for a stream that delivers nothing: 100 ms.
const MIX_LAG_FRAMES: i64 = 4_800;
/// The most frames in one mixed packet: 20 ms.
const MIX_CHUNK_FRAMES: i64 = 960;

/// One packet of interleaved 32 bit float samples from a running capture.
pub struct AudioPacket {
    /// QPC timestamp of the first frame in 100 nanosecond units, comparable with video frames.
    pub qpc_100ns: i64,
    /// Number of frames (sample sets) in the packet.
    pub frames: u32,
    /// Channel count, the same for every packet of one capture.
    pub channels: u16,
    /// Sample rate in Hz, the same for every packet of one capture.
    pub sample_rate: u32,
    /// Interleaved 32 bit float samples, frames times channels values.
    pub samples: Vec<f32>,
    /// True when WASAPI flagged a gap before this packet.
    pub discontinuity: bool,
    /// True when WASAPI flagged the buffer as silence; samples are zeros.
    pub silent: bool,
}

/// The app a per-app capture is about, named by its executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppTarget {
    /// The executable's file name in lowercase.
    exe: String,
}

impl AppTarget {
    /// Names an app by its executable; a path or any letter case is accepted.
    pub fn new(exe: &str) -> Self {
        Self { exe: normalize_exe_name(exe) }
    }

    /// The executable's file name in lowercase, for example `spotify.exe`.
    pub fn exe(&self) -> &str {
        &self.exe
    }
}

/// What a per-app capture hears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopbackMode {
    /// Only the app's sound: every instance with its child processes.
    Include,
    /// All system sound except one instance of the app with its child processes.
    Exclude,
}

/// One running app as shown in the app picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioApp {
    /// The executable's file name in lowercase, the app's stable identity.
    pub exe: String,
    /// The executable's file description, else the file name.
    pub display_name: String,
    /// The IDs of every running process of the executable, ascending.
    pub pids: Vec<u32>,
    /// True when one of the app's audio sessions is making sound now.
    pub active: bool,
}

/// The lowercase file name in `name`, which may be a path, quoted or padded.
pub(crate) fn normalize_exe_name(name: &str) -> String {
    let name = name.trim().trim_matches('"').trim();
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.to_lowercase()
}

/// One row of the process table.
#[derive(Clone, Debug)]
pub(crate) struct ProcessEntry {
    /// The process ID.
    pub pid: u32,
    /// The ID of the process that started it; that process may be gone and the ID reused.
    pub parent_pid: u32,
    /// The executable's file name in lowercase.
    pub exe: String,
    /// When the process was created, in FILETIME units; None when Windows will not say.
    pub created: Option<u64>,
}

/// The top-level processes of `exe`, oldest first: those with no ancestor that
/// runs the same executable. Processes of unknown age sort last.
pub(crate) fn top_level_processes<'a>(table: &'a [ProcessEntry], exe: &str) -> Vec<&'a ProcessEntry> {
    let by_pid: BTreeMap<u32, &ProcessEntry> = table.iter().map(|process| (process.pid, process)).collect();
    let mut top: Vec<&ProcessEntry> = table.iter().filter(|process| process.exe == exe && !has_same_exe_ancestor(&by_pid, process)).collect();
    top.sort_by_key(|process| (process.created.unwrap_or(u64::MAX), process.pid));
    top
}

/// Walks the parent chain of `process`, looking for its own executable. The
/// chain ends at a parent that is gone, or at a "parent" created after one of
/// its descendants, which only reuses the real parent's ID. A chain that leads
/// back to `process` is not a real family, so it counts as no ancestor.
fn has_same_exe_ancestor(by_pid: &BTreeMap<u32, &ProcessEntry>, process: &ProcessEntry) -> bool {
    let mut seen = BTreeSet::from([process.pid]);
    // The age of the nearest descendant that has a known one.
    let mut descendant_created = process.created;
    let mut parent_pid = process.parent_pid;
    let mut found = false;
    while let Some(parent) = by_pid.get(&parent_pid) {
        if parent.pid == process.pid {
            return false;
        }
        if !seen.insert(parent.pid) || matches!((parent.created, descendant_created), (Some(p), Some(d)) if p > d) {
            break;
        }
        found |= parent.exe == process.exe;
        descendant_created = parent.created.or(descendant_created);
        parent_pid = parent.parent_pid;
    }
    found
}

/// Mixes the packets of several 48 kHz stereo streams into one stream, placing
/// them by their QPC timestamps. A frame is emitted once every stream has
/// delivered past it, or once it is older than the mix lag.
pub(crate) struct Mixer {
    /// The QPC time of frame 0: the first packet's timestamp.
    origin_100ns: Option<i64>,
    /// The frame index of the first buffered frame.
    base: i64,
    /// The frames not emitted yet, interleaved stereo.
    buffer: VecDeque<f32>,
    /// Per stream, the frame after its last packet; None before its first one.
    ends: Vec<Option<i64>>,
    /// The frame after the last emitted one; None before the first output.
    emitted_to: Option<i64>,
    /// True when the next output packet follows a gap.
    gap: bool,
}

impl Mixer {
    /// A mixer for `streams` streams.
    pub(crate) fn new(streams: usize) -> Self {
        Self { origin_100ns: None, base: 0, buffer: VecDeque::new(), ends: vec![None; streams], emitted_to: None, gap: false }
    }

    /// Adds a packet of one stream into the mix. Frames that were already emitted are dropped.
    pub(crate) fn push(&mut self, stream: usize, packet: &AudioPacket) {
        let origin = *self.origin_100ns.get_or_insert(packet.qpc_100ns);
        let channels = usize::from(APP_CHANNELS);
        let frames = (packet.samples.len() / channels) as i64;
        let stamped = frame_at(origin, packet.qpc_100ns);
        let Some(end) = self.ends.get_mut(stream) else {
            return;
        };
        let start = match *end {
            Some(previous) if !packet.discontinuity && (stamped - previous).abs() <= SNAP_FRAMES => previous,
            _ => stamped,
        };
        *end = Some(start + frames);
        let skip = self.emitted_to.map_or(0, |emitted| (emitted - start).clamp(0, frames));
        if skip == frames {
            return;
        }
        let first = start + skip;
        if self.buffer.is_empty() {
            self.gap |= self.emitted_to.is_some_and(|emitted| first > emitted);
            self.base = first;
        }
        while first < self.base {
            for _ in 0..channels {
                self.buffer.push_front(0.0);
            }
            self.base -= 1;
        }
        let offset = (first - self.base) as usize * channels;
        let samples = &packet.samples[skip as usize * channels..frames as usize * channels];
        if self.buffer.len() < offset + samples.len() {
            self.buffer.resize(offset + samples.len(), 0.0);
        }
        for (mixed, sample) in self.buffer.range_mut(offset..).zip(samples) {
            *mixed += sample;
        }
        self.gap |= packet.discontinuity;
    }

    /// The next mixed packet that is ready at `now_100ns`, if any.
    pub(crate) fn pop(&mut self, now_100ns: i64) -> Option<AudioPacket> {
        let horizon = frame_at(self.origin_100ns?, now_100ns) - MIX_LAG_FRAMES;
        // A stream without a packet yet leaves only the lag to decide.
        let all_delivered = self.ends.iter().copied().collect::<Option<Vec<i64>>>().and_then(|ends| ends.into_iter().min());
        self.take_until(all_delivered.map_or(horizon, |delivered| delivered.max(horizon)))
    }

    /// The next mixed packet without waiting for anything; for the end of a capture.
    pub(crate) fn flush(&mut self) -> Option<AudioPacket> {
        self.take_until(i64::MAX)
    }

    /// Removes up to one packet of the buffered frames before `limit`.
    fn take_until(&mut self, limit: i64) -> Option<AudioPacket> {
        let origin = self.origin_100ns?;
        let channels = usize::from(APP_CHANNELS);
        let buffered = (self.buffer.len() / channels) as i64;
        let frames = limit.saturating_sub(self.base).min(buffered).min(MIX_CHUNK_FRAMES);
        if frames <= 0 {
            return None;
        }
        // The sum of several streams can leave the float range.
        let samples: Vec<f32> = self.buffer.drain(..frames as usize * channels).map(|sample| sample.clamp(-1.0, 1.0)).collect();
        let qpc_100ns = origin + (i128::from(self.base) * UNITS_PER_SECOND / i128::from(APP_SAMPLE_RATE)) as i64;
        self.base += frames;
        self.emitted_to = Some(self.base);
        Some(AudioPacket {
            qpc_100ns,
            frames: frames as u32,
            channels: APP_CHANNELS,
            sample_rate: APP_SAMPLE_RATE,
            silent: samples.iter().all(|&sample| sample == 0.0),
            samples,
            discontinuity: std::mem::take(&mut self.gap),
        })
    }
}

/// The frame index of a QPC time, to the nearest frame, counted from `origin_100ns`.
fn frame_at(origin_100ns: i64, qpc_100ns: i64) -> i64 {
    let units = i128::from(qpc_100ns) - i128::from(origin_100ns);
    (units * i128::from(APP_SAMPLE_RATE) + UNITS_PER_SECOND / 2).div_euclid(UNITS_PER_SECOND) as i64
}

/// Per-app audio on targets other than Windows: every entry point reports that it is unsupported.
#[cfg(not(windows))]
mod unsupported {
    use std::convert::Infallible;
    use std::sync::mpsc::Receiver;

    use super::{AppTarget, AudioApp, AudioPacket, LoopbackMode};
    use crate::CaptureError;

    /// Stands in for the Windows capture; no value of it can exist.
    pub struct AppLoopback {
        never: Infallible,
    }

    impl AppLoopback {
        /// Always fails: per-app capture needs Windows.
        pub fn start(_target: AppTarget, _mode: LoopbackMode) -> Result<Self, CaptureError> {
            Err(CaptureError::Unsupported("per-app audio capture"))
        }

        /// The packet queue.
        pub fn receiver(&self) -> &Receiver<AudioPacket> {
            match self.never {}
        }

        /// How many packets were produced.
        pub fn packets(&self) -> u64 {
            match self.never {}
        }

        /// How many packets were dropped.
        pub fn overflows(&self) -> u64 {
            match self.never {}
        }

        /// Whether the capture ended.
        pub fn lost(&self) -> bool {
            match self.never {}
        }

        /// How many instances are excluded.
        pub fn excluded_instances(&self) -> usize {
            match self.never {}
        }

        /// How many instances were running.
        pub fn total_instances(&self) -> usize {
            match self.never {}
        }

        /// The instances that were running.
        pub fn instance_pids(&self) -> &[u32] {
            match self.never {}
        }

        /// How many timestamps were estimated.
        pub fn estimated_timestamps(&self) -> u64 {
            match self.never {}
        }

        /// Stops the capture.
        pub fn stop(self) {}
    }

    /// Always fails: listing apps needs Windows.
    pub fn list_audio_apps(_include_all: bool) -> Result<Vec<AudioApp>, CaptureError> {
        Err(CaptureError::Unsupported("listing audio apps"))
    }

    /// Always empty: listing processes needs Windows.
    pub fn top_level_pids(_exe: &str) -> Vec<u32> {
        Vec::new()
    }
}

#[cfg(not(windows))]
pub use unsupported::{AppLoopback, list_audio_apps, top_level_pids};

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of a fake process table.
    fn process(pid: u32, parent_pid: u32, exe: &str, created: u64) -> ProcessEntry {
        ProcessEntry { pid, parent_pid, exe: exe.to_owned(), created: Some(created) }
    }

    /// The IDs of the top-level processes of `exe`.
    fn top_pids(table: &[ProcessEntry], exe: &str) -> Vec<u32> {
        top_level_processes(table, exe).iter().map(|process| process.pid).collect()
    }

    #[test]
    fn exe_names_are_lowercase_file_names() {
        assert_eq!(normalize_exe_name("Spotify.exe"), "spotify.exe");
        assert_eq!(normalize_exe_name(r"C:\Program Files\Game\GAME.EXE"), "game.exe");
        assert_eq!(normalize_exe_name("  \"C:/Apps/Discord.exe\" "), "discord.exe");
        assert_eq!(AppTarget::new(r"D:\OBS64.exe").exe(), "obs64.exe");
    }

    #[test]
    fn top_level_processes_are_those_without_a_parent_of_the_same_exe_oldest_first() {
        let table = [
            process(4, 0, "explorer.exe", 1),
            // The second instance was started first in the table but later in time.
            process(20, 4, "app.exe", 50),
            process(10, 4, "app.exe", 30),
            // Children and a grandchild of the same exe are covered by their tree.
            process(11, 10, "app.exe", 31),
            process(12, 11, "app.exe", 32),
            process(13, 10, "helper.exe", 33),
            // A launcher plus game: the parent has gone.
            process(40, 999, "app.exe", 40),
        ];
        assert_eq!(top_pids(&table, "app.exe"), [10, 40, 20]);
        assert_eq!(top_pids(&table, "helper.exe"), [13]);
        assert!(top_pids(&table, "missing.exe").is_empty());
    }

    #[test]
    fn a_reused_parent_id_and_an_unknown_age_are_handled() {
        let mut table = vec![
            // 30 names 31 as its parent, but 31 is younger: the ID was reused.
            process(30, 31, "app.exe", 10),
            process(31, 4, "app.exe", 20),
            process(2, 4, "app.exe", 5),
        ];
        table[2].created = None;
        assert_eq!(top_pids(&table, "app.exe"), [30, 31, 2]);
    }

    #[test]
    fn a_launcher_that_restarts_the_app_counts_as_one_top_level_process() {
        let table = [
            process(4, 0, "explorer.exe", 1),
            // The app, a launcher it started, the app again, and that one's child.
            process(10, 4, "app.exe", 10),
            process(11, 10, "launcher.exe", 11),
            process(12, 11, "app.exe", 12),
            process(13, 12, "app.exe", 13),
        ];
        assert_eq!(top_pids(&table, "app.exe"), [10]);
    }

    #[test]
    fn a_reused_id_further_up_the_chain_ends_the_walk() {
        let mut table = vec![
            // 51 was reused by a process younger than 52, so 50 is no ancestor of 52.
            process(50, 4, "app.exe", 10),
            process(51, 50, "other.exe", 90),
            process(52, 51, "app.exe", 20),
        ];
        assert_eq!(top_pids(&table, "app.exe"), [50, 52]);
        // A link of unknown age does not hide that its parent is too young.
        table[0].created = Some(30);
        table[1].created = None;
        assert_eq!(top_pids(&table, "app.exe"), [52, 50]);
    }

    #[test]
    fn cycles_in_the_parent_links_end_the_walk() {
        let mut table = vec![
            // Two processes that name each other, and a child of one of them.
            process(60, 61, "app.exe", 10),
            process(61, 60, "app.exe", 10),
            process(62, 60, "app.exe", 11),
            // A cycle above the process that does not lead back to it.
            process(70, 71, "app.exe", 20),
            process(71, 72, "other.exe", 20),
            process(72, 71, "other.exe", 20),
            // A process that names itself.
            process(80, 80, "app.exe", 30),
        ];
        assert_eq!(top_pids(&table, "app.exe"), [60, 61, 70, 80]);
        for row in &mut table {
            row.created = None;
        }
        assert_eq!(top_pids(&table, "app.exe"), [60, 61, 70, 80]);
    }

    /// 10 ms of one stereo value in 100 nanosecond units and in frames.
    const TEN_MS: i64 = 100_000;
    const FRAMES: usize = 480;
    /// An arbitrary QPC start time.
    const START: i64 = 5_000_000_000;

    /// 10 ms of `value` stamped at `qpc_100ns`.
    fn packet(qpc_100ns: i64, value: f32) -> AudioPacket {
        AudioPacket {
            qpc_100ns,
            frames: FRAMES as u32,
            channels: APP_CHANNELS,
            sample_rate: APP_SAMPLE_RATE,
            samples: vec![value; FRAMES * 2],
            discontinuity: false,
            silent: value == 0.0,
        }
    }

    /// The first value of each 10 ms block of a packet.
    fn blocks(packet: &AudioPacket) -> Vec<f32> {
        packet.samples.chunks(FRAMES * 2).map(|block| block[0]).collect()
    }

    #[test]
    fn two_streams_are_summed_where_their_qpc_times_overlap() -> Result<(), &'static str> {
        let mut mixer = Mixer::new(2);
        mixer.push(0, &packet(START, 0.25));
        assert!(mixer.pop(START + TEN_MS).is_none(), "the second stream has not delivered yet");
        // The second stream starts half a packet later.
        mixer.push(1, &packet(START + TEN_MS / 2, 0.5));
        let mixed = mixer.pop(START + 2 * TEN_MS).ok_or("both streams delivered the first packet's frames")?;
        assert_eq!((mixed.qpc_100ns, mixed.frames, mixed.channels, mixed.sample_rate), (START, 480, 2, 48_000));
        assert!(mixed.samples[..480].iter().all(|&sample| sample == 0.25), "only the first stream so far");
        assert!(mixed.samples[480..].iter().all(|&sample| sample == 0.75), "both streams");
        assert!(!mixed.silent && !mixed.discontinuity);
        // The rest of the second stream waits for the first, then both are there.
        assert!(mixer.pop(START + 2 * TEN_MS).is_none());
        mixer.push(0, &packet(START + TEN_MS, 0.25));
        let rest = mixer.pop(START + 2 * TEN_MS);
        assert_eq!(rest.map(|rest| (rest.qpc_100ns, rest.frames, rest.samples[0])), Some((START + TEN_MS, 240, 0.75)));
        Ok(())
    }

    #[test]
    fn two_full_scale_streams_are_clamped() {
        let mut mixer = Mixer::new(2);
        for stream in 0..2 {
            mixer.push(stream, &packet(START, 1.0));
            mixer.push(stream, &packet(START + TEN_MS, -1.0));
        }
        let mixed = mixer.flush();
        assert_eq!(mixed.as_ref().map(blocks), Some(vec![1.0, -1.0]));
        assert!(mixed.is_some_and(|mixed| mixed.samples.iter().all(|sample| sample.abs() == 1.0)));
    }

    #[test]
    fn a_stalled_stream_only_holds_the_mix_back_for_the_lag() {
        let mut mixer = Mixer::new(2);
        mixer.push(0, &packet(START, 0.25));
        mixer.push(0, &packet(START + TEN_MS, 0.5));
        assert!(mixer.pop(START + 10 * TEN_MS).is_none(), "still inside the 100 ms lag");
        let mixed = mixer.pop(START + 12 * TEN_MS);
        assert_eq!(mixed.map(|mixed| blocks(&mixed)), Some(vec![0.25, 0.5]));
        // What the stalled stream delivers for that time now is too late.
        mixer.push(1, &packet(START + TEN_MS, 1.0));
        assert!(mixer.flush().is_none());
    }

    #[test]
    fn timestamp_jitter_is_absorbed_and_a_real_gap_is_flagged() {
        let mut mixer = Mixer::new(1);
        mixer.push(0, &packet(START, 0.25));
        // 1 ms late: still the continuation of the first packet.
        mixer.push(0, &packet(START + TEN_MS + 10_000, 0.5));
        let mixed = mixer.flush();
        assert_eq!(mixed.map(|mixed| (blocks(&mixed), mixed.discontinuity)), Some((vec![0.25, 0.5], false)));
        // A second of nothing is not filled with silence; the jump is flagged.
        mixer.push(0, &packet(START + 100 * TEN_MS, 0.0));
        let after = mixer.flush();
        assert_eq!(after.map(|after| (after.qpc_100ns, after.discontinuity, after.silent)), Some((START + 100 * TEN_MS, true, true)));
    }

    #[test]
    fn an_earlier_packet_from_another_stream_moves_the_start_back() {
        let mut mixer = Mixer::new(2);
        mixer.push(0, &packet(START + TEN_MS, 0.25));
        mixer.push(1, &packet(START, 0.5));
        mixer.push(1, &packet(START + TEN_MS, 0.5));
        let mixed = mixer.flush();
        assert_eq!(mixed.map(|mixed| (mixed.qpc_100ns, blocks(&mixed))), Some((START, vec![0.5, 0.75])));
    }
}
