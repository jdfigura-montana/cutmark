//! Auto-zoom (spec "Zoom", Auto-zoom): activity points from the input track, grouped into the zoom
//! lane's segments.

use serde::{Deserialize, Serialize};

use super::path::{CursorPoint, cursor_points};

/// The lowest zoom level a segment can have.
pub const MIN_LEVEL: f32 = 1.25;
/// The highest zoom level a segment can have.
pub const MAX_LEVEL: f32 = 4.0;

/// How long the cursor has to stay still after moving to count as a dwell.
const DWELL_US: i64 = 600_000;
/// How far, in source pixels, a still cursor may drift from where it came to rest.
const STILL_RADIUS_PX: f32 = 4.0;
/// Points closer together than this share a cluster.
const CLUSTER_GAP_US: i64 = 3_000_000;
/// How long before its first point a segment starts.
const LEAD_US: i64 = 500_000;
/// How long after its last point a segment ends.
const TRAIL_US: i64 = 1_500_000;
/// Segments closer together than this become one.
const MERGE_GAP_US: i64 = 2_000_000;
/// Shorter segments are dropped.
const MIN_LENGTH_US: i64 = 1_000_000;

/// One block of the zoom lane. Times are microseconds from the take's first video frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoomSegment {
    pub start_us: i64,
    pub end_us: i64,
    /// The zoom level, 1.25 to 4.
    pub level: f32,
    /// The view follows the cursor. While true, `fixed_center` is not used.
    pub follow: bool,
    /// Where the view stays when it does not follow, in source pixels. None keeps it where the
    /// cursor is when the segment has eased in.
    pub fixed_center: Option<(f32, f32)>,
    /// The user changed or added this segment, so Regenerate asks before replacing it.
    pub user_edited: bool,
}

impl Default for ZoomSegment {
    fn default() -> Self {
        Self { start_us: 0, end_us: 0, level: 2.0, follow: true, fixed_center: None, user_edited: false }
    }
}

/// What made an activity point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Click,
    Shortcut,
    /// The cursor came to rest; the point's time is when it did.
    Dwell,
}

/// Something the user did that is worth zooming in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityPoint {
    pub time_us: i64,
    pub kind: ActivityKind,
}

/// The activity points of a take, in time order. Clicks and dwells outside the captured frame (see
/// `to_source` in [`super`]) are left out, as is anything outside `0..=duration_us`. A shortcut has
/// no position, so every one counts.
pub fn activity_points(track: &events::EventsReader, duration_us: i64, to_source: impl Fn(i64, i32, i32) -> Option<(f32, f32)>) -> Vec<ActivityPoint> {
    let in_take = |time_us: i64| (0..=duration_us).contains(&time_us);
    let clicks = track.clicks().filter(|click| to_source(click.time_us, click.event.x, click.event.y).is_some());
    let mut points: Vec<ActivityPoint> = clicks.map(|click| ActivityPoint { time_us: click.time_us, kind: ActivityKind::Click }).collect();
    points.extend(track.chords().iter().map(|chord| ActivityPoint { time_us: chord.time_us, kind: ActivityKind::Shortcut }));
    points.extend(dwells(&cursor_points(track, &to_source), duration_us).into_iter().map(|time_us| ActivityPoint { time_us, kind: ActivityKind::Dwell }));
    points.retain(|point| in_take(point.time_us));
    points.sort_by_key(|point| point.time_us);
    points
}

/// When the cursor came to rest: it moved somewhere and then stayed within [`STILL_RADIUS_PX`] for
/// [`DWELL_US`] or more. A cursor that has not moved yet, or has just reappeared, has not dwelt. The
/// samples may come at a steady rate or only on changes.
fn dwells(cursor: &[CursorPoint], duration_us: i64) -> Vec<i64> {
    struct Rest {
        at: (f32, f32),
        since_us: i64,
        after_moving: bool,
    }
    fn close(rest: Option<&Rest>, until_us: i64, found: &mut Vec<i64>) {
        if let Some(rest) = rest
            && rest.after_moving
            && until_us - rest.since_us >= DWELL_US
        {
            found.push(rest.since_us);
        }
    }
    let mut found = Vec::new();
    let mut rest: Option<Rest> = None;
    for point in cursor.iter().take_while(|point| point.time_us <= duration_us) {
        let Some((x, y)) = point.position else {
            close(rest.as_ref(), point.time_us, &mut found);
            rest = None;
            continue;
        };
        let moved = rest.as_ref().is_some_and(|rest| (x - rest.at.0).hypot(y - rest.at.1) > STILL_RADIUS_PX);
        if moved || rest.is_none() {
            close(rest.as_ref(), point.time_us, &mut found);
            rest = Some(Rest { at: (x, y), since_us: point.time_us, after_moving: moved });
        }
    }
    close(rest.as_ref(), duration_us, &mut found);
    found
}

/// The first and last time of each cluster of `times`, which must be sorted.
fn clusters(times: impl IntoIterator<Item = i64>) -> Vec<(i64, i64)> {
    let mut found: Vec<(i64, i64)> = Vec::new();
    for time in times {
        match found.last_mut() {
            Some((_, last)) if time - *last < CLUSTER_GAP_US => *last = time,
            _ => found.push((time, time)),
        }
    }
    found
}

/// Joins the spans, sorted by start, that are less than [`MERGE_GAP_US`] apart.
fn merge(spans: impl IntoIterator<Item = (i64, i64)>) -> Vec<(i64, i64)> {
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some((_, last_end)) if start - *last_end < MERGE_GAP_US => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

/// The zoom segments for activity `points` in time order, at the panel's `amount`.
pub fn segments(points: &[ActivityPoint], amount: f32, duration_us: i64) -> Vec<ZoomSegment> {
    let level = amount.clamp(MIN_LEVEL, MAX_LEVEL);
    let spans =
        clusters(points.iter().map(|point| point.time_us)).into_iter().map(|(first, last)| ((first - LEAD_US).max(0), (last + TRAIL_US).min(duration_us)));
    merge(spans)
        .into_iter()
        .filter(|(start, end)| end - start >= MIN_LENGTH_US)
        .map(|(start_us, end_us)| ZoomSegment { start_us, end_us, level, ..ZoomSegment::default() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000_000;

    fn clicks_at(times: &[i64]) -> Vec<ActivityPoint> {
        times.iter().map(|&time_us| ActivityPoint { time_us, kind: ActivityKind::Click }).collect()
    }

    fn spans(segments: &[ZoomSegment]) -> Vec<(i64, i64)> {
        segments.iter().map(|segment| (segment.start_us, segment.end_us)).collect()
    }

    #[test]
    fn points_less_than_three_seconds_apart_share_a_cluster() {
        assert_eq!(clusters([10 * S, 12 * S, 15 * S - 1, 18 * S - 1, 30 * S]), [(10 * S, 18 * S - 1 - 3 * S), (18 * S - 1, 18 * S - 1), (30 * S, 30 * S)]);
        assert!(clusters([]).is_empty());
    }

    #[test]
    fn a_segment_leads_its_first_point_and_trails_its_last() {
        let made = segments(&clicks_at(&[10 * S, 11 * S, 20 * S]), 2.5, 60 * S);
        assert_eq!(spans(&made), [(10 * S - LEAD_US, 11 * S + TRAIL_US), (20 * S - LEAD_US, 20 * S + TRAIL_US)]);
        assert!(made.iter().all(|segment| segment.level == 2.5 && segment.follow && !segment.user_edited));
    }

    #[test]
    fn spans_less_than_two_seconds_apart_are_merged() {
        assert_eq!(merge([(0, 2 * S), (4 * S - 1, 5 * S), (7 * S, 9 * S), (7 * S + 1, 8 * S)]), [(0, 5 * S), (7 * S, 9 * S)]);
    }

    #[test]
    fn segments_cut_short_by_the_take_are_dropped_below_a_second() {
        // The first starts at 0 and lasts 1.6 s; the last is cut to 0.9 s by the end of the take.
        let made = segments(&clicks_at(&[S / 10, 20 * S]), 2.0, 20 * S + 400_000);
        assert_eq!(spans(&made), [(0, S / 10 + TRAIL_US)]);
    }

    #[test]
    fn the_level_comes_from_the_amount_within_its_range() {
        for (amount, level) in [(1.0, MIN_LEVEL), (3.0, 3.0), (9.0, MAX_LEVEL)] {
            assert_eq!(segments(&clicks_at(&[10 * S]), amount, 60 * S)[0].level, level);
        }
    }

    #[test]
    fn a_cursor_dwells_only_after_moving_and_resting_long_enough() {
        let at = |time_us, x: f32| CursorPoint { time_us, position: Some((x, 0.0)) };
        // Rests from the start (not after moving), moves, pauses too briefly, then comes to rest at
        // 3 s with jitter, and rests again at the end of the take.
        let cursor = [at(0, 0.0), at(2 * S, 100.0), at(2 * S + 500_000, 200.0), at(3 * S, 300.0), at(3 * S + 400_000, 303.0), at(5 * S, 400.0)];
        assert_eq!(dwells(&cursor, 6 * S), [3 * S, 5 * S]);
        // A hidden cursor ends a rest, and the one that reappears has not moved yet.
        let hidden = [at(0, 0.0), at(S, 50.0), CursorPoint { time_us: S + 100_000, position: None }, at(2 * S, 50.0)];
        assert!(dwells(&hidden, 9 * S).is_empty());
    }

    #[test]
    fn the_input_track_gives_clicks_shortcuts_and_dwells_inside_the_frame() -> Result<(), events::EventsError> {
        use events::{
            CaptureSource, Chord, CursorSample, EventsReader, EventsWriter, Header, InputEvent, Modifiers, MouseAction, MouseButton, MouseEvent, ScreenRect,
            Streams,
        };

        let header = Header {
            qpc_frequency: S,
            first_frame_qpc: 0,
            source: CaptureSource::Monitor(ScreenRect::default()),
            displays: Vec::new(),
            cursor_size: 32,
            streams: Streams { cursor: true, clicks: true, keys: true, apps: false },
        };
        let sample = |qpc, x| InputEvent::Sample(CursorSample { qpc, x, y: 10, shape: None, visible: true });
        let mouse = |qpc, x, action| InputEvent::Mouse(MouseEvent { qpc, x, y: 10, action });
        let mut writer = EventsWriter::new(Vec::new(), &header)?;
        for event in [
            sample(0, 10),
            sample(4 * S, 500),
            sample(8 * S, -500),
            mouse(S, 10, MouseAction::Down(MouseButton::Left)),
            mouse(S + 100, 10, MouseAction::Up(MouseButton::Left)),
            mouse(2 * S, -500, MouseAction::Down(MouseButton::Left)),
            mouse(3 * S, 10, MouseAction::Wheel(120)),
            InputEvent::Chord(Chord { qpc: 6 * S, modifiers: Modifiers { ctrl: true, ..Modifiers::default() }, key_name: "S".to_owned() }),
            InputEvent::Chord(Chord { qpc: 30 * S, modifiers: Modifiers::default(), key_name: "F5".to_owned() }),
        ] {
            writer.push(event)?;
        }
        let track = EventsReader::from_bytes(&writer.finish(10 * S)?)?;
        // Negative desktop positions are another monitor.
        let points = activity_points(&track, 10 * S, |_, x, y| (x >= 0).then_some((x as f32, y as f32)));
        let found: Vec<(i64, ActivityKind)> = points.iter().map(|point| (point.time_us, point.kind)).collect();
        assert_eq!(found, [(S, ActivityKind::Click), (4 * S, ActivityKind::Dwell), (6 * S, ActivityKind::Shortcut)]);
        Ok(())
    }
}
