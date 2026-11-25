//! ui-kit: the Cutmark suite's base theme.
//!
//! The design tokens ([`tokens`]), one shared compatibility palette
//! ([`palette`]), one shared font set ([`fonts`]), the Phosphor icon glyphs
//! ([`icons`]) and one [`apply`] call that installs the fonts plus the egui
//! `Style` and `Visuals` built from the tokens and the runtime [`Theme`], so
//! every app in the suite (the Recorder, and the audio editor once it moves
//! onto this crate) gets the Glass Deck look.
//!
//! The crate sets no renderer: apps choose the eframe renderer themselves.
//!
//! # Font licenses
//!
//! Inter and JetBrains Mono are both SIL OFL 1.1. Their license files
//! sit in `assets/fonts` and must ship with any installer that uses ui-kit.

pub mod accent;
pub mod appearance;
pub mod chrome;
pub mod fonts;
pub mod help;
pub mod icons;
pub mod meter;
pub mod palette;
pub mod shortcuts;
pub mod theme;
pub mod tokens;
pub mod tour;
pub mod widgets;

pub use appearance::{AppAppearance, Preset};
pub use meter::{level_meter, level_meter_vertical, stereo_meter_vertical};
pub use theme::Theme;
pub use widgets::{
    AboutInfo, CaptionRects, ClipKind, ClipLook, DeckState, MainToolbar, MarkerKind, Menu, MenuBarOutput, PlayKind, SliderKind, Title, Toggle, ToggleKind,
    about_dialog, accent_picker, appearance_dialog, caption_buttons, chip, deck_button, dialog, fader, focus_ring, glass_frame, icon_button, inspector_group,
    inspector_header, inspector_row, list_row, main_toolbar, marker_color, menu_bar, paint_clip, paint_glass_highlight, paint_marker, paint_marquee,
    paint_pending_band, paint_playhead, paint_ruler, paint_snap_line, paint_track_area_empty, paint_track_header_column, paint_track_row, paint_waveform,
    panel, play_button, primary_button, scroll_thumb, search_field, segmented, segmented_filled, set_toast_time, show_toasts, slider, status_bar, switch,
    text_field, timecode, title_block, toast, tool_strip, track_header, transport_deck, waveform_color,
};

use tokens::{color, radius, size, space, typography};

/// The 1 px hairline stroke width.
const HAIRLINE: f32 = 1.0;
/// The 2 px selection and focus stroke width.
const SELECTION_STROKE: f32 = 2.0;
/// Alpha of the accent behind selected text (spec "States": selection).
const SELECTION_BG_ALPHA: f32 = 0.35;
/// Opacity of the scrollbar handle at rest (spec "Scrollbar": tertiary at 40%).
const SCROLL_DORMANT_OPACITY: f32 = 0.4;
/// Opacity of the scrollbar handle on hover and while dragging (spec "Scrollbar": 70%).
const SCROLL_ACTIVE_OPACITY: f32 = 0.7;
/// Minimum length of the scrollbar handle.
const SCROLL_HANDLE_MIN_LENGTH: f32 = 24.0;
/// Height of a menu and menu bar row (spec "Components": "Menus and
/// popovers": 32px rows). egui sizes menu items from `interact_size.y`.
const MENU_ROW_HEIGHT: f32 = 32.0;

/// Sets the fonts and the egui `Style` and `Visuals` once at startup.
///
/// Builds the whole look from the design tokens and the runtime
/// [`Theme`] (spec: "Glass Deck"): the token surfaces, the glass popup
/// shadow, the States-table widget fills, one accent color for selection and
/// focus, Inter (Proportional) and JetBrains Mono (Monospace) with named
/// Inter families for the weight steps, the phosphor icon font, and the
/// token spacing grid.
///
/// The app's own appearance file feeds the theme; the app is the one stored
/// by [`apply_for`] (tests that skip it get [`appearance::AppAppearance::FALLBACK`]).
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts::definitions());
    // The app's appearance file feeds the runtime theme before anything reads it.
    Theme::set(ctx, appearance::load(&appearance::app(ctx)));
    let theme = Theme::get(ctx);
    let accent = theme.accent.accent;
    let mut v = egui::Visuals::dark();

    // Surfaces from the surface tokens; menus and popovers are glass (spec
    // "Menus and popovers" and "Elevation and glass").
    v.panel_fill = color::BG_PANEL;
    apply_glass(&mut v);
    v.extreme_bg_color = color::BG_CONTROL; // recessed surfaces (fader slot, etc.)
    v.faint_bg_color = color::BG_TRACK_ALT; // alternate row shade
    v.window_stroke = egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE);
    v.window_corner_radius = radius::GROUP.into();
    v.menu_corner_radius = radius::GROUP.into();

    // One accent for selection, focus and links.
    apply_accent(&mut v, accent);

    // Widgets: every state takes its fill from the States table, no border
    // stroke except the hairline on noninteractive elements, and its text
    // from the text tokens (tertiary captions, secondary idle, primary on
    // hover and press).
    v.override_text_color = Some(color::TEXT_SECONDARY);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = radius::SEGMENT.into();
    }
    v.widgets.noninteractive.bg_fill = color::BG_PANEL;
    v.widgets.noninteractive.weak_bg_fill = color::BG_PANEL;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(HAIRLINE, color::STROKE_HAIRLINE);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(HAIRLINE, color::TEXT_TERTIARY);
    v.widgets.inactive.bg_fill = color::BG_CONTROL;
    v.widgets.inactive.weak_bg_fill = color::BG_CONTROL;
    v.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(HAIRLINE, color::TEXT_SECONDARY);
    v.widgets.hovered.bg_fill = color::BG_HOVER;
    v.widgets.hovered.weak_bg_fill = color::BG_HOVER;
    v.widgets.hovered.bg_stroke = egui::Stroke::NONE;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(HAIRLINE, color::TEXT_PRIMARY);
    v.widgets.active.bg_fill = color::BG_SELECTED;
    v.widgets.active.weak_bg_fill = color::BG_SELECTED;
    v.widgets.active.bg_stroke = egui::Stroke::NONE;
    v.widgets.active.fg_stroke = egui::Stroke::new(HAIRLINE, color::TEXT_PRIMARY);
    v.widgets.open.bg_fill = color::BG_SELECTED;
    v.widgets.open.weak_bg_fill = color::BG_SELECTED;
    v.widgets.open.bg_stroke = egui::Stroke::NONE;
    v.widgets.open.fg_stroke = egui::Stroke::new(HAIRLINE, color::TEXT_SECONDARY);

    // Text styles from the type tokens; the token spacing grid; the
    // overlay scrollbar (spec "Scrollbar": 6px thumb, tertiary at 40%, 70%
    // on hover and while dragging).
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(egui::TextStyle::Body, fonts::medium(typography::BODY_SIZE));
        style.text_styles.insert(egui::TextStyle::Button, fonts::medium(typography::BODY_SIZE));
        style.text_styles.insert(egui::TextStyle::Small, egui::FontId::new(typography::CAPTION_SIZE, egui::FontFamily::Proportional));
        style.text_styles.insert(egui::TextStyle::Heading, fonts::semibold(typography::TITLE_SIZE));
        style.text_styles.insert(egui::TextStyle::Monospace, fonts::mono(typography::VALUE_SIZE));

        style.spacing.item_spacing = egui::vec2(space::SCALE_8, space::SCALE_6);
        style.spacing.button_padding = egui::vec2(space::SCALE_12, space::SCALE_6);
        style.spacing.menu_margin = egui::Margin::same(space::SCALE_6 as i8);
        style.spacing.window_margin = egui::Margin::same(space::SCALE_8 as i8);
        // egui sizes menu items and the menu bar from this minimum, so it sets
        // the 32px menu rows (spec "Menus and popovers").
        style.spacing.interact_size.y = MENU_ROW_HEIGHT;

        let scroll = &mut style.spacing.scroll;
        scroll.floating = true;
        scroll.foreground_color = true;
        scroll.bar_width = size::SCROLLBAR_THUMB_WIDTH;
        scroll.handle_min_length = SCROLL_HANDLE_MIN_LENGTH;
        scroll.dormant_handle_opacity = SCROLL_DORMANT_OPACITY;
        scroll.active_handle_opacity = SCROLL_ACTIVE_OPACITY;
        scroll.interact_handle_opacity = SCROLL_ACTIVE_OPACITY;
        scroll.dormant_background_opacity = 0.0;
        scroll.active_background_opacity = 0.0;
        scroll.interact_background_opacity = 0.0;
    });

    ctx.set_visuals(v);
}

/// [`apply`] for one app: stores the app's id and default accent first, so
/// the theme comes from `<Cutmark folder>/<app>/appearance.json` and the
/// accent picker resets to the app's default.
///
/// Usage:
///
/// ```ignore
/// ui_kit::apply_for(&cc.egui_ctx, AppAppearance::new("audio", Preset::Blue));
/// ```
pub fn apply_for(ctx: &egui::Context, app: appearance::AppAppearance) {
    appearance::set_app(ctx, app);
    apply(ctx);
}

/// Re-applies the theme-dependent parts of the `Visuals` from the runtime
/// [`Theme`]: the accent and the glass popup look. [`appearance::poll`] calls
/// this when it loads a new theme.
pub fn refresh(ctx: &egui::Context) {
    let theme = Theme::get(ctx);
    ctx.all_styles_mut(|style| {
        apply_accent(&mut style.visuals, theme.accent.accent);
        apply_glass(&mut style.visuals);
    });
}

/// Writes the accent into selection (35% alpha fill, 2px stroke) and links.
fn apply_accent(v: &mut egui::Visuals, accent: egui::Color32) {
    v.selection.bg_fill = accent.gamma_multiply(SELECTION_BG_ALPHA);
    v.selection.stroke = egui::Stroke::new(SELECTION_STROKE, accent);
    v.hyperlink_color = accent;
}

/// Writes the glass popup look (spec "Menus and popovers", "Elevation and
/// glass"): menus and popovers take the glass fill and the glass shadow.
fn apply_glass(v: &mut egui::Visuals) {
    v.window_fill = color::GLASS_FILL;
    v.popup_shadow = widgets::glass::glass_shadow();
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

    /// apply builds the Visuals from the tokens and the loaded theme.
    #[test]
    fn apply_builds_visuals_from_tokens_and_the_theme() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let visuals = &ctx.style_of(ctx.theme()).visuals;
        assert_eq!(visuals.panel_fill, tokens::color::BG_PANEL);
        assert_eq!(visuals.widgets.hovered.weak_bg_fill, tokens::color::BG_HOVER);
        assert_eq!(visuals.selection.stroke.color, Theme::get(&ctx).accent.accent, "selection stroke follows the theme accent");
        assert_eq!(visuals.hyperlink_color, Theme::get(&ctx).accent.accent);
    }

    /// refresh() moves the accent-dependent Visuals to a newly set theme.
    #[test]
    fn refresh_follows_a_new_theme() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let violet = accent::derive(tokens::accent::PRESET_VIOLET);
        Theme::set(&ctx, Theme { accent: violet, reduce_motion: false });
        refresh(&ctx);
        let visuals = &ctx.style_of(ctx.theme()).visuals;
        assert_eq!(visuals.selection.stroke.color, violet.accent, "selection stroke follows Violet after refresh");
        assert_eq!(visuals.selection.bg_fill, violet.accent.gamma_multiply(0.35), "selection fill is the accent at 35% alpha");
        assert_eq!(visuals.hyperlink_color, violet.accent);
    }

    /// Menus and popovers always take the glass fill and shadow, also after a refresh.
    #[test]
    fn menus_and_popovers_are_always_glass() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let visuals = &ctx.style_of(ctx.theme()).visuals;
        assert_eq!(visuals.window_fill, tokens::color::GLASS_FILL, "menus and popovers use the glass fill");
        assert_eq!(visuals.popup_shadow.color, tokens::color::GLASS_SHADOW, "popups cast the glass shadow");
        assert_eq!(visuals.menu_corner_radius.nw, tokens::radius::GROUP as u8, "menu corners use radius.group");

        refresh(&ctx);
        let visuals = &ctx.style_of(ctx.theme()).visuals;
        assert_eq!(visuals.window_fill, tokens::color::GLASS_FILL, "still glass after refresh");
    }
}
