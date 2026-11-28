//! One take as Studio shows it: its input track, its settings and the zoom and cursor path, turned
//! into what the renderer draws at any time. No UI and no GPU work happens here.

use std::path::Path;

use cursor_render::CaptureMap;
use events::{Chord, EventsReader};
use studio_render as render;

use super::autozoom;
use super::path::{CursorPoint, StudioPath, cursor_points};
use super::settings::{self, BackgroundKind, ClickEffect, Highlight, KeysPosition, KeysSize, SettingsError, Shadow, StudioSettings};

/// Why a take could not be opened in Studio.
#[derive(Debug, thiserror::Error)]
pub enum StudioError {
    /// The take's events file cannot be read.
    #[error("the take's input track could not be read")]
    Track,
    /// The settings file is damaged or cannot be read.
    #[error(transparent)]
    Settings(#[from] SettingsError),
}

/// What Studio needs to know about the take's video.
#[derive(Debug, Clone, Copy)]
pub struct VideoFacts {
    /// The size of the file's picture, in pixels.
    pub size: (u32, u32),
    /// When the first video frame is, in microseconds from the file's start; the track counts from
    /// that frame.
    pub start_us: i64,
    /// How long the take is from its first video frame.
    pub duration_us: i64,
    /// Whether the video shows the screen. Without it no desktop position is in the picture.
    pub screen: bool,
}

/// From desktop pixels to pixels of the take's video: the `to_source` function of [`super`]. It
/// follows the events file's capture source and geometry changes, as cursor-render does, and answers
/// None for a position outside the captured picture.
pub fn to_source(track: &EventsReader, video: VideoFacts) -> impl Fn(i64, i32, i32) -> Option<(f32, f32)> {
    let (width, height) = video.size;
    let map = CaptureMap::new(width, height);
    move |time_us, x, y| {
        let projection = video.screen.then(|| map.project(track, time_us, width, height)).flatten()?;
        let (x, y) = (f64::from(x), f64::from(y));
        projection.contains(x, y).then(|| {
            let (x, y) = projection.to_frame(x, y);
            (x as f32, y as f32)
        })
    }
}

/// What one preview frame is drawn from, in pixels of the source picture handed to the renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameValues<'a> {
    pub view: render::ViewRect,
    pub cursor: Option<render::CursorDraw>,
    pub clicks: Vec<render::ClickDraw>,
    /// The shortcut on screen now.
    pub chord: Option<&'a str>,
}

/// A take opened in Studio.
pub struct StudioTake {
    track: EventsReader,
    video: VideoFacts,
    pub settings: StudioSettings,
    path: StudioPath,
    /// The times the cursor moved or reappeared, in order.
    moves: Vec<i64>,
    /// The clicks inside the picture, in order, in video pixels.
    clicks: Vec<render::ClickDraw>,
    /// The shortcuts' times and texts, in order.
    chords: Vec<(i64, String)>,
}

impl StudioTake {
    /// Opens the take whose main file is `video_path`. The first time, auto-zoom fills the lane and
    /// the settings file is written; a write that fails is only logged, so the take still opens.
    pub fn open(video_path: &Path, events: &Path, video: VideoFacts) -> Result<Self, StudioError> {
        let track = crate::review::load_track(events).ok_or(StudioError::Track)?;
        let file = settings::settings_path(video_path);
        let loaded = settings::load(&file)?;
        let mut settings = loaded.settings;
        let map = to_source(&track, video);
        if loaded.first_open {
            if settings.zoom.auto {
                let points = autozoom::activity_points(&track, video.duration_us, &map);
                settings.zoom.segments = autozoom::segments(&points, settings.zoom.amount, video.duration_us);
            }
            if let Err(err) = settings::save(&file, &settings) {
                log::warn!("the Studio settings were not saved: {err}");
            }
        }
        let cursor = cursor_points(&track, &map);
        let path = StudioPath::new(video.size, video.duration_us, &cursor, &settings.zoom.segments, settings.zoom.speed_s);
        let clicks = track
            .clicks()
            .filter_map(|click| map(click.time_us, click.event.x, click.event.y).map(|(x, y)| render::ClickDraw { x, y, time_us: click.time_us }))
            .collect();
        let chords = track.chords().iter().map(|chord| (chord.time_us, chord_text(&chord.event))).collect();
        drop(map);
        let moves = move_times(&cursor);
        Ok(Self { track, video, settings, path, moves, clicks, chords })
    }

    /// The input track; the renderer takes its cursor shapes from it.
    pub fn track(&self) -> &EventsReader {
        &self.track
    }

    /// The size of the file's picture.
    pub fn video_size(&self) -> (u32, u32) {
        self.video.size
    }

    /// The track time of a position in the file.
    pub fn track_time_us(&self, file_ms: u64) -> i64 {
        i64::try_from(file_ms).unwrap_or(i64::MAX).saturating_mul(1000).saturating_sub(self.video.start_us)
    }

    /// Where each zoom segment starts, as positions in the file in milliseconds, rounded up so that
    /// a seek there is inside the segment.
    pub fn zoom_starts_ms(&self) -> Vec<u64> {
        let to_file = |start_us: i64| u64::try_from(start_us.saturating_add(self.video.start_us)).unwrap_or(0).div_ceil(1000);
        self.settings.zoom.segments.iter().map(|segment| to_file(segment.start_us)).collect()
    }

    /// What the frame at `time_us` gets, for a source picture of `frame` pixels. The preview's
    /// picture can be smaller than the video, so every position is scaled to it.
    pub fn frame_values(&self, time_us: i64, frame: (u32, u32)) -> FrameValues<'_> {
        let (width, height) = self.video.size;
        let (kx, ky) = (frame.0 as f32 / width.max(1) as f32, frame.1 as f32 / height.max(1) as f32);
        let view = self.path.view_at(time_us);
        let view = render::ViewRect { x: view.x * kx, y: view.y * ky, width: view.width * kx, height: view.height * ky };
        let first = self.clicks.partition_point(|click| click.time_us <= time_us.saturating_sub(render::CLICK_EFFECT_US));
        let clicks = self.clicks.iter().skip(first).take_while(|click| click.time_us <= time_us).map(|click| render::ClickDraw {
            x: click.x * kx,
            y: click.y * ky,
            ..*click
        });
        let chord = self.settings.keys.show.then(|| chord_at(&self.chords, time_us, self.settings.keys.duration_s)).flatten();
        FrameValues { view, cursor: self.cursor_draw(time_us, (kx, ky)), clicks: clicks.collect(), chord }
    }

    fn cursor_draw(&self, time_us: i64, (kx, ky): (f32, f32)) -> Option<render::CursorDraw> {
        let (x, y) = self.path.cursor_at(time_us, self.settings.cursor.smoothing)?;
        let state = self.track.cursor_at(time_us)?;
        let (width, height) = self.video.size;
        // Video pixels per desktop pixel, which is what the shapes are measured in.
        let (sx, sy) = CaptureMap::new(width, height).project(&self.track, time_us, width, height)?.scale();
        let scale = ((sx * f64::from(kx) + sy * f64::from(ky)) / 2.0) as f32;
        // An id no shape has draws no picture and keeps the spotlight.
        let shape = state.shape.map_or(u32::MAX, |shape| shape.id);
        Some(render::CursorDraw { x: x * kx, y: y * ky, shape, scale, idle_us: idle_us(&self.moves, time_us) })
    }
}

/// When the cursor moved, or came back after being hidden or outside the picture. `cursor` is in
/// time order.
fn move_times(cursor: &[CursorPoint]) -> Vec<i64> {
    let mut last = None;
    let mut moves = Vec::new();
    for point in cursor {
        if point.position.is_some() && point.position != last {
            moves.push(point.time_us);
        }
        last = point.position;
    }
    moves
}

/// How long the cursor has been still at `time_us`.
fn idle_us(moves: &[i64], time_us: i64) -> i64 {
    let since = moves.partition_point(|&moved| moved <= time_us).checked_sub(1).map_or(0, |latest| moves[latest]);
    time_us.saturating_sub(since).max(0)
}

/// A chord as its keys joined by `+`.
fn chord_text(chord: &Chord) -> String {
    let held = chord.modifiers;
    let modifiers = [(held.ctrl, "Ctrl"), (held.alt, "Alt"), (held.shift, "Shift"), (held.win, "Win")];
    let mut keys: Vec<&str> = modifiers.iter().filter(|(down, _)| *down).map(|(_, name)| *name).collect();
    keys.push(&chord.key_name);
    keys.join("+")
}

/// The shortcut on screen at `time_us`: the newest one so far, for `duration_s` from its time. A
/// newer one replaces it. `chords` is in time order.
fn chord_at(chords: &[(i64, String)], time_us: i64, duration_s: f32) -> Option<&str> {
    let latest = chords.partition_point(|(at, _)| *at <= time_us).checked_sub(1)?;
    let (at, text) = &chords[latest];
    let shown_for = (f64::from(duration_s) * 1e6).round() as i64;
    (time_us.saturating_sub(*at) < shown_for).then_some(text.as_str())
}

/// The Studio settings as the renderer takes them. `image` is the loaded background picture; an
/// image background without one shows the gradient preset instead.
pub fn render_settings(settings: &StudioSettings, image: Option<render::Background>) -> render::RenderSettings {
    let (cursor, keys, frame) = (&settings.cursor, &settings.keys, &settings.frame);
    let background = match frame.background {
        BackgroundKind::Solid => render::Background::Solid(frame.solid_color),
        BackgroundKind::Gradient => render::Background::Gradient(frame.gradient_preset),
        BackgroundKind::Image => image.unwrap_or(render::Background::Gradient(frame.gradient_preset)),
    };
    render::RenderSettings {
        cursor: render::CursorLook {
            visible: cursor.visible,
            size: cursor.size_percent as f32 / 100.0,
            hide_idle_s: cursor.hide_idle_s,
            click_effect: match cursor.click_effect {
                ClickEffect::None => render::ClickEffect::None,
                ClickEffect::Ring => render::ClickEffect::Ring,
                ClickEffect::Ripple => render::ClickEffect::Ripple,
            },
            click_color: cursor.click_color,
            click_size: cursor.click_size_percent as f32 / 100.0,
            highlight: match cursor.highlight {
                Highlight::Off => render::Highlight::Off,
                Highlight::Spotlight => render::Highlight::Spotlight { radius: cursor.highlight_radius_px as f32 },
            },
        },
        keys: render::KeysLook {
            show: keys.show,
            position: match keys.position {
                KeysPosition::BottomCenter => render::KeysPosition::BottomCenter,
                KeysPosition::BottomLeft => render::KeysPosition::BottomLeft,
                KeysPosition::BottomRight => render::KeysPosition::BottomRight,
                KeysPosition::TopCenter => render::KeysPosition::TopCenter,
            },
            size: match keys.size {
                KeysSize::Small => render::KeysSize::Small,
                KeysSize::Medium => render::KeysSize::Medium,
                KeysSize::Large => render::KeysSize::Large,
            },
        },
        frame: render::FrameStyle {
            enabled: frame.enabled,
            background,
            padding: frame.padding_percent / 100.0,
            corner_radius: frame.corner_radius_px as f32,
            shadow: match frame.shadow {
                Shadow::Off => render::Shadow::Off,
                Shadow::Soft => render::Shadow::Soft,
                Shadow::Strong => render::Shadow::Strong,
            },
        },
    }
}

#[cfg(test)]
#[path = "take_tests.rs"]
mod tests;
