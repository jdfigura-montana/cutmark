//! The per-track audio pipeline: any device's packets in, 48 kHz stereo locked
//! to the recording timeline out. Pure logic, no WASAPI, no files, no egui.
//!
//! See the recorder spec: Audio capture and tracks (Per-track controls, Format)
//! and Sync and reliability rules (Audio, Pause).

use capture::AudioPacket;
use rubato::audioadapter_buffers::direct::{InterleavedSlice, SequentialSliceOfVecs};
use rubato::{Adjustable, Async, FixedAsync, PolynomialDegree, Resampler, Resizable};

/// Proportional gain of the drift controller, in ratio per frame of error.
const DRIFT_KP: f64 = 2.0e-4;
/// Integral gain of the drift controller, in ratio per accumulated frame of error.
const DRIFT_KI: f64 = 2.0e-6;
/// Largest relative ratio correction the controller may apply, plus or minus 0.2 percent.
const ADJUST_LIMIT: f64 = 0.002;
/// Relative ratio bound passed to the resampler, leaving headroom over the controller.
const RATIO_HEADROOM: f64 = 1.01;
/// Errors larger than this fraction of a second force a resync instead of a trim.
const RESYNC_LIMIT_S: f64 = 0.05;
/// Interpolation quality of the resampler.
const POLY_DEGREE: PolynomialDegree = PolynomialDegree::Cubic;

/// The timeline index a packet sample stamped qpc_100ns lands on, in output frames.
fn expected_index(qpc_100ns: i64, offset_100ns: i64, start_100ns: i64, out_rate: u32) -> f64 {
    let raw = (i128::from(qpc_100ns) + i128::from(offset_100ns) - i128::from(start_100ns)) * i128::from(out_rate) / 10_000_000;
    raw.max(0) as f64
}

/// Converts, resamples and time-locks one audio device onto the recording timeline.
///
/// Input packets carry QPC stamps; their samples are placed at
/// `(qpc_100ns + offset_100ns - start_100ns) * out_rate / 10_000_000` on the
/// output timeline. A gentle proportional-integral controller trims the resampler
/// ratio so a device clock a fraction of a percent off still stays locked.
#[allow(dead_code)]
pub struct TrackPipeline {
    /// The output sample rate, always 48000 in the recorder.
    out_rate: u32,
    /// The QPC value that recording time 0 maps to, shifted forward by pauses.
    start_100ns: i64,
    /// The per-track sync offset, in 100 nanosecond units.
    offset_100ns: i64,
    /// Linear output gain applied when frames are taken.
    gain: f32,
    /// Whether the track output is muted.
    muted: bool,
    /// The QPC value when pause was called, if currently paused.
    paused_at: Option<i64>,
    /// The resampler, created on the first packet once the input rate is known.
    resampler: Option<Async<f32>>,
    /// The nominal output over input ratio for the current device.
    base_ratio: f64,
    /// Fixed input chunk size of the resampler, about 10 ms of input frames.
    chunk: usize,
    /// The resampler's delay in output frames, accounted for when placing audio.
    delay: usize,
    /// Interleaved stereo input frames staged until they fill one resampler chunk.
    staged: Vec<f32>,
    /// Interleaved stereo output frames ready to be taken.
    out: Vec<f32>,
    /// Total output frames produced so far, silence fill included.
    out_frames: u64,
    /// The current relative ratio correction of the drift controller.
    adj: f64,
    /// The accumulated error of the drift controller.
    integ: f64,
    /// Per channel scratch buffers the resampler writes each chunk into.
    out_data: Vec<Vec<f32>>,
    /// The output peak since the last read of the meter.
    peak: f32,
    /// Whether output has clipped since the last read of the meter.
    clip: bool,
    /// The highest output peak since the pipeline was created; reading the meter does not reset it.
    peak_hold: f32,
}

#[allow(dead_code)]
impl TrackPipeline {
    /// Creates a pipeline whose timeline starts at start_100ns and outputs at out_rate.
    pub fn new(start_100ns: i64, out_rate: u32) -> Self {
        Self {
            out_rate,
            start_100ns,
            offset_100ns: 0,
            gain: 1.0,
            muted: false,
            paused_at: None,
            resampler: None,
            base_ratio: 1.0,
            chunk: 480,
            delay: 0,
            staged: Vec::new(),
            out: Vec::new(),
            out_frames: 0,
            adj: 0.0,
            integ: 0.0,
            out_data: Vec::new(),
            peak: 0.0,
            clip: false,
            peak_hold: 0.0,
        }
    }

    /// Places one device packet on the timeline; packets pushed inside a pause are discarded.
    pub fn push(&mut self, p: &AudioPacket) {
        if self.paused_at.is_some() || p.sample_rate == 0 || p.frames == 0 {
            return;
        }
        let first = self.resampler.is_none();
        if first && !self.init_resampler(p.sample_rate) {
            return;
        }
        self.stage_as_stereo(p);
        let expected = expected_index(p.qpc_100ns, self.offset_100ns, self.start_100ns, self.out_rate);
        let limit = f64::from(self.out_rate) * RESYNC_LIMIT_S;
        // The packet's first sample comes out of the resampler delay frames from now.
        let err = expected - (self.out_frames + self.delay as u64) as f64;
        if first || p.discontinuity || err.abs() > limit {
            self.adj = 0.0;
            self.integ = 0.0;
            if err > 0.0 {
                // Ahead of us: pad silence up to the expected index.
                self.pad_silence(err.round() as u64);
            } else if err < 0.0 {
                // Behind us: drop input so the stream catches up.
                let ratio = self.base_ratio * (1.0 + self.adj);
                self.drop_staged(((-err / ratio).round()) as usize);
            }
        } else {
            let clamp = ADJUST_LIMIT / DRIFT_KI;
            self.integ = (self.integ + err).clamp(-clamp, clamp);
            self.adj = (DRIFT_KP * err + DRIFT_KI * self.integ).clamp(-ADJUST_LIMIT, ADJUST_LIMIT);
        }
        if let Some(res) = self.resampler.as_mut() {
            let _ = res.set_resample_ratio(self.base_ratio * (1.0 + self.adj), false);
        }
        self.process_staged();
    }

    /// Pads silence so the output reaches at least index frames on the timeline.
    pub fn fill_silence_to(&mut self, index: u64) {
        if self.out_frames < index {
            self.pad_silence(index - self.out_frames);
        }
    }

    /// Stops the timeline at now_100ns; packets pushed while paused are discarded.
    pub fn pause(&mut self, now_100ns: i64) {
        if self.paused_at.is_none() {
            self.paused_at = Some(now_100ns);
        }
    }

    /// Resumes after a pause, shifting the timeline by the paused time so there is no gap.
    pub fn resume(&mut self, now_100ns: i64) {
        if let Some(at) = self.paused_at.take() {
            self.start_100ns += now_100ns - at;
        }
    }

    /// Sets the per-track output gain in dB.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain = 10f32.powf(gain_db / 20.0);
    }

    /// Mutes or unmutes the track output.
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Sets the sync offset in ms, clamped to -500..500; it shifts later packets.
    pub fn set_offset_ms(&mut self, offset_ms: f32) {
        self.offset_100ns = (offset_ms.clamp(-500.0, 500.0) * 10_000.0) as i64;
    }

    /// Drains up to max_frames of interleaved stereo output with gain and mute applied.
    pub fn take(&mut self, max_frames: usize) -> Vec<f32> {
        let count = max_frames.min(self.out.len() / 2) * 2;
        let mut taken = Vec::with_capacity(count);
        for value in self.out.drain(..count) {
            let sample = if self.muted { 0.0 } else { value * self.gain };
            let magnitude = sample.abs();
            if magnitude > self.peak {
                self.peak = magnitude;
            }
            if magnitude > self.peak_hold {
                self.peak_hold = magnitude;
            }
            if sample >= 1.0 || sample <= -1.0 {
                self.clip = true;
            }
            taken.push(sample);
        }
        taken
    }

    /// Total output frames produced so far, silence fill included.
    pub fn produced_frames(&self) -> u64 {
        self.out_frames
    }

    /// The linear output peak since the last call, which resets it.
    pub fn peak_since_last_read(&mut self) -> f32 {
        let peak = self.peak;
        self.peak = 0.0;
        peak
    }

    /// The highest output peak since the pipeline was created, in dBFS; -100.0 for silence.
    pub fn peak_hold_dbfs(&self) -> f32 {
        if self.peak_hold > 0.0 { 20.0 * self.peak_hold.log10() } else { -100.0 }
    }

    /// True if any output sample reached 1.0 or more since the last call, which resets it.
    pub fn clipped(&mut self) -> bool {
        let clip = self.clip;
        self.clip = false;
        clip
    }

    /// Builds the resampler for a device at in_rate; false if the rate is unusable.
    fn init_resampler(&mut self, in_rate: u32) -> bool {
        let base = f64::from(self.out_rate) / f64::from(in_rate);
        if !base.is_finite() || base <= 0.0 {
            return false;
        }
        // About 10 ms of input frames per resampler chunk.
        let chunk = (in_rate / 100).clamp(16, 8192) as usize;
        match Async::new_poly(base, RATIO_HEADROOM, POLY_DEGREE, chunk, 2, FixedAsync::Input) {
            Ok(res) => {
                self.delay = res.output_delay();
                let out_max = res.output_frames_max();
                self.out_data = vec![vec![0.0; out_max]; 2];
                self.resampler = Some(res);
                self.base_ratio = base;
                self.chunk = chunk;
                true
            }
            Err(_) => false,
        }
    }

    /// Converts a packet to stereo and stages it; mono doubles, more than 2 keeps the first two.
    fn stage_as_stereo(&mut self, p: &AudioPacket) {
        let channels = usize::from(p.channels.max(1));
        let frames = (p.samples.len() / channels).min(p.frames as usize);
        if channels == 1 {
            for value in &p.samples[..frames] {
                self.staged.push(*value);
                self.staged.push(*value);
            }
        } else {
            for frame in 0..frames {
                self.staged.push(p.samples[frame * channels]);
                self.staged.push(p.samples[frame * channels + 1]);
            }
        }
    }

    /// Appends n frames of silence to the output.
    fn pad_silence(&mut self, frames: u64) {
        for _ in 0..frames {
            self.out.push(0.0);
            self.out.push(0.0);
        }
        self.out_frames += frames;
    }

    /// Removes up to frames of staged input from the front.
    fn drop_staged(&mut self, frames: usize) {
        let count = frames.min(self.staged.len() / 2) * 2;
        self.staged.drain(..count);
    }

    /// Feeds all staged input through the resampler into the output.
    ///
    /// The chunk size shrinks to the staged amount, so nothing waits in staging
    /// and the output count stays locked to the timeline.
    fn process_staged(&mut self) {
        while self.staged.len() >= 2 {
            let frames = (self.staged.len() / 2).min(self.chunk);
            let mut produced = 0usize;
            {
                let res = match self.resampler.as_mut() {
                    Some(res) => res,
                    None => break,
                };
                let _ = res.set_chunk_size(frames);
                let out_len = self.out_data[0].len();
                let window = &self.staged[..frames * 2];
                let input = InterleavedSlice::new(window, 2, frames);
                let output = SequentialSliceOfVecs::new_mut(&mut self.out_data, 2, out_len);
                if let (Ok(input), Ok(mut output)) = (input, output)
                    && let Ok((_, count)) = res.process_into_buffer(&input, &mut output, None)
                {
                    produced = count;
                }
            }
            for (left, right) in self.out_data[0].iter().zip(&self.out_data[1]).take(produced) {
                self.out.push(*left);
                self.out.push(*right);
            }
            self.out_frames += produced as u64;
            // Always drain the chunk, even on error, so the stream keeps moving.
            self.staged.drain(..frames * 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT_RATE: u32 = 48_000;
    const PACKET_100NS: i64 = 100_000;

    /// One 10 ms packet of a sine wave at the given device rate and channel count.
    fn sine_packet(qpc: i64, rate: u32, channels: u16, freq: f64, amp: f32, phase: f64) -> AudioPacket {
        let frames = rate / 100;
        let mut samples = Vec::with_capacity(frames as usize * usize::from(channels));
        for i in 0..frames {
            let t = f64::from(i) / f64::from(rate);
            let value = amp * ((2.0 * core::f64::consts::PI * freq * t + phase).sin() as f32);
            for _ in 0..channels {
                samples.push(value);
            }
        }
        AudioPacket { qpc_100ns: qpc, frames, channels, sample_rate: rate, samples, discontinuity: false, silent: false }
    }

    /// Drains every ready output frame from the pipeline.
    fn drain_all(pipe: &mut TrackPipeline) -> Vec<f32> {
        let mut all = Vec::new();
        loop {
            let chunk = pipe.take(OUT_RATE as usize);
            if chunk.is_empty() {
                break;
            }
            all.extend(chunk);
        }
        all
    }

    #[test]
    fn fast_48k_source_stays_locked_for_30_minutes() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let packets = 180_090; // 30 minutes of 10 ms packets at a clock 0.05 percent fast
        let mut worst: i128 = 0;
        for n in 0..packets {
            let qpc = (n as f64 * PACKET_100NS as f64 / 1.0005).round() as i64;
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            if n % 100 == 0 {
                let timeline_end = (i128::from(qpc) + i128::from(PACKET_100NS)) * i128::from(OUT_RATE) / 10_000_000;
                let diff = (timeline_end - i128::from(pipe.produced_frames())).abs();
                worst = worst.max(diff);
            }
        }
        assert!(worst <= 800, "worst timeline error was {worst} frames");
        let final_diff = (i128::from(pipe.produced_frames()) - 30 * 60 * i128::from(OUT_RATE)).abs();
        assert!(final_diff <= 48, "final length off by {final_diff} frames");
    }

    #[test]
    fn mono_44k1_source_gives_the_right_stereo_length() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..1000 {
            pipe.push(&sine_packet(qpc, 44_100, 1, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let diff = (i128::from(pipe.produced_frames()) - 10 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 48, "length off by {diff} frames");
        let out = drain_all(&mut pipe);
        assert_eq!(out.len() / 2, pipe.produced_frames() as usize);
    }

    #[test]
    fn late_first_packet_starts_with_two_seconds_of_silence() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        pipe.push(&sine_packet(2 * 10_000_000, 48_000, 2, 440.0, 0.9, 0.0));
        let out = drain_all(&mut pipe);
        let quiet = (96_000 - 8) * 2;
        assert!(out.len() >= quiet, "only {} output frames", out.len() / 2);
        assert!(out[..quiet].iter().all(|v| *v == 0.0), "expected pure silence first");
        let start = first_loud(&out, 0, 0.05);
        assert!((96_000 - 8..=96_000 + 8).contains(&start), "signal started at {start}");
    }

    #[test]
    fn ten_second_gap_with_silence_fill_stays_continuous() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        // A loopback device sends nothing from 1 s to 11 s; the caller fills silence.
        pipe.fill_silence_to(u64::from(5 * OUT_RATE));
        pipe.fill_silence_to(u64::from(11 * OUT_RATE));
        // A late packet from inside the gap overlaps filled output and is dropped.
        pipe.push(&sine_packet(10 * 10_000_000 + 5 * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        // Packets resume at 11 s with a loud marker whose first sample peaks.
        pipe.push(&sine_packet(11 * 10_000_000, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        qpc = 11 * 10_000_000 + PACKET_100NS;
        for _ in 1..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let out = drain_all(&mut pipe);
        assert!(out[..96_000].iter().any(|v| v.abs() > 0.1), "pre-gap signal missing");
        assert!(out[100_000..1_000_000].iter().all(|v| *v == 0.0), "gap is not continuous silence");
        let land = first_loud(&out, 500_000, 0.8);
        assert!((528_000 - 48..=528_000 + 48).contains(&land), "packet after the gap landed at {land}");
        let diff = (i128::from(pipe.produced_frames()) - 12 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 96, "total length off by {diff} frames");
    }

    #[test]
    fn offset_plus_200ms_shifts_placement_by_9600_frames() {
        let mut plain = TrackPipeline::new(0, OUT_RATE);
        plain.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        let plain_start = first_loud(&drain_all(&mut plain), 0, 0.5);
        let mut shifted = TrackPipeline::new(0, OUT_RATE);
        shifted.set_offset_ms(200.0);
        shifted.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, core::f64::consts::FRAC_PI_2));
        let shifted_start = first_loud(&drain_all(&mut shifted), 0, 0.5);
        let shift = shifted_start as i64 - plain_start as i64;
        assert!((shift - 9_600).abs() <= 4, "shift was {shift} frames");
    }

    #[test]
    fn gain_mute_and_clip_indicator_follow_the_output() {
        let mut plain = TrackPipeline::new(0, OUT_RATE);
        for n in 0..100 {
            plain.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        }
        drain_all(&mut plain);
        let peak_plain = plain.peak_since_last_read();
        assert!((0.8..=1.0).contains(&peak_plain), "peak was {peak_plain}");

        let mut halved = TrackPipeline::new(0, OUT_RATE);
        halved.set_gain_db(-6.0);
        for n in 0..100 {
            halved.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.9, 0.0));
        }
        drain_all(&mut halved);
        let peak_half = halved.peak_since_last_read();
        assert!((peak_half - 0.5 * peak_plain).abs() <= 0.01 * peak_plain, "gain -6 dB peak {peak_half} vs {peak_plain}");

        let mut muted = TrackPipeline::new(0, OUT_RATE);
        muted.set_muted(true);
        muted.push(&sine_packet(0, 48_000, 2, 440.0, 0.9, 0.0));
        let out = drain_all(&mut muted);
        assert!(out.iter().all(|v| *v == 0.0), "muted output is not silent");
        assert_eq!(muted.peak_since_last_read(), 0.0);
        assert!(!muted.clipped());

        let mut hot = TrackPipeline::new(0, OUT_RATE);
        hot.push(&sine_packet(0, 48_000, 2, 440.0, 1.2, 0.0));
        drain_all(&mut hot);
        assert!(hot.clipped(), "a 1.2 amplitude sine must report clipping");
    }

    #[test]
    fn the_peak_hold_keeps_the_loudest_output_across_meter_reads() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        assert_eq!(pipe.peak_hold_dbfs(), -100.0);
        for n in 0..50 {
            pipe.push(&sine_packet(i64::from(n) * PACKET_100NS, 48_000, 2, 440.0, 0.5, core::f64::consts::FRAC_PI_2));
        }
        drain_all(&mut pipe);
        assert!(pipe.peak_since_last_read() > 0.0);
        // A later quiet stretch and a meter read do not lower the hold.
        pipe.fill_silence_to(pipe.produced_frames() + 4_800);
        drain_all(&mut pipe);
        assert_eq!(pipe.peak_since_last_read(), 0.0);
        let hold = pipe.peak_hold_dbfs();
        // A 0.5 amplitude sine is -6 dBFS; the resampler's interpolation overshoots it a little.
        assert!((-7.0..=-4.5).contains(&hold), "peak hold was {hold} dBFS");
    }

    #[test]
    fn five_second_pause_adds_no_frames_and_no_gap() {
        let mut pipe = TrackPipeline::new(0, OUT_RATE);
        let mut qpc = 0i64;
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let before = pipe.produced_frames();
        pipe.pause(qpc);
        // Five wall seconds pass; packets arriving during the pause are discarded.
        for _ in 0..500 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        assert_eq!(pipe.produced_frames(), before);
        pipe.resume(qpc);
        for _ in 0..100 {
            pipe.push(&sine_packet(qpc, 48_000, 2, 440.0, 0.5, 0.0));
            qpc += PACKET_100NS;
        }
        let diff = (i128::from(pipe.produced_frames()) - 2 * i128::from(OUT_RATE)).abs();
        assert!(diff <= 48, "total length off by {diff} frames");
        let out = drain_all(&mut pipe);
        assert!(out[48_000..48_060].iter().any(|v| v.abs() > 0.1), "resume content did not continue at the timeline position");
    }

    /// The first frame index at or after from whose sample magnitude exceeds threshold.
    fn first_loud(samples: &[f32], from: usize, threshold: f32) -> usize {
        let frames = samples.len() / 2;
        (from..frames).find(|&i| samples[2 * i].abs() > threshold || samples[2 * i + 1].abs() > threshold).unwrap_or(usize::MAX)
    }
}
