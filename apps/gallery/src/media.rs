//! The Media page: every media widget in one believable mini timeline (spec:
//! Design Language "Components" rows Transport deck, Meter, Track header,
//! Clip, Playhead, Marker, Pending cut band). The page owns a fake project:
//! fixed-seed sine-plus-noise waveforms, a playhead that runs while Play is
//! on, a fake meter signal that peaks into the clip zone every few seconds,
//! and the same transport deck again in two small panels for the glass demo.

use eframe::egui::{self, Pos2, Rect};
use ui_kit::fonts;
use ui_kit::icons::{fill, regular};
use ui_kit::tokens::{color, radius, size, space, typography};
use ui_kit::{
    ClipKind, ClipLook, MarkerKind, PlayKind, SliderKind, Theme, Toggle, ToggleKind, chip, deck_button, fader, icon_button, level_meter, paint_clip,
    paint_marquee, paint_pending_band, paint_playhead, paint_ruler, paint_snap_line, paint_track_row, paint_waveform, panel, play_button, segmented, slider,
    stereo_meter_vertical, timecode, track_header, transport_deck, waveform_color,
};

/// The F1 context tag for the Media page (spec: S1 Help "Topic files").
pub const CONTEXT_TAG: &str = "media";

/// Total height of the mock timeline panel.
const TIMELINE_HEIGHT: f32 = 520.0;
/// Width of the track header column.
const HEADER_WIDTH: f32 = 220.0;
/// Width of the demo zoom slider.
const ZOOM_WIDTH: f32 = 160.0;
/// Side of the Fit icon button (spec "Icon button": 32 px).
const FIT_BUTTON: f32 = 32.0;
/// Length of the fake project: the ruler runs 0:00 to 2:00.
const TIMELINE_SECONDS: f64 = 120.0;
/// Where the playhead rests before Play is pressed (38% of the timeline).
const PLAYHEAD_START: f64 = 0.38 * TIMELINE_SECONDS;
/// Opacity of the dragged ghost clip (spec "Clip": 50%).
const GHOST_OPACITY: f32 = 0.5;
/// Width of one waveform peak, in px.
const WAVE_PIXELS_PER_PEAK: f32 = 3.0;
/// Fewest peaks drawn for one clip.
const MIN_PEAKS: usize = 12;
/// Most peaks drawn for one clip.
const MAX_PEAKS: usize = 400;
/// Height of the stereo demo meter.
const METER_HEIGHT: f32 = 140.0;
/// Width of the horizontal demo meter.
const METER_WIDTH: f32 = 240.0;
/// Height of the demo faders.
const FADER_HEIGHT: f32 = 150.0;
/// Width of one small glass deck panel.
const GLASS_PANEL_WIDTH: f32 = 400.0;
/// Height of the two small glass deck panels.
const GLASS_PANEL_HEIGHT: f32 = 120.0;
/// Time the decorative meter signal freezes on under reduced motion.
const FROZEN_TIME: f64 = 1.35;
/// The clip zone's lower edge in dBFS: peaks above it latch the clip light.
const CLIP_ZONE_DB: f32 = -3.0;
/// The 1 px hairline stroke width (the same rule ui-kit uses).
const HAIRLINE: f32 = 1.0;

/// The demo timeline's tracks.
const TRACKS: [&str; 4] = ["Voice", "Game audio", "Music", "SFX"];
/// The demo tool strip.
const TOOLS: [&str; 3] = ["Select", "Razor", "Fade"];
/// Total length label of the fake project.
const PROJECT_LENGTH: &str = "2:00";
/// The menu's five plain rows.
const MENU_ROWS: [&str; 5] = ["Import audio", "Rename take", "Duplicate", "Trim silence", "Delete"];
/// The submenu's rows.
const SUBMENU_ROWS: [&str; 2] = ["Mixdown WAV", "Stems"];
/// Every marker category at its demo position, in seconds.
const MARKERS: [(f64, MarkerKind); 6] = [
    (20.0, MarkerKind::Note),
    (35.0, MarkerKind::Retake),
    (50.0, MarkerKind::Cut),
    (70.0, MarkerKind::BRoll),
    (90.0, MarkerKind::System),
    (105.0, MarkerKind::User),
];
/// The pending cut band's demo range, in seconds.
const PENDING_BAND: (f64, f64) = (55.0, 65.0);
/// The marquee selection's demo range, in seconds, on the second track.
const MARQUEE: (f64, f64) = (72.0, 84.0);
/// Where the snap line sits, in seconds (the start of Interview B).
const SNAP_AT: f64 = 45.0;

/// One clip of the mock timeline.
struct DemoClip {
    /// The track row the clip sits on.
    track: usize,
    /// Start in seconds.
    from: f64,
    /// End in seconds.
    to: f64,
    /// The clip label drawn in `type.micro`.
    label: &'static str,
    /// Fixed seed of the sine-plus-noise peaks.
    seed: u64,
    /// Drawn selected: accent tint body, 2 px accent rim.
    selected: bool,
    /// A dragged ghost at 50% opacity.
    ghost: bool,
}

/// The fake project's clips: one selected, one ghost, Music dimmed by mute.
const CLIPS: [DemoClip; 8] = [
    DemoClip { track: 0, from: 5.0, to: 40.0, label: "Interview A", seed: 11, selected: false, ghost: false },
    DemoClip { track: 0, from: 45.0, to: 80.0, label: "Interview B", seed: 23, selected: true, ghost: false },
    DemoClip { track: 0, from: 85.0, to: 112.0, label: "Interview B", seed: 23, selected: false, ghost: true },
    DemoClip { track: 1, from: 0.0, to: 60.0, label: "Sfx bed", seed: 37, selected: false, ghost: false },
    DemoClip { track: 1, from: 70.0, to: 90.0, label: "Explosion", seed: 41, selected: false, ghost: false },
    DemoClip { track: 2, from: 0.0, to: 120.0, label: "Score", seed: 53, selected: false, ghost: false },
    DemoClip { track: 3, from: 10.0, to: 16.0, label: "Whoosh", seed: 67, selected: false, ghost: false },
    DemoClip { track: 3, from: 30.0, to: 33.0, label: "Pop", seed: 71, selected: false, ghost: false },
];

/// The Media page's demo state.
pub struct MediaState {
    /// The tool strip's selected entry.
    tool: usize,
    /// The Snap chip.
    snap: bool,
    /// The Ripple chip.
    ripple: bool,
    /// The zoom slider, 0 to 1.
    zoom: f32,
    /// True while the deck's Play is on.
    playing: bool,
    /// True while the deck's Loop is on: the playhead wraps at the end.
    looping: bool,
    /// True while recording; the deck's Record and the recorder panel share it.
    recorder: bool,
    /// The playhead position in seconds.
    playhead: f64,
    /// The last frame's input time, for the playhead delta.
    last_time: f64,
    /// The selected track row.
    selected_track: usize,
    /// Per-track mute toggles; Music starts muted.
    mutes: [bool; 4],
    /// Per-track solo toggles.
    solos: [bool; 4],
    /// Per-track record arm toggles; Voice starts armed.
    arms: [bool; 4],
    /// The meter clip light, latched until clicked.
    meter_clipped: bool,
    /// The two demo fader levels in dB.
    faders: [f32; 2],
    /// The fake meter's left level in dBFS, updated each frame.
    meter_l: f32,
    /// The fake meter's right level in dBFS, updated each frame.
    meter_r: f32,
}

impl MediaState {
    /// Fresh demo state: playhead at 38%, Voice armed, Music muted, Snap on.
    pub fn new() -> Self {
        Self {
            tool: 0,
            snap: true,
            ripple: false,
            zoom: 1.0,
            playing: false,
            looping: false,
            recorder: false,
            playhead: PLAYHEAD_START,
            last_time: 0.0,
            selected_track: 0,
            mutes: [false, false, true, false],
            solos: [false; 4],
            arms: [true, false, false, false],
            meter_clipped: false,
            faders: [0.0, -6.0],
            meter_l: -60.0,
            meter_r: -60.0,
        }
    }
}

impl Default for MediaState {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws the whole Media page: the mock timeline panel, then the widget row.
pub fn show(ui: &mut egui::Ui, state: &mut MediaState) {
    ui_kit::help::context(ui, CONTEXT_TAG);
    tick(ui, state);
    timeline_panel(ui, state);
    bottom_row(ui, state);
}

/// Advances the playhead and the fake meters, and repaints only while
/// something moves; the meters freeze when the theme asks for reduced motion.
fn tick(ui: &mut egui::Ui, state: &mut MediaState) {
    let now = ui.input(|i| i.time);
    let dt = (now - state.last_time).clamp(0.0, 0.1);
    state.last_time = now;
    if state.playing {
        state.playhead += dt;
        if state.playhead >= TIMELINE_SECONDS {
            if state.looping {
                state.playhead %= TIMELINE_SECONDS;
            } else {
                state.playhead = TIMELINE_SECONDS;
                state.playing = false;
            }
        }
    }
    let theme = Theme::get(ui.ctx());
    let meter_time = if theme.reduce_motion { FROZEN_TIME } else { now };
    let (l, r) = meter_levels(meter_time);
    state.meter_l = l;
    state.meter_r = r;
    if !state.meter_clipped && (l > CLIP_ZONE_DB || r > CLIP_ZONE_DB) {
        state.meter_clipped = true;
    }
    if state.playing || !theme.reduce_motion {
        ui.ctx().request_repaint();
    }
}

/// The mock audio timeline panel: toolbar row, ruler with markers, four track
/// rows and the transport deck floating over them.
fn timeline_panel(ui: &mut egui::Ui, state: &mut MediaState) {
    let theme = Theme::get(ui.ctx());
    panel(ui, |ui| {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, TIMELINE_HEIGHT), egui::Sense::hover());
        let painter = ui.painter().clone();
        let toolbar_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + size::PANEL_TOOLBAR_HEIGHT));
        let ruler_rect =
            Rect::from_min_max(Pos2::new(rect.left() + HEADER_WIDTH, toolbar_rect.bottom()), Pos2::new(rect.max.x, toolbar_rect.bottom() + size::RULER_HEIGHT));
        let tracks_rect = Rect::from_min_max(
            Pos2::new(ruler_rect.left(), ruler_rect.bottom()),
            Pos2::new(ruler_rect.right(), ruler_rect.bottom() + TRACKS.len() as f32 * size::TRACK_AUDIO_DEFAULT),
        );
        show_toolbar(ui, state, toolbar_rect);
        let ruler = ui.interact(ruler_rect, egui::Id::new("gallery-media-ruler"), egui::Sense::click_and_drag());
        let scrubbing = ruler.is_pointer_button_down_on();
        if scrubbing && let Some(pos) = ruler.interact_pointer_pos() {
            state.playhead = seconds_at(pos.x, tracks_rect);
        }
        paint_ruler(&painter, ruler_rect, &ruler_ticks(ruler_rect), &ruler_markers(tracks_rect));
        for (i, name) in TRACKS.iter().enumerate() {
            let top = tracks_rect.top() + i as f32 * size::TRACK_AUDIO_DEFAULT;
            let row = Rect::from_min_max(Pos2::new(tracks_rect.left(), top), Pos2::new(tracks_rect.right(), top + size::TRACK_AUDIO_DEFAULT));
            paint_track_row(&painter, row, i, state.selected_track == i);
            let header_rect = Rect::from_min_max(Pos2::new(rect.left(), top), Pos2::new(tracks_rect.left(), row.bottom()));
            let mut header = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(egui::Id::new(("gallery-media-header", i)))
                    .max_rect(header_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let selected = state.selected_track == i;
            let meter = if selected { Some((state.meter_l, state.meter_clipped)) } else { None };
            let mut toggles = [
                Toggle { label: "M", on: &mut state.mutes[i], kind: ToggleKind::Normal, always_visible: true },
                Toggle { label: "S", on: &mut state.solos[i], kind: ToggleKind::Normal, always_visible: true },
                Toggle { label: "R", on: &mut state.arms[i], kind: ToggleKind::RecordArm, always_visible: true },
            ];
            if track_header(&mut header, i + 1, name, selected, &mut toggles, meter).clicked() {
                state.selected_track = i;
            }
            let lanes = painter.with_clip_rect(row);
            for clip in CLIPS.iter().filter(|clip| clip.track == i) {
                let clip_rect = Rect::from_min_max(
                    Pos2::new(x_at(clip.from, tracks_rect), row.top() + space::SCALE_4),
                    Pos2::new(x_at(clip.to, tracks_rect), row.bottom() - space::SCALE_4),
                );
                let mut look = ClipLook::new(clip.label, ClipKind::Audio);
                look.selected = clip.selected;
                look.opacity = if clip.ghost { GHOST_OPACITY } else { 1.0 };
                look.dimmed = state.mutes[clip.track];
                let content = paint_clip(&lanes, clip_rect, &look, &theme);
                let count = ((clip_rect.width() / WAVE_PIXELS_PER_PEAK) as usize).clamp(MIN_PEAKS, MAX_PEAKS);
                paint_waveform(&lanes, content, &wave_peaks(clip.seed, count), waveform_color(look.selected, &theme));
            }
        }
        show_overlays(state, &theme, &painter, ruler_rect, tracks_rect);
        transport_deck(ui, rect, egui::Id::new("gallery-media-deck"), scrubbing, |ui| {
            deck_contents(ui, state);
        });
    });
}

/// The demo markers at their ruler x positions, for `paint_ruler`.
fn ruler_markers(tracks_rect: Rect) -> Vec<(f32, MarkerKind)> {
    MARKERS.iter().map(|(at, kind)| (x_at(*at, tracks_rect), *kind)).collect()
}

/// The timeline overlays: pending cut band, marquee, snap line and playhead.
fn show_overlays(state: &mut MediaState, theme: &Theme, painter: &egui::Painter, ruler_rect: egui::Rect, tracks_rect: egui::Rect) {
    let overlays = painter.with_clip_rect(tracks_rect);
    paint_pending_band(
        &overlays,
        Rect::from_min_max(
            Pos2::new(x_at(PENDING_BAND.0, tracks_rect), tracks_rect.top() + space::SCALE_4),
            Pos2::new(x_at(PENDING_BAND.1, tracks_rect), tracks_rect.bottom() - space::SCALE_4),
        ),
    );
    let marquee_top = tracks_rect.top() + size::TRACK_AUDIO_DEFAULT;
    paint_marquee(
        &overlays,
        Rect::from_min_max(
            Pos2::new(x_at(MARQUEE.0, tracks_rect), marquee_top + space::SCALE_4),
            Pos2::new(x_at(MARQUEE.1, tracks_rect), marquee_top + size::TRACK_AUDIO_DEFAULT - space::SCALE_4),
        ),
        theme,
    );
    paint_snap_line(&overlays, x_at(SNAP_AT, tracks_rect), tracks_rect, theme);
    paint_playhead(painter, ruler_rect, tracks_rect, x_at(state.playhead, tracks_rect), theme);
}

/// The deck's buttons in spec order: timecode, back to start, Play, Stop,
/// Record, Loop, total length.
fn deck_contents(ui: &mut egui::Ui, state: &mut MediaState) {
    let (main, fraction) = timecode_parts(state.playhead);
    timecode(ui, &main, &fraction);
    if deck_button(ui, fill::SKIP_BACK, "Back to start").clicked() {
        state.playhead = 0.0;
    }
    if play_button(ui, state.playing, PlayKind::Play).clicked() {
        state.playing = !state.playing;
    }
    if deck_button(ui, fill::STOP, "Stop").clicked() {
        state.playing = false;
    }
    if deck_button(ui, fill::RECORD, "Record").clicked() {
        state.recorder = true;
    }
    if deck_button(ui, fill::ARROWS_CLOCKWISE, "Loop").clicked() {
        state.looping = !state.looping;
    }
    ui.add_space(space::SCALE_4);
    ui.label(egui::RichText::new(PROJECT_LENGTH).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_SECONDARY));
}

/// The panel toolbar row: segmented tools and chips on the left, the zoom
/// slider and the Fit button on the right (still reading left to right).
fn show_toolbar(ui: &mut egui::Ui, state: &mut MediaState, rect: egui::Rect) {
    let mut left = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    segmented(&mut left, "media-tools", &mut state.tool, &TOOLS);
    chip(&mut left, &mut state.snap, "Snap");
    chip(&mut left, &mut state.ripple, "Ripple");
    let fit_rect = Rect::from_min_max(Pos2::new(rect.right() - FIT_BUTTON, rect.top()), Pos2::new(rect.right(), rect.bottom()));
    let zoom_rect =
        Rect::from_min_max(Pos2::new(fit_rect.left() - space::SCALE_4 - ZOOM_WIDTH, rect.top()), Pos2::new(fit_rect.left() - space::SCALE_4, rect.bottom()));
    let mut fit = ui.new_child(egui::UiBuilder::new().max_rect(fit_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    if icon_button(&mut fit, regular::FRAME_CORNERS, "Zoom to fit").clicked() {
        state.zoom = 1.0;
    }
    let mut zoom = ui.new_child(egui::UiBuilder::new().max_rect(zoom_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    slider(&mut zoom, &mut state.zoom, 0.0..=1.0, SliderKind::Zoom);
}

/// Ruler ticks: minor every 5 s, a label every 15 s from 0:00 to 2:00.
fn ruler_ticks(ruler_rect: egui::Rect) -> Vec<(f32, Option<String>)> {
    let mut ticks = Vec::new();
    for second in (0..=TIMELINE_SECONDS as u64).step_by(5) {
        let label = if second % 15 == 0 { Some(format!("{}:{:02}", second / 60, second % 60)) } else { None };
        ticks.push((x_at(second as f64, ruler_rect), label));
    }
    ticks
}

/// The panel row under the timeline: meters, fader, recorder button, menus
/// and the glass decks.
fn bottom_row(ui: &mut egui::Ui, state: &mut MediaState) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = space::WINDOW_GAP;
        meters_panel(ui, state);
        fader_panel(ui, state);
        recorder_panel(ui, state);
        menus_panel(ui);
        glass_panel(ui, state);
    });
}

/// The meters panel: a horizontal bar and a stereo vertical pair with scale,
/// driven by the fake signal; the clip light latches until clicked.
fn meters_panel(ui: &mut egui::Ui, state: &mut MediaState) {
    panel(ui, |ui| {
        panel_title(ui, "Meters");
        let horizontal = level_meter(ui, state.meter_l, state.meter_clipped, METER_WIDTH);
        ui.add_space(space::SCALE_2);
        let stereo = stereo_meter_vertical(ui, state.meter_l, state.meter_r, state.meter_clipped, METER_HEIGHT, true);
        let reset = ui.interact(horizontal.rect.union(stereo.rect), egui::Id::new("gallery-media-meter-reset"), egui::Sense::click());
        if reset.clicked() {
            state.meter_clipped = false;
        }
        caption(ui, "fake signal; the clip light latches until clicked");
    });
}

/// The fader panel: two vertical faders side by side with their dB readouts.
fn fader_panel(ui: &mut egui::Ui, state: &mut MediaState) {
    panel(ui, |ui| {
        panel_title(ui, "Fader");
        ui.horizontal(|ui| {
            caption_cell(ui, "left", |ui| {
                fader(ui, &mut state.faders[0], FADER_HEIGHT);
            });
            caption_cell(ui, "right", |ui| {
                fader(ui, &mut state.faders[1], FADER_HEIGHT);
            });
        });
    });
}

/// The recorder center button panel: the record capsule idle and recording.
fn recorder_panel(ui: &mut egui::Ui, state: &mut MediaState) {
    panel(ui, |ui| {
        panel_title(ui, "Recorder center button");
        ui.horizontal(|ui| {
            caption_cell(ui, "live, click to record", |ui| {
                if play_button(ui, state.recorder, PlayKind::Record).clicked() {
                    state.recorder = !state.recorder;
                }
            });
            caption_cell(ui, "recording", |ui| {
                play_button(ui, true, PlayKind::Record);
            });
        });
    });
}

/// The menus panel: a menu button opening the glass menu with five rows and
/// a submenu (egui sizes the 32 px rows; the popup follows the glass switch).
fn menus_panel(ui: &mut egui::Ui) {
    panel(ui, |ui| {
        panel_title(ui, "Menus");
        ui.menu_button("Menu", |ui| {
            for row in MENU_ROWS {
                if ui.button(row).clicked() {
                    ui.close();
                }
            }
            ui.menu_button("Export", |ui| {
                for row in SUBMENU_ROWS {
                    if ui.button(row).clicked() {
                        ui.close();
                    }
                }
            });
        });
        caption(ui, "glass popup, 32 px rows, five rows and a submenu");
    });
}

/// The glass on and off panel: the same deck in two small content panels.
/// `Theme.glass` is a global switch read from the egui context each frame and
/// the API has no per-panel override, so the Appearance switch covers both.
fn glass_panel(ui: &mut egui::Ui, state: &mut MediaState) {
    panel(ui, |ui| {
        panel_title(ui, "Glass on and off");
        caption(ui, "Theme.glass is global; the API has no per-panel override, so the Appearance switch covers both decks");
        let width = ui.available_width().min(2.0 * GLASS_PANEL_WIDTH + space::SCALE_8);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, GLASS_PANEL_HEIGHT), egui::Sense::hover());
        let painter = ui.painter().clone();
        let half = (rect.width() - space::SCALE_8) / 2.0;
        for n in 0..2u8 {
            let sub = Rect::from_min_size(Pos2::new(rect.left() + f32::from(n) * (half + space::SCALE_8), rect.top()), egui::vec2(half, rect.height()));
            painter.rect_filled(sub, radius::GROUP, color::BG_GROUND);
            painter.rect_stroke(sub, radius::GROUP, egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE), egui::StrokeKind::Inside);
            transport_deck(ui, sub, egui::Id::new(("gallery-media-glass-deck", n)), false, |ui| {
                let (main, fraction) = timecode_parts(state.playhead);
                timecode(ui, &main, &fraction);
                if deck_button(ui, fill::SKIP_BACK, "Back to start").clicked() {
                    state.playhead = 0.0;
                }
                if play_button(ui, state.playing, PlayKind::Play).clicked() {
                    state.playing = !state.playing;
                }
                if deck_button(ui, fill::STOP, "Stop").clicked() {
                    state.playing = false;
                }
                ui.add_space(space::SCALE_4);
                ui.label(egui::RichText::new(PROJECT_LENGTH).font(fonts::mono(typography::VALUE_SIZE)).color(color::TEXT_SECONDARY));
            });
        }
    });
}

/// A `type.headline` panel title.
fn panel_title(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).font(fonts::semibold(typography::HEADLINE_SIZE)).color(color::TEXT_PRIMARY));
    ui.add_space(space::SCALE_2);
}

/// A `type.caption` note in `text.tertiary`.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.add_space(space::SCALE_2);
    ui.label(egui::RichText::new(text).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
}

/// Demo content with a `type.caption` label under it.
fn caption_cell(ui: &mut egui::Ui, text: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        add(ui);
        ui.add_space(space::SCALE_2);
        ui.label(egui::RichText::new(text).font(fonts::medium(typography::CAPTION_SIZE)).color(color::TEXT_TERTIARY));
    });
}

/// Maps a timeline position in seconds to an x inside the lanes rect.
fn x_at(seconds: f64, lanes: egui::Rect) -> f32 {
    lanes.left() + (seconds / TIMELINE_SECONDS) as f32 * lanes.width()
}

/// Maps an x inside the lanes rect back to seconds, clamped to the timeline.
fn seconds_at(x: f32, lanes: egui::Rect) -> f64 {
    f64::from(((x - lanes.left()) / lanes.width()).clamp(0.0, 1.0)) * TIMELINE_SECONDS
}

/// Splits seconds into the deck timecode: `00:00:45` and `.60`.
fn timecode_parts(seconds: f64) -> (String, String) {
    let total_cs = (seconds.max(0.0) * 100.0).round() as u64;
    let total_s = total_cs / 100;
    let main = format!("{:02}:{:02}:{:02}", total_s / 3600, (total_s / 60) % 60, total_s % 60);
    (main, format!(".{:02}", total_cs % 100))
}

/// `count` deterministic sine-plus-noise peaks in -1 to 1, each `(min, max)`.
fn wave_peaks(seed: u64, count: usize) -> Vec<(f32, f32)> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x2545_F491_4F6C_DD1D);
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as f32 / u64::MAX as f32 * 2.0 - 1.0
    };
    (0..count)
        .map(|i| {
            let t = i as f32 / count.max(1) as f32;
            let envelope = 0.45 + 0.4 * (std::f32::consts::TAU * 3.0 * t).sin().abs();
            let top = (envelope + 0.15 * noise()).clamp(0.05, 1.0);
            (-top, top)
        })
        .collect()
}

/// The fake meter level of one channel at `t` in dBFS: a slow wander that
/// peaks into the clip zone every 4 s.
fn channel_level(t: f64, phase: f64) -> f32 {
    let wander = -18.0 + 6.0 * (std::f64::consts::TAU * (t / 3.0) + phase).sin();
    let cycle = (t % 4.0 - 3.4) as f32;
    let bump = 15.0 * (-cycle * cycle / 0.05).exp();
    (wander as f32 + bump).clamp(-60.0, 0.0)
}

/// The fake stereo meter signal at `t`, in dBFS.
fn meter_levels(t: f64) -> (f32, f32) {
    (channel_level(t, 0.0), channel_level(t, 1.3))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The timecode splits hours, minutes, seconds and the fraction.
    #[test]
    fn timecode_parts_split_the_position() {
        assert_eq!(timecode_parts(45.6), ("00:00:45".to_owned(), ".60".to_owned()));
        assert_eq!(timecode_parts(0.0), ("00:00:00".to_owned(), ".00".to_owned()));
        assert_eq!(timecode_parts(3725.5), ("01:02:05".to_owned(), ".50".to_owned()));
        assert_eq!(timecode_parts(-1.0), ("00:00:00".to_owned(), ".00".to_owned()));
    }

    /// The playhead starts at 38% of the fake project.
    #[test]
    fn the_playhead_starts_at_38_percent() {
        assert!((PLAYHEAD_START - 45.6).abs() < 1e-9);
    }

    /// The waveform peaks are deterministic and stay inside -1 to 1.
    #[test]
    fn wave_peaks_are_deterministic_and_in_range() {
        for seed in [1u64, 7, 23] {
            let peaks = wave_peaks(seed, 64);
            assert_eq!(peaks.len(), 64);
            assert_eq!(peaks, wave_peaks(seed, 64), "the same seed must give the same peaks");
            for (lo, hi) in peaks {
                assert!((-1.0..=1.0).contains(&lo));
                assert!((-1.0..=1.0).contains(&hi));
                assert!(lo < hi);
            }
        }
        assert_ne!(wave_peaks(1, 64), wave_peaks(2, 64), "different seeds must differ");
    }

    /// The fake signal peaks into the clip zone every few seconds and never
    /// leaves the meter range.
    #[test]
    fn the_fake_signal_peaks_into_the_clip_zone() {
        let mut peaked = false;
        for step in 0..240 {
            let t = f64::from(step) * 0.05;
            let (l, r) = meter_levels(t);
            assert!((-60.0..=0.0).contains(&l));
            assert!((-60.0..=0.0).contains(&r));
            peaked |= l > CLIP_ZONE_DB || r > CLIP_ZONE_DB;
        }
        assert!(peaked, "within 12 s the signal must reach the clip zone");
    }

    /// The frozen decorative level (reduced motion) stays out of the clip
    /// zone, so a still page never latches the clip light by itself.
    #[test]
    fn the_frozen_level_stays_out_of_the_clip_zone() {
        let (l, r) = meter_levels(FROZEN_TIME);
        assert!(l < CLIP_ZONE_DB && r < CLIP_ZONE_DB);
    }

    /// Seconds and x round-trip across the lanes rect.
    #[test]
    fn seconds_and_x_round_trip() {
        let lanes = egui::Rect::from_min_max(egui::pos2(100.0, 0.0), egui::pos2(700.0, 96.0));
        for seconds in [0.0, 30.0, 45.6, 119.0] {
            let back = seconds_at(x_at(seconds, lanes), lanes);
            assert!((back - seconds).abs() < 1e-3);
        }
        assert_eq!(seconds_at(-50.0, lanes), 0.0);
        assert_eq!(seconds_at(10_000.0, lanes), TIMELINE_SECONDS);
    }
}
