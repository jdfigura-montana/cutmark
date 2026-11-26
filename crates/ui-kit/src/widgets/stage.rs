//! The stage panel (spec: Design Language E3 amendment, "1. The notch"): the
//! app's main content panel, ending in a bottom bar with a round notch cut out
//! of its bottom edge, and the primary transport button sitting in the notch.

use egui::text::LayoutJob;
use egui::{
    Color32, CornerRadius, Id, LayerId, Pos2, Rect, Response, Sense, Shadow, Shape, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, WidgetInfo, WidgetType,
    pos2, vec2,
};

use crate::HAIRLINE;
use crate::Theme;
use crate::fonts;
use crate::tokens::{color, radius, space, typography};
use crate::widgets::{PlayKind, capsule_radius, focus};

/// Height of the stage's bottom bar.
const BAR_HEIGHT: f32 = 64.0;
/// Width of the empty center column of the bottom bar, kept for the notch.
const BAR_CENTER_WIDTH: f32 = 112.0;
/// Radius of the notch cut out of the panel's bottom edge.
const NOTCH_RADIUS: f32 = 42.0;
/// Diameter of the primary button.
const PRIMARY_DIAMETER: f32 = 68.0;
/// How far the primary button's center (and the notch's, so the two stay
/// concentric) sits above the panel's bottom edge.
const PRIMARY_CENTER_RISE: f32 = 8.0;
/// How far the primary button reaches below the stage panel. The stage does
/// not allocate this space: whatever sits under it must keep its center free.
pub const STAGE_PRIMARY_OVERHANG: f32 = PRIMARY_DIAMETER / 2.0 - PRIMARY_CENTER_RISE;

/// The primary button's shadow: offset 0, 8, blur 24.
const SHADOW_OFFSET_Y: i8 = 8;
/// The primary button's shadow blur.
const SHADOW_BLUR: u8 = 24;
/// The primary button's shadow is the button color at this strength.
const SHADOW_STRENGTH: f32 = 0.25;
/// How far below the panel the button and its shadow may paint.
const PRIMARY_PAINT_BELOW: f32 = STAGE_PRIMARY_OVERHANG + SHADOW_OFFSET_Y as f32 + SHADOW_BLUR as f32;
/// Opacity of the disabled primary button (spec "States": 40%, no hover).
const DISABLED_OPACITY: f32 = 0.4;
/// Size of the play and pause glyphs in the primary button.
const PRIMARY_GLYPH_SIZE: f32 = 28.0;
/// Diameter of the idle record button's white dot.
const RECORD_DOT_DIAMETER: f32 = 24.0;
/// Side of the white rounded square shown while recording.
const RECORD_SQUARE_SIDE: f32 = 22.0;
/// Corner radius of the recording square.
const RECORD_SQUARE_RADIUS: u8 = 6;

/// Diameter of a bottom bar button, for callers that work out what fits in a
/// bar side.
pub const STAGE_BAR_BUTTON_DIAMETER: f32 = 40.0;
/// Glyph size inside a bottom bar button.
const BAR_ICON_SIZE: f32 = 18.0;
/// Size of the bottom bar timecode (spec "Type": 22px Medium).
const TIMECODE_SIZE: f32 = 22.0;

/// Straight segments used to draw the notch arc.
const NOTCH_SEGMENTS: u16 = 48;

/// The primary transport button in the stage's notch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimaryButton {
    /// [`PlayKind::Play`] for the editors, [`PlayKind::Record`] for the Recorder.
    pub kind: PlayKind,
    /// True while playing (the button shows pause) or recording (it shows the
    /// rounded square).
    pub active: bool,
    /// A disabled button draws at 40% opacity and reports no clicks.
    pub enabled: bool,
}

/// What [`stage_panel`] drew.
#[derive(Debug, Clone)]
pub struct StageResponse {
    /// The stage panel. The primary button reaches below it.
    pub panel_rect: Rect,
    /// The whole bottom bar inside the panel.
    pub bar_rect: Rect,
    /// The bar's empty center column. Neither bar side can draw into it.
    pub bar_center_rect: Rect,
    /// The primary button's response.
    pub primary: Response,
}

impl StageResponse {
    /// True when the primary button was clicked this frame.
    pub fn clicked(&self) -> bool {
        self.primary.clicked()
    }
}

/// The stage panel: `bg.panel`, `radius.panel` and a hairline rim, filling the
/// available space. `add_content` gets the area above a 64 tall bottom bar.
/// `add_bar_left` and `add_bar_right` lay out the bar's two sides left to
/// right; the right side starts next to the notch. Each side is clipped so the
/// 112 wide center column stays empty.
///
/// A round notch is cut out of the bottom edge by painting it in `bg.ground`,
/// so the stage must sit on the window ground. The primary button sits in the
/// notch on its own layer and reaches [`STAGE_PRIMARY_OVERHANG`] below the
/// panel: it draws above everything on the stage's layer and below every
/// window, popup, menu, dialog and tooltip over it. `id` must be unique per
/// stage.
///
/// Usage:
///
/// ```ignore
/// let primary = ui_kit::PrimaryButton { kind: ui_kit::PlayKind::Play, active: playing, enabled: true };
/// let stage = ui_kit::stage_panel(
///     ui,
///     egui::Id::new("timeline-stage"),
///     primary,
///     |ui| { ui.label("Timeline"); },
///     |ui| { ui_kit::timecode_large(ui, "00:01:23", ":12"); },
///     |ui| { ui_kit::bar_button(ui, ui_kit::icons::fill::STOP, "Stop"); },
/// );
/// if stage.clicked() {
///     playing = !playing;
/// }
/// ```
pub fn stage_panel(
    ui: &mut Ui,
    id: Id,
    primary: PrimaryButton,
    add_content: impl FnOnce(&mut Ui),
    add_bar_left: impl FnOnce(&mut Ui),
    add_bar_right: impl FnOnce(&mut Ui),
) -> StageResponse {
    let available = ui.available_size_before_wrap();
    let (panel_rect, _) = ui.allocate_exact_size(vec2(available.x, available.y.max(BAR_HEIGHT)), Sense::hover());
    let bar_rect = Rect::from_min_max(pos2(panel_rect.left(), panel_rect.bottom() - BAR_HEIGHT), panel_rect.max);
    let bar_center_rect = Rect::from_center_size(bar_rect.center(), vec2(BAR_CENTER_WIDTH, BAR_HEIGHT));
    let primary_center = pos2(panel_rect.center().x, panel_rect.bottom() - PRIMARY_CENTER_RISE);
    if ui.is_rect_visible(panel_rect) {
        paint_stage(ui, panel_rect, bar_rect.top(), primary_center);
    }

    let content_area = Rect::from_min_max(panel_rect.min, bar_rect.right_top());
    child_in(ui, id.with("content"), content_area.shrink(space::PANEL_PADDING), content_area, egui::Layout::top_down(egui::Align::Min), add_content);

    let row = egui::Layout::left_to_right(egui::Align::Center);
    let left_clip = Rect::from_min_max(bar_rect.min, bar_center_rect.left_bottom());
    let left = Rect::from_min_max(
        pos2(left_clip.left() + space::SCALE_16, left_clip.top()),
        pos2((left_clip.right() - space::SCALE_8).max(left_clip.left() + space::SCALE_16), left_clip.bottom()),
    );
    child_in(ui, id.with("bar-left"), left, left_clip, row, add_bar_left);

    // Drawn between the two sides, so Tab moves through the bar in reading order.
    let primary = primary_in_notch(ui, id.with("primary"), primary, primary_center, panel_rect);

    let right_clip = Rect::from_min_max(bar_center_rect.right_top(), bar_rect.max);
    let right = Rect::from_min_max(
        pos2(right_clip.left() + space::SCALE_8, right_clip.top()),
        pos2((right_clip.right() - space::SCALE_16).max(right_clip.left() + space::SCALE_8), right_clip.bottom()),
    );
    child_in(ui, id.with("bar-right"), right, right_clip, row, add_bar_right);

    StageResponse { panel_rect, bar_rect, bar_center_rect, primary }
}

/// Runs `add_contents` in a child ui at `rect` that can neither paint nor take
/// clicks outside `clip`.
fn child_in(ui: &mut Ui, id: Id, rect: Rect, clip: Rect, layout: egui::Layout, add_contents: impl FnOnce(&mut Ui)) {
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(layout));
    child.set_clip_rect(clip.intersect(ui.clip_rect()));
    add_contents(&mut child);
}

/// Paints the panel, the bar's separator line and the notch.
fn paint_stage(ui: &Ui, panel: Rect, bar_top: f32, notch_center: Pos2) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let hairline = Stroke::new(HAIRLINE, neutrals.stroke_hairline);
    let painter = ui.painter();
    painter.rect(panel, radius::PANEL, neutrals.bg_panel, hairline, StrokeKind::Inside);
    painter.hline(panel.shrink(HAIRLINE).x_range(), bar_top + HAIRLINE / 2.0, hairline);
    // The cut: the parent ground painted over the panel. It reaches one stroke
    // below the edge, so it also removes the bottom rim and its soft edge.
    let cut = notch_arc(notch_center, NOTCH_RADIUS, PRIMARY_CENTER_RISE + HAIRLINE);
    painter.add(Shape::convex_polygon(cut, neutrals.bg_ground, Stroke::NONE));
    // The rim follows the arc just inside the panel, like the rest of the rim.
    let half = HAIRLINE / 2.0;
    painter.add(Shape::line(notch_arc(notch_center, NOTCH_RADIUS + half, PRIMARY_CENTER_RISE - half), hairline));
}

/// The notch outline from left to right over the top of the circle, ending on
/// both sides `depth` below the center.
fn notch_arc(center: Pos2, arc_radius: f32, depth: f32) -> Vec<Pos2> {
    let below = (depth / arc_radius).clamp(-1.0, 1.0).asin();
    let start = std::f32::consts::PI - below;
    let sweep = std::f32::consts::PI + 2.0 * below;
    (0..=NOTCH_SEGMENTS)
        .map(|step| {
            let angle = start + sweep * f32::from(step) / f32::from(NOTCH_SEGMENTS);
            center + arc_radius * vec2(angle.cos(), angle.sin())
        })
        .collect()
}

/// The primary button in its own [`egui::Area`], so it can reach below the
/// panel over whatever is drawn there. The area is a sublayer of the stage's
/// layer: egui keeps it directly above that layer, so the panels never cover
/// it and anything on a higher layer does.
fn primary_in_notch(ui: &Ui, id: Id, button: PrimaryButton, center: Pos2, panel: Rect) -> Response {
    let rect = Rect::from_center_size(center, vec2(PRIMARY_DIAMETER, PRIMARY_DIAMETER));
    // The layer does not inherit the parent's clip. It gets the parent's,
    // opened downward for the overhang, but only while the panel's bottom edge
    // is itself in view: a stage scrolled out of view keeps its button hidden.
    let mut clip = ui.clip_rect();
    if panel.bottom() <= clip.bottom() + HAIRLINE {
        clip.max.y = clip.max.y.max(panel.bottom() + PRIMARY_PAINT_BELOW);
    }
    let enabled = button.enabled && ui.is_enabled();
    let opacity = ui.opacity() * if button.enabled { 1.0 } else { DISABLED_OPACITY };
    let visible = ui.is_visible();
    // egui has no order between the panels and the windows, and forgets
    // sublayers after each pass, so this is registered every time.
    let parent = ui.layer_id();
    ui.ctx().set_sublayer(parent, LayerId::new(parent.order, id));
    egui::Area::new(id)
        .order(parent.order)
        .fixed_pos(rect.min)
        .constrain(false)
        .fade_in(false)
        .show(ui.ctx(), |ui| {
            ui.set_clip_rect(clip);
            ui.set_opacity(opacity);
            if !visible {
                ui.set_invisible();
            }
            paint_primary(ui, button, enabled)
        })
        .inner
}

/// The 68 px round primary button: Play on the accent with a play or pause
/// glyph in `accent.on`, Record on `sem.record` with a white dot, or a white
/// rounded square while recording. Both cast a soft shadow in their own color.
fn paint_primary(ui: &mut Ui, button: PrimaryButton, enabled: bool) -> Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(PRIMARY_DIAMETER, PRIMARY_DIAMETER), sense);
    let accent = Theme::get(ui.ctx()).accent;
    let label = match (button.kind, button.active) {
        (PlayKind::Play, false) => "Play",
        (PlayKind::Play, true) => "Pause",
        (PlayKind::Record, false) => "Record",
        (PlayKind::Record, true) => "Stop recording",
    };
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let round = capsule_radius(PRIMARY_DIAMETER);
    let (base, fill) = match button.kind {
        PlayKind::Play => {
            let fill = if enabled && response.is_pointer_button_down_on() {
                accent.pressed
            } else if enabled && response.hovered() {
                accent.hover
            } else {
                accent.accent
            };
            (accent.accent, fill)
        }
        PlayKind::Record => (color::SEM_RECORD, color::SEM_RECORD),
    };
    let shadow = Shadow { offset: [0, SHADOW_OFFSET_Y], blur: SHADOW_BLUR, spread: 0, color: base.gamma_multiply(SHADOW_STRENGTH) };
    let painter = ui.painter();
    painter.add(shadow.as_shape(rect, round));
    painter.circle_filled(rect.center(), PRIMARY_DIAMETER / 2.0, fill);
    match button.kind {
        PlayKind::Play => {
            let icon = if button.active { crate::icons::fill::PAUSE } else { crate::icons::fill::PLAY };
            let galley = painter.layout_no_wrap(icon.to_owned(), fonts::icon_fill(PRIMARY_GLYPH_SIZE), accent.on);
            painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
        }
        PlayKind::Record if button.active => {
            let square = Rect::from_center_size(rect.center(), vec2(RECORD_SQUARE_SIDE, RECORD_SQUARE_SIDE));
            painter.rect_filled(square, CornerRadius::from(RECORD_SQUARE_RADIUS), Color32::WHITE);
        }
        PlayKind::Record => {
            painter.circle_filled(rect.center(), RECORD_DOT_DIAMETER / 2.0, Color32::WHITE);
        }
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, round);
    }
    response
}

/// The two sizes of the bottom bar timecode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimecodeSize {
    /// 22, the normal bar timecode.
    Large,
    /// `type.value`, for a bar side too narrow for [`TimecodeSize::Large`].
    Value,
}

/// The bottom bar timecode: Martian Mono 22 Medium with -0.03em letter
/// spacing, `main` in `text.primary` and `frames` (with its separator, for
/// example `":12"`) in `text.disabled`.
pub fn timecode_large(ui: &mut Ui, main: &str, frames: &str) -> Response {
    timecode_sized(ui, main, frames, TimecodeSize::Large)
}

/// [`timecode_large`] at either [`TimecodeSize`].
pub fn timecode_sized(ui: &mut Ui, main: &str, frames: &str, size: TimecodeSize) -> Response {
    let job = timecode_job(ui, main, frames, size);
    ui.add(egui::Label::new(job).extend())
}

/// How wide [`timecode_sized`] draws, so a bar can pick a size before drawing it.
pub fn timecode_width(ui: &Ui, main: &str, frames: &str, size: TimecodeSize) -> f32 {
    ui.painter().layout_job(timecode_job(ui, main, frames, size)).size().x
}

/// The two-color timecode text.
fn timecode_job(ui: &Ui, main: &str, frames: &str, size: TimecodeSize) -> LayoutJob {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let points = match size {
        TimecodeSize::Large => TIMECODE_SIZE,
        TimecodeSize::Value => typography::VALUE_SIZE,
    };
    let font = fonts::mono_semibold(points);
    let mut job = fonts::tracked(main, font.clone(), neutrals.text_primary, fonts::TIMECODE_LETTER_SPACING_EM);
    let format = TextFormat { extra_letter_spacing: font.size * fonts::TIMECODE_LETTER_SPACING_EM, ..TextFormat::simple(font, neutrals.text_disabled) };
    job.append(frames, 0.0, format);
    job
}

/// A secondary bottom bar button: a 40 px circle in `bg.control`, `bg.hover`
/// on hover, with a phosphor fill glyph in `text.secondary` (`text.primary`
/// hovered). The tooltip doubles as the accessible label.
pub fn bar_button(ui: &mut Ui, icon: &str, tooltip: &str) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, response) = ui.allocate_exact_size(vec2(STAGE_BAR_BUTTON_DIAMETER, STAGE_BAR_BUTTON_DIAMETER), Sense::click());
    let response = response.on_hover_text(tooltip);
    let enabled = ui.is_enabled();
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, tooltip));
    if ui.is_rect_visible(rect) {
        let hovered = response.hovered();
        let fill = if hovered { neutrals.bg_hover } else { neutrals.bg_control };
        ui.painter().circle_filled(rect.center(), STAGE_BAR_BUTTON_DIAMETER / 2.0, fill);
        let text = if hovered { neutrals.text_primary } else { neutrals.text_secondary };
        let galley = ui.painter().layout_no_wrap(icon.to_owned(), fonts::icon_fill(BAR_ICON_SIZE), text);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::PLACEHOLDER);
    }
    if response.has_focus() {
        focus::focus_ring(ui, rect, capsule_radius(STAGE_BAR_BUTTON_DIAMETER));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The notch arc starts and ends at the asked depth, mirrors around the
    /// center and tops out one radius above it.
    #[test]
    fn the_notch_arc_spans_the_cut() {
        let center = pos2(400.0, 392.0);
        let points = notch_arc(center, NOTCH_RADIUS, PRIMARY_CENTER_RISE);
        let (first, last) = (points[0], points[points.len() - 1]);
        assert!((first.y - 400.0).abs() < 1e-3 && (last.y - 400.0).abs() < 1e-3, "{first:?} {last:?}");
        assert!((center.x - first.x - (last.x - center.x)).abs() < 1e-3);
        assert!(first.x < center.x && last.x > center.x);
        let top = points.iter().map(|point| point.y).fold(f32::MAX, f32::min);
        assert!((top - (center.y - NOTCH_RADIUS)).abs() < 1e-3, "{top}");
    }

    /// The button and its notch leave room inside the bar's empty column.
    #[test]
    fn the_notch_fits_the_bar_center_column() {
        const { assert!(2.0 * NOTCH_RADIUS < BAR_CENTER_WIDTH) };
        const { assert!(PRIMARY_CENTER_RISE + NOTCH_RADIUS < BAR_HEIGHT) };
        assert_eq!(STAGE_PRIMARY_OVERHANG, 26.0);
    }
}
