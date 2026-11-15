//! ui-kit: the Cutmark suite's base dark theme.
//!
//! One shared palette ([`palette`]), one shared font set ([`fonts`]) and one
//! [`apply`] call that installs both plus the egui `Style` and `Visuals`, so
//! every app in the suite (the Recorder, and the audio editor once it moves
//! onto this crate) looks the same.
//!
//! The crate sets no renderer: apps choose the eframe renderer themselves.
//!
//! # Font licenses
//!
//! IBM Plex Sans and JetBrains Mono are both SIL OFL 1.1. Their license files
//! sit in `assets/fonts` and must ship with any installer that uses ui-kit.

pub mod fonts;
pub mod palette;

use palette::{
    ACCENT, BORDER, CONTROL_BORDER, CONTROL_FILL, CONTROL_HOVER, CONTROL_ROUNDING, FAINT_BG, PANEL_BG, RECESSED, TEXT_MUTED, TEXT_NORMAL, TEXT_PRIMARY,
};

/// The 1 px hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// Small spacing (4).
const SPACE_SM: f32 = 4.0;
/// Medium spacing (8).
const SPACE_MD: f32 = 8.0;
/// Corner radius of windows and menu popups.
const MENU_RADIUS: f32 = 8.0;
/// Inner padding of menu popups.
const MENU_PAD: i8 = 6;

/// Sets the fonts and the egui `Style` and `Visuals` once at startup.
///
/// Owns the whole look: the dark palette surfaces, a single accent color for
/// selection and focus, IBM Plex Sans (Proportional) and JetBrains Mono
/// (Monospace) with a named "PlexSemiBold" family for headings, the phosphor
/// icon font, and 4/8 px default spacing.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts::definitions());
    let mut v = egui::Visuals::dark();

    // Surfaces from the palette.
    v.panel_fill = PANEL_BG;
    v.window_fill = PANEL_BG;
    v.extreme_bg_color = RECESSED; // recessed surfaces (fader slot, etc.)
    v.faint_bg_color = FAINT_BG; // alternate row shade
    v.window_stroke = egui::Stroke::new(HAIRLINE, CONTROL_BORDER);
    v.window_corner_radius = MENU_RADIUS.into();
    v.menu_corner_radius = MENU_RADIUS.into();
    v.popup_shadow = egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(90) };

    // One accent for selection, focus and links.
    v.selection.stroke = egui::Stroke::new(HAIRLINE, ACCENT);
    v.selection.bg_fill = ACCENT;
    v.hyperlink_color = ACCENT;

    // Widgets: every state takes its fills from the control tokens, its
    // strokes from the border tokens and its text from the text tokens.
    v.override_text_color = Some(TEXT_NORMAL);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = CONTROL_ROUNDING.into();
    }
    v.widgets.noninteractive.bg_fill = PANEL_BG;
    v.widgets.noninteractive.weak_bg_fill = PANEL_BG;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(HAIRLINE, BORDER);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(HAIRLINE, TEXT_MUTED);
    v.widgets.inactive.bg_fill = CONTROL_FILL;
    v.widgets.inactive.weak_bg_fill = CONTROL_FILL;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(HAIRLINE, CONTROL_BORDER);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(HAIRLINE, TEXT_NORMAL);
    v.widgets.hovered.bg_fill = CONTROL_HOVER;
    v.widgets.hovered.weak_bg_fill = CONTROL_HOVER;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(HAIRLINE, CONTROL_BORDER);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(HAIRLINE, TEXT_NORMAL);
    v.widgets.active.bg_fill = CONTROL_HOVER;
    v.widgets.active.weak_bg_fill = CONTROL_HOVER;
    v.widgets.active.bg_stroke = egui::Stroke::new(HAIRLINE, CONTROL_BORDER);
    v.widgets.active.fg_stroke = egui::Stroke::new(HAIRLINE, TEXT_PRIMARY);
    v.widgets.open.bg_fill = CONTROL_FILL;
    v.widgets.open.weak_bg_fill = CONTROL_FILL;
    v.widgets.open.bg_stroke = egui::Stroke::new(HAIRLINE, CONTROL_BORDER);
    v.widgets.open.fg_stroke = egui::Stroke::new(HAIRLINE, TEXT_NORMAL);

    // 4/8 px spacing grid; headings use the SemiBold face.
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(SPACE_MD, SPACE_SM);
        style.spacing.button_padding = egui::vec2(SPACE_MD, SPACE_SM);
        style.spacing.menu_margin = egui::Margin::same(MENU_PAD);
        style.spacing.window_margin = SPACE_MD.into();
        let heading_size = style.text_styles.get(&egui::TextStyle::Heading).map(|font| font.size).unwrap_or(18.0);
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(heading_size, egui::FontFamily::Name(fonts::SEMIBOLD_FAMILY.into())));
    });

    ctx.set_visuals(v);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_installs_non_empty_font_families() {
        let ctx = egui::Context::default();
        apply(&ctx);
        // set_fonts takes effect at the start of the next pass.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        // No renderer consumes the texture deltas here; egui panics if they are dropped unapplied.
        output.textures_delta.clear();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let non_empty = ctx.fonts(|f| f.definitions().families.get(&family).is_some_and(|names| !names.is_empty()));
            assert!(non_empty, "{family:?} must hold at least one font");
        }
    }
}
