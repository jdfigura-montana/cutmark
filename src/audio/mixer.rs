//! audio::mixer: render(plan, start, frames) producing stereo f32; shared with export.

use crate::audio::mix_plan::{ClipPlan, MixPlan};

/// Renders `out.len() / 2` stereo interleaved frames starting at timeline position `start`.
/// No allocation, no locks, no panics: safe for the audio thread.
pub fn render(plan: &MixPlan, start: u64, out: &mut [f32]) {
    for s in out.iter_mut() {
        *s = 0.0;
    }
    if out.len() < 2 {
        return;
    }
    let frames = out.len() / 2;
    let block_end = start + frames as u64;
    for track in &plan.tracks {
        if !track.audible {
            continue;
        }
        // First clip whose end is past `start`; clips are sorted by start.
        let first = track
            .clips
            .partition_point(|c| c.start.saturating_add(c.len) <= start);
        for clip in &track.clips[first..] {
            if clip.start >= block_end {
                break;
            }
            mix_clip(clip, start, track.gain_l, track.gain_r, out);
        }
    }
    let master = plan.master_gain;
    for s in out.iter_mut() {
        *s *= master;
    }
}

/// Adds one clip's contribution to the output block, reading safely past the buffer end as 0.0.
fn mix_clip(clip: &ClipPlan, start: u64, gain_l: f32, gain_r: f32, out: &mut [f32]) {
    let samples: Option<&[f32]> = clip.samples.as_deref();
    let clip_end = clip.start.saturating_add(clip.len);
    let last = clip_end.min(start + (out.len() / 2) as u64);
    if clip_end <= start || last <= clip.start {
        return;
    }
    // First frame in this block that falls inside the clip.
    let first_t = clip.start.max(start) - start;
    let count = (last - clip.start.max(start)) as usize;
    // Local offset from clip.start for the first frame of the clip inside this block.
    let base_local = clip.start.max(start) - clip.start;
    for i in 0..count {
        let t = first_t + i as u64; // block-local frame index
        let local = base_local + i as u64; // offset from clip.start
        let env = clip.gain * fade_factor(clip, local);
        let src = clip.src_in + local;
        let (l, r) = match clip.channels {
            2 => {
                let l = read_sample(samples, src * 2) * env;
                let r = read_sample(samples, src * 2 + 1) * env;
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
    }
}

/// Linear fade factor at `local` samples from the clip start, 1.0 outside fades.
fn fade_factor(clip: &ClipPlan, local: u64) -> f32 {
    let mut f = 1.0f32;
    if clip.fade_in > 0 && local < clip.fade_in {
        f = f.min(local as f32 / clip.fade_in as f32);
    }
    if clip.fade_out > 0 {
        let from_end = clip.len.saturating_sub(local);
        if from_end < clip.fade_out {
            f = f.min(from_end as f32 / clip.fade_out as f32);
        }
    }
    f
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

    use crate::audio::mix_plan::{ClipPlan, MixPlan, TrackPlan};
    use crate::util::db::db_to_linear;

    const PAN_CENTER: f32 = 0.707_106_77;

    /// A plan with one audible track and one clip; gains are applied as given.
    fn one_clip_plan(clip: ClipPlan, gain_l: f32, gain_r: f32, master_gain: f32) -> MixPlan {
        MixPlan {
            rate: 48_000,
            length: clip.start + clip.len,
            master_gain,
            tracks: vec![TrackPlan {
                audible: true,
                gain_l,
                gain_r,
                clips: vec![clip],
            }],
        }
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
        crate::audio::mixer::render(plan, from, &mut out);
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
        crate::audio::mixer::render(&plan, 0, &mut pieced[0..154]);
        crate::audio::mixer::render(&plan, 77, &mut pieced[154..320]);
        crate::audio::mixer::render(&plan, 160, &mut pieced[320..600]);
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
}
