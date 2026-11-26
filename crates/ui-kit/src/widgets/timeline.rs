//! Timeline painter helpers (spec "Components": rows "Clip", "Playhead",
//! "Marker", "Pending cut band", "Scrollbar"; S1 spec row
//! "timeline/interaction.rs visuals"): pure painting into a given rect. Apps
//! keep their own interaction code and call these for the pixels.

use egui::{Color32, Mesh, Painter, Pos2, Rect, Stroke, StrokeKind, vec2};

use crate::Theme;
use crate::fonts;
use crate::tokens::{color, radius, space, typography};

/// Opacity of a muted or hidden clip (spec "Clip": 45%).
const DIMMED_OPACITY: f32 = 0.45;
/// Opacity of a dragged ghost clip (S1 spec, row "timeline/interaction.rs
/// visuals": the Clip at 50% opacity). The app sets [`ClipLook::opacity`]
/// itself, so until then only the tests read the constant.
#[cfg(test)]
const GHOST_OPACITY: f32 = 0.5;
/// Playhead line width (spec "Playhead": 1.5px).
const PLAYHEAD_STROKE: f32 = 1.5;
/// Playhead head width (spec "Playhead": 16 x 20).
const HEAD_WIDTH: f32 = 16.0;
/// Playhead head height (spec "Playhead": 16 x 20).
const HEAD_HEIGHT: f32 = 20.0;
/// Playhead head corner rounding.
const HEAD_ROUNDING: f32 = 5.0;
/// Marker dot diameter (spec "Marker": 7px).
const MARKER_DIAMETER: f32 = 7.0;
/// User marker ring width (spec "Marker": 1.5px ring).
const USER_RING_STROKE: f32 = 1.5;
/// Pending band corner radius (spec "Pending cut band": 6px).
const BAND_ROUNDING: f32 = 6.0;
/// Pending band background wash alpha (spec "Pending cut band": 28% and 6%).
const BAND_WASH_ALPHA: f32 = 0.06;
/// Pending band hatch line alpha (spec "Pending cut band": 28% and 6%).
const BAND_HATCH_ALPHA: f32 = 0.28;
/// Marquee fill alpha (S1 spec, row "timeline/interaction.rs visuals": 12%).
const MARQUEE_FILL_ALPHA: f32 = 0.12;

/// What a clip holds, which decides its corner radius (spec "Clip").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipKind {
    /// An audio clip: `radius.clip.audio` (10).
    Audio,
    /// A video clip: `radius.clip.video` (8).
    Video,
}

/// The visual state of one clip, read-only: apps own the interaction and pass
/// the state in.
#[derive(Debug, Clone, Copy)]
pub struct ClipLook<'a> {
    /// The clip name drawn at the top-left in `type.micro`.
    pub label: &'a str,
    /// Selected: `accent.tint` body, 2px `accent` rim.
    pub selected: bool,
    /// Muted or hidden: the whole clip at 45% opacity.
    pub dimmed: bool,
    /// Audio or video, which decides the corner radius.
    pub kind: ClipKind,
    /// Overall opacity, 1.0 normally and 0.5 for a dragged ghost.
    pub opacity: f32,
}

impl<'a> ClipLook<'a> {
    /// A plain, full-opacity clip with the given label and kind.
    pub fn new(label: &'a str, kind: ClipKind) -> Self {
        Self { label, selected: false, dimmed: false, kind, opacity: 1.0 }
    }

    /// The clip's corner radius from its kind (spec "Clip").
    fn clip_radius(&self) -> f32 {
        match self.kind {
            ClipKind::Audio => radius::CLIP_AUDIO,
            ClipKind::Video => radius::CLIP_VIDEO,
        }
    }
}

/// The clip body color: `accent.tint` when selected, `bg.clip` otherwise.
fn clip_body_color(look: &ClipLook, theme: &Theme) -> Color32 {
    if look.selected { theme.accent.tint } else { theme.neutrals.bg_clip }
}

/// The clip rim: 2px `accent` when selected, 1px `stroke.clip` otherwise.
fn clip_rim(look: &ClipLook, theme: &Theme) -> Stroke {
    if look.selected { Stroke::new(crate::SELECTION_STROKE, theme.accent.accent) } else { Stroke::new(crate::HAIRLINE, theme.neutrals.stroke_clip) }
}

/// The clip's overall opacity: `opacity` times 45% when dimmed.
fn clip_opacity(look: &ClipLook) -> f32 {
    look.opacity * if look.dimmed { DIMMED_OPACITY } else { 1.0 }
}

/// A stroke faded by `alpha` (a multiplied color; egui strokes carry no alpha
/// helper of their own).
fn faded_stroke(stroke: Stroke, alpha: f32) -> Stroke {
    Stroke::new(stroke.width, stroke.color.gamma_multiply(alpha))
}

/// Paints one clip into `rect` and returns the content rect for the waveform:
/// inside the clip, below the label. The label is drawn in `type.micro` at
/// the top-left, inset 8px from the left and 4px from the top so even the
/// 2px selected rim never cuts it, and clipped to the clip rect.
pub fn paint_clip(painter: &Painter, rect: Rect, look: &ClipLook, theme: &Theme) -> Rect {
    let alpha = clip_opacity(look);
    let rounding = look.clip_radius();
    painter.rect_filled(rect, rounding, clip_body_color(look, theme).gamma_multiply(alpha));
    painter.rect_stroke(rect, rounding, faded_stroke(clip_rim(look, theme), alpha), StrokeKind::Inside);

    let label_galley = painter.layout_no_wrap(look.label.to_owned(), fonts::medium(typography::MICRO_SIZE), theme.neutrals.text_primary.gamma_multiply(alpha));
    let label_left = rect.left() + space::SCALE_8;
    let label_top = rect.top() + space::SCALE_4;
    let label_height = label_galley.size().y;
    painter.with_clip_rect(rect).galley(Pos2::new(label_left, label_top), label_galley, theme.neutrals.text_primary);

    Rect::from_min_max(
        Pos2::new(rect.left() + space::SCALE_8, label_top + label_height),
        Pos2::new(rect.right() - space::SCALE_4, rect.bottom() - space::SCALE_4),
    )
}

/// The waveform color: `accent.content` in a selected clip, `text.tertiary`
/// otherwise (spec "Clip").
pub fn waveform_color(selected: bool, theme: &Theme) -> Color32 {
    if selected { theme.accent.content } else { theme.neutrals.text_tertiary }
}

/// Builds the waveform envelope as one filled mesh: two vertices per column
/// (max above the mid line, min below), joined into a strip across `rect`.
/// Peaks run -1 to 1 and are clamped to it.
fn waveform_mesh(rect: Rect, peaks: &[(f32, f32)], color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    if peaks.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return mesh;
    }
    let column_width = rect.width() / peaks.len() as f32;
    let mid_y = rect.center().y;
    let half_height = rect.height() / 2.0;
    for (index, &(min, max)) in peaks.iter().enumerate() {
        let x = rect.left() + index as f32 * column_width;
        let top = mid_y - max.clamp(-1.0, 1.0) * half_height;
        let bottom = mid_y - min.clamp(-1.0, 1.0) * half_height;
        let top_index = mesh.vertices.len() as u32;
        mesh.colored_vertex(Pos2::new(x, top), color);
        mesh.colored_vertex(Pos2::new(x, bottom), color);
        if index > 0 {
            let (previous_top, previous_bottom) = (top_index - 2, top_index - 1);
            mesh.add_triangle(previous_top, previous_bottom, top_index);
            mesh.add_triangle(previous_bottom, top_index, top_index + 1);
        }
    }
    mesh
}

/// Paints one min/max waveform envelope as one filled mesh (spec "Clip":
/// waveform `text.tertiary`, `accent.content` when selected; pass
/// [`waveform_color`]'s result).
pub fn paint_waveform(painter: &Painter, rect: Rect, peaks: &[(f32, f32)], color: Color32) {
    painter.add(waveform_mesh(rect, peaks, color));
}

/// Paints the playhead: a 1.5px `accent` line through the ruler and the lanes,
/// with a rounded 16 x 20 head centered in the ruler (spec "Playhead").
pub fn paint_playhead(painter: &Painter, ruler_rect: Rect, lanes_rect: Rect, x: f32, theme: &Theme) {
    let stroke = Stroke::new(PLAYHEAD_STROKE, theme.accent.accent);
    painter.line_segment([Pos2::new(x, ruler_rect.top()), Pos2::new(x, lanes_rect.bottom())], stroke);
    let head = Rect::from_center_size(Pos2::new(x, ruler_rect.center().y), vec2(HEAD_WIDTH, HEAD_HEIGHT));
    painter.rect_filled(head, HEAD_ROUNDING, theme.accent.accent);
}

/// A marker category: its dot color (spec "Marker"; colors from "Color:
/// meaning" and never follow the accent).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    /// `marker.note`.
    Note,
    /// `marker.retake`.
    Retake,
    /// `marker.cut`.
    Cut,
    /// `marker.broll`.
    BRoll,
    /// `marker.system`.
    System,
    /// A user marker: 1.5px `text.tertiary` ring instead of a dot.
    User,
}

/// The color of a marker kind (spec "Marker": category color, user ring
/// `text.tertiary`). It has no context, so a user marker gets the untinted
/// fallback here; [`paint_marker`] uses the theme's `text.tertiary`.
pub fn marker_color(kind: MarkerKind) -> Color32 {
    match kind {
        MarkerKind::Note => color::MARKER_NOTE,
        MarkerKind::Retake => color::MARKER_RETAKE,
        MarkerKind::Cut => color::MARKER_CUT,
        MarkerKind::BRoll => color::MARKER_BROLL,
        MarkerKind::System => color::MARKER_SYSTEM,
        MarkerKind::User => color::TEXT_TERTIARY,
    }
}

/// Paints one 7px marker dot at `center` in its category color; a user marker
/// is a 1.5px `text.tertiary` ring (spec "Marker").
pub fn paint_marker(painter: &Painter, center: Pos2, kind: MarkerKind) {
    let marker_radius = MARKER_DIAMETER / 2.0;
    match kind {
        MarkerKind::User => {
            let ring = Theme::get(painter.ctx()).neutrals.text_tertiary;
            painter.circle_stroke(center, marker_radius, Stroke::new(USER_RING_STROKE, ring));
        }
        _ => {
            painter.circle_filled(center, marker_radius, marker_color(kind));
        }
    }
}

/// Paints a pending cut band: `sem.pending` at 6% wash with a 28% diagonal
/// hatch, 6px radius, clipped to `rect` (spec "Pending cut band").
pub fn paint_pending_band(painter: &Painter, rect: Rect) {
    painter.rect_filled(rect, BAND_ROUNDING, color::SEM_PENDING.gamma_multiply(BAND_WASH_ALPHA));
    let hatch = painter.with_clip_rect(rect);
    let spacing = space::SCALE_8;
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        hatch.line_segment(
            [Pos2::new(x, rect.bottom()), Pos2::new(x + rect.height(), rect.top())],
            Stroke::new(crate::HAIRLINE, color::SEM_PENDING.gamma_multiply(BAND_HATCH_ALPHA)),
        );
        x += spacing;
    }
}

/// Paints the marquee selection: `accent` at 12% fill with a 1px `accent` rim
/// (S1 spec, row "timeline/interaction.rs visuals").
pub fn paint_marquee(painter: &Painter, rect: Rect, theme: &Theme) {
    painter.rect_filled(rect, 0.0, theme.accent.accent.gamma_multiply(MARQUEE_FILL_ALPHA));
    painter.rect_stroke(rect, 0.0, Stroke::new(crate::HAIRLINE, theme.accent.accent), StrokeKind::Inside);
}

/// Paints the snap line: a 1px vertical `accent` line at `x` across `rect`
/// (S1 spec, row "timeline/interaction.rs visuals").
pub fn paint_snap_line(painter: &Painter, x: f32, rect: Rect, theme: &Theme) {
    painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(crate::HAIRLINE, theme.accent.accent));
}

/// Paints the timeline ruler: tick lines in `stroke.hairline`, labels in
/// `type.value` `text.tertiary` in the top half, and the marker dots in a
/// strip in the bottom half, never on top of a label (spec "Ruler",
/// "Marker"). Ticks are `(x, label)`; `None` labels draw a shorter minor
/// tick. Markers are `(x, kind)` on the same x scale as the ticks.
pub fn paint_ruler(painter: &Painter, rect: Rect, ticks: &[(f32, Option<String>)], markers: &[(f32, MarkerKind)]) {
    let neutrals = Theme::get(painter.ctx()).neutrals;
    let clipped = painter.with_clip_rect(rect);
    let label_font = fonts::mono(typography::VALUE_SIZE);
    let label_height = ticks
        .iter()
        .find_map(|(_, label)| label.as_ref())
        .map_or(0.0, |text| clipped.layout_no_wrap(text.clone(), label_font.clone(), neutrals.text_tertiary).size().y);
    for (x, label) in ticks {
        let tick_top = if label.is_some() { rect.top() } else { rect.center().y };
        clipped.line_segment([Pos2::new(*x, tick_top), Pos2::new(*x, rect.bottom())], Stroke::new(crate::HAIRLINE, neutrals.stroke_hairline));
        if let Some(text) = label {
            let galley = clipped.layout_no_wrap(text.clone(), label_font.clone(), neutrals.text_tertiary);
            clipped.galley(Pos2::new(x + space::SCALE_4, rect.top() + space::SCALE_2), galley, neutrals.text_tertiary);
        }
    }
    let dot_y = ruler_marker_center(rect, label_height);
    for (x, kind) in markers {
        paint_marker(&clipped, Pos2::new(*x, dot_y), *kind);
    }
}

/// The y center of the marker dot strip: the bottom half of the ruler, pushed
/// below the time labels so a dot never sits on top of one, and kept inside
/// the ruler.
fn ruler_marker_center(rect: Rect, label_height: f32) -> f32 {
    let labels_bottom = rect.top() + space::SCALE_2 + label_height;
    let half = MARKER_DIAMETER / 2.0;
    ((rect.center().y + rect.bottom()) / 2.0).max(labels_bottom + half + space::SCALE_2).min(rect.bottom() - half - 1.0)
}

#[cfg(test)]
mod tests {
    use egui::LayerId;

    use super::*;

    /// A painter over a context with the ui-kit fonts installed, plus the
    /// default Blue theme.
    fn painter_and_theme() -> (Painter, Theme) {
        let ctx = egui::Context::default();
        ctx.set_fonts(crate::fonts::definitions());
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        (Painter::new(ctx, LayerId::background(), Rect::EVERYTHING), Theme::default())
    }

    /// `paint_clip` returns a content rect inside the clip rect and below the
    /// label, for both clip kinds.
    #[test]
    fn paint_clip_returns_a_content_rect_below_the_label() {
        let (painter, theme) = painter_and_theme();
        for kind in [ClipKind::Audio, ClipKind::Video] {
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 60.0));
            let content = paint_clip(&painter, rect, &ClipLook::new("Take 1", kind), &theme);
            assert!(rect.contains_rect(content), "{kind:?}: content {content:?} must sit inside clip {rect:?}");
            assert!(content.top() > rect.top(), "{kind:?}: the content must start below the label");
        }
    }

    /// A selected clip paints the `accent.tint` body and the 2px accent rim; an
    /// unselected one paints `bg.clip` and `stroke.clip`.
    #[test]
    fn a_selected_clip_uses_the_accent_tint() {
        let theme = Theme::default();
        let mut look = ClipLook::new("Take 1", ClipKind::Audio);
        assert_eq!(clip_body_color(&look, &theme), color::BG_CLIP);
        assert_eq!(clip_rim(&look, &theme), Stroke::new(crate::HAIRLINE, color::STROKE_CLIP));
        look.selected = true;
        assert_eq!(clip_body_color(&look, &theme), theme.accent.tint);
        assert_eq!(clip_rim(&look, &theme), Stroke::new(crate::SELECTION_STROKE, theme.accent.accent));
    }

    /// Dimming multiplies the clip's opacity down to 45%; a ghost drag is 50%.
    #[test]
    fn dimmed_alpha_is_45_percent() {
        let mut look = ClipLook::new("Take 1", ClipKind::Audio);
        assert!((clip_opacity(&look) - 1.0).abs() < 1e-4, "a plain clip is full opacity");
        look.dimmed = true;
        assert!((clip_opacity(&look) - DIMMED_OPACITY).abs() < 1e-4, "a dimmed clip is 45%");
        look.opacity = GHOST_OPACITY;
        assert!((clip_opacity(&look) - 0.5 * 0.45).abs() < 1e-4, "a dimmed ghost stacks both factors");
    }

    /// The waveform mesh holds two vertices per column, max and min.
    #[test]
    fn the_waveform_mesh_holds_two_vertices_per_column() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 40.0));
        let peaks = vec![(0.5, 0.8), (-0.2, 0.3), (-0.9, -0.4)];
        let mesh = waveform_mesh(rect, &peaks, Color32::WHITE);
        assert_eq!(mesh.vertices.len(), 2 * peaks.len(), "two vertices per column");
        let empty = waveform_mesh(rect, &[], Color32::WHITE);
        assert!(empty.vertices.is_empty(), "no columns, no vertices");
    }

    /// Every marker kind maps to its token color (spec "Color: meaning",
    /// marker categories; user markers are `text.tertiary`).
    #[test]
    fn each_marker_kind_maps_to_its_token_color() {
        assert_eq!(marker_color(MarkerKind::Note), color::MARKER_NOTE);
        assert_eq!(marker_color(MarkerKind::Retake), color::MARKER_RETAKE);
        assert_eq!(marker_color(MarkerKind::Cut), color::MARKER_CUT);
        assert_eq!(marker_color(MarkerKind::BRoll), color::MARKER_BROLL);
        assert_eq!(marker_color(MarkerKind::System), color::MARKER_SYSTEM);
        assert_eq!(marker_color(MarkerKind::User), color::TEXT_TERTIARY);
    }

    /// `waveform_color` picks `accent.content` when selected and
    /// `text.tertiary` otherwise.
    #[test]
    fn the_waveform_color_follows_the_selection() {
        let theme = Theme::default();
        assert_eq!(waveform_color(false, &theme), color::TEXT_TERTIARY);
        assert_eq!(waveform_color(true, &theme), theme.accent.content);
    }

    /// The marker dots sit in the bottom half of the ruler, fully below the
    /// time labels, so a dot never sits on top of a label (spec "Ruler",
    /// "Marker").
    #[test]
    fn ruler_marker_dots_sit_below_the_labels() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 26.0));
        let label_height = 15.0;
        let y = ruler_marker_center(rect, label_height);
        let labels_bottom = rect.top() + space::SCALE_2 + label_height;
        assert!(y - MARKER_DIAMETER / 2.0 >= labels_bottom, "the dot must start below the labels");
        assert!(y > rect.center().y, "the dot strip is in the bottom half");
        assert!(y + MARKER_DIAMETER / 2.0 <= rect.bottom(), "the dot stays inside the ruler");
    }

    /// Even a cramped ruler keeps the dots fully inside.
    #[test]
    fn ruler_marker_dots_stay_inside_a_short_ruler() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(600.0, 18.0));
        let y = ruler_marker_center(rect, 16.0);
        assert!(y - MARKER_DIAMETER / 2.0 >= rect.top());
        assert!(y + MARKER_DIAMETER / 2.0 <= rect.bottom());
    }
}
