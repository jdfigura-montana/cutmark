//! ui::timeline: the central timeline view (ruler, lanes, playhead, zoom, scroll).

use eframe::egui;
use ui_kit::icons::regular;
use ui_kit::tokens::{color, size, space, typography};
use uuid::Uuid;

use crate::editor::Editor;
use crate::editor::message::Message;
use crate::editor::transport::TransportState;
use crate::ui::theme::THEME;
use crate::ui::widgets::meter;
use crate::util::time::seconds_to_samples;

pub mod clip_view;
pub mod deck;
pub mod interaction;
pub mod lane;
pub mod ruler;
pub mod track_header;
pub mod waveform;

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

/// Width of the zoom slider in the panel toolbar, in points.
const ZOOM_SLIDER_W: f32 = 140.0;
/// Width of the waveform height slider in the panel toolbar, in points.
const WAVE_SLIDER_W: f32 = 90.0;
/// Width reserved for the zoom readout ("100%"), in points.
const ZOOM_READOUT_W: f32 = 56.0;
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

    // The timeline sits in a ui-kit panel: the panel toolbar row on top, the
    // ruler, headers and lanes in the rest.
    let mut messages = Vec::new();
    ui_kit::panel(ui, |ui| {
        let content_rect = ui.max_rect();
        let lane_width = ui.available_width() - THEME.header_col_w;
        panel_toolbar(ui, editor, lane_width);
        messages.extend(show_frame(ui, editor, view_menu, meters).into_iter().map(Message::media));
        // The transport deck floats over the panel's content area.
        messages.extend(deck::show(ui, editor, content_rect, ruler::is_scrubbing(ui.ctx())));
    });
    messages
}

/// The panel toolbar row (44 tall): the Ripple and Snap chips on the left, the
/// waveform height slider, the zoom slider, its readout and the Fit button on
/// the right. The chips read
/// the editor's view state every frame, so they stay in sync with the View
/// menu and the R and N keys. Zoom stays live while a take is recorded; the
/// chips do not.
fn panel_toolbar(ui: &mut egui::Ui, editor: &mut Editor, lane_width: f32) {
    let recording = editor.is_recording();
    let row = egui::vec2(ui.available_width(), size::PANEL_TOOLBAR_HEIGHT);
    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = space::SCALE_8;
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
        // zoom slider, readout, Fit.
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
                color::TEXT_SECONDARY,
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
            wave_scale_slider(ui, editor);
        });
    });
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
    let (icon, _) = ui.allocate_exact_size(egui::vec2(size::ICON_BUTTON / 2.0, size::SEGMENT_HEIGHT), egui::Sense::hover());
    let font = ui_kit::fonts::icon_regular(typography::HEADLINE_SIZE);
    ui.painter().text(icon.center(), egui::Align2::CENTER_CENTER, regular::WAVEFORM, font, color::TEXT_SECONDARY);
}

/// The ruler, header column, lanes and scrollbar in the space the panel
/// toolbar left over, with all their input.
fn show_frame(ui: &mut egui::Ui, editor: &mut Editor, view_menu: ViewMenuAction, meters: &mut meter::MeterState) -> Vec<String> {
    let avail = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(avail, egui::Sense::click());

    let lane_left = avail.left() + THEME.header_col_w;
    let lane_right = avail.right();
    let lane_width = lane_right - lane_left;
    let ruler_rect = egui::Rect::from_min_max(egui::pos2(lane_left, avail.top()), egui::pos2(lane_right, avail.top() + THEME.ruler_h));
    let header_rect = egui::Rect::from_min_max(egui::pos2(avail.left(), ruler_rect.bottom()), egui::pos2(lane_left, avail.bottom()));
    let lane_rect = egui::Rect::from_min_max(egui::pos2(lane_left, ruler_rect.bottom()), egui::pos2(lane_right, avail.bottom() - THEME.scrollbar_h));
    let scroll_rect = egui::Rect::from_min_max(egui::pos2(lane_left, lane_rect.bottom()), egui::pos2(lane_right, avail.bottom()));

    let content_h: f32 = editor.project.tracks.iter().map(|t| t.height).sum::<f32>() + track_header::ADD_ROW_H;
    let max_scroll_y = (content_h - lane_rect.height()).max(0.0);

    // Old project files may carry an out-of-range zoom (or 0); fit instead.
    if !(MIN_SPP..=MAX_SPP).contains(&editor.view.samples_per_pixel) {
        apply_fit(editor, lane_width);
    }
    // \ zooms to fit, Ctrl+\ zooms to the selection, wherever the pointer is.
    if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Backslash)) {
        if ui.input(|i| i.modifiers.ctrl) {
            apply_view_menu(editor, ViewMenuAction::ZoomToSelection, lane_width);
        } else {
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
    // A press on the transport deck belongs to the deck, not to what lies
    // under it: the pointer handlers skip that frame's press.
    let deck_press = ui.input(|i| i.pointer.primary_pressed()) && ui.input(|i| i.pointer.latest_pos()).is_some_and(|p| deck::covers(ui.ctx(), p));
    let recording = editor.is_recording() || deck_press;
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

        // + / - (and = and the numpad keys) zoom by 1.5, anchored on the
        // playhead if visible, else the view center.
        let factor = if i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals) {
            Some(1.5)
        } else if i.key_pressed(egui::Key::Minus) {
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
    });
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
    let all = ruler_rect.union(header_rect).union(lane_rect).union(scroll_rect);
    let painter = ui.painter_at(all);
    let view = editor.view.clone();

    // No background fill: the panel's `bg.panel` shows through the unshaded
    // track rows, the header column and the scrollbar strip.

    // Track lanes: track rows and separators (ui::timeline::lane), with the
    // clips of each track drawn over its lane. A track is selected while the
    // range selection covers it.
    let selected_tracks = range_tracks(editor);
    let lane_painter = ui.painter_at(lane_rect);
    let mut y = lane_rect.top() - view.scroll_y;
    for (i, track) in editor.project.tracks.iter().enumerate() {
        let bottom = y + track.height;
        if bottom >= lane_rect.top() && y <= lane_rect.bottom() {
            let track_rect = egui::Rect::from_min_max(egui::pos2(g.lane_left, y), egui::pos2(lane_rect.right(), bottom));
            lane::draw(&lane_painter, track_rect, i, selected_tracks.contains(&track.id), egui::Stroke::new(THEME.hairline, THEME.border));
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
    painter.rect_filled(ruler_rect, 0.0, THEME.ruler_bg);
    ruler::draw(&painter, ruler_rect, lane_rect, g, editor.project.sample_rate, editor.project_length(), &editor.project.markers, editor.selected_marker());

    // Range selection: a 3 px accent bar along the ruler bottom across the
    // range (the tint and boundary lines are drawn by ui::timeline::interaction).
    if let crate::editor::selection::Selection::Range { start, end, .. } = editor.selection() {
        let x0 = g.sample_to_x(*start).max(g.lane_left);
        let x1 = g.sample_to_x(*end).min(lane_rect.right());
        if x1 > x0 {
            painter.rect_filled(egui::Rect::from_min_max(egui::pos2(x0, ruler_rect.bottom() - 3.0), egui::pos2(x1, ruler_rect.bottom())), 0.0, THEME.accent);
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
    let thumb_w = (scroll_rect.width() * frac).max(THEME.scrollbar_thumb_min);
    let scroll_max = max_scroll_x as f32;
    let thumb_x = g.lane_left + if scroll_max > 0.0 { (view.scroll_x as f32 / scroll_max) * (scroll_rect.width() - thumb_w) } else { 0.0 };
    let thumb_rect =
        egui::Rect::from_min_max(egui::pos2(thumb_x, scroll_rect.top() + THEME.hairline), egui::pos2(thumb_x + thumb_w, scroll_rect.bottom() - THEME.hairline));
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
