//! The accent picker (spec: Design Language "Accent picker"): presets, a hue
//! and chroma wheel in OKLCH, a lightness slider, a hex field, a live preview
//! strip, the readability and near-record notes and the reset button, in one
//! inspector group. Every change applies live through
//! [`Theme::set`] and [`crate::refresh`]; the appearance file is saved on
//! release of a drag or on a discrete change, never every frame.

use egui::{Color32, Context, Id, Margin, Mesh, Pos2, Rect, Response, Sense, Stroke, Ui, WidgetInfo, WidgetType, vec2};

use crate::HAIRLINE;
use crate::accent;
use crate::appearance;
use crate::fonts;
use crate::theme::Theme;
use crate::tokens::{color, motion, radius, space, typography};
use crate::widgets::{
    ClipKind, ClipLook, PlayKind, SliderKind, dialog, inspector_group, paint_clip, paint_playhead, paint_waveform, play_button, primary_button, show_toasts,
    slider, switch, toast, waveform_color,
};

/// Diameter of a swatch circle (spec "Accent picker": 28px circles).
const SWATCH_SIZE: f32 = 28.0;
/// Width of the selected swatch ring (spec "Accent picker": a 2px ring).
const SWATCH_RING: f32 = 2.0;
/// Side of the hue and chroma wheel (spec "Accent picker": 200px).
const WHEEL_SIZE: f32 = 200.0;
/// Chroma at the wheel rim; the rim area falls outside sRGB and is gamut mapped.
const WHEEL_CHROMA: f32 = 0.33;
/// Hue steps of the wheel mesh.
const WHEEL_HUE_STEPS: usize = 48;
/// Radius steps of the wheel mesh.
const WHEEL_RADII_STEPS: usize = 20;
/// Height of the live preview strip.
const PREVIEW_HEIGHT: f32 = 64.0;
/// Height of the mini ruler above the preview clip (fits the playhead head).
const PREVIEW_RULER_HEIGHT: f32 = 20.0;
/// Width of the hex text field.
const HEX_FIELD_WIDTH: f32 = 120.0;
/// Diameter of the small before/after dots in the adjusted note.
const NOTE_DOT: f32 = 12.0;
/// A short fixed waveform for the preview clip, as `(max, min)` per column.
const PREVIEW_PEAKS: [(f32, f32); 16] = [
    (0.30, -0.28),
    (0.55, -0.50),
    (0.80, -0.72),
    (0.62, -0.58),
    (0.44, -0.40),
    (0.72, -0.66),
    (0.90, -0.84),
    (0.70, -0.62),
    (0.48, -0.44),
    (0.64, -0.58),
    (0.84, -0.78),
    (0.58, -0.52),
    (0.38, -0.34),
    (0.60, -0.55),
    (0.42, -0.38),
    (0.25, -0.22),
];

/// The picker state kept in the egui context data between frames.
#[derive(Debug, Clone, Default)]
struct PickerState {
    /// The wheel and slider lightness, initialized from the current pick.
    lightness: Option<f32>,
    /// The hex field text; follows the pick while the field is not focused.
    hex_text: String,
    /// True while the hex field holds keyboard focus.
    hex_focused: bool,
    /// True while the hex text does not parse: the field rim turns `sem.record`.
    hex_invalid: bool,
    /// The pick seen on the previous frame, for the 150ms color ease.
    last_pick: Option<Color32>,
    /// The accent set seen on the previous frame, for the 150ms color ease.
    last_accent: Option<accent::Accent>,
    /// The accent set the ease fades from; none while no change is animating.
    fade_from: Option<accent::Accent>,
    /// When the current ease started (egui time in seconds).
    fade_started: Option<f64>,
}

/// The accent picker: swatches, wheel, lightness, hex field, live preview,
/// notes and reset, top to bottom in one inspector group. The theme is saved
/// to the app's own file (the app [`crate::apply_for`] stored).
/// Returns true when the theme changed this frame.
///
/// Usage:
///
/// ```ignore
/// if ui_kit::accent_picker(ui) { /* the app may react to a new accent */ }
/// ```
pub fn accent_picker(ui: &mut Ui) -> bool {
    let ctx = ui.ctx().clone();
    let before = Theme::get(&ctx);
    let id = Id::new("cutmark_accent_picker");
    let mut state = ctx.data(|data| data.get_temp::<PickerState>(id)).unwrap_or_default();
    if state.lightness.is_none() {
        state.lightness = Some(accent::to_oklch(before.accent.pick)[0]);
    }
    if !state.hex_focused {
        state.hex_text = pick_hex(before.accent.pick);
        state.hex_invalid = false;
    }
    let fade_t = fade_fraction(&ctx, &mut state, before);
    let shown = Theme { accent: fade_blend(state.fade_from, before.accent, fade_t), ..before };

    inspector_group(ui, |ui| {
        ui.add_space(space::SCALE_4);
        swatch_row(ui, &ctx, &mut state, before);
        ui.add_space(space::SCALE_8);
        wheel(ui, &ctx, &mut state);
        ui.add_space(space::SCALE_8);
        lightness_row(ui, &ctx, &mut state, before);
        ui.add_space(space::SCALE_8);
        hex_row(ui, &ctx, &mut state);
        ui.add_space(space::SCALE_8);
        preview_strip(ui, &shown);
        ui.add_space(space::SCALE_8);
        notes(ui, before);
        ui.add_space(space::SCALE_8);
        reset_row(ui, &ctx, &mut state, before);
        ui.add_space(space::SCALE_4);
    });
    ctx.data_mut(|data| data.insert_temp(id, state));
    Theme::get(&ctx) != before
}

/// The Appearance dialog: the picker inside an "Appearance" modal with a
/// "Done" primary button in the footer. Save errors surface as toasts.
///
/// Usage:
///
/// ```ignore
/// ui_kit::appearance_dialog(ctx, &mut open);
/// ```
pub fn appearance_dialog(ctx: &Context, open: &mut bool) {
    let mut done = false;
    dialog(
        ctx,
        Id::new("cutmark_appearance_dialog"),
        "Appearance",
        open,
        |ui| {
            accent_picker(ui);
        },
        |ui| {
            if primary_button(ui, "Done").clicked() {
                done = true;
            }
        },
    );
    if done {
        *open = false;
    }
    show_toasts(ctx);
}

/// Applies a new pick to the theme live: lightness follows the pick, the
/// derivation runs and the styles refresh immediately (spec: live preview).
fn apply_pick(ctx: &Context, pick: Color32, state: &mut PickerState) {
    state.lightness = Some(accent::to_oklch(pick)[0]);
    Theme::set(ctx, Theme { accent: accent::derive(pick), ..Theme::get(ctx) });
    crate::refresh(ctx);
}

/// Saves the theme to the appearance file; an error queues a toast.
fn persist(ctx: &Context) {
    if let Err(error) = appearance::save(&appearance::app(ctx), &Theme::get(ctx)) {
        toast(ctx, format!("Cannot save the appearance: {error}"));
    }
}

/// Formats a color as the `#RRGGBB` text the hex field and the file use.
fn pick_hex(pick: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", pick.r(), pick.g(), pick.b())
}

/// Parses `#RRGGBB` (the `#` optional, surrounding whitespace ignored).
fn parse_hex(text: &str) -> Option<Color32> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |range: std::ops::Range<usize>| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some(Color32::from_rgb(byte(0..2)?, byte(2..4)?, byte(4..6)?)) // style-lint: allow parses the user's hex text, not a design value
}

/// Blends two colors channel-wise; `t` 0 gives `from`, 1 gives `to`.
fn blend(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b())) // style-lint: allow interpolates two theme colors, no design value
}

/// The ease fraction for the 150ms accent color fade (ease-out cubic,
/// instant when the theme asks for reduced motion); 1.0 when settled.
fn fade_fraction(ctx: &Context, state: &mut PickerState, theme: Theme) -> f32 {
    if state.last_pick != Some(theme.accent.pick) {
        if state.last_pick.is_some() {
            state.fade_from = state.last_accent;
            state.fade_started = Some(ctx.input(|input| input.time));
        }
        state.last_pick = Some(theme.accent.pick);
        state.last_accent = Some(theme.accent);
    }
    let duration = f64::from(motion::ACCENT_CHANGE_MS) / 1000.0;
    match (state.fade_started, theme.reduce_motion) {
        (Some(start), false) if duration > 0.0 => {
            let t = ((ctx.input(|input| input.time) - start) / duration).clamp(0.0, 1.0);
            (1.0 - (1.0 - t).powi(3)) as f32
        }
        _ => 1.0,
    }
}

/// The accent set the preview paints with: `from` blended toward the current
/// set by `t`; the current set unchanged when there is no fade running.
fn fade_blend(from: Option<accent::Accent>, to: accent::Accent, t: f32) -> accent::Accent {
    match from {
        Some(from) if t < 1.0 => accent::Accent {
            pick: blend(from.pick, to.pick, t),
            accent: blend(from.accent, to.accent, t),
            hover: blend(from.hover, to.hover, t),
            pressed: blend(from.pressed, to.pressed, t),
            on: to.on,
            tint: blend(from.tint, to.tint, t),
            content: blend(from.content, to.content, t),
            adjusted: to.adjusted,
            near_record: to.near_record,
        },
        _ => to,
    }
}

/// One 28px swatch circle with its accessible name and the selection ring.
fn swatch(ui: &mut Ui, name: &str, fill: Color32, selected: bool) -> Response {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    let (rect, response) = ui.allocate_exact_size(vec2(SWATCH_SIZE, SWATCH_SIZE), Sense::click());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();
        painter.circle_filled(center, SWATCH_SIZE / 2.0, fill);
        if selected {
            painter.circle_stroke(center, SWATCH_SIZE / 2.0, Stroke::new(SWATCH_RING, neutrals.text_primary));
        }
    }
    response
}

/// The swatch row: Blue, Violet, Teal, Orange, then Custom; the selected one
/// carries the ring (Custom when the pick is not a preset).
fn swatch_row(ui: &mut Ui, ctx: &Context, state: &mut PickerState, theme: Theme) {
    ui.horizontal(|ui| {
        for (name, preset) in accent::presets() {
            let response = swatch(ui, name, preset, theme.accent.pick == preset);
            if response.clicked() {
                apply_pick(ctx, preset, state);
                persist(ctx);
            }
        }
        let is_preset = accent::presets().iter().any(|(_, preset)| *preset == theme.accent.pick);
        swatch(ui, "Custom", theme.accent.pick, !is_preset);
    });
}

/// The wheel mesh: hue around the circle, chroma outward, at the given
/// lightness; out-of-gamut cells come back gamut mapped from [`accent::from_oklch`].
fn wheel_mesh(center: Pos2, lightness: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let hue_step = 360.0 / WHEEL_HUE_STEPS as f32;
    let radius = WHEEL_SIZE / 2.0;
    for i in 0..WHEEL_HUE_STEPS {
        let a0 = i as f32 * hue_step;
        let a1 = (i + 1) as f32 * hue_step;
        for j in 1..=WHEEL_RADII_STEPS {
            let r0 = radius * (j - 1) as f32 / WHEEL_RADII_STEPS as f32;
            let r1 = radius * j as f32 / WHEEL_RADII_STEPS as f32;
            let hue = (i as f32 + 0.5) * hue_step;
            let chroma = (r0 + r1) / 2.0 / radius * WHEEL_CHROMA;
            let fill = accent::from_oklch(lightness, chroma, hue);
            let (c0, s0, c1, s1) = (a0.cos(), a0.sin(), a1.cos(), a1.sin());
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(center + vec2(r0 * c0, r0 * s0), fill);
            mesh.colored_vertex(center + vec2(r0 * c1, r0 * s1), fill);
            mesh.colored_vertex(center + vec2(r1 * c0, r1 * s0), fill);
            mesh.colored_vertex(center + vec2(r1 * c1, r1 * s1), fill);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    mesh
}

/// The hue and chroma wheel, 200px, at the current lightness. Clicking or
/// dragging picks hue and chroma, selects Custom and previews live; the
/// appearance file is saved only on release (or on a plain click).
fn wheel(ui: &mut Ui, ctx: &Context, state: &mut PickerState) {
    let lightness = state.lightness.unwrap_or(0.7);
    let row = ui.available_rect_before_wrap();
    ui.allocate_rect(Rect::from_min_size(row.min, vec2(row.width(), WHEEL_SIZE)), Sense::hover());
    let wheel_rect = Rect::from_min_size(Pos2::new(row.center().x - WHEEL_SIZE / 2.0, row.top()), vec2(WHEEL_SIZE, WHEEL_SIZE));
    let response = ui.interact(wheel_rect, Id::new("cutmark_accent_wheel"), Sense::click_and_drag());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, "Accent wheel"));
    if ui.is_rect_visible(wheel_rect) {
        ui.painter_at(wheel_rect).add(wheel_mesh(wheel_rect.center(), lightness));
    }
    let pick_here = |pos: Pos2| {
        let d = pos - wheel_rect.center();
        let radius = (d.length() / (WHEEL_SIZE / 2.0)).clamp(0.0, 1.0);
        let hue = d.y.atan2(d.x).to_degrees().rem_euclid(360.0);
        accent::from_oklch(lightness, radius * WHEEL_CHROMA, hue)
    };
    // Only a real pointer movement picks: holding the button still must not
    // re-apply the pick (and repaint) every frame.
    if response.dragged()
        && response.drag_delta() != vec2(0.0, 0.0)
        && let Some(pos) = response.interact_pointer_pos()
    {
        apply_pick(ctx, pick_here(pos), state);
    }
    if response.drag_stopped() {
        persist(ctx);
    } else if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            apply_pick(ctx, pick_here(pos), state);
        }
        persist(ctx);
    }
}

/// The lightness slider under the wheel: `SliderKind::Zoom` look. Changes
/// preview live; the file is saved on release or on a discrete click.
fn lightness_row(ui: &mut Ui, ctx: &Context, state: &mut PickerState, theme: Theme) {
    let mut lightness = state.lightness.unwrap_or(0.7);
    let response = slider(ui, &mut lightness, 0.0..=1.0, SliderKind::Zoom);
    state.lightness = Some(lightness);
    if response.changed() {
        let current = accent::to_oklch(theme.accent.pick);
        apply_pick(ctx, accent::from_oklch(lightness, current[1], current[2]), state);
    }
    if response.drag_stopped() || (response.changed() && !response.dragged() && !response.drag_stopped()) {
        persist(ctx);
    }
}

/// The hex field: `#RRGGBB`, accepts paste; invalid text turns the field rim
/// `sem.record` and changes nothing.
fn hex_row(ui: &mut Ui, ctx: &Context, state: &mut PickerState) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Hex").font(fonts::medium(typography::BODY_SIZE)).color(neutrals.text_secondary));
        let rim = if state.hex_invalid { Stroke::new(HAIRLINE, color::SEM_RECORD) } else { Stroke::NONE };
        egui::Frame::new()
            .fill(neutrals.bg_group)
            .corner_radius(radius::GROUP)
            .stroke(rim)
            .inner_margin(Margin::symmetric(space::SCALE_8 as i8, space::SCALE_4 as i8))
            .show(ui, |ui| {
                ui.scope(|ui| {
                    // egui takes the hint color from `weak_text_color`.
                    ui.style_mut().visuals.weak_text_color = Some(neutrals.text_tertiary);
                    let response = egui::TextEdit::singleline(&mut state.hex_text)
                        .hint_text("#RRGGBB")
                        .desired_width(HEX_FIELD_WIDTH)
                        .font(fonts::mono(typography::VALUE_SIZE))
                        .frame(egui::Frame::NONE)
                        .show(ui)
                        .response;
                    response.widget_info(|| WidgetInfo::labeled(WidgetType::TextEdit, true, "Hex value"));
                    if response.gained_focus() {
                        state.hex_text.clear();
                        state.hex_focused = true;
                    }
                    if response.lost_focus() {
                        state.hex_focused = false;
                    }
                    if response.changed() {
                        match parse_hex(&state.hex_text) {
                            Some(pick) => {
                                state.hex_invalid = false;
                                apply_pick(ctx, pick, state);
                                persist(ctx);
                            }
                            None => state.hex_invalid = !state.hex_text.trim().is_empty(),
                        }
                    }
                });
            });
    });
}

/// The live preview strip: a play capsule, a switch that is on, a selected
/// clip with a short waveform and a playhead, all in the (fading) theme.
fn preview_strip(ui: &mut Ui, shown: &Theme) {
    ui.horizontal(|ui| {
        play_button(ui, false, PlayKind::Play);
        let mut on = true;
        let response = switch(ui, &mut on);
        response.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, true, "Preview switch"));
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), PREVIEW_HEIGHT), Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let ruler = Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + PREVIEW_RULER_HEIGHT));
            let lanes = Rect::from_min_max(Pos2::new(rect.min.x, ruler.bottom()), rect.max);
            let look = ClipLook { selected: true, ..ClipLook::new("Preview", ClipKind::Audio) };
            let content = paint_clip(painter, lanes, &look, shown);
            paint_waveform(painter, content, &PREVIEW_PEAKS, waveform_color(true, shown));
            paint_playhead(painter, ruler, lanes, rect.left() + rect.width() * 0.6, shown);
        }
    });
}

/// One small non-interactive color dot for the adjusted note.
fn color_dot(ui: &mut Ui, fill: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(NOTE_DOT, NOTE_DOT), Sense::hover());
    ui.painter().circle_filled(rect.center(), NOTE_DOT / 2.0, fill);
}

/// The note rows: "Adjusted for readability" with the picked and applied
/// swatches when the clamp changed the pick, the near-record note when the
/// hue sits within 20 degrees of `sem.record`.
fn notes(ui: &mut Ui, theme: Theme) {
    let neutrals = Theme::get(ui.ctx()).neutrals;
    if theme.accent.adjusted {
        ui.horizontal(|ui| {
            color_dot(ui, theme.accent.pick);
            color_dot(ui, theme.accent.accent);
            ui.label(egui::RichText::new("Adjusted for readability").font(fonts::medium(typography::CAPTION_SIZE)).color(neutrals.text_secondary));
        });
    }
    if theme.accent.near_record {
        ui.label(
            egui::RichText::new("Close to the record color. Record buttons will still show their R and dot.")
                .font(fonts::medium(typography::CAPTION_SIZE))
                .color(neutrals.text_secondary),
        );
    }
}

/// The "Reset to default" text button: back to the app's default accent.
fn reset_row(ui: &mut Ui, ctx: &Context, state: &mut PickerState, theme: Theme) {
    let reset =
        ui.add(egui::Button::new(egui::RichText::new("Reset to default").font(fonts::medium(typography::BODY_SIZE)).color(theme.accent.accent)).frame(false));
    if reset.clicked() {
        apply_pick(ctx, appearance::app(ctx).default_preset.pick(), state);
        persist(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#RRGGBB` text parses; the `#` is optional and whitespace is ignored.
    #[test]
    fn hex_text_parses() {
        assert_eq!(parse_hex("#4C9BE8"), Some(Color32::from_rgb(0x4C, 0x9B, 0xE8))); // style-lint: allow test fixture, not a design value
        assert_eq!(parse_hex(" 4C9BE8 "), Some(Color32::from_rgb(0x4C, 0x9B, 0xE8))); // style-lint: allow test fixture, not a design value
        assert_eq!(parse_hex("nope"), None);
        assert_eq!(parse_hex("#4C9BE"), None);
        assert_eq!(parse_hex(""), None);
    }

    /// The blend endpoints are exact: 0 gives `from`, 1 gives `to`.
    #[test]
    fn blend_endpoints_are_exact() {
        let (a, b) = (Color32::from_rgb(0, 0, 0), Color32::from_rgb(255, 128, 64)); // style-lint: allow test fixture, not a design value
        assert_eq!(blend(a, b, 0.0), a);
        assert_eq!(blend(a, b, 1.0), b);
        assert_eq!(blend(a, b, 0.5), Color32::from_rgb(128, 64, 32)); // style-lint: allow test fixture, not a design value
    }

    /// With no fade running the preview shows the current accent unchanged.
    #[test]
    fn fade_blend_without_a_fade_returns_the_current_accent() {
        let current = accent::default_accent();
        assert_eq!(fade_blend(None, current, 0.0), current);
        assert_eq!(fade_blend(Some(current), current, 0.5), current);
    }
}
