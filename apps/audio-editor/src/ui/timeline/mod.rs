//! ui::timeline: the central timeline view (ruler, lanes, playhead, zoom, scroll).

use std::cell::RefCell;

use eframe::egui;
use ui_kit::icons::{fill, regular};
use ui_kit::tokens::{color, size, space, typography};
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::message::Message;
use crate::editor::transport::TransportState;
use crate::ui::widgets::meter;
use crate::util::time::{samples_to_timecode, seconds_to_samples};

pub mod clip_view;
pub mod interaction;
pub mod lane;
pub mod ruler;
pub mod track_header;
pub mod waveform;

/// Timeline geometry and drawing constants: sizes and opacities with no
/// ui-kit token. Colors come from `ui_kit::tokens` and the runtime theme.
pub(crate) mod metrics {
    /// Grab distance of a range boundary line.
    pub(crate) const BOUNDARY_GRAB_PX: f32 = 4.0;
    /// Opacity of a live clip body fill (the record color, translucent).
    pub(crate) const CLIP_BODY_ALPHA: f32 = 34.0 / 255.0;
    /// Font size of the clip gain value in the header strip.
    pub(crate) const CLIP_GAIN_SIZE: f32 = 10.0;
    /// Opacity of a live clip header strip fill.
    pub(crate) const CLIP_HEADER_ALPHA: f32 = 102.0 / 255.0;
    /// Vertical inset of a clip rect from its track lane edges.
    pub(crate) const CLIP_MARGIN: f32 = 2.0;
    /// Minimum width of the clip context menu.
    pub(crate) const CLIP_MENU_MIN_W: f32 = 150.0;
    /// Gap between a clip header's name and the value at its right.
    pub(crate) const CLIP_NAME_GAP: f32 = 6.0;
    /// Narrowest width in which a clip header still shows its name.
    pub(crate) const CLIP_NAME_MIN_W: f32 = 8.0;
    /// Distance from the visible left edge at which a clip name pins when its own left edge is scrolled out of view.
    pub(crate) const CLIP_NAME_PIN: f32 = 16.0;
    /// Corner radius of a clip.
    pub(crate) const CLIP_RADIUS: f32 = 5.0;
    /// Horizontal padding of the clip header text.
    pub(crate) const CLIP_TEXT_PAD: f32 = 4.0;
    /// Level where the header meter strip turns from warn to clip.
    pub(crate) const METER_CLIP_DB: f32 = -3.0;
    /// Meter floor of the header meter strip, in dBFS.
    pub(crate) const METER_FLOOR_DB: f32 = -60.0;
    /// Level where the header meter strip turns from ok to warn.
    pub(crate) const METER_WARN_DB: f32 = -12.0;
    /// Height of the meter strip along the bottom of a track header.
    pub(crate) const METER_STRIP_H: f32 = 4.0;
    /// Small corner radius for filled rects.
    pub(crate) const CORNER_R: f32 = 2.0;
    /// Pointer travel that turns a press into a drag.
    pub(crate) const DRAG_START_PX: f32 = 3.0;
    /// Extra grab margin around a fade handle square.
    pub(crate) const FADE_HANDLE_GRAB: f32 = 2.0;
    /// Side length of a fade handle square.
    pub(crate) const FADE_HANDLE_SZ: f32 = 8.0;
    /// Opacity of the darkening outside the fade curves (bg.ground).
    pub(crate) const FADE_SHADE_ALPHA: f32 = 0.35;
    /// Opacity of the fade curve lines (text.primary).
    pub(crate) const GAIN_CURVE_ALPHA: f32 = 0.7;
    /// Width of the inline gain edit field.
    pub(crate) const GAIN_EDIT_W: f32 = 52.0;
    /// Vertical grab distance of the clip gain line.
    pub(crate) const GAIN_HIT_PX: f32 = 4.0;
    /// Width of the gain value text area at the right of the clip header.
    pub(crate) const GAIN_TEXT_W: f32 = 56.0;
    /// Dash length of the dashed ghost border.
    pub(crate) const GHOST_DASH: f32 = 6.0;
    /// Gap length of the dashed ghost border.
    pub(crate) const GHOST_GAP: f32 = 4.0;
    /// The 1 px hairline stroke width.
    pub(crate) const HAIRLINE: f32 = 1.0;
    /// Width of the track header column.
    pub(crate) const HEADER_COL_W: f32 = 208.0;
    /// Width of the `bg.ground` divider between the header column and the lanes.
    pub(crate) const COLUMN_GUTTER_W: f32 = 3.0;
    /// Width of the track number column at the left of each header.
    pub(crate) const NUMBER_GUTTER_W: f32 = 28.0;
    /// Height of the clip header strip (name + gain row).
    pub(crate) const HEADER_STRIP_H: f32 = 18.0;
    /// Width of the insert line.
    pub(crate) const INSERT_LINE_W: f32 = 2.0;
    /// Half width of a marker hit area on the ruler.
    pub(crate) const MARKER_TRI_W: f32 = 5.0;
    /// Opacity of an offline clip's body (sem.record).
    pub(crate) const OFFLINE_BODY_ALPHA: f32 = 0.6;
    /// Width of the range boundary lines.
    pub(crate) const RANGE_LINE_W: f32 = 1.0;
    /// Width of the reorder insertion line.
    pub(crate) const REORDER_LINE_W: f32 = 2.0;
    /// Minimum width in points a ruler tick must have to be drawn.
    pub(crate) const RULER_MIN_TICK_PX: f32 = 60.0;
    /// Height of the horizontal scrollbar strip.
    pub(crate) const SCROLLBAR_H: f32 = 12.0;
    /// Minimum scrollbar thumb width.
    pub(crate) const SCROLLBAR_THUMB_MIN: f32 = 20.0;
    /// Border width of a selected clip.
    pub(crate) const SELECTION_BORDER_W: f32 = 2.0;
    /// Snap distance in points (the "8 px snap distance").
    pub(crate) const SNAP_PX: f32 = 8.0;
    /// Clip width below which a clip draws as a single hairline.
    pub(crate) const THIN_CLIP_W: f32 = 3.0;
    /// Height of the resize strip at the header's bottom edge.
    pub(crate) const TRACK_RESIZE_H: f32 = 4.0;
    /// Width of the trimmable clip edge zones.
    pub(crate) const TRIM_EDGE_W: f32 = 6.0;
    /// Radius of the drawn sample dots at extreme zoom-in.
    pub(crate) const WAVE_DOT_R: f32 = 1.5;
    /// Space left above and below a waveform inside its lane.
    pub(crate) const WAVE_LANE_PAD: f32 = 1.0;
    /// Stroke width of the waveform lines.
    pub(crate) const WAVE_WIDTH: f32 = 1.0;
    /// Opacity of the idle (not hovered) clip gain line (text.primary).
    pub(crate) const GAIN_LINE_IDLE_ALPHA: f32 = 76.0 / 255.0;
    /// Opacity of the waveform's lane center guide (wave color).
    pub(crate) const GUIDE_CENTER_ALPHA: f32 = 51.0 / 255.0;
    /// Opacity of the divider between the two lanes of stereo media (bg.ground).
    pub(crate) const GUIDE_DIVIDER_ALPHA: f32 = 89.0 / 255.0;
    /// Opacity of the ghost clip fill while a bin drag hovers (accent).
    pub(crate) const GHOST_FILL_ALPHA: f32 = 90.0 / 255.0;
    /// Opacity of the ghost clip border (accent).
    pub(crate) const GHOST_STROKE_ALPHA: f32 = 220.0 / 255.0;
    /// Opacity of the insert line on Ctrl while a bin drag hovers (accent).
    pub(crate) const INSERT_LINE_ALPHA: f32 = 230.0 / 255.0;

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The timeline geometry the spec fixes: header strip, trim zones,
        /// snap distance and the clip-body opacities.
        #[test]
        fn metrics_hold_the_spec_values() {
            assert_eq!(HEADER_STRIP_H, 18.0);
            assert_eq!(COLUMN_GUTTER_W, 3.0);
            assert_eq!(NUMBER_GUTTER_W, 28.0);
            assert_eq!(TRIM_EDGE_W, 6.0);
            assert_eq!(SNAP_PX, 8.0);
            assert_eq!(FADE_HANDLE_SZ, 8.0);
            assert_eq!((CLIP_BODY_ALPHA * 255.0).round(), 34.0);
            assert_eq!((CLIP_HEADER_ALPHA * 255.0).round(), 102.0);
        }
    }
}

/// Smallest zoom: one thirty-second of a sample per pixel.
pub const MIN_SPP: f64 = 1.0 / 32.0;
/// Largest zoom: 4 194 304 samples per pixel.
pub const MAX_SPP: f64 = 4_194_304.0;

/// Zoom command from the View menu, applied by `timeline::show`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMenuAction {
    /// Nothing chosen.
    #[default]
    None,
    /// Zoom in, anchored at the lane center.
    ZoomIn,
    /// Zoom out, anchored at the lane center.
    ZoomOut,
    /// Zoom so the whole project fits.
    ZoomToFit,
    /// Zoom so the range selection fills the lane area.
    ZoomToSelection,
}

/// Ruler tick step, in seconds, from the coarsest-fitting list.
const TICK_STEPS: [f64; 21] =
    [0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0, 3600.0];

/// Geometry of the timeline lane area for one frame.
pub struct Geom {
    /// Left edge of the lane area in window points.
    pub lane_left: f32,
    /// Width of the lane area in points.
    pub lane_width: f32,
    /// Horizontal zoom in samples per pixel.
    pub spp: f64,
    /// First visible sample.
    pub scroll_x: u64,
}

impl Geom {
    /// Window x position of a timeline sample.
    pub fn sample_to_x(&self, sample: u64) -> f32 {
        self.lane_left + ((sample as f64 - self.scroll_x as f64) / self.spp) as f32
    }

    /// Timeline sample at a window x position, clamped at 0.
    pub fn x_to_sample(&self, x: f32) -> u64 {
        let dx = (x - self.lane_left) as f64;
        ((dx * self.spp + self.scroll_x as f64).max(0.0)) as u64
    }

    /// First and one-past-last visible sample.
    pub fn visible_range(&self) -> (u64, u64) {
        (self.scroll_x, self.scroll_x + (self.lane_width as f64 * self.spp).ceil() as u64)
    }
}

/// Zooms horizontally by `factor`, keeping the sample under the anchor pixel fixed.
///
/// `anchor_offset_px` is measured from the left edge of the lane area. The new
/// samples-per-pixel is clamped to MIN_SPP..=MAX_SPP and the new scroll
/// position never goes below zero.
pub fn zoom_at(spp: f64, scroll_x: u64, factor: f64, anchor_offset_px: f32) -> (f64, u64) {
    let new_spp = (spp * factor).clamp(MIN_SPP, MAX_SPP);
    let anchor = scroll_x as f64 + f64::from(anchor_offset_px) * spp;
    let new_scroll = anchor - f64::from(anchor_offset_px) * new_spp;
    (new_spp, new_scroll.round().max(0.0) as u64)
}

/// Samples per pixel and scroll that fit the whole project with a small margin.
///
/// The span is the larger of the project length and 10 seconds of samples,
/// times 1.05, spread over `lane_width`. Clamped to MIN_SPP..=MAX_SPP; the
/// scroll starts at 0.
pub fn zoom_to_fit(project_length: u64, sample_rate: u32, lane_width: f32) -> (f64, u64) {
    if lane_width <= 0.0 {
        return (MAX_SPP, 0);
    }
    let span = project_length.max(seconds_to_samples(10.0, sample_rate)) as f64;
    let spp = (span / f64::from(lane_width) * 1.05).clamp(MIN_SPP, MAX_SPP);
    (spp, 0)
}

/// Zoom and scroll that show the range selection: samples-per-pixel is the
/// range length over the lane width times 1.1, and the scroll puts the range
/// start 5 percent of the visible width in from the left edge, not below 0.
/// Both are clamped to the usual zoom limits.
pub fn zoom_to_selection(start: u64, end: u64, lane_width: f32) -> (f64, u64) {
    if lane_width <= 0.0 {
        return (MAX_SPP, 0);
    }
    let len = (end.saturating_sub(start)) as f64;
    let spp = (len / f64::from(lane_width) * 1.1).clamp(MIN_SPP, MAX_SPP);
    let visible = f64::from(lane_width) * spp;
    let scroll = (start as f64 - 0.05 * visible).round().max(0.0) as u64;
    (spp, scroll)
}

/// Smallest ruler tick step (in samples) that is at least `min_px` wide at
/// the given zoom and sample rate. Uses the largest step when none fit.
pub fn tick_step(spp: f64, rate: u32, min_px: f32) -> u64 {
    let min_px = f64::from(min_px);
    for step in TICK_STEPS {
        let samples = seconds_to_samples(step, rate);
        if samples as f64 / spp >= min_px {
            return samples;
        }
    }
    seconds_to_samples(TICK_STEPS[TICK_STEPS.len() - 1], rate)
}

/// Id of the stage panel.
const STAGE_ID: &str = "timeline_stage";
/// Size of the Record glyph, the same as the other bottom bar glyphs.
const BAR_GLYPH_SIZE: f32 = 18.0;
/// Width of the zoom slider in the panel toolbar, in points.
const ZOOM_SLIDER_W: f32 = 140.0;
/// Width of the waveform height slider in the panel toolbar, in points.
const WAVE_SLIDER_W: f32 = 90.0;
/// Width reserved for the zoom readout ("100%"), in points.
const ZOOM_READOUT_W: f32 = 56.0;
/// Height of the separators in the panel toolbar, in points.
const TOOLBAR_SEPARATOR_H: f32 = 20.0;
/// Waveform height slider range: log2 of the 1x to 16x display scale.
const WAVE_LOG2_RANGE: std::ops::RangeInclusive<f32> = 0.0..=4.0;

/// Zoom slider position for a samples-per-pixel value: the negated log2, so
/// the right end is the most zoomed in. Out-of-range input clamps first.
fn zoom_to_slider(spp: f64) -> f32 {
    -(spp.clamp(MIN_SPP, MAX_SPP).log2()) as f32
}

/// Samples-per-pixel for a zoom slider position (the inverse of
/// [`zoom_to_slider`]).
fn slider_to_zoom(value: f32) -> f64 {
    (-f64::from(value)).exp2().clamp(MIN_SPP, MAX_SPP)
}

/// The zoom slider's range: the most zoomed out to the most zoomed in.
fn zoom_slider_range() -> std::ops::RangeInclusive<f32> {
    zoom_to_slider(MAX_SPP)..=zoom_to_slider(MIN_SPP)
}

/// The zoom readout: the zoom relative to Fit as a percent (Fit is 100%,
/// zoomed in reads higher). `fit_spp` is the samples-per-pixel Fit would set.
fn zoom_readout(spp: f64, fit_spp: f64) -> String {
    let percent = fit_spp / spp * 100.0;
    if percent >= 10.0 {
        format!("{percent:.0}%")
    } else if percent >= 1.0 {
        format!("{percent:.1}%")
    } else {
        format!("{percent:.2}%")
    }
}

/// Total scrollable span in samples: the larger of project length and visible
/// width, plus 10 percent extra.
fn scroll_extent(project_length: u64, visible: u64) -> u64 {
    (project_length.max(visible) as f64 * 1.1).ceil() as u64
}

/// How far the view must be able to scroll and fit: the project length, or the
/// end of the take being recorded when that runs past the last clip.
fn content_length(editor: &Editor) -> u64 {
    let length = editor.project_length();
    editor.recording_status().map_or(length, |take| length.max(take.start.saturating_add(take.frames)))
}

/// Draws the whole timeline into the remaining center space and handles its
/// input (zoom, scroll, playhead clicks). Not a ScrollArea: everything is
/// painted by hand over one allocated rect.
pub fn show(ui: &mut egui::Ui, editor: &mut Editor, view_menu: ViewMenuAction, meters: &mut meter::MeterState) -> Vec<Message> {
    // The timeline owns horizontal zoom; egui's own keyboard and Ctrl+wheel
    // UI zoom must not fire here.
    ui.ctx().options_mut(|o| o.zoom_with_keyboard = false);

    // The timeline sits in the ui-kit stage panel: the panel toolbar row on
    // top, the ruler, headers and lanes under it, and the transport in the
    // bottom bar with Play in the notch.
    let mut messages = Vec::new();
    let mut left = None;
    let mut right = None;
    let playing = is_playing(editor);
    let primary = ui_kit::PrimaryButton { kind: ui_kit::PlayKind::Play, active: playing, enabled: true };
    // The content and the two bar sides take turns with the editor.
    let shared = RefCell::new(&mut *editor);
    let stage = ui_kit::stage_panel(
        ui,
        egui::Id::new(STAGE_ID),
        primary,
        |ui| {
            // F1 over the lanes and ruler opens Working with clips; the more
            // specific track-header tag wins over this one.
            ui_kit::help::context(ui, crate::help::CONTEXT_TIMELINE);
            let editor = &mut **shared.borrow_mut();
            let lane_width = ui.available_width() - metrics::HEADER_COL_W - metrics::COLUMN_GUTTER_W;
            panel_toolbar(ui, editor, lane_width);
            messages.extend(show_frame(ui, editor, view_menu, meters).into_iter().map(Message::media));
        },
        |ui| left = bar_left(ui, &shared.borrow()),
        |ui| right = bar_right(ui, &shared.borrow()),
    );
    let _ = stage.primary.clone().on_hover_text(if playing { "Pause (P)" } else { "Play (Space)" });
    let play = stage.clicked().then_some(TransportAction::Play);
    if let Some(action) = play.or(left).or(right) {
        messages.extend(run_transport(editor, action));
    }
    // F1 over the bottom bar and the Play button opens Playback and the
    // transport; the tour spotlights the same area.
    let transport_rect = stage.bar_rect.union(stage.primary.rect);
    ui_kit::help::context(&ui.new_child(egui::UiBuilder::new().max_rect(transport_rect)), crate::help::CONTEXT_TRANSPORT);
    ui_kit::tour::anchor(ui, crate::help::ANCHOR_TRANSPORT, transport_rect);
    messages
}

/// What one transport control asks the editor to do when clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransportAction {
    /// Back to the start.
    JumpStart,
    /// Play, or pause while playing.
    Play,
    /// Stop playback.
    Stop,
    /// Start a take, or end it (keeping it) while one is live.
    Record,
    /// Forward to the end.
    JumpEnd,
}

/// True while playback runs, so the Play button shows Pause. A take being
/// recorded does not count.
fn is_playing(editor: &Editor) -> bool {
    editor.transport_state() == TransportState::Playing && !editor.is_recording()
}

/// Runs one transport control; returns the message for the user when a Record
/// click could not start a take.
fn run_transport(editor: &mut Editor, action: TransportAction) -> Option<Message> {
    match action {
        TransportAction::JumpStart => editor.jump_start(),
        // Play pauses while playing and plays otherwise; while a take runs it
        // ends the take (the editor routes it).
        TransportAction::Play => {
            if is_playing(editor) {
                editor.toggle_pause();
            } else {
                editor.toggle_play();
            }
        }
        TransportAction::Stop => editor.stop(),
        // Record starts a take when idle and ends it (keeping it) while
        // recording; a take that is already being saved ignores clicks.
        TransportAction::Record => {
            if editor.take_is_live() {
                editor.stop_recording();
            } else if !editor.is_recording()
                && let Err(error) = editor.start_recording()
            {
                return Some(Message::recording(error.to_string()));
            }
        }
        TransportAction::JumpEnd => editor.jump_end(),
    }
    None
}

/// Splits `HH:MM:SS.mmm` into the main part and the `.mmm` fraction.
fn split_timecode(timecode: &str) -> (&str, &str) {
    timecode.split_at(timecode.find('.').unwrap_or(timecode.len()))
}

/// The size of the bar's timecode: large, unless the left side, `column` wide,
/// has no room for it beside back to start. `large` is the width of the large
/// timecode.
fn timecode_size(column: f32, large: f32) -> ui_kit::TimecodeSize {
    if large + space::SCALE_12 + ui_kit::STAGE_BAR_BUTTON_DIAMETER <= column { ui_kit::TimecodeSize::Large } else { ui_kit::TimecodeSize::Value }
}

/// What the bar's right side shows, from everything to its narrowest form.
/// Stop and Record are in every one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightFit {
    /// Everything.
    Full,
    /// No total length.
    NoLength,
    /// No total length and no go to end (the End key and the View menu stay).
    NoJumpEnd,
}

/// What fits in the right side, `column` wide. `length` is the width of the
/// total length.
fn right_fit(column: f32, length: f32) -> RightFit {
    let button = ui_kit::STAGE_BAR_BUTTON_DIAMETER;
    let three_buttons = 3.0 * button + 2.0 * space::SCALE_8;
    if three_buttons + space::SCALE_8 + space::SCALE_4 + length <= column {
        RightFit::Full
    } else if three_buttons <= column {
        RightFit::NoLength
    } else {
        RightFit::NoJumpEnd
    }
}

/// The bar's timecode text: the playhead, or the record position while a take
/// is recorded.
fn position_text(editor: &Editor) -> String {
    let position = editor.record_playhead().unwrap_or(editor.view.playhead);
    samples_to_timecode(position, editor.project.sample_rate)
}

/// The total length as the bar draws it.
fn length_job(ui: &egui::Ui, editor: &Editor) -> egui::text::LayoutJob {
    let length = samples_to_timecode(editor.project_length(), editor.project.sample_rate);
    let tint = crate::ui::neutrals(ui.ctx()).text_tertiary;
    ui_kit::fonts::tracked(length, ui_kit::fonts::mono(typography::VALUE_SIZE), tint, ui_kit::fonts::MONO_LETTER_SPACING_EM)
}

/// The bottom bar's left side: the timecode (the record position while a take
/// is recorded) and back to start, which is disabled then. A narrow side draws
/// the timecode smaller ([`timecode_size`]).
fn bar_left(ui: &mut egui::Ui, editor: &Editor) -> Option<TransportAction> {
    let timecode = position_text(editor);
    let (main, fraction) = split_timecode(&timecode);
    let size = timecode_size(ui.available_width(), ui_kit::timecode_width(ui, main, fraction, ui_kit::TimecodeSize::Large));
    ui.spacing_mut().item_spacing.x = space::SCALE_12;
    ui_kit::timecode_sized(ui, main, fraction, size);
    let back = ui.add_enabled_ui(!editor.is_recording(), |ui| ui_kit::bar_button(ui, fill::SKIP_BACK, "Go to start (Home)")).inner;
    back.clicked().then_some(TransportAction::JumpStart)
}

/// The bottom bar's right side: Stop, Record, forward to the end (disabled
/// while a take is recorded) and the project's total length. A narrow side
/// leaves out the length first, then go to end ([`right_fit`]).
fn bar_right(ui: &mut egui::Ui, editor: &Editor) -> Option<TransportAction> {
    let length = length_job(ui, editor);
    let fit = right_fit(ui.available_width(), ui.painter().layout_job(length.clone()).size().x);
    ui.spacing_mut().item_spacing.x = space::SCALE_8;
    let mut action = None;
    if ui_kit::bar_button(ui, fill::STOP, "Stop").clicked() {
        action = Some(TransportAction::Stop);
    }
    if record_button(ui, editor.take_is_live()).clicked() {
        action = Some(TransportAction::Record);
    }
    if fit != RightFit::NoJumpEnd {
        let forward = ui.add_enabled_ui(!editor.is_recording(), |ui| ui_kit::bar_button(ui, fill::SKIP_FORWARD, "Go to end (End)")).inner;
        if forward.clicked() {
            action = Some(TransportAction::JumpEnd);
        }
    }
    if fit == RightFit::Full {
        ui.add_space(space::SCALE_4);
        ui.add(egui::Label::new(length).extend());
    }
    action
}

/// The Record button: a bottom bar button whose glyph keeps the record color.
/// It shows Stop while the take is live.
fn record_button(ui: &mut egui::Ui, live: bool) -> egui::Response {
    // The ui-kit bar button has no glyph color, so it draws the circle with an
    // empty glyph and the colored one goes on top.
    let response = ui_kit::bar_button(ui, "", "Record (Ctrl+R)");
    let glyph = if live { fill::STOP } else { fill::RECORD };
    let font = ui_kit::fonts::icon_fill(BAR_GLYPH_SIZE);
    ui.painter().text(response.rect.center(), egui::Align2::CENTER_CENTER, glyph, font, color::SEM_RECORD);
    response
}

/// The panel toolbar row (44 tall): Undo and Redo, a separator and the Ripple
/// and Snap chips on the left; on the right the waveform icon and height
/// slider, a separator, the magnifier icon and zoom slider, the readout and
/// the Fit button. Undo and Redo read the editor's history every frame and
/// are disabled when there is nothing to undo or redo. The chips read the
/// editor's view state every frame, so they stay in sync with the View menu
/// and the R and N keys. Zoom stays live while a take is recorded; the chips
/// do not.
fn panel_toolbar(ui: &mut egui::Ui, editor: &mut Editor, lane_width: f32) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let recording = editor.is_recording();
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    let bar = ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = space::SCALE_8;
        undo_redo_buttons(ui, editor);
        toolbar_separator(ui);
        ui.add_enabled_ui(!recording, |ui| {
            let mut ripple = editor.view.ripple;
            if ui_kit::chip(ui, &mut ripple, "Ripple").on_hover_text("Ripple edit (R)").clicked() {
                editor.toggle_ripple();
            }
            let mut snap = editor.view.snap;
            if ui_kit::chip(ui, &mut snap, "Snap").on_hover_text("Snapping (N)").clicked() {
                editor.toggle_snap();
            }
        });

        // Added right to left, so on screen it reads waveform icon and slider,
        // separator, magnifier icon and zoom slider, readout, Fit.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui_kit::icon_button(ui, regular::ARROWS_OUT_LINE_HORIZONTAL, "Zoom to fit (\\)").clicked() {
                apply_fit(editor, lane_width);
            }
            let spp = editor.view.samples_per_pixel;
            let (readout, _) = ui.allocate_exact_size(egui::vec2(ZOOM_READOUT_W, size::SEGMENT_HEIGHT), egui::Sense::hover());
            ui.painter().text(
                readout.left_center(),
                egui::Align2::LEFT_CENTER,
                zoom_readout(spp, zoom_to_fit(content_length(editor), editor.project.sample_rate, lane_width).0),
                ui_kit::fonts::mono(typography::VALUE_SIZE),
                neutral.text_secondary,
            );
            let shown = zoom_to_slider(spp);
            let mut value = shown;
            ui.allocate_ui(egui::vec2(ZOOM_SLIDER_W, size::SEGMENT_HEIGHT), |ui| {
                ui_kit::slider(ui, &mut value, zoom_slider_range(), ui_kit::SliderKind::Zoom);
            });
            if (value - shown).abs() > 1e-4 {
                let (new_spp, scroll_x) = zoom_at(spp, editor.view.scroll_x, slider_to_zoom(value) / spp, lane_width / 2.0);
                editor.set_view_x(new_spp, clamp_scroll_x(editor, new_spp, lane_width, scroll_x));
            }
            toolbar_icon(ui, regular::MAGNIFYING_GLASS);
            toolbar_separator(ui);
            wave_scale_slider(ui, editor);
        });
    });
    // The toolbar row is the tour's "Timeline toolbar" target.
    ui_kit::tour::anchor(ui, crate::help::ANCHOR_TIMELINE_TOOLBAR, bar.response.rect);
}

/// The Undo and Redo icon buttons at the left of the panel toolbar, wired to
/// the editor's undo and redo (the same actions as the Edit menu and Ctrl+Z,
/// Ctrl+Y) and disabled while there is nothing to undo or redo.
fn undo_redo_buttons(ui: &mut egui::Ui, editor: &mut Editor) {
    let can_undo = editor.undo_label().is_some();
    ui.add_enabled_ui(can_undo, |ui| {
        if ui_kit::icon_button(ui, regular::ARROW_COUNTER_CLOCKWISE, "Undo").clicked() {
            let _ = editor.undo();
        }
    });
    let can_redo = editor.redo_label().is_some();
    ui.add_enabled_ui(can_redo, |ui| {
        if ui_kit::icon_button(ui, regular::ARROW_CLOCKWISE, "Redo").clicked() {
            let _ = editor.redo();
        }
    });
}

/// A 1 x 20 `stroke.separator.strong` line between groups in the panel
/// toolbar.
fn toolbar_separator(ui: &mut egui::Ui) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, TOOLBAR_SEPARATOR_H), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, neutral.stroke_separator_strong);
}

/// A decorative Phosphor icon in `text.secondary` in the panel toolbar (the
/// waveform and magnifier marks beside their sliders).
fn toolbar_icon(ui: &mut egui::Ui, glyph: &str) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let (icon, _) = ui.allocate_exact_size(egui::vec2(size::ICON_BUTTON / 2.0, size::SEGMENT_HEIGHT), egui::Sense::hover());
    let font = ui_kit::fonts::icon_regular(typography::HEADLINE_SIZE);
    ui.painter().text(icon.center(), egui::Align2::CENTER_CENTER, glyph, font, neutral.text_secondary);
}

/// The waveform height control on the right of the panel toolbar: a Phosphor
/// waveform icon and a 90 px zoom-style slider over the 1x to 16x display
/// scale (edited as log2). Double-click resets to 1x; Alt+wheel on the lanes
/// still scales too. Display only, never changes the audio. Called in a
/// right-to-left layout, so the slider lands left of the zoom slider with the
/// icon to its left.
fn wave_scale_slider(ui: &mut egui::Ui, editor: &mut Editor) {
    let shown = editor.wave_scale().log2();
    let mut value = shown;
    let mut reset = false;
    ui.allocate_ui(egui::vec2(WAVE_SLIDER_W, size::SEGMENT_HEIGHT), |ui| {
        let resp = ui_kit::slider(ui, &mut value, WAVE_LOG2_RANGE, ui_kit::SliderKind::Zoom).on_hover_text("Waveform height");
        reset = resp.double_clicked();
    });
    if reset {
        editor.set_wave_scale(1.0);
    } else if (value - shown).abs() > 1e-4 {
        editor.set_wave_scale(value.exp2());
    }
    toolbar_icon(ui, regular::WAVEFORM);
}

/// The ruler, header column, lanes and scrollbar in the space the panel
/// toolbar left over, with all their input.
fn show_frame(ui: &mut egui::Ui, editor: &mut Editor, view_menu: ViewMenuAction, meters: &mut meter::MeterState) -> Vec<String> {
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click());

    let header_right = avail.left() + metrics::HEADER_COL_W;
    let lane_left = header_right + metrics::COLUMN_GUTTER_W;
    let lane_right = avail.right();
    let lane_width = lane_right - lane_left;
    let ruler_rect = egui::Rect::from_min_max(egui::pos2(lane_left, avail.top()), egui::pos2(lane_right, avail.top() + size::RULER_HEIGHT));
    let header_rect = egui::Rect::from_min_max(egui::pos2(avail.left(), ruler_rect.bottom()), egui::pos2(header_right, avail.bottom()));
    // F1 over the track headers opens Tracks; the tour spotlights the column.
    ui_kit::help::context(&ui.new_child(egui::UiBuilder::new().max_rect(header_rect)), crate::help::CONTEXT_TRACKS);
    ui_kit::tour::anchor(ui, crate::help::ANCHOR_TRACKS, header_rect);
    let lane_rect = egui::Rect::from_min_max(egui::pos2(lane_left, ruler_rect.bottom()), egui::pos2(lane_right, avail.bottom() - metrics::SCROLLBAR_H));
    let scroll_rect = egui::Rect::from_min_max(egui::pos2(lane_left, lane_rect.bottom()), egui::pos2(lane_right, avail.bottom()));

    let content_h: f32 = editor.project.tracks.iter().map(|t| t.height).sum::<f32>() + track_header::ADD_ROW_H;
    let max_scroll_y = (content_h - lane_rect.height()).max(0.0);

    // Old project files may carry an out-of-range zoom (or 0); fit instead.
    if !(MIN_SPP..=MAX_SPP).contains(&editor.view.samples_per_pixel) {
        apply_fit(editor, lane_width);
    }
    // \ zooms to fit, Ctrl+\ zooms to the selection, wherever the pointer is.
    if !ui.ctx().egui_wants_keyboard_input() {
        let keys = crate::shortcuts::table();
        if keys.pressed(ui.ctx(), "zoom_to_selection") {
            apply_view_menu(editor, ViewMenuAction::ZoomToSelection, lane_width);
        } else if keys.pressed(ui.ctx(), "zoom_to_fit") {
            apply_fit(editor, lane_width);
        }
    }
    apply_view_menu(editor, view_menu, lane_width);

    // Input first: it may change spp, scroll_x, scroll_y and the playhead.
    // Marker interaction on the ruler comes first and wins over the plain
    // ruler click (playhead), so the flag suppresses that click this frame.
    // While a take is recorded the mouse only zooms and scrolls: the ruler,
    // the lanes, the clips and the bin drag ignore it (no seek, no scrub, no
    // selection, no edits).
    let recording = editor.is_recording();
    let marker_g = Geom { lane_left, lane_width, spp: editor.view.samples_per_pixel, scroll_x: editor.view.scroll_x };
    let ruler_taken = !recording && ruler::handle_markers(ui, editor, &marker_g, ruler_rect);
    handle_input(ui, editor, lane_left, lane_width, max_scroll_y, &resp);
    // Audible scrubbing on the ruler (and the playhead handle): press starts,
    // drag scrubs, release ends. Marker interaction wins.
    if !recording {
        ruler::handle_scrub(ui, editor, ruler_rect, ruler_taken);
    }

    // Follow the playhead while playing or recording: when it leaves the
    // visible lanes on either side, page so it sits 5 percent in from the left
    // edge. Never follows while stopped. While a take is recorded the playhead
    // is the end of what was recorded so far.
    if editor.transport_state() == TransportState::Playing || editor.take_is_live() {
        let spp = editor.view.samples_per_pixel;
        let playhead = editor.view.playhead;
        let visible_end = editor.view.scroll_x + (lane_width as f64 * spp) as u64;
        if playhead < editor.view.scroll_x || playhead > visible_end {
            let left_margin = (0.05 * lane_width as f64 * spp) as u64;
            let new_scroll = playhead.saturating_sub(left_margin);
            editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, new_scroll));
        }
    }

    let view = editor.view.clone();
    let g = Geom { lane_left, lane_width, spp: view.samples_per_pixel, scroll_x: view.scroll_x };
    // A bin item dragged over the lanes: Esc cancels, release over a lane drops.
    let mut messages = Vec::new();
    if !recording {
        messages = interaction::handle_bin_drag(ui, editor, &g, lane_rect, view.scroll_y);
        // Clip mouse interaction: header moves, edge trims, clip menu, cursors.
        // When it takes the press, the lane interaction must not see it.
        let clip_handled = interaction::handle_clip_input(ui, editor, &g, lane_rect, view.scroll_y);
        // Lane mouse interaction: range selection, boundary drags, playhead
        // clicks.
        if !clip_handled {
            interaction::handle_lane_input(ui, editor, &g, lane_rect, view.scroll_y);
        }
    }
    let visible = g.visible_range().1 - g.visible_range().0;
    let length = content_length(editor);
    let max_scroll_x = scroll_extent(length, visible).saturating_sub(visible);

    draw(ui, editor, &g, ruler_rect, header_rect, lane_rect, scroll_rect, visible, max_scroll_x, meters);
    // Clip drag snap line (over the clips) and the open clip context menu.
    interaction::draw_clip_snap_line(ui, &g, lane_rect);
    messages.extend(interaction::draw_clip_menu(ui, editor));
    ruler::draw_marker_menu(ui, editor);
    messages.extend(interaction::draw_gain_edit(ui, editor));
    messages.extend(crate::ui::dialogs::gain::show(ui.ctx(), editor));
    messages.extend(crate::ui::dialogs::normalize::show(ui.ctx(), editor));
    messages
}

/// Zooms so the whole project fits, scroll at the start.
fn apply_fit(editor: &mut Editor, lane_width: f32) {
    let (spp, scroll_x) = zoom_to_fit(content_length(editor), editor.project.sample_rate, lane_width);
    editor.set_view_x(spp, scroll_x);
}

/// Applies a View menu zoom command, anchored at the lane center.
fn apply_view_menu(editor: &mut Editor, action: ViewMenuAction, lane_width: f32) {
    match action {
        ViewMenuAction::None => {}
        ViewMenuAction::ZoomToFit => apply_fit(editor, lane_width),
        ViewMenuAction::ZoomToSelection => {
            if let crate::editor::selection::Selection::Range { start, end, .. } = editor.selection() {
                let (spp, scroll_x) = zoom_to_selection(*start, *end, lane_width);
                editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
            }
        }
        ViewMenuAction::ZoomIn | ViewMenuAction::ZoomOut => {
            let factor = if action == ViewMenuAction::ZoomIn { 1.5 } else { 1.0 / 1.5 };
            let (spp, scroll_x) = zoom_at(editor.view.samples_per_pixel, editor.view.scroll_x, factor, lane_width / 2.0);
            editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
        }
    }
}

/// Wheel, keyboard and click input over the timeline.
#[allow(clippy::too_many_arguments)]
fn handle_input(ui: &egui::Ui, editor: &mut Editor, lane_left: f32, lane_width: f32, max_scroll_y: f32, resp: &egui::Response) {
    if !resp.hovered() {
        return;
    }

    // Ruler clicks are handled by ruler::handle_scrub (audible scrubbing);
    // lane clicks keep their seek-while-playing behavior in
    // interaction::handle_lane_input.
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }

    ui.input(|i| {
        let zoom = i.zoom_delta();
        if zoom != 1.0 {
            // Ctrl + wheel: zoom anchored at the mouse, 1.25 per notch.
            // egui keeps the delta out of smooth_scroll_delta when the zoom
            // modifier is down, so this does exactly one thing.
            if let Some(p) = i.pointer.latest_pos() {
                let offset = (p.x - lane_left).clamp(0.0, lane_width);
                let factor = if zoom > 1.0 { 1.25 } else { 1.0 / 1.25 };
                let (spp, scroll_x) = zoom_at(editor.view.samples_per_pixel, editor.view.scroll_x, factor, offset);
                editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
            }
        } else if i.modifiers.alt {
            // Alt + wheel: vertical waveform scale, 1.25 per notch. This
            // branch replaces the scroll branches, so Alt + wheel never
            // scrolls.
            let d = i.smooth_scroll_delta().y;
            if d != 0.0 {
                let factor = if d > 0.0 { 1.25_f32 } else { 1.0_f32 / 1.25 };
                editor.set_wave_scale(editor.wave_scale * factor);
            }
        } else if i.modifiers.shift {
            // Shift + wheel: egui already folds the wheel delta into the
            // horizontal component; scroll left/right by ~100 px per notch.
            let d = i.smooth_scroll_delta().x;
            if d != 0.0 {
                let spp = editor.view.samples_per_pixel;
                let new_x = (editor.view.scroll_x as f64 - d as f64).round().clamp(0.0, f64::from(u32::MAX)) as u64;
                editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, new_x));
            }
        } else {
            // Plain wheel: scroll tracks vertically.
            let d = i.smooth_scroll_delta().y;
            if d != 0.0 {
                let y = (editor.view.scroll_y - d).clamp(0.0, max_scroll_y);
                editor.set_scroll_y(y);
            }
        }
    });

    // + / - (and = and the numpad keys) zoom by 1.5, anchored on the playhead
    // if visible, else the view center.
    let keys = crate::shortcuts::table();
    let factor = if keys.pressed(ui.ctx(), "zoom_in") {
        Some(1.5)
    } else if keys.pressed(ui.ctx(), "zoom_out") {
        Some(1.0 / 1.5)
    } else {
        None
    };
    if let Some(factor) = factor {
        let g = Geom { lane_left, lane_width, spp: editor.view.samples_per_pixel, scroll_x: editor.view.scroll_x };
        let playhead_x = g.sample_to_x(editor.view.playhead);
        let offset = if playhead_x >= lane_left && playhead_x <= lane_left + lane_width { playhead_x - lane_left } else { lane_width / 2.0 };
        let (spp, scroll_x) = zoom_at(editor.view.samples_per_pixel, editor.view.scroll_x, factor, offset);
        editor.set_view_x(spp, clamp_scroll_x(editor, spp, lane_width, scroll_x));
    }
}

/// Largest legal scroll_x for the given zoom, so the view cannot scroll past
/// the (10 percent padded) end of the project.
pub(crate) fn clamp_scroll_x(editor: &Editor, spp: f64, lane_width: f32, scroll_x: u64) -> u64 {
    let visible = (lane_width as f64 * spp).ceil() as u64;
    let length = content_length(editor);
    scroll_x.min(scroll_extent(length, visible).saturating_sub(visible))
}

/// Draws the header column, lanes, ruler, markers, playhead and scrollbar.
#[allow(clippy::too_many_arguments)]
fn draw(
    ui: &mut egui::Ui,
    editor: &mut Editor,
    g: &Geom,
    ruler_rect: egui::Rect,
    header_rect: egui::Rect,
    lane_rect: egui::Rect,
    scroll_rect: egui::Rect,
    visible: u64,
    max_scroll_x: u64,
    meters: &mut meter::MeterState,
) {
    let neutral = crate::ui::neutrals(ui.ctx());
    let all = ruler_rect.union(header_rect).union(lane_rect).union(scroll_rect);
    let painter = ui.painter_at(all);
    let view = editor.view.clone();

    // The track area backdrop goes under everything: the two surfaces, the
    // header column and the lanes.
    paint_backdrop(&painter, ruler_rect, header_rect, lane_rect.union(scroll_rect));

    // Track lanes: selected tint and separators (ui::timeline::lane), with the
    // clips of each track drawn over its lane. A track is selected while the
    // range selection covers it.
    let selected_tracks = range_tracks(editor);
    let lane_painter = ui.painter_at(lane_rect);
    let mut y = lane_rect.top() - view.scroll_y;
    for track in &editor.project.tracks {
        let bottom = y + track.height;
        if bottom >= lane_rect.top() && y <= lane_rect.bottom() {
            let track_rect = egui::Rect::from_min_max(egui::pos2(g.lane_left, y), egui::pos2(lane_rect.right(), bottom));
            lane::draw(&lane_painter, track_rect, selected_tracks.contains(&track.id));
            // Clips (no waveforms yet, ui::timeline::clip_view).
            let gain_active = interaction::gain_line_active(ui.ctx());
            for clip in &track.clips {
                let (name, offline) = media_info(editor, clip.media_id);
                let loaded = if offline { None } else { editor.media(clip.media_id) };
                let wave_scale = editor.wave_scale();
                let selected = matches!(
                    editor.selection(),
                    crate::editor::selection::Selection::Clips(ids) if ids.contains(&clip.id)
                );
                let hover = ui.input(|i| i.pointer.latest_pos()).and_then(|p| clip_view::hit_test(clip, clip_view::clip_rect(clip, g, track_rect), g.spp, p));
                clip_view::draw(&lane_painter, clip, name, offline, loaded, g, track_rect, wave_scale, selected, hover, track.mute, gain_active);
            }
            // The take being recorded grows over this track's clips.
            if let Some(take) = editor.recording_status()
                && take.track_id == track.id
            {
                clip_view::draw_live(&lane_painter, &take, editor.project.sample_rate, g, track_rect, editor.wave_scale());
            }
        }
        y = bottom;
    }

    // Ghost clip while a bin item is dragged over the lanes.
    interaction::draw_drop_ghost(ui, editor, g, lane_rect, view.scroll_y);

    // Range selection: tint on selected tracks, boundary lines, snap line.
    interaction::draw_range(ui, editor, g, lane_rect, view.scroll_y);

    // Ruler: ticks, labels, markers and their faint lines.
    painter.rect_filled(ruler_rect, 0.0, neutral.bg_panel);
    ruler::draw(&painter, ruler_rect, lane_rect, g, editor.project.sample_rate, editor.project_length(), &editor.project.markers, editor.selected_marker());
    // `stroke.track.header` under the ruler, across the corner cell too.
    let ruler_line =
        egui::Rect::from_min_max(egui::pos2(header_rect.left(), ruler_rect.bottom() - metrics::HAIRLINE), egui::pos2(ruler_rect.right(), ruler_rect.bottom()));
    painter.rect_filled(ruler_line, 0.0, neutral.stroke_track_header);

    // The divider between the header column and the lanes, from the top of the
    // ruler row to the bottom of the track area. After the ruler line, so that
    // line does not cross it.
    let gutter = egui::Rect::from_min_max(egui::pos2(header_rect.right(), ruler_rect.top()), egui::pos2(g.lane_left, scroll_rect.bottom()));
    painter.rect_filled(gutter, 0.0, neutral.bg_ground);

    // Range selection: a 3 px accent bar along the ruler bottom across the
    // range (the tint and boundary lines are drawn by ui::timeline::interaction).
    if let crate::editor::selection::Selection::Range { start, end, .. } = editor.selection() {
        let x0 = g.sample_to_x(*start).max(g.lane_left);
        let x1 = g.sample_to_x(*end).min(lane_rect.right());
        if x1 > x0 {
            painter.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x0, ruler_rect.bottom() - 3.0), egui::pos2(x1, ruler_rect.bottom())),
                0.0,
                ui_kit::Theme::get(painter.ctx()).accent.accent,
            );
        }
    }

    // Playhead: the ui-kit component, a line through ruler and lanes with a
    // rounded head in the ruler. Clipped to the lane columns so the head never
    // spills over the header column.
    let ph_x = g.sample_to_x(view.playhead);
    if ph_x >= g.lane_left - 1.0 && ph_x <= lane_rect.right() + 1.0 {
        let clip = egui::Rect::from_min_max(egui::pos2(g.lane_left, ruler_rect.top()), egui::pos2(lane_rect.right(), lane_rect.bottom()));
        ui_kit::paint_playhead(&painter.with_clip_rect(clip), ruler_rect, lane_rect, ph_x, &ui_kit::Theme::get(ui.ctx()));
    }

    // Horizontal scrollbar: the ui-kit thumb, no track fill. The thumb keeps
    // its old hit area (the strip height by the thumb length); the 6 px
    // capsule is painted centered in it.
    let frac = (visible as f32 / scroll_extent(content_length(editor), visible) as f32).min(1.0);
    let thumb_w = (scroll_rect.width() * frac).max(metrics::SCROLLBAR_THUMB_MIN);
    let scroll_max = max_scroll_x as f32;
    let thumb_x = g.lane_left + if scroll_max > 0.0 { (view.scroll_x as f32 / scroll_max) * (scroll_rect.width() - thumb_w) } else { 0.0 };
    let thumb_rect = egui::Rect::from_min_max(
        egui::pos2(thumb_x, scroll_rect.top() + metrics::HAIRLINE),
        egui::pos2(thumb_x + thumb_w, scroll_rect.bottom() - metrics::HAIRLINE),
    );
    let thumb = ui.interact(thumb_rect, egui::Id::new("timeline_hscroll_thumb"), egui::Sense::click_and_drag());
    let capsule = egui::Rect::from_center_size(thumb_rect.center(), egui::vec2(thumb_rect.width(), size::SCROLLBAR_THUMB_WIDTH));
    ui_kit::scroll_thumb(&painter, capsule, thumb.hovered() || thumb.dragged());
    if thumb.dragged() {
        let d = thumb.drag_delta().x;
        if d != 0.0 {
            let new_x = (view.scroll_x as f64 - d as f64 * g.spp).round().clamp(0.0, max_scroll_x as f64) as u64;
            editor.set_view_x(view.samples_per_pixel, new_x);
        }
    }

    // Track headers: grip, rename, menu, resize, controls, meter, reorder.
    track_header::show(ui, editor, header_rect, lane_rect, view.scroll_y, &mut meters.tracks);
}

/// Paints the track area backdrop: the header column as `bg.panel` with the
/// `bg.group` corner cell (labelled "Tracks") beside the ruler, and the lanes
/// surface (`bg.stage.inner`) over `lanes`. The track headers paint their own
/// `bg.group` blocks over the column, so it stays `bg.panel` below the last one.
fn paint_backdrop(painter: &egui::Painter, ruler_rect: egui::Rect, header_rect: egui::Rect, lanes: egui::Rect) {
    let neutral = crate::ui::neutrals(painter.ctx());
    painter.rect_filled(lanes, 0.0, neutral.bg_stage_inner);
    painter.rect_filled(header_rect, 0.0, neutral.bg_panel);
    let corner = egui::Rect::from_min_max(egui::pos2(header_rect.left(), ruler_rect.top()), egui::pos2(header_rect.right(), ruler_rect.bottom()));
    painter.rect_filled(corner, 0.0, neutral.bg_group);
    painter.text(
        egui::pos2(corner.left() + space::SCALE_8, corner.center().y),
        egui::Align2::LEFT_CENTER,
        "Tracks",
        ui_kit::fonts::medium(typography::CAPTION_SIZE),
        neutral.text_tertiary,
    );
}

/// Display name and offline flag of a media item, for clip drawing.
fn media_info(editor: &Editor, media_id: Uuid) -> (&str, bool) {
    let name = editor.project.media.iter().find(|media| media.id == media_id).map(|media| media.display_name.as_str()).unwrap_or("");
    (name, editor.is_media_offline(media_id))
}

/// Ids of the tracks the range selection covers, empty for any other
/// selection.
pub(crate) fn range_tracks(editor: &Editor) -> Vec<Uuid> {
    match editor.selection() {
        crate::editor::selection::Selection::Range { tracks, .. } => tracks.clone(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geom(spp: f64, scroll_x: u64) -> Geom {
        Geom { lane_left: 10.0, lane_width: 1000.0, spp, scroll_x }
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_1() {
        let g = geom(1.0, 0);
        assert_eq!(g.sample_to_x(500), 510.0);
        assert_eq!(g.x_to_sample(510.0), 500);
        assert_eq!(g.x_to_sample(g.sample_to_x(1_000)), 1_000);
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_100() {
        let g = geom(100.0, 2_000);
        assert_eq!(g.sample_to_x(5_000), 40.0);
        assert_eq!(g.x_to_sample(40.0), 5_000);
    }

    #[test]
    fn sample_to_x_and_x_to_sample_round_trip_at_spp_quarter() {
        let g = geom(0.25, 0);
        assert_eq!(g.sample_to_x(7), 38.0);
        assert_eq!(g.x_to_sample(38.0), 7);
    }

    #[test]
    fn x_to_sample_clamps_at_zero() {
        let g = geom(1.0, 0);
        assert_eq!(g.x_to_sample(0.0), 0);
        assert_eq!(g.x_to_sample(-50.0), 0);
    }

    #[test]
    fn x_left_of_the_view_maps_inside_the_timeline() {
        // Only x below lane_left - scroll_x/spp clamps; otherwise it is a
        // legitimate sample before the view start.
        let g = geom(1.0, 1_000);
        assert_eq!(g.x_to_sample(0.0), 990);
    }

    #[test]
    fn visible_range_follows_scroll_and_zoom() {
        assert_eq!(geom(1.0, 0).visible_range(), (0, 1000));
        assert_eq!(geom(10.0, 2_500).visible_range(), (2_500, 12_500));
    }

    #[test]
    fn zoom_at_keeps_the_anchor_sample_at_factors_half_and_two() {
        let spp = 100.0;
        let scroll_x = 100_000;
        let offset = 50.0;
        let anchor = scroll_x as f64 + offset as f64 * spp;

        let (spp_in, scroll_in) = zoom_at(spp, scroll_x, 2.0, offset);
        let kept = scroll_in as f64 + offset as f64 * spp_in;
        assert!((kept - anchor).abs() <= 1.0);

        let (spp_out, scroll_out) = zoom_at(spp, scroll_x, 0.5, offset);
        let kept = scroll_out as f64 + offset as f64 * spp_out;
        assert!((kept - anchor).abs() <= 1.0);
    }

    #[test]
    fn zoom_at_clamps_at_min_and_max_spp() {
        let (spp, _) = zoom_at(MAX_SPP, 1_000, 2.0, 100.0);
        assert_eq!(spp, MAX_SPP);
        let (spp, _) = zoom_at(MIN_SPP, 1_000, 0.5, 100.0);
        assert_eq!(spp, MIN_SPP);
    }

    #[test]
    fn zoom_at_never_makes_scroll_x_negative() {
        let (_, scroll_x) = zoom_at(1_000_000.0, 1_000, 2.0, 10.0);
        assert_eq!(scroll_x, 0);
        let (_, scroll_x) = zoom_at(1_000_000.0, 0, 2.0, 0.0);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn tick_step_48k_spp_10_min_90_px() {
        assert_eq!(tick_step(10.0, 48_000, 90.0), 960);
    }

    #[test]
    fn tick_step_48k_spp_48000_min_90_px() {
        assert_eq!(tick_step(48_000.0, 48_000, 90.0), 5_760_000);
    }

    #[test]
    fn tick_step_falls_back_to_largest() {
        // Extremely zoomed in: even 1 ms is far more than 90 px.
        assert_eq!(tick_step(0.1, 48_000, 90.0), 48);
        // Extremely zoomed out: 3600 s is the last step.
        assert_eq!(tick_step(4_194_304.0, 48_000, 90.0), 172_800_000);
    }

    #[test]
    fn zoom_to_fit_fits_the_project_with_a_margin() {
        // 1 000 000 samples over 1 000 px: 1000 spp * 1.05, scroll at 0.
        let (spp, scroll_x) = zoom_to_fit(1_000_000, 48_000, 1_000.0);
        assert!((spp - 1_050.0).abs() < 1e-9);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn zoom_to_fit_uses_ten_seconds_for_short_projects() {
        // Empty project: 10 s at 48 kHz over 1 000 px * 1.05.
        let (spp, _) = zoom_to_fit(0, 48_000, 1_000.0);
        let expected = 480_000.0 / 1_000.0 * 1.05;
        assert!((spp - expected).abs() < 1e-9);
    }

    #[test]
    fn zoom_to_selection_fits_the_range_with_a_margin() {
        // 100 000 samples over 1 000 px: spp = 100 * 1.1; start 0 stays 0.
        let (spp, scroll_x) = zoom_to_selection(0, 100_000, 1_000.0);
        assert!((spp - 110.0).abs() < 1e-9);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn zoom_to_selection_scrolls_so_the_start_is_five_percent_in() {
        // spp 1.1 -> visible 1 100; scroll = 200 - 55 = 145.
        let (spp, scroll_x) = zoom_to_selection(200, 1_200, 1_000.0);
        assert!((spp - 1.1).abs() < 1e-9);
        assert_eq!(scroll_x, 145);
    }

    #[test]
    fn zoom_to_selection_never_scrolls_below_zero() {
        // The start is before the 5 percent margin would be.
        let (_, scroll_x) = zoom_to_selection(40, 1_040, 1_000.0);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn zoom_to_selection_clamps_spp() {
        // A one-sample range at a huge lane width clamps at MIN_SPP.
        let (spp, _) = zoom_to_selection(0, 1, 1_000_000_000.0);
        assert_eq!(spp, MIN_SPP);
    }

    #[test]
    fn zoom_to_selection_handles_empty_and_zero_width() {
        assert_eq!(zoom_to_selection(500, 500, 1_000.0).0, MIN_SPP);
        assert_eq!(zoom_to_selection(0, 100, 0.0), (MAX_SPP, 0));
    }

    #[test]
    fn zoom_to_fit_clamps_to_min_and_max_spp() {
        // Far too small a span for the width: clamps at MIN_SPP.
        let (spp, _) = zoom_to_fit(1, 48_000, 100_000_000.0);
        assert_eq!(spp, MIN_SPP);
        // Far too large: clamps at MAX_SPP.
        let (spp, _) = zoom_to_fit(u64::MAX, 48_000, 1.0);
        assert_eq!(spp, MAX_SPP);
        // Zero-width lane area: the largest zoom.
        let (spp, scroll_x) = zoom_to_fit(1_000, 48_000, 0.0);
        assert_eq!(spp, MAX_SPP);
        assert_eq!(scroll_x, 0);
    }

    #[test]
    fn zoom_slider_maps_the_zoom_limits_to_its_ends_and_back() {
        let range = zoom_slider_range();
        assert_eq!(*range.start(), zoom_to_slider(MAX_SPP), "zoomed out is the left end");
        assert_eq!(*range.end(), zoom_to_slider(MIN_SPP), "zoomed in is the right end");
        for spp in [MIN_SPP, 0.5, 1.0, 480.0, 1_050.0, MAX_SPP] {
            assert!((slider_to_zoom(zoom_to_slider(spp)) / spp - 1.0).abs() < 1e-5, "round trip at {spp}");
        }
        // A bad saved zoom clamps instead of producing infinity.
        assert_eq!(zoom_to_slider(0.0), *range.end());
    }

    #[test]
    fn zoom_readout_reads_percent_of_fit() {
        assert_eq!(zoom_readout(480.0, 480.0), "100%", "Fit is 100%");
        assert_eq!(zoom_readout(240.0, 480.0), "200%", "zoomed in reads higher");
        assert_eq!(zoom_readout(1_920.0, 480.0), "25%");
        assert_eq!(zoom_readout(4_800.0, 480.0), "10%");
        assert_eq!(zoom_readout(9_600.0, 480.0), "5.0%");
        assert_eq!(zoom_readout(480_000.0, 480.0), "0.10%");
    }

    /// One frame of the whole timeline over three tracks, in a 1400 x 700 window.
    fn track_area_frame() -> crate::ui::test_util::Out {
        let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[(0, 48_000)], &[], &[]], "area");
        let mut meters = meter::MeterState::default();
        let mut rig = crate::ui::test_util::Headless::new(1400.0, 700.0);
        rig.frame(Vec::new(), |ui| {
            show(ui, &mut editor, ViewMenuAction::None, &mut meters);
        })
    }

    /// The corner cell and the lanes surface of one painted frame.
    fn surfaces(rects: &[egui::epaint::RectShape]) -> (egui::Rect, egui::Rect) {
        let corner = rects
            .iter()
            .find(|r| r.fill == color::BG_GROUP && r.rect.size() == egui::vec2(metrics::HEADER_COL_W, size::RULER_HEIGHT))
            .expect("the corner cell");
        let lanes = rects.iter().filter(|r| r.fill == color::BG_STAGE_INNER).map(|r| r.rect).max_by(|a, b| a.area().total_cmp(&b.area()));
        (corner.rect, lanes.expect("the lanes surface"))
    }

    #[test]
    fn the_header_column_and_the_lanes_are_two_surfaces() {
        let out = track_area_frame();
        let rects = out.rects();
        let (corner, lanes) = surfaces(&rects);
        assert_eq!(lanes.left(), corner.right() + metrics::COLUMN_GUTTER_W, "the lanes start past the divider");
        assert_eq!(lanes.top(), corner.bottom(), "the corner cell is exactly as tall as the ruler");
        let column = rects
            .iter()
            .find(|r| r.fill == color::BG_PANEL && r.rect.left() == corner.left() && r.rect.width() == metrics::HEADER_COL_W && r.rect.top() == corner.bottom())
            .expect("the bg.panel header column")
            .rect;
        assert_eq!(column.bottom(), lanes.bottom(), "the column reaches the bottom of the track area");
        let blocks = rects.iter().filter(|r| r.fill == color::BG_GROUP && r.rect.width() == metrics::HEADER_COL_W && r.rect.top() >= corner.bottom()).count();
        assert_eq!(blocks, 3, "one bg.group block per track");
    }

    #[test]
    fn the_corner_cell_holds_the_tracks_label_and_the_add_row_sits_under_the_last_track() {
        let out = track_area_frame();
        let rects = out.rects();
        let (corner, _) = surfaces(&rects);
        let label = out.texts().into_iter().find(|t| t.text == "Tracks").expect("the Tracks label");
        assert_eq!(label.color, color::TEXT_TERTIARY);
        assert!(corner.contains_rect(label.rect), "the label is in the corner cell");
        let under_ruler = rects
            .iter()
            .find(|r| r.fill == color::STROKE_TRACK_HEADER && r.rect.height() == metrics::HAIRLINE && r.rect.left() == corner.left())
            .expect("stroke.track.header under the ruler")
            .rect;
        assert_eq!(under_ruler.bottom(), corner.bottom(), "the line is the ruler's bottom edge");
        assert!(under_ruler.width() > corner.width() + 100.0, "and runs over the lanes too");
        let tracks_bottom = corner.bottom() + 3.0 * size::TRACK_AUDIO_DEFAULT;
        let add = out.text_rect("+ Add track").expect("the Add track label");
        assert!(add.top() > tracks_bottom && add.bottom() < tracks_bottom + track_header::ADD_ROW_H, "under the last track: {add:?}");
        assert!(add.right() < corner.right(), "in the header column");
        let line_below = rects.iter().any(|r| r.fill == color::BG_GROUND && r.rect.height() == metrics::HAIRLINE && r.rect.bottom() > tracks_bottom);
        assert!(!line_below, "no line under the Add track row");
    }

    #[test]
    fn every_track_has_one_row_line_across_header_and_lanes() {
        let out = track_area_frame();
        let rects = out.rects();
        let (corner, lanes) = surfaces(&rects);
        for track in 1..=3 {
            let bottom = corner.bottom() + track as f32 * size::TRACK_AUDIO_DEFAULT;
            let lines: Vec<_> = rects
                .iter()
                .filter(|r| r.fill == color::BG_GROUND && r.rect.height() == metrics::HAIRLINE && r.rect.bottom() == bottom)
                .map(|r| r.rect)
                .collect();
            assert!(lines.iter().any(|l| l.left() == corner.left() && l.right() == corner.right()), "the header part of line {track}: {lines:?}");
            assert!(lines.iter().any(|l| l.left() == lanes.left() && l.right() == lanes.right()), "the lane part of line {track}: {lines:?}");
        }
    }

    /// The divider is there at full height however many tracks there are: from
    /// the top of the ruler row to the bottom of the track area.
    #[test]
    fn the_gutter_kittest_spans_the_track_area_with_0_1_and_3_tracks() {
        use std::cell::Cell;

        /// Every painted rect with its clip rect, in paint order, nested shapes
        /// flattened.
        fn painted(output: &egui::FullOutput) -> Vec<(egui::Rect, egui::epaint::RectShape)> {
            let mut rects = Vec::new();
            for clipped in &output.shapes {
                let mut stack = vec![&clipped.shape];
                while let Some(shape) = stack.pop() {
                    match shape {
                        egui::Shape::Vec(inner) => stack.extend(inner.iter().rev()),
                        egui::Shape::Rect(rect) => rects.push((clipped.clip_rect, rect.clone())),
                        _ => {}
                    }
                }
            }
            rects
        }

        let layouts: [&[&[(u64, u64)]]; 3] = [&[], &[&[(0, 48_000)]], &[&[(0, 48_000)], &[], &[]]];
        for tracks in layouts {
            let count = tracks.len();
            let paths = crate::io::paths::ProjectPaths { root: std::path::PathBuf::from("unused") };
            let project = crate::model::fixtures::project_with_clips(tracks);
            let mut editor = Editor::with_project(project, crate::model::project::ViewState::default(), paths);
            let mut meters = meter::MeterState::default();
            let first_pass = Cell::new(true);
            let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1108.0, 700.0)).build_ui(move |ui| {
                // The first pass installs the ui-kit theme and fonts; egui
                // applies a font change at the start of the next pass.
                if first_pass.get() {
                    first_pass.set(false);
                    ui_kit::apply(ui.ctx());
                    return;
                }
                show(ui, &mut editor, ViewMenuAction::None, &mut meters);
            });
            harness.step();
            harness.step();

            let painted = painted(harness.output());
            let rects: Vec<_> = painted.iter().map(|(_, rect)| rect.clone()).collect();
            let (corner, lanes) = surfaces(&rects);
            let gutters: Vec<_> = rects.iter().enumerate().filter(|(_, r)| r.fill == color::BG_GROUND && r.rect.width() == metrics::COLUMN_GUTTER_W).collect();
            assert_eq!(gutters.len(), 1, "one divider with {count} tracks: {gutters:?}");
            let (painted_at, gutter) = (gutters[0].0, gutters[0].1.rect);
            assert_eq!((gutter.left(), gutter.right()), (corner.right(), lanes.left()), "between the column and the lanes with {count} tracks");
            assert_eq!(gutter.top(), corner.top(), "from the top of the ruler row with {count} tracks");
            assert_eq!(gutter.bottom(), lanes.bottom(), "to the bottom of the track area with {count} tracks");
            assert!(gutter.height() > 400.0, "far below the last track with {count} tracks: {gutter:?}");
            // Nothing painted later covers it (row lines are the same color).
            // The clip rect counts: the playhead head at 0 is cut at the lanes.
            for (clip, later) in &painted[painted_at + 1..] {
                let overlap = later.rect.intersect(*clip).intersect(gutter);
                let covers = later.fill.a() > 0 && later.fill != color::BG_GROUND && overlap.width() > 0.01 && overlap.height() > 0.01;
                assert!(!covers, "{later:?} covers the divider with {count} tracks");
            }
        }
    }

    #[test]
    fn the_timecode_splits_before_the_fraction() {
        assert_eq!(split_timecode("00:01:02.345"), ("00:01:02", ".345"));
        assert_eq!(split_timecode("00:01:02"), ("00:01:02", ""));
    }

    /// The bottom bar's items sit on one center line, 32 above the panel's
    /// bottom edge, and stay inside their side of the notch, idle and while a
    /// take is recorded. 704 wide is the panel in the smallest window (1100)
    /// with the Media panel at its default width.
    #[test]
    fn the_bar_items_sit_on_one_center_line_clear_of_the_notch() {
        for recording in [false, true] {
            let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[(0, 144_000)]], "bar");
            if recording {
                editor.start_recording().expect("a take");
            }
            let mut meters = meter::MeterState::default();
            let mut rig = crate::ui::test_util::Headless::new(704.0, 600.0);
            let mut draw = |ui: &mut egui::Ui| {
                show(ui, &mut editor, ViewMenuAction::None, &mut meters);
            };
            let _ = rig.frame(Vec::new(), &mut draw);
            let out = rig.frame(Vec::new(), &mut draw);
            // The bar is the panel's last 64; the panel fills the window.
            let center_y = 600.0 - 32.0;
            let in_bar = |text: &str| out.texts().into_iter().map(|t| (t.text, t.rect)).find(|(t, r)| t == text && r.center().y > 600.0 - 64.0).map(|(_, r)| r);
            let record = if recording { fill::STOP } else { fill::RECORD };
            let timecode = in_bar("00:00:00.000").expect("the timecode");
            let back = in_bar(fill::SKIP_BACK).expect("back to start");
            let stop = in_bar(fill::STOP).expect("Stop");
            let record = out.texts().into_iter().filter(|t| t.text == record).map(|t| t.rect).max_by(|a, b| a.center().x.total_cmp(&b.center().x));
            let record = record.expect("Record");
            let forward = in_bar(fill::SKIP_FORWARD).expect("go to end");
            let length = in_bar("00:00:03.000").expect("the total length");
            let items = [timecode, back, stop, record, forward, length];
            for rect in items {
                assert!((rect.center().y - center_y).abs() <= 1.5, "{rect:?} is off the bar's center line {center_y}");
            }
            for pair in items.windows(2) {
                assert!(pair[0].right() < pair[1].left(), "left to right: {pair:?}");
            }
            let notch = (352.0 - 56.0)..=(352.0 + 56.0);
            assert!(back.right() < *notch.start() && stop.left() > *notch.end(), "the center column is empty: {back:?} {stop:?}");
            assert!(length.right() <= 704.0 - 8.0, "the length fits its side: {length:?}");
        }
    }

    /// The left side shrinks its timecode only when the large one and back to
    /// start no longer fit it.
    #[test]
    fn a_narrow_left_side_shrinks_the_timecode() {
        // A timecode of 100 and back to start take 152.
        assert_eq!(timecode_size(152.0, 100.0), ui_kit::TimecodeSize::Large);
        assert_eq!(timecode_size(151.0, 100.0), ui_kit::TimecodeSize::Value);
    }

    /// The right side gives up the total length, then go to end, each only
    /// when it no longer fits that side.
    #[test]
    fn a_narrow_right_side_gives_up_the_length_then_go_to_end() {
        // Three buttons take 136, and 248 with a length of 100.
        let fit = |column| right_fit(column, 100.0);
        assert_eq!(fit(248.0), RightFit::Full);
        assert_eq!(fit(247.0), RightFit::NoLength);
        assert_eq!(fit(136.0), RightFit::NoLength);
        assert_eq!(fit(135.0), RightFit::NoJumpEnd);
        assert_eq!(fit(0.0), RightFit::NoJumpEnd, "Stop and Record have no further step");
    }

    #[test]
    fn panel_toolbar_chips_toggle_ripple_and_snap() {
        let (mut editor, _dir) = crate::ui::test_util::armed_editor(&[&[(0, 48_000)]], "chips");
        let mut meters = meter::MeterState::default();
        let mut rig = crate::ui::test_util::Headless::new(1400.0, 700.0);
        let mut run = |rig: &mut crate::ui::test_util::Headless, editor: &mut Editor, click: Option<egui::Pos2>| {
            let mut draw = |ui: &mut egui::Ui| {
                show(ui, editor, ViewMenuAction::None, &mut meters);
            };
            match click {
                Some(pos) => rig.click(pos, &mut draw),
                None => rig.frame(Vec::new(), &mut draw),
            }
        };
        let out = run(&mut rig, &mut editor, None);
        let ripple_at = out.text_rect("Ripple").expect("the Ripple chip").center();
        let snap_at = out.text_rect("Snap").expect("the Snap chip").center();
        let (ripple, snap) = (editor.view.ripple, editor.view.snap);
        let _ = run(&mut rig, &mut editor, Some(ripple_at));
        assert_eq!(editor.view.ripple, !ripple, "the Ripple chip toggles ripple");
        assert_eq!(editor.view.snap, snap, "and only ripple");
        let _ = run(&mut rig, &mut editor, Some(snap_at));
        assert_eq!(editor.view.snap, !snap, "the Snap chip toggles snap");
    }
}
