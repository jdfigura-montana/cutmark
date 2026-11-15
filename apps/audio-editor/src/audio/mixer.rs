//! audio::mixer: render(plan, start, frames) producing stereo f32; shared with export.

use crate::audio::mix_plan::{ClipPlan, MixPlan};
use crate::util::fade::fade_gain;

/// Local per-track meter accumulators for one render call.
#[derive(Default)]
struct TrackAccum {
    /// Largest absolute left sample.
    peak_l: f32,
    /// Largest absolute right sample.
    peak_r: f32,
    /// Sum of squared left samples.
    sq_l: f32,
    /// Sum of squared right samples.
    sq_r: f32,
    /// Frames rendered.
    frames: u32,
    /// True when any accumulated sample reached 0 dBFS.
    clipped: bool,
}

/// Renders `out.len() / 2` stereo interleaved frames starting at timeline position `start`.
/// When `meter` is true, writes per-track and master peak / RMS values to the plan's meters.
/// No allocation, no locks, no panics: safe for the audio thread.
pub fn render(plan: &MixPlan, start: u64, out: &mut [f32], meter: bool) {
    for s in out.iter_mut() {
        *s = 0.0;
    }
    if out.len() < 2 {
        return;
    }
    let frames = out.len() / 2;
    // Saturating: a playhead near u64::MAX must not wrap (or panic in debug).
    let block_end = start.saturating_add(frames as u64);
    for (index, track) in plan.tracks.iter().enumerate() {
        if !track.audible {
            continue;
        }
        // Clips on one track never overlap, so one accumulator spans all clips.
        let mut acc = TrackAccum::default();
        // First clip whose end is past `start`; clips are sorted by start.
        let first = track
            .clips
            .partition_point(|c| c.start.saturating_add(c.len) <= start);
        for clip in &track.clips[first..] {
            if clip.start >= block_end {
                break;
            }
            mix_clip(
                clip,
                start,
                track.gain_l,
                track.gain_r,
                out,
                meter,
                &mut acc,
            );
        }
        if meter
            && acc.frames > 0
            && let Some(m) = plan.meters.tracks.get(index)
        {
            m.l.add(acc.peak_l, acc.sq_l, acc.frames, acc.clipped);
            m.r.add(acc.peak_r, acc.sq_r, acc.frames, acc.clipped);
        }
    }
    let master = plan.master_gain;
    for s in out.iter_mut() {
        *s *= master;
    }
    if meter {
        let mut acc = TrackAccum::default();
        for [l, r] in out.as_chunks::<2>().0 {
            accumulate(&mut acc, *l, *r);
        }
        plan.master
            .l
            .add(acc.peak_l, acc.sq_l, frames as u32, acc.clipped);
        plan.master
            .r
            .add(acc.peak_r, acc.sq_r, frames as u32, acc.clipped);
    }
}

/// Adds one clip's contribution to the output block, reading safely past the buffer end as 0.0.
fn mix_clip(
    clip: &ClipPlan,
    start: u64,
    gain_l: f32,
    gain_r: f32,
    out: &mut [f32],
    meter: bool,
    acc: &mut TrackAccum,
) {
    let samples: Option<&[f32]> = clip.samples.as_deref();
    let clip_end = clip.start.saturating_add(clip.len);
    let block_end = start.saturating_add((out.len() / 2) as u64);
    let last = clip_end.min(block_end);
    if clip_end <= start || last <= clip.start {
        return;
    }
    // First frame in this block that falls inside the clip. Past the guard
    // above `last` is beyond both `start` and `clip.start`, so every
    // subtraction below stays non-negative; saturating keeps it panic-free
    // regardless, and `first_t + count <= frames` holds, so `out[o]` is in range.
    let first_abs = clip.start.max(start);
    let first_t = first_abs.saturating_sub(start);
    let count = last.saturating_sub(first_abs) as usize;
    // Local offset from clip.start for the first frame of the clip inside this block.
    let base_local = first_abs.saturating_sub(clip.start);
    for i in 0..count {
        let t = first_t.saturating_add(i as u64); // block-local frame index
        let local = base_local.saturating_add(i as u64); // offset from clip.start
        let env = clip.gain * fade_gain(local, clip.len, clip.fade_in, clip.fade_out);
        // A saturated index reads past the buffer end, which is silence.
        let src = clip.src_in.saturating_add(local);
        let (l, r) = match clip.channels {
            2 => {
                let l = read_sample(samples, src.saturating_mul(2)) * env;
                let r = read_sample(samples, src.saturating_mul(2).saturating_add(1)) * env;
                (l, r)
            }
            _ => {
                let s = read_sample(samples, src) * env;
                (s, s)
            }
        };
        let o = (t as usize) * 2;
        out[o] += l * gain_l;
        out[o + 1] += r * gain_r;
        if meter {
            accumulate(acc, out[o], out[o + 1]);
        }
    }
}

/// Folds one post-track-gain frame into the local meter accumulators.
#[inline]
fn accumulate(acc: &mut TrackAccum, l: f32, r: f32) {
    let (al, ar) = (l.abs(), r.abs());
    if al > acc.peak_l {
        acc.peak_l = al;
    }
    if ar > acc.peak_r {
        acc.peak_r = ar;
    }
    acc.sq_l += l * l;
    acc.sq_r += r * r;
    acc.frames += 1;
    if al >= 1.0 || ar >= 1.0 {
        acc.clipped = true;
    }
}

/// Reads one sample, returning 0.0 for any index past the buffer end.
#[inline]
fn read_sample(samples: Option<&[f32]>, index: u64) -> f32 {
    let idx = index as usize;
    match samples.and_then(|s| s.get(idx)) {
        Some(v) => *v,
        None => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use crate::audio::meters::{MeterBank, StereoMeter};
    use crate::audio::mix_plan::{ClipPlan, MixPlan, TrackPlan};
    use crate::util::db::db_to_linear;

    const PAN_CENTER: f32 = 0.707_106_77;

    /// A plan with one audible track and one clip; gains are applied as given.
    fn one_clip_plan(clip: ClipPlan, gain_l: f32, gain_r: f32, master_gain: f32) -> MixPlan {
        MixPlan {
            rate: 48_000,
            length: clip.start.saturating_add(clip.len),
            master_gain,
            tracks: vec![TrackPlan {
                audible: true,
                gain_l,
                gain_r,
                clips: vec![clip],
            }],
            meters: bank(1),
            master: Arc::new(StereoMeter::new()),
        }
    }

    /// A meter bank with `n` anonymous track meters.
    fn bank(n: usize) -> Arc<MeterBank> {
        Arc::new(MeterBank {
            track_ids: (0..n).map(|_| Uuid::new_v4()).collect(),
            tracks: (0..n).map(|_| StereoMeter::new()).collect(),
        })
    }

    fn mono_clip(
        start: u64,
        len: u64,
        src_in: u64,
        gain: f32,
        samples: Option<Arc<[f32]>>,
    ) -> ClipPlan {
        ClipPlan {
            start,
            len,
            src_in,
            channels: 1,
            gain,
            fade_in: 0,
            fade_out: 0,
            samples,
        }
    }

    fn const_samples(value: f32, frames: usize, channels: u16) -> Arc<[f32]> {
        Arc::from(vec![value; frames * channels as usize].into_boxed_slice())
    }

    fn render_all(plan: &MixPlan, from: u64, frames: usize) -> Vec<f32> {
        let mut out = vec![f32::NAN; frames * 2];
        crate::audio::mixer::render(plan, from, &mut out, true);
        out
    }

    #[test]
    fn mono_clip_at_start_100_plays_only_between_100_and_200() {
        let plan = one_clip_plan(
            mono_clip(100, 100, 0, 1.0, Some(const_samples(0.5, 100, 1))),
            PAN_CENTER,
            PAN_CENTER,
            1.0,
        );
        let out = render_all(&plan, 0, 300);
        for f in 0..300 {
            let (l, r) = (out[f * 2], out[f * 2 + 1]);
            if (100..200).contains(&f) {
                assert!((l - 0.5 * PAN_CENTER).abs() < 1e-5, "frame {f} L {l}");
                assert!((r - 0.5 * PAN_CENTER).abs() < 1e-5, "frame {f} R {r}");
            } else {
                assert_eq!(l, 0.0, "frame {f} L");
                assert_eq!(r, 0.0, "frame {f} R");
            }
        }
    }

    #[test]
    fn clip_gain_plus_six_db_doubles_the_output() {
        let gain = db_to_linear(6.0206);
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, gain, Some(const_samples(0.5, 100, 1))),
            1.0,
            1.0,
            1.0,
        );
        let out = render_all(&plan, 0, 100);
        assert!((out[0] - 1.0).abs() < 1e-3, "{}", out[0]);
        assert!((out[1] - 1.0).abs() < 1e-3, "{}", out[1]);
    }

    #[test]
    fn pan_hard_left_sends_nothing_right() {
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1))),
            1.0,
            0.0,
            1.0,
        );
        let out = render_all(&plan, 0, 100);
        assert!((out[0] - 0.5).abs() < 1e-5);
        assert_eq!(out[1], 0.0);
    }

    #[test]
    fn fade_in_is_linear_from_zero() {
        let mut clip = mono_clip(100, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        clip.fade_in = 100;
        let plan = one_clip_plan(clip, 1.0, 1.0, 1.0);
        let out = render_all(&plan, 0, 200);
        // Frame 100 is local 0: fully faded out.
        assert_eq!(out[200], 0.0);
        assert_eq!(out[201], 0.0);
        // Frame 150 is local 50: exactly half the unfaded level.
        assert!((out[300] - 0.25).abs() < 1e-6, "{}", out[300]);
        assert!((out[301] - 0.25).abs() < 1e-6, "{}", out[301]);
    }

    #[test]
    fn fade_out_tapers_to_zero_at_the_clip_end() {
        let mut clip = mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        clip.fade_out = 100;
        let plan = one_clip_plan(clip, 1.0, 1.0, 1.0);
        let out = render_all(&plan, 0, 100);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!((out[100] - 0.25).abs() < 1e-5, "frame 50");
        // Frame 99: (len - local) / fade_out = 1/100 -> half the sample value.
        assert!((out[198] - 0.005).abs() < 1e-6, "frame 99 was {}", out[198]);
    }

    #[test]
    fn overlapping_tracks_sum() {
        let clip = || mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        let plan = MixPlan {
            rate: 48_000,
            length: 100,
            master_gain: 1.0,
            tracks: vec![
                TrackPlan {
                    audible: true,
                    gain_l: 1.0,
                    gain_r: 1.0,
                    clips: vec![clip()],
                },
                TrackPlan {
                    audible: true,
                    gain_l: 1.0,
                    gain_r: 1.0,
                    clips: vec![clip()],
                },
            ],
            meters: bank(2),
            master: Arc::new(StereoMeter::new()),
        };
        let out = render_all(&plan, 0, 100);
        assert!((out[0] - 1.0).abs() < 1e-5, "{}", out[0]);
        assert!((out[1] - 1.0).abs() < 1e-5);
    }
    #[test]
    fn muted_track_is_silent() {
        let clip = mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        let plan = MixPlan {
            rate: 48_000,
            length: 100,
            master_gain: 1.0,
            tracks: vec![TrackPlan {
                audible: false,
                gain_l: 1.0,
                gain_r: 1.0,
                clips: vec![clip],
            }],
            meters: bank(1),
            master: Arc::new(StereoMeter::new()),
        };
        let out = render_all(&plan, 0, 100);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn solo_silences_other_tracks() {
        let samples = || const_samples(0.5, 100, 1);
        let plan = MixPlan {
            rate: 48_000,
            length: 100,
            master_gain: 1.0,
            tracks: vec![
                TrackPlan {
                    audible: false,
                    gain_l: 1.0,
                    gain_r: 1.0,
                    clips: vec![mono_clip(0, 100, 0, 1.0, Some(samples()))],
                },
                TrackPlan {
                    audible: true,
                    gain_l: 1.0,
                    gain_r: 1.0,
                    clips: vec![mono_clip(0, 100, 0, 1.0, Some(samples()))],
                },
            ],
            meters: bank(2),
            master: Arc::new(StereoMeter::new()),
        };
        let out = render_all(&plan, 0, 100);
        assert!(
            (out[0] - 0.5).abs() < 1e-5,
            "only the soloed track is heard"
        );
    }

    #[test]
    fn master_gain_scales_the_whole_mix() {
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1))),
            1.0,
            1.0,
            0.5,
        );
        let out = render_all(&plan, 0, 100);
        assert!((out[0] - 0.25).abs() < 1e-5);
        assert!((out[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn rendering_in_blocks_matches_one_big_call() {
        let mut clip = mono_clip(
            50,
            200,
            10,
            db_to_linear(-3.0),
            Some(const_samples(0.4, 256, 2)),
        );
        clip.fade_in = 30;
        clip.fade_out = 60;
        let plan = one_clip_plan(clip, PAN_CENTER, PAN_CENTER, 0.8);
        let whole = render_all(&plan, 0, 300);
        let mut pieced = vec![0.0f32; 600];
        crate::audio::mixer::render(&plan, 0, &mut pieced[0..154], true);
        crate::audio::mixer::render(&plan, 77, &mut pieced[154..320], true);
        crate::audio::mixer::render(&plan, 160, &mut pieced[320..600], true);
        for (a, b) in whole.iter().zip(pieced.iter()) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }
    }

    #[test]
    fn src_past_the_buffer_reads_zero_without_panicking() {
        // Clip is 100 frames long but the buffer holds only 50 mono frames.
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 50, 1))),
            1.0,
            1.0,
            1.0,
        );
        let out = render_all(&plan, 0, 100);
        for f in 0..50 {
            assert!((out[f * 2] - 0.5).abs() < 1e-6, "frame {f}");
        }
        for f in 50..100 {
            assert_eq!(out[f * 2], 0.0, "frame {f} should read silence");
            assert_eq!(out[f * 2 + 1], 0.0);
        }
    }

    #[test]
    fn missing_media_plays_silence() {
        let plan = one_clip_plan(mono_clip(0, 100, 0, 1.0, None), 1.0, 1.0, 1.0);
        let out = render_all(&plan, 0, 100);
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn stereo_clip_reads_l_and_r_separately() {
        let samples: Arc<[f32]> = Arc::from([0.1f32, 0.9].repeat(50).into_boxed_slice());
        let plan = one_clip_plan(
            ClipPlan {
                start: 0,
                len: 50,
                src_in: 0,
                channels: 2,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
                samples: Some(samples),
            },
            1.0,
            1.0,
            1.0,
        );
        let out = render_all(&plan, 0, 50);
        assert!((out[0] - 0.1).abs() < 1e-6);
        assert!((out[1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn meters_measure_track_and_master_after_gains() {
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1))),
            PAN_CENTER,
            PAN_CENTER,
            0.5,
        );
        let expected = 0.5 * PAN_CENTER;
        let out = render_all(&plan, 0, 100);
        let _ = out;
        for m in [&plan.meters.tracks[0].l, &plan.meters.tracks[0].r] {
            let (peak, sum_sq, frames, clipped) = m.take();
            assert!((peak - expected).abs() < 1e-5, "track peak {peak}");
            assert!((sum_sq - 100.0 * expected * expected).abs() < 1e-3);
            assert_eq!(frames, 100);
            assert!(!clipped);
        }
        for m in [&plan.master.l, &plan.master.r] {
            let (peak, _sq, frames, clipped) = m.take();
            assert!((peak - expected * 0.5).abs() < 1e-5, "master peak {peak}");
            assert_eq!(frames, 100);
            assert!(!clipped);
        }
    }

    #[test]
    fn inaudible_track_meter_stays_zero() {
        let plan = MixPlan {
            rate: 48_000,
            length: 100,
            master_gain: 1.0,
            tracks: vec![TrackPlan {
                audible: false,
                gain_l: 1.0,
                gain_r: 1.0,
                clips: vec![mono_clip(0, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)))],
            }],
            meters: bank(1),
            master: Arc::new(StereoMeter::new()),
        };
        let _ = render_all(&plan, 0, 100);
        let (peak, sq, frames, clipped) = plan.meters.tracks[0].l.take();
        assert_eq!(peak, 0.0);
        assert_eq!(sq, 0.0);
        assert_eq!(frames, 0);
        assert!(!clipped);
    }

    #[test]
    fn a_sample_at_unity_sets_clipped() {
        let plan = one_clip_plan(
            mono_clip(0, 100, 0, 1.0, Some(const_samples(1.0, 100, 1))),
            1.0,
            1.0,
            1.0,
        );
        let _ = render_all(&plan, 0, 100);
        assert!(plan.meters.tracks[0].l.take().3);
        assert!(plan.master.r.take().3);
    }

    /// Extreme positions from a hostile project file must neither wrap nor
    /// panic (the tests run with overflow checks on).
    #[test]
    fn extreme_positions_saturate_without_panicking() {
        // Playhead at the very end of the u64 range, clip covering all of it.
        let huge = mono_clip(0, u64::MAX, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        let plan = one_clip_plan(huge, 1.0, 1.0, 1.0);
        let out = render_all(&plan, u64::MAX - 5, 64);
        assert!(out.iter().all(|s| s.is_finite()));

        // Source offset so large that `src_in + local` and `src * 2` would overflow.
        let stereo = ClipPlan {
            channels: 2,
            ..mono_clip(10, 100, u64::MAX - 3, 1.0, Some(const_samples(0.5, 100, 2)))
        };
        let plan = one_clip_plan(stereo, 1.0, 1.0, 1.0);
        let out = render_all(&plan, 0, 200);
        // Every read lands past the buffer end: silence, not a wrapped read.
        assert!(out.iter().all(|s| *s == 0.0));

        // A clip starting near the end of the range.
        let late = mono_clip(u64::MAX - 10, 100, 0, 1.0, Some(const_samples(0.5, 100, 1)));
        let plan = one_clip_plan(late, 1.0, 1.0, 1.0);
        let out = render_all(&plan, u64::MAX - 20, 64);
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
