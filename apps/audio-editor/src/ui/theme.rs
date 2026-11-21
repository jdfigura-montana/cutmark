//! ui::theme: the single source for every visual constant (colors, sizes,
//! fonts and spacing) and the egui Style/Visuals applied once at startup.
//!
//! Shim during S1. Each UI file moves to ui_kit directly in phases 11 to 13;
//! this module is deleted in phase 13.

use std::sync::LazyLock;

use eframe::egui;
use ui_kit::tokens;

/// The application theme, constructed once and shared by every ui module.
///
/// `apply` sets the egui Style/Visuals at startup; the fields are read
/// directly by the drawing code so no color or pixel value is hardcoded
/// anywhere else in `src/ui`.
pub struct Theme {
    // ---- Surfaces ----
    /// The app background behind every panel.
    pub app_bg: egui::Color32,
    /// Panel, window and central-area background.
    pub panel_bg: egui::Color32,
    /// Menu bar and status bar background.
    pub bar_bg: egui::Color32,
    /// Timecode ruler background.
    pub ruler_bg: egui::Color32,
    /// Border color of controls.
    pub control_border: egui::Color32,
    /// Fill of an idle control.
    pub control_fill: egui::Color32,

    // ---- Clip visuals ----
    /// Height of the clip header strip (name + gain row).
    pub header_strip_h: f32,
    /// Width of the trimmable clip edge zones.
    pub trim_edge_w: f32,
    /// Side length of a fade handle square.
    pub fade_handle_sz: f32,
    /// Extra grab margin around a fade handle square.
    pub fade_handle_grab: f32,
    /// Vertical grab distance of the clip gain line.
    pub gain_hit_px: f32,
    /// Width of the gain value text area at the right of the clip header.
    pub gain_text_w: f32,
    /// Horizontal padding of the clip header text.
    pub clip_text_pad: f32,
    /// Corner radius of a clip.
    pub clip_radius: f32,
    /// Text color inside a clip header.
    pub clip_text_color: egui::Color32,
    /// Font size of the clip gain value in the header strip.
    pub clip_gain_size: f32,
    /// Vertical inset of a clip rect from its track lane edges.
    pub clip_margin: f32,
    /// Clip width below which a clip draws as a single hairline.
    pub thin_clip_w: f32,
    /// Border width of a selected clip.
    pub selection_border_w: f32,
    /// Fill of an offline clip's body.
    pub offline_body: egui::Color32,
    /// Text color inside an offline clip.
    pub offline_text: egui::Color32,
    /// Bright white line color of the clip gain line.
    pub gain_line: egui::Color32,
    /// Amber curve color of the fade lines.
    pub gain_curve: egui::Color32,
    /// Darkening fill outside the fade curves.
    pub fade_shade: egui::Color32,
    /// Fill of a fade handle square.
    pub fade_handle_fill: egui::Color32,
    /// Fill of a fade handle square while hovered.
    pub fade_handle_hover: egui::Color32,
    /// Stroke of a fade handle square.
    pub fade_handle_stroke: egui::Color32,

    // ---- Waveform ----
    /// Stroke width of the waveform lines.
    pub wave_width: f32,
    /// Radius of the drawn sample dots at extreme zoom-in.
    pub wave_dot_r: f32,

    // ---- Timeline frame ----
    /// Height of the ruler strip (the ui-kit ruler token).
    pub ruler_h: f32,
    /// Width of the track header column.
    pub header_col_w: f32,
    /// Height of the horizontal scrollbar strip.
    pub scrollbar_h: f32,
    /// Minimum scrollbar thumb width.
    pub scrollbar_thumb_min: f32,
    /// Minimum width in points a ruler tick must have to be drawn.
    pub ruler_min_tick_px: f32,
    /// Half width of a marker hit area on the ruler.
    pub marker_tri_w: f32,

    // ---- Interaction ----
    /// Snap distance in points (the "8 px snap distance").
    pub snap_px: f32,
    /// Pointer travel that turns a press into a drag.
    pub drag_start_px: f32,
    /// Grab distance of a range boundary line.
    pub boundary_grab_px: f32,
    /// Solid boundary lines of a range selection.
    pub range_line: egui::Color32,
    /// Width of the range boundary lines.
    pub range_line_w: f32,
    /// Bright vertical insert line on Ctrl while a bin drag hovers.
    pub insert_line_color: egui::Color32,
    /// Width of the insert line.
    pub insert_line_w: f32,
    /// Reorder insertion line color.
    pub reorder_line_color: egui::Color32,
    /// Width of the reorder insertion line.
    pub reorder_line_w: f32,
    /// Translucent fill of the ghost clip while a bin drag hovers.
    pub ghost_fill: egui::Color32,
    /// Border of the ghost clip.
    pub ghost_stroke: egui::Color32,
    /// Dash length of the dashed ghost border.
    pub ghost_dash: f32,
    /// Gap length of the dashed ghost border.
    pub ghost_gap: f32,
    /// Height of the dashed "new track" row (the default track height).
    pub new_track_h: f32,
    /// Minimum width of the clip context menu.
    pub clip_menu_min_w: f32,
    /// Width of the inline gain edit field.
    pub gain_edit_w: f32,

    // ---- Track header ----
    /// Height of the resize strip at the header's bottom edge.
    pub track_resize_h: f32,
    /// Header height below which the header uses the compact layout.
    pub compact_track_h: f32,
    /// Background of an armed record-arm button.
    pub record_red: egui::Color32,
    /// Alpha of a clip body fill (the track color, translucent).
    pub clip_body_alpha: u8,
    /// Alpha of a clip header strip fill.
    pub clip_header_alpha: u8,
    /// Distance from the visible left edge at which a clip name pins when its
    /// own left edge is scrolled out of view.
    pub clip_name_pin: f32,
    /// Gap between a clip header's name and the value at its right.
    pub clip_name_gap: f32,
    /// Narrowest width in which a clip header still shows its name.
    pub clip_name_min_w: f32,
    /// Space left above and below a waveform inside its lane.
    pub wave_lane_pad: f32,

    // ---- Meters ----
    /// Meter bar color above -6 dBFS.
    pub meter_red: egui::Color32,
    /// Meter bar color above -18 dBFS.
    pub meter_yellow: egui::Color32,
    /// Meter bar color below -18 dBFS.
    pub meter_green: egui::Color32,

    // ---- Errors / dialogs ----
    /// Softer error color on the start screen.
    pub error_soft: egui::Color32,

    // ---- Spacing ----

    // ---- Shared shapes ----
    /// Small corner radius for filled rects.
    pub corner_r: f32,
    /// The 1 px hairline stroke width.
    pub hairline: f32,

    // ---- Accent & dialogs ----
    /// The single accent color: selection, playhead, focus, snap and range.
    pub accent: egui::Color32,
    /// Faintest text (hints, disabled labels).
    pub text_faint: egui::Color32,
    /// Corner radius of a menu popup.
    pub menu_radius: f32,
    /// Inner padding of a menu popup.
    pub menu_pad: f32,
    /// Height of a menu item.
    /// Corner radius of a menu item.
    pub menu_item_radius: f32,
    /// Soft shadow under a menu popup.
    pub menu_shadow: egui::Shadow,
    /// Padding between the menu bar's project readout and the window edge.
    /// Unused since phase 10b (the readout became the toolbar's document
    /// title); kept until the theme shim is deleted in phase 13.
    #[allow(dead_code)]
    pub menu_right_pad: f32,
    /// Diameter of the painted "unsaved changes" dot in the menu bar.
    /// Unused since phase 10b (the dot is 8 px of `space::SCALE_8` now);
    /// kept until the theme shim is deleted in phase 13.
    #[allow(dead_code)]
    pub unsaved_dot_d: f32,

    // ---- Start screen ----
    /// Status bar height (26), reused by the start screen status bar.
    pub status_bar_h: f32,
    /// Minimum margin around the start screen panel (48).
    pub start_card_margin: f32,
    /// Start screen hero panel width (420); the recent panel takes the rest.
    pub start_panel_w: f32,
    /// Start screen panel height (640).
    pub start_panel_h: f32,
    /// Start screen content max width (1120).
    pub start_max_w: f32,
    /// Gap between the two start screen panels (16).
    pub start_gap: f32,
    /// Side of the hero's app mark (44).
    pub start_mark: f32,
    /// Height of the hero's decorative waveform band (180).
    pub start_band_h: f32,
    /// Opacity of the hero band and of each of its layers (0.55).
    pub start_band_alpha: f32,
    /// Padding inside an action card, x then y (16, 14).
    pub start_card_pad: egui::Vec2,
    /// Side of the icon tile in an action card (44).
    pub start_card_tile: f32,
    /// Font size of an action card title and the recent panel title (15).
    pub start_card_title_size: f32,
    /// Stroke width of the drop zone border (1.5).
    pub start_drop_stroke: f32,
    /// Color of the drop zone dashed border, white at 14 percent.
    pub start_drop_border: egui::Color32,
    // Unused since 10c, kept until the shim is deleted in phase 13.
    /// Start screen card width (1000).
    #[allow(dead_code)]
    pub start_card_w: f32,
    /// Start screen card height (600).
    #[allow(dead_code)]
    pub start_card_h: f32,
    /// Smallest start screen card width when the window is narrow (760).
    #[allow(dead_code)]
    pub start_card_min_w: f32,
    /// Corner radius of an action card (14).
    pub start_card_radius: f32,
    /// Width of the start screen card's left column (360).
    #[allow(dead_code)]
    pub start_col_w: f32,
    /// Top padding of the left column (36).
    #[allow(dead_code)]
    pub start_pad_top: f32,
    /// Side padding of the left column (32).
    #[allow(dead_code)]
    pub start_pad_side: f32,
    /// Bottom padding of the left column (28).
    #[allow(dead_code)]
    pub start_pad_bottom: f32,
    /// Top padding of the right column (32).
    #[allow(dead_code)]
    pub start_pad_r_top: f32,
    /// Side padding of the right column (28).
    #[allow(dead_code)]
    pub start_pad_r_side: f32,
    /// Bottom padding of the right column (20).
    #[allow(dead_code)]
    pub start_pad_r_bottom: f32,
    /// Height of the logo bar box (40).
    #[allow(dead_code)]
    pub start_logo_h: f32,
    /// Width of one logo bar (4).
    #[allow(dead_code)]
    pub start_bar_w: f32,
    /// Horizontal pitch between logo bars (6).
    #[allow(dead_code)]
    pub start_bar_pitch: f32,
    /// Gap between the logo and the title (20).
    #[allow(dead_code)]
    pub start_title_gap: f32,
    /// Gap between the version line and the buttons (40).
    #[allow(dead_code)]
    pub start_buttons_gap: f32,
    /// Height of the New/Open buttons (52).
    #[allow(dead_code)]
    pub start_btn_h: f32,
    /// Gap between the two buttons (10).
    #[allow(dead_code)]
    pub start_btn_gap: f32,
    /// Corner radius of the buttons (8).
    #[allow(dead_code)]
    pub start_btn_radius: f32,
    /// Gap between the buttons and the drop zone (20).
    #[allow(dead_code)]
    pub start_drop_gap: f32,
    /// Height of the dashed drop zone (84).
    pub start_drop_h: f32,
    /// Corner radius of the drop zone (14).
    pub start_drop_radius: f32,
    /// White text on the accent button.
    #[allow(dead_code)]
    pub start_on_accent: egui::Color32,
    /// Gap between the recent rows (2).
    pub start_row_gap: f32,
    /// Height of one recent row (58).
    pub start_row_h: f32,
    /// Size of the recent row's project tile (40); the action card tile is 44.
    pub start_tile: f32,
    /// Fixed width of the last-opened date column (96).
    #[allow(dead_code)]
    pub start_last_col_w: f32,
    /// Size of the per-row remove button (28).
    pub start_x_btn: f32,
    /// Fill of the MISSING pill.
    #[allow(dead_code)]
    pub start_missing_fill: egui::Color32,
    /// At-rest background of the most recently opened recent row (#1F2328).
    #[allow(dead_code)]
    pub start_row_recent: egui::Color32,
    /// Text color of the MISSING pill.
    #[allow(dead_code)]
    pub start_missing_text: egui::Color32,
    /// Icon color of a missing project's tile.
    #[allow(dead_code)]
    pub start_missing_icon: egui::Color32,
    /// Hero app name font size (26).
    pub start_title_size: f32,
    /// Small mono font size: folder path in a recent row (11).
    pub start_small_size: f32,
    /// Button label font size (14).
    #[allow(dead_code)]
    pub start_btn_text_size: f32,
    /// Hint font size: drop zone, footnotes, dates, Clear missing (12).
    pub start_hint_size: f32,
    /// Recent row name font size (14).
    #[allow(dead_code)]
    pub start_name_size: f32,
    /// MISSING pill font size (10.5).
    #[allow(dead_code)]
    pub start_pill_size: f32,
    /// Phosphor icon size in tiles and the drop zone (20).
    pub start_icon_size: f32,
    /// Height of the recent-projects header row (22).
    pub start_header_h: f32,
    /// Corner radius of a recent row (10).
    pub start_row_radius: f32,
    /// Dash length of the drop-zone dashed border (4).
    pub start_dash: f32,
    /// Gap between dashes of the drop-zone border (4).
    pub start_dash_gap: f32,
}

impl Theme {
    /// Monospace font at `size`, used for every timecode readout.
    pub fn mono(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Monospace)
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            // Surfaces, from the ui-kit tokens (the ui_kit::palette mapping).
            app_bg: tokens::color::BG_GROUND,
            panel_bg: tokens::color::BG_PANEL,
            bar_bg: tokens::color::BG_GROUND,
            // The ruler and the track lanes have no dedicated tokens yet;
            // these are the closest surfaces. Phase 11 restyles them.
            ruler_bg: tokens::color::BG_PANEL,
            control_border: tokens::color::STROKE_HAIRLINE,
            control_fill: tokens::color::BG_CONTROL,
            // Clip visuals
            header_strip_h: 18.0,
            trim_edge_w: 6.0,
            fade_handle_sz: 8.0,
            fade_handle_grab: 2.0,
            gain_hit_px: 4.0,
            gain_text_w: 56.0,
            clip_text_pad: 4.0,
            clip_radius: 5.0,
            clip_text_color: tokens::color::TEXT_PRIMARY,
            clip_gain_size: 10.0,
            clip_margin: 2.0,
            thin_clip_w: 3.0,
            selection_border_w: 2.0,
            offline_body: egui::Color32::from_rgba_unmultiplied(150, 62, 62, 220),
            offline_text: egui::Color32::WHITE,
            gain_line: tokens::color::TEXT_PRIMARY,
            gain_curve: text_alpha(178),
            fade_shade: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 90),
            fade_handle_fill: tokens::color::TEXT_PRIMARY,
            fade_handle_hover: tokens::accent::ACCENT,
            fade_handle_stroke: egui::Color32::from_rgb(18, 20, 23),
            // Waveform
            wave_width: 1.0,
            wave_dot_r: 1.5,
            // Timeline frame
            ruler_h: tokens::size::RULER_HEIGHT,
            header_col_w: 208.0,
            scrollbar_h: 12.0,
            scrollbar_thumb_min: 20.0,
            ruler_min_tick_px: 60.0,
            marker_tri_w: 5.0,
            // Interaction
            snap_px: 8.0,
            drag_start_px: 3.0,
            boundary_grab_px: 4.0,
            range_line: tokens::accent::ACCENT,
            range_line_w: 1.0,
            insert_line_color: accent_alpha(230),
            insert_line_w: 2.0,
            reorder_line_color: tokens::accent::ACCENT,
            reorder_line_w: 2.0,
            ghost_fill: accent_alpha(90),
            ghost_stroke: accent_alpha(220),
            ghost_dash: 6.0,
            ghost_gap: 4.0,
            new_track_h: 96.0,
            clip_menu_min_w: 150.0,
            gain_edit_w: 52.0,
            // Track header
            track_resize_h: 4.0,
            compact_track_h: 64.0,
            record_red: tokens::color::SEM_RECORD,
            clip_body_alpha: 0x22,
            clip_header_alpha: 0x66,
            clip_name_pin: 16.0,
            clip_name_gap: 6.0,
            clip_name_min_w: 8.0,
            wave_lane_pad: 1.0,
            // Meters
            meter_red: tokens::color::SEM_METER_CLIP,
            meter_yellow: tokens::color::SEM_METER_WARN,
            meter_green: tokens::color::SEM_METER_OK,

            // Bin panel
            // Errors / dialogs
            error_soft: egui::Color32::from_rgb(235, 140, 140),
            // Spacing: 4/8 px grid, from the ui-kit space scale.
            // Shared shapes
            corner_r: 2.0,
            hairline: 1.0,
            // Accent, text roles & dialogs, from the ui-kit tokens.
            accent: tokens::accent::ACCENT,
            text_faint: tokens::color::TEXT_DISABLED,
            // Menus keep their pre-restyle popup look until phase 13 turns
            // them into ui-kit glass popovers; no token matches these yet.
            menu_radius: 8.0,
            menu_pad: 6.0,
            menu_item_radius: 5.0,
            menu_shadow: egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(90) },
            menu_right_pad: 8.0,
            unsaved_dot_d: 6.0,
            // Start screen
            status_bar_h: tokens::size::STATUS_BAR_HEIGHT,
            start_card_margin: 48.0,
            start_panel_w: 420.0,
            start_panel_h: 640.0,
            start_max_w: 1120.0,
            start_gap: 16.0,
            start_mark: 44.0,
            start_band_h: 180.0,
            start_band_alpha: 0.55,
            start_card_pad: egui::vec2(16.0, 14.0),
            start_card_tile: 44.0,
            start_card_title_size: 15.0,
            start_drop_stroke: 1.5,
            start_drop_border: egui::Color32::from_white_alpha(36),
            start_card_w: 1000.0,
            start_card_h: 600.0,
            start_card_min_w: 760.0,
            start_card_radius: 14.0,
            start_col_w: 360.0,
            start_pad_top: 36.0,
            start_pad_side: 32.0,
            start_pad_bottom: 28.0,
            start_pad_r_top: 32.0,
            start_pad_r_side: 28.0,
            start_pad_r_bottom: 20.0,
            start_logo_h: 40.0,
            start_bar_w: 4.0,
            start_bar_pitch: 6.0,
            start_title_gap: 20.0,
            start_buttons_gap: 40.0,
            start_btn_h: 52.0,
            start_btn_gap: 10.0,
            start_btn_radius: 8.0,
            start_drop_gap: 20.0,
            start_drop_h: 84.0,
            start_drop_radius: 14.0,
            start_on_accent: egui::Color32::WHITE,
            start_row_gap: 2.0,
            start_row_h: 58.0,
            start_tile: 40.0,
            start_last_col_w: 96.0,
            start_x_btn: 28.0,
            start_missing_fill: egui::Color32::from_rgb(0x3A, 0x22, 0x24),
            start_row_recent: egui::Color32::from_rgb(0x1F, 0x23, 0x28),
            start_missing_text: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            start_missing_icon: egui::Color32::from_rgb(0x5A, 0x61, 0x6D),
            start_title_size: 26.0,
            start_small_size: 11.0,
            start_btn_text_size: 14.0,
            start_hint_size: 12.0,
            start_name_size: 14.0,
            start_pill_size: 10.5,
            start_icon_size: 20.0,
            start_header_h: 22.0,
            start_row_radius: 10.0,
            start_dash: 4.0,
            start_dash_gap: 4.0,
        }
    }
}

/// The shared theme instance every ui module reads.
pub static THEME: LazyLock<Theme> = LazyLock::new(Theme::default);

/// `tokens::accent::ACCENT` at `alpha`: the alpha-composited accent colors
/// (soft highlights, hover overlay, insert line, ghost clip). Computed at
/// runtime because the alpha variants have no token of their own.
fn accent_alpha(alpha: u8) -> egui::Color32 {
    let a = tokens::accent::ACCENT;
    egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), alpha)
}

/// `tokens::color::TEXT_PRIMARY` at `alpha` (the gain curve line).
fn text_alpha(alpha: u8) -> egui::Color32 {
    let a = tokens::color::TEXT_PRIMARY;
    egui::Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), alpha)
}

/// The menu popup frame: panel_bg fill, 1 px control_border, radius 8,
/// 6 px padding and a soft shadow.
pub fn menu_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(THEME.panel_bg)
        .stroke(egui::Stroke::new(THEME.hairline, THEME.control_border))
        .corner_radius(THEME.menu_radius)
        .shadow(THEME.menu_shadow)
        .inner_margin(egui::Margin::same(THEME.menu_pad as i8))
}

/// The menu item / popup style: egui's menu style plus 26 px items with
/// radius 5, control-fill hover, faint disabled text and 4 px separators.
///
/// Passed to `MenuConfig::style` / `Popup::style`, or applied directly with
/// `ui.style_mut()` for hand-drawn menus.
pub fn menu_item_style(style: &mut egui::Style) {
    eframe::egui::containers::menu::menu_style(style);
    style.spacing.button_padding = egui::vec2(8.0, 5.0);
    let t = &*THEME;
    for w in [&mut style.visuals.widgets.inactive, &mut style.visuals.widgets.hovered, &mut style.visuals.widgets.active, &mut style.visuals.widgets.open] {
        w.corner_radius = t.menu_item_radius.into();
    }
    style.visuals.widgets.hovered.bg_fill = t.control_fill;
    style.visuals.widgets.hovered.weak_bg_fill = t.control_fill;
    style.visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(t.hairline, t.text_faint);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The phosphor icon glyphs resolve, and both cuts sit in the fallback
    /// list of every font family THEME draws with, under the ui-kit font set
    /// `ui_kit::apply` installs.
    ///
    /// `Fonts::has_glyph` only checks the FIRST face of a family, so the
    /// fallback membership is asserted structurally; real text layout walks
    /// the fallbacks and renders the icons.
    #[test]
    fn icons_resolve_in_every_theme_family() {
        use eframe::egui::epaint::text::{Fonts, TextOptions};
        let defs = ui_kit::fonts::definitions();
        let families = [
            egui::FontFamily::Proportional,
            egui::FontFamily::Monospace,
            egui::FontFamily::Name(ui_kit::fonts::MEDIUM_FAMILY.into()),
            egui::FontFamily::Name(ui_kit::fonts::SEMIBOLD_FAMILY.into()),
        ];
        // Both phosphor cuts are in every family's fallback list (the
        // tofu-icons regression this test guards).
        for family in &families {
            let keys = defs.families.get(family).expect("every THEME family is defined");
            for cut in [ui_kit::fonts::PHOSPHOR_REGULAR_FONT, ui_kit::fonts::PHOSPHOR_FILL_FONT] {
                assert!(keys.iter().any(|key| key == cut), "phosphor cut {cut} missing from {family:?}");
            }
        }
        let mut fonts = Fonts::new(TextOptions::default(), defs);
        // Strip every icon-capable face: the phosphor cuts AND egui's own
        // icon font defaults. What is left holds text only, so the icons
        // must NOT resolve: this proves the probe is not blind.
        let mut bare = ui_kit::fonts::definitions();
        for keys in bare.families.values_mut() {
            keys.retain(|key| key.starts_with("Inter-") || key.starts_with("JetBrainsMono-") || key == "Ubuntu-Light" || key == "NotoSans-Regular");
        }
        let mut bare_fonts = Fonts::new(TextOptions::default(), bare);
        for (name, icon) in [
            ("FOLDER_OPEN", ui_kit::icons::fill::FOLDER_OPEN),
            ("FILE_AUDIO", ui_kit::icons::fill::FILE_AUDIO),
            ("PLUS", ui_kit::icons::fill::PLUS),
            ("X", ui_kit::icons::fill::X),
            ("WAVEFORM", ui_kit::icons::fill::WAVEFORM),
        ] {
            let c = icon.chars().next().unwrap();
            // The glyphs live in the phosphor font: both dedicated cut
            // families (holding that cut alone) resolve them.
            for cut in [ui_kit::fonts::PHOSPHOR_REGULAR_FONT, ui_kit::fonts::PHOSPHOR_FILL_FONT] {
                let id = egui::FontId::new(16.0, egui::FontFamily::Name(cut.into()));
                assert!(fonts.has_glyph(&id, c), "{name} (U+{:04X}) missing from the {cut} family", c as u32);
            }
            // The text-only set has none of them.
            for family in &families {
                let id = egui::FontId::new(16.0, family.clone());
                assert!(!bare_fonts.has_glyph(&id, c), "{name} (U+{:04X}) resolves WITHOUT an icon font in {family:?}: the probe is blind", c as u32);
            }
        }
    }

    /// THEME takes its values from the ui-kit tokens (the S1 shim contract).
    #[test]
    fn theme_holds_the_spec_values() {
        let t = Theme::default();
        assert_eq!(t.header_strip_h, 18.0);
        assert_eq!(t.trim_edge_w, 6.0);
        assert_eq!(t.snap_px, 8.0);
        assert_eq!(t.fade_handle_sz, 8.0);
        assert_eq!(t.ruler_h, tokens::size::RULER_HEIGHT);
        assert_eq!(t.new_track_h, 96.0);
        assert_eq!(t.meter_red, tokens::color::SEM_METER_CLIP);
        assert_eq!(t.meter_yellow, tokens::color::SEM_METER_WARN);
        assert_eq!(t.meter_green, tokens::color::SEM_METER_OK);
        assert_eq!(t.accent, tokens::accent::ACCENT, "one accent for selection, lit toggles, range lines and focus");
        assert_eq!(t.unsaved_dot_d, 6.0);
        assert_eq!(t.record_red, tokens::color::SEM_RECORD);
        assert_eq!((t.clip_body_alpha, t.clip_header_alpha), (0x22, 0x66));
        assert_eq!(t.menu_right_pad, 8.0);
        assert_eq!(t.app_bg, tokens::color::BG_GROUND);
        assert_eq!(t.status_bar_h, tokens::size::STATUS_BAR_HEIGHT);
    }
}
