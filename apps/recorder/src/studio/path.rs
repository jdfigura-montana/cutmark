//! The zoom view and the smoothed cursor over a whole take, computed ahead at [`STEPS_PER_SECOND`]
//! so that a lookup at any time is instant (spec "Rendering", 4).
//!
//! A segment eases in over its first `speed` seconds and out over its last, or over half of itself
//! each when it is shorter than twice that. The view moves in a straight line between the whole
//! frame and the zoomed rectangle, so it is inside the frame whenever both are. While easing, the
//! zoomed rectangle holds still, which makes the ease a plain zoom to and from one spot. In between
//! it follows the cursor: nothing moves while the cursor stays in the dead zone, and past its edge
//! the view glides after it.

use super::autozoom::ZoomSegment;
use super::settings::Smoothing;

/// The table's rate.
pub const STEPS_PER_SECOND: i64 = 120;

/// The dead zone's share of the zoomed view, each way.
const DEAD_ZONE: f32 = 0.4;
/// The time constant of the view's glide after the cursor, in seconds.
const FOLLOW_TAU_S: f32 = 0.25;
/// Steps on each side that Light smoothing averages over: 50 ms in all.
const LIGHT_HALF_STEPS: usize = 3;
/// Steps on each side that Strong smoothing averages over: 200 ms in all.
const STRONG_HALF_STEPS: usize = 12;

type Point = (f32, f32);

/// The cursor at one moment of the input track.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorPoint {
    pub time_us: i64,
    /// In source pixels; None while Windows hid the cursor or it was outside the captured frame.
    pub position: Option<Point>,
}

/// The track's cursor samples in source pixels (see `to_source` in [`super`]).
pub fn cursor_points(track: &events::EventsReader, to_source: impl Fn(i64, i32, i32) -> Option<Point>) -> Vec<CursorPoint> {
    track
        .samples()
        .iter()
        .map(|sample| {
            let position = sample.event.visible.then(|| to_source(sample.time_us, sample.event.x, sample.event.y)).flatten();
            CursorPoint { time_us: sample.time_us, position }
        })
        .collect()
}

/// The part of the source frame that fills the output, in source pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewRect {
    /// The part of the way from `self` to `to` that `weight`, 0 to 1, says.
    fn toward(self, to: Self, weight: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * weight;
        Self { x: mix(self.x, to.x), y: mix(self.y, to.y), width: mix(self.width, to.width), height: mix(self.height, to.height) }
    }
}

/// The precomputed table of one take. The cursor part depends only on the input track; the views are
/// redone with [`StudioPath::set_zoom`] after a lane or Speed change.
pub struct StudioPath {
    /// The source frame's width and height in pixels.
    size: (f32, f32),
    /// The cursor at each step, as recorded and with each smoothing.
    raw: Vec<Option<Point>>,
    light: Vec<Option<Point>>,
    strong: Vec<Option<Point>>,
    views: Vec<ViewRect>,
}

/// The time of a step.
fn step_time(step: usize) -> i64 {
    step as i64 * 1_000_000 / STEPS_PER_SECOND
}

/// The last step at or before a time that is not negative.
fn step_at(time_us: i64) -> usize {
    // One more than the plain division when the rounded-down time of the next step is reached.
    (((time_us + 1) * STEPS_PER_SECOND - 1) / 1_000_000) as usize
}

impl StudioPath {
    /// Builds the table for a take of `duration_us` with a source frame of `size` pixels. `cursor`
    /// must be in time order; `speed_s` is the panel's Speed.
    pub fn new(size: (u32, u32), duration_us: i64, cursor: &[CursorPoint], segments: &[ZoomSegment], speed_s: f32) -> Self {
        // Steps run from 0 to the first one at or past the end.
        let end = duration_us.max(0);
        let steps = step_at(end) + 1 + usize::from(step_time(step_at(end)) < end);
        let mut next = 0;
        let raw: Vec<Option<Point>> = (0..steps)
            .map(|step| {
                while cursor.get(next).is_some_and(|point| point.time_us <= step_time(step)) {
                    next += 1;
                }
                next.checked_sub(1).and_then(|latest| cursor[latest].position)
            })
            .collect();
        let size = (size.0 as f32, size.1 as f32);
        let light = smoothed(&raw, LIGHT_HALF_STEPS);
        let strong = smoothed(&raw, STRONG_HALF_STEPS);
        let mut path = Self { size, raw, light, strong, views: Vec::new() };
        path.set_zoom(segments, speed_s);
        path
    }

    /// The whole source frame.
    fn full(&self) -> ViewRect {
        ViewRect { x: 0.0, y: 0.0, width: self.size.0, height: self.size.1 }
    }

    /// Redoes the views. Segments are not meant to overlap; where they do, the later one in the
    /// list wins.
    pub fn set_zoom(&mut self, segments: &[ZoomSegment], speed_s: f32) {
        let mut views = vec![self.full(); self.raw.len()];
        for segment in segments {
            self.zoom_segment(segment, speed_s, &mut views);
        }
        self.views = views;
    }

    fn zoom_segment(&self, segment: &ZoomSegment, speed_s: f32, views: &mut [ViewRect]) {
        let length = segment.end_us - segment.start_us;
        if length <= 0 || segment.end_us < 0 {
            return;
        }
        let ramp = ((f64::from(speed_s) * 1e6) as i64).min(length / 2).max(1);
        let (follow_from, follow_to) = (segment.start_us + ramp, segment.end_us - ramp);
        let level = segment.level.max(1.0);
        let (width, height) = (self.size.0 / level, self.size.1 / level);
        // A view centered here stays inside the frame.
        let inside = |(x, y): Point| (x.clamp(width / 2.0, self.size.0 - width / 2.0), y.clamp(height / 2.0, self.size.1 - height / 2.0));
        let dead = (width * DEAD_ZONE / 2.0, height * DEAD_ZONE / 2.0);
        let glide = 1.0 - (-1.0 / (STEPS_PER_SECOND as f32 * FOLLOW_TAU_S)).exp();

        let fixed = segment.fixed_center.filter(|_| !segment.follow);
        let anchor = fixed.or_else(|| self.cursor_near(follow_from)).unwrap_or((self.size.0 / 2.0, self.size.1 / 2.0));
        let mut goal = inside(anchor);
        let mut center = goal;

        let first = step_at(segment.start_us.max(0));
        for (step, view) in views.iter_mut().enumerate().skip(first) {
            let time = step_time(step);
            if time > segment.end_us {
                break;
            }
            if time < segment.start_us {
                continue;
            }
            if segment.follow && time > follow_from && time <= follow_to {
                if let Some(cursor) = self.raw[step] {
                    // The goal moves only as far as keeps the cursor on the dead zone's edge.
                    goal = inside((goal.0.clamp(cursor.0 - dead.0, cursor.0 + dead.0), goal.1.clamp(cursor.1 - dead.1, cursor.1 + dead.1)));
                }
                center = (center.0 + (goal.0 - center.0) * glide, center.1 + (goal.1 - center.1) * glide);
            }
            let edge = (time - segment.start_us).min(segment.end_us - time);
            let weight = smoothstep((edge as f32 / ramp as f32).clamp(0.0, 1.0));
            let zoomed = ViewRect { x: center.0 - width / 2.0, y: center.1 - height / 2.0, width, height };
            *view = self.full().toward(zoomed, weight);
        }
    }

    /// The cursor at the step of a time, or the last one known before it, or the first after it.
    fn cursor_near(&self, time_us: i64) -> Option<Point> {
        let step = step_at(time_us.max(0)).min(self.raw.len().saturating_sub(1));
        let before = self.raw.get(..=step)?.iter().rev().find_map(|position| *position);
        before.or_else(|| self.raw[step..].iter().find_map(|position| *position))
    }

    /// The two steps around a time, and how far it is from the first to the second, 0 to 1.
    fn around(&self, time_us: i64) -> (usize, usize, f32) {
        let last = self.raw.len() - 1;
        let time = time_us.max(0);
        let step = step_at(time);
        if step >= last {
            return (last, last, 0.0);
        }
        let (from, to) = (step_time(step), step_time(step + 1));
        (step, step + 1, (time - from) as f32 / (to - from) as f32)
    }

    /// The zoom view at a time; times outside the take get its first or last view.
    pub fn view_at(&self, time_us: i64) -> ViewRect {
        let (from, to, part) = self.around(time_us);
        let view = self.views[from].toward(self.views[to], part);
        // Rounding must not put an edge outside the frame.
        let (width, height) = (view.width.min(self.size.0), view.height.min(self.size.1));
        ViewRect { x: view.x.clamp(0.0, self.size.0 - width), y: view.y.clamp(0.0, self.size.1 - height), width, height }
    }

    /// Where to draw the cursor at a time, in source pixels; None while it is not there to draw.
    pub fn cursor_at(&self, time_us: i64, smoothing: Smoothing) -> Option<Point> {
        let table = match smoothing {
            Smoothing::Off => &self.raw,
            Smoothing::Light => &self.light,
            Smoothing::Strong => &self.strong,
        };
        let (from, to, part) = self.around(time_us);
        match (table[from], table[to]) {
            (Some(a), Some(b)) => Some((a.0 + (b.0 - a.0) * part, a.1 + (b.1 - a.1) * part)),
            (a, b) => {
                if part < 0.5 {
                    a
                } else {
                    b
                }
            }
        }
    }
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// A centred moving average of up to `half` steps on each side. The window stays symmetric, so it
/// shrinks near the ends of the take and of each visible run instead of pulling the cursor to one
/// side, and it never reaches across a gap.
fn smoothed(raw: &[Option<Point>], half: usize) -> Vec<Option<Point>> {
    (0..raw.len())
        .map(|step| {
            let (x, y) = raw[step]?;
            let (mut sum_x, mut sum_y, mut count) = (x, y, 1.0_f32);
            for offset in 1..=half {
                let before = step.checked_sub(offset).and_then(|at| raw[at]);
                let after = raw.get(step + offset).copied().flatten();
                let (Some(before), Some(after)) = (before, after) else { break };
                sum_x += before.0 + after.0;
                sum_y += before.1 + after.1;
                count += 2.0;
            }
            Some((sum_x / count, sum_y / count))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000;
    const SIZE: (u32, u32) = (1920, 1080);
    const FULL: ViewRect = ViewRect { x: 0.0, y: 0.0, width: 1920.0, height: 1080.0 };

    fn at(time_us: i64, x: f32, y: f32) -> CursorPoint {
        CursorPoint { time_us, position: Some((x, y)) }
    }

    fn segment(start_us: i64, end_us: i64) -> ZoomSegment {
        ZoomSegment { start_us, end_us, ..ZoomSegment::default() }
    }

    fn near(a: ViewRect, b: ViewRect) -> bool {
        [a.x - b.x, a.y - b.y, a.width - b.width, a.height - b.height].iter().all(|difference| difference.abs() < 0.05)
    }

    #[test]
    fn steps_and_times_agree() {
        for step in 0..1_000 {
            assert_eq!(step_at(step_time(step)), step);
            assert_eq!(step_at(step_time(step + 1) - 1), step);
        }
    }

    #[test]
    fn a_segment_eases_from_the_whole_frame_to_its_level_and_back() {
        let path = StudioPath::new(SIZE, 10 * S, &[at(0, 960.0, 540.0)], &[segment(S, 5 * S)], 0.6);
        let zoomed = ViewRect { x: 480.0, y: 270.0, width: 960.0, height: 540.0 };
        for time in [0, S, 5 * S, 10 * S, -S, 99 * S] {
            assert!(near(path.view_at(time), FULL), "{time}");
        }
        for time in [S + 600_000, 3 * S, 5 * S - 600_000] {
            assert!(near(path.view_at(time), zoomed), "{time}");
        }
        // Halfway through an ease the view is halfway there.
        assert!(near(path.view_at(S + 300_000), FULL.toward(zoomed, 0.5)));
        assert!(near(path.view_at(5 * S - 300_000), FULL.toward(zoomed, 0.5)));
        // The ease starts and ends gently.
        assert!(FULL.width - path.view_at(S + 60_000).width < 0.1 * (FULL.width - zoomed.width));
    }

    #[test]
    fn a_short_segment_eases_over_half_of_itself() {
        let path = StudioPath::new(SIZE, 5 * S, &[at(0, 960.0, 540.0)], &[segment(S, 2 * S)], 1.2);
        assert!((path.view_at(S + 500_000).width - 960.0).abs() < 0.05);
    }

    #[test]
    fn the_view_never_leaves_the_frame() {
        // The cursor jumps around the corners and past them.
        let corners = [(-50.0, -50.0), (1919.0, 0.0), (2500.0, 1500.0), (0.0, 1079.0), (960.0, 540.0)];
        let cursor: Vec<CursorPoint> = (0..40).map(|index| at(index * S / 4, corners[index as usize % 5].0, corners[index as usize % 5].1)).collect();
        let outside = ZoomSegment { follow: false, fixed_center: Some((-900.0, 9_000.0)), level: 4.0, ..segment(7 * S, 9 * S) };
        let path = StudioPath::new(SIZE, 10 * S, &cursor, &[ZoomSegment { level: 3.0, ..segment(0, 6 * S) }, outside], 0.3);
        for time in (0..=10 * S).step_by(3_331) {
            let view = path.view_at(time);
            assert!(view.x >= 0.0 && view.y >= 0.0 && view.x + view.width <= 1920.001 && view.y + view.height <= 1080.001, "{time}: {view:?}");
        }
    }

    #[test]
    fn the_dead_zone_holds_the_view_still_for_small_moves() {
        // At 2x the view is 960x540, so the dead zone reaches 192 px sideways and 108 px up and down.
        let cursor = [at(0, 960.0, 540.0), at(2 * S, 1140.0, 640.0), at(3 * S, 780.0, 440.0), at(4 * S, 1352.0, 540.0)];
        let path = StudioPath::new(SIZE, 10 * S, &cursor, &[segment(0, 8 * S)], 0.6);
        let still = path.view_at(S);
        for time in (S..4 * S).step_by(50_000) {
            assert!(near(path.view_at(time), still), "{time}");
        }
        // A move past the edge pulls the view just far enough to put the cursor back on it,
        // gradually.
        let early = path.view_at(4 * S + 50_000).x - still.x;
        assert!(early > 0.0 && early < 100.0, "{early}");
        assert!((path.view_at(7 * S).x - (still.x + 200.0)).abs() < 0.5);
        assert!((path.view_at(7 * S).y - still.y).abs() < 0.05);
    }

    #[test]
    fn a_fixed_center_ignores_the_cursor() {
        let cursor: Vec<CursorPoint> = (0..20).map(|index| at(index * S / 2, (index * 97) as f32, (index * 53) as f32)).collect();
        let fixed = ZoomSegment { follow: false, fixed_center: Some((600.0, 400.0)), ..segment(S, 9 * S) };
        let path = StudioPath::new(SIZE, 10 * S, &cursor, &[fixed], 0.6);
        let held = ViewRect { x: 120.0, y: 130.0, width: 960.0, height: 540.0 };
        for time in (S + 600_000..=9 * S - 600_000).step_by(100_000) {
            assert!(near(path.view_at(time), held), "{time}");
        }
        // With follow on, the stored center is not used.
        let following = StudioPath::new(SIZE, 10 * S, &cursor, &[ZoomSegment { follow: true, ..fixed }], 0.6);
        assert!(!near(following.view_at(5 * S), held));
    }

    #[test]
    fn a_new_lane_replaces_the_views() {
        let mut path = StudioPath::new(SIZE, 5 * S, &[at(0, 960.0, 540.0)], &[segment(0, 4 * S)], 0.6);
        assert!(path.view_at(2 * S).width < 1000.0);
        path.set_zoom(&[], 0.6);
        assert!(near(path.view_at(2 * S), FULL));
    }

    #[test]
    fn a_lookup_between_steps_interpolates() {
        // Steps 0 and 1 are at 0 and 8333 us.
        let path = StudioPath::new(SIZE, S, &[at(0, 100.0, 200.0), at(8_000, 200.0, 400.0)], &[segment(0, S)], 0.3);
        let halfway = path.cursor_at(4_166, Smoothing::Off);
        assert!(halfway.is_some_and(|(x, y)| (x - 150.0).abs() < 0.1 && (y - 300.0).abs() < 0.2), "{halfway:?}");
        let (a, b, between) = (path.view_at(step_time(10)), path.view_at(step_time(11)), path.view_at((step_time(10) + step_time(11)) / 2));
        assert!(a.width > between.width && between.width > b.width);
        assert!((between.width - (a.width + b.width) / 2.0).abs() < 0.1);
        // The table reaches the end of a take that ends between steps.
        let short = StudioPath::new(SIZE, 10_000, &[at(9_000, 5.0, 5.0)], &[], 0.6);
        assert_eq!(short.cursor_at(10_000, Smoothing::Off), None);
        assert_eq!(short.cursor_at(step_time(2), Smoothing::Off), Some((5.0, 5.0)));
    }

    #[test]
    fn more_smoothing_spreads_a_jump_wider_without_lag() {
        let path = StudioPath::new(SIZE, 2 * S, &[at(0, 0.0, 0.0), at(S, 1000.0, 0.0)], &[], 0.6);
        let x = |time, smoothing| path.cursor_at(time, smoothing).map_or(f32::NAN, |position| position.0);
        // 50 ms before the jump only Strong has started to move.
        assert_eq!((x(S - 50_000, Smoothing::Off), x(S - 50_000, Smoothing::Light)), (0.0, 0.0));
        assert!(x(S - 50_000, Smoothing::Strong) > 0.0);
        let light = x(S - 16_667, Smoothing::Light);
        assert!(light > 0.0 && light < 500.0, "{light}");
        for smoothing in [Smoothing::Off, Smoothing::Light, Smoothing::Strong] {
            assert_eq!((x(0, smoothing), x(2 * S, smoothing)), (0.0, 1000.0));
        }
    }

    #[test]
    fn smoothing_never_reaches_across_a_hidden_cursor() {
        let cursor = [at(0, 0.0, 0.0), CursorPoint { time_us: S, position: None }, at(S + 20_000, 1000.0, 0.0)];
        let path = StudioPath::new(SIZE, 2 * S, &cursor, &[], 0.6);
        assert_eq!(path.cursor_at(S - 10_000, Smoothing::Strong), Some((0.0, 0.0)));
        assert_eq!(path.cursor_at(S + 10_000, Smoothing::Strong), None);
        assert_eq!(path.cursor_at(S + 30_000, Smoothing::Strong), Some((1000.0, 0.0)));
    }
}
