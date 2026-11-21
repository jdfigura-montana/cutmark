//! ui::theme: the single source for every visual constant (colors, sizes,
//! fonts and spacing) and the egui Style/Visuals applied once at startup.
//!
//! Shim during S1. Each UI file moves to ui_kit directly in phases 11 to 13;
//! this module is deleted in phase 13.

use std::sync::LazyLock;

use eframe::egui;
use ui_kit::tokens;

/// Named family holding the Inter Medium face (ui-kit's medium weight step).
const MEDIUM_FAMILY: &str = ui_kit::fonts::MEDIUM_FAMILY;

/// The named egui font family holding the Inter SemiBold face.
const SEMIBOLD_FAMILY: &str = ui_kit::fonts::SEMIBOLD_FAMILY;

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
    /// Recessed surface of the meter bars (unlit).
    pub recessed_a: egui::Color32,
    /// Timecode ruler background.
    pub ruler_bg: egui::Color32,
    /// General separator and window border color.
    pub border: egui::Color32,
    /// Border color of controls.
    pub control_border: egui::Color32,
    /// Fill of an idle control.
    pub control_fill: egui::Color32,
    /// Fill of a hovered control.
    pub control_hover: egui::Color32,
    /// Corner rounding of controls.
    pub control_rounding: f32,
    /// Track color of an egui slider.
    pub slider_track: egui::Color32,
    /// Filled part of an egui slider.
    pub slider_fill: egui::Color32,
    /// Thumb color of an egui slider.
    pub slider_thumb: egui::Color32,
    /// Green of the transport play icon.
    pub play_green: egui::Color32,

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
    /// Thickness of the custom slider track.
    pub track_slider_track_w: f32,
    /// Diameter of the custom slider thumb.
    pub track_slider_thumb: f32,
    /// Header height below which the header uses the compact layout.
    pub compact_track_h: f32,
    /// Background of an armed record-arm button.
    pub record_red: egui::Color32,
    /// Glyph color on a `record_red` fill (an armed Arm button, the Record
    /// button while recording).
    pub on_record_glyph: egui::Color32,
    /// Diameter of the round toolbar Record button.
    pub record_btn_d: f32,
    /// Font size of the glyph on the toolbar Record button.
    pub record_btn_glyph: f32,
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
    /// Background fill of a meter bar area.
    pub meter_bar_bg: egui::Color32,
    /// Meter bar color above -6 dBFS.
    pub meter_red: egui::Color32,
    /// Meter bar color above -18 dBFS.
    pub meter_yellow: egui::Color32,
    /// Meter bar color below -18 dBFS.
    pub meter_green: egui::Color32,
    /// Peak-hold marker color.
    pub meter_hold: egui::Color32,
    /// Clip light color while latched.
    pub clip_light_on: egui::Color32,
    /// Clip light color while clear.
    pub clip_light_off: egui::Color32,

    // ---- Bin panel ----
    /// Height of one bin item row.
    pub bin_row_h: f32,
    /// Vertical gap between bin rows.
    pub bin_row_gap: f32,
    /// Corner radius of the bin row background.
    pub bin_row_rounding: f32,
    /// Font size of the MEDIA header caption.
    pub bin_header_size: f32,
    /// Height of the header Import / sort buttons.
    pub bin_btn_h: f32,
    /// Font size of the header buttons.
    pub bin_btn_text_size: f32,
    /// Side length of the square waveform tile.
    pub bin_tile_sz: f32,
    /// Font size of the tile's waveform icon.
    pub bin_tile_icon_size: f32,
    /// Font size of the item name.
    pub bin_name_size: f32,
    /// Font size of the monospace meta line.
    pub bin_meta_size: f32,
    /// Fill of the usage pill.
    pub bin_pill_fill: egui::Color32,
    /// Height of the usage pill.
    pub bin_pill_h: f32,
    /// Corner radius of the usage pill.
    pub bin_pill_rounding: f32,
    /// Font size of the usage pill text.
    pub bin_pill_text_size: f32,
    /// Height of the dashed drop zone.
    pub bin_drop_h: f32,
    /// Font size of the drop zone text.
    pub bin_drop_text_size: f32,
    /// Height of the preview progress bar in a bin row.
    pub bin_progress_h: f32,
    /// Width taken from the panel before the inline rename field.
    pub bin_rename_pad: f32,
    /// Minimum width of the inline rename field.
    pub bin_rename_min_w: f32,
    /// Color of offline bin item names.
    pub bin_offline: egui::Color32,

    // ---- Errors / dialogs ----
    /// Error text color inside dialogs.
    pub error: egui::Color32,
    /// Softer error color on the start screen.
    pub error_soft: egui::Color32,

    // ---- Spacing ----
    /// Extra-small spacing (2).
    pub space_xs: f32,
    /// Small spacing (4).
    pub space_sm: f32,
    /// Medium spacing (8).
    pub space_md: f32,
    /// Large spacing (16).
    pub space_lg: f32,
    /// Vertical grid spacing inside dialogs.

    // ---- Shared shapes ----
    /// Small corner radius for filled rects.
    pub corner_r: f32,
    /// Smallest corner radius (handles, accent bars).
    pub corner_xs: f32,
    /// The 1 px hairline stroke width.
    pub hairline: f32,

    // ---- Accent & dialogs ----
    /// The single accent color: selection, playhead, focus, snap and range.
    pub accent: egui::Color32,
    /// The accent at low alpha, for soft highlights.
    pub accent_soft: egui::Color32,
    /// Brightest text (strong text, headings).
    pub text_primary: egui::Color32,
    /// Default text color.
    pub text_normal: egui::Color32,
    /// Muted secondary text.
    pub text_muted: egui::Color32,
    /// Faintest text (hints, disabled labels).
    pub text_faint: egui::Color32,
    /// Translucent accent overlay drawn on hovered clickable controls.
    pub hover_overlay: egui::Color32,
    /// Default (and minimum) width of every modal dialog.
    pub dialog_w: f32,
    /// Translucent shade drawn over the UI behind modal windows.
    pub modal_shade: egui::Color32,
    /// Content width of a standard dialog card (420 px).
    pub export_dialog_w: f32,
    /// Corner radius of a dialog card.
    pub card_radius: f32,
    /// Inner padding of a dialog card.
    pub card_pad: f32,
    /// Soft shadow under a dialog card.
    pub card_shadow: egui::Shadow,
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
    /// Height of a dialog text field / segmented control / footer button.
    pub field_h: f32,
    /// Width of the form label column in a dialog.
    pub form_label_w: f32,
    /// Vertical gap between form rows.
    pub form_row_gap: f32,
    /// Padding added to a segmented-control label (both sides together).
    pub segment_pad: f32,
    /// Smallest segment padding when the labels are too wide for the row.
    pub segment_pad_min: f32,
    /// Width of the typed-value box next to a dialog slider.
    pub value_box_w: f32,
    /// Gap between a dialog slider and its typed-value box.
    pub value_gap: f32,
    /// Dialog title font size (PlexSemiBold).
    pub dialog_title_size: f32,
    /// Dialog description font size.
    pub dialog_desc_size: f32,
    /// Form label font size in a dialog.
    pub dialog_label_size: f32,
    /// Destructive footer-button text color.
    pub destructive: egui::Color32,
    /// Accent fill of the primary footer button while hovered.
    pub accent_hover: egui::Color32,

    // ---- Start screen ----
    /// Status bar height (26), reused by the start screen status bar.
    pub status_bar_h: f32,
    /// Minimum margin around the start screen panel (48).
    pub start_card_margin: f32,
    /// Start screen panel width (560).
    pub start_panel_w: f32,
    /// Start screen panel height (560).
    pub start_panel_h: f32,
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
    /// Corner radius of the start screen card (14).
    #[allow(dead_code)]
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
    /// Height of the dashed drop zone (52).
    #[allow(dead_code)]
    pub start_drop_h: f32,
    /// Corner radius of the drop zone (8).
    #[allow(dead_code)]
    pub start_drop_radius: f32,
    /// White text on the accent button.
    #[allow(dead_code)]
    pub start_on_accent: egui::Color32,
    /// Gap between the recent rows (2).
    #[allow(dead_code)]
    pub start_row_gap: f32,
    /// Height of one recent row (64).
    #[allow(dead_code)]
    pub start_row_h: f32,
    /// Size of the recent row's project tile (40).
    #[allow(dead_code)]
    pub start_tile: f32,
    /// Fixed width of the last-opened date column (96).
    #[allow(dead_code)]
    pub start_last_col_w: f32,
    /// Size of the per-row remove button (28).
    #[allow(dead_code)]
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
    /// Title font size under the logo (24).
    #[allow(dead_code)]
    pub start_title_size: f32,
    /// Small mono font size: version line, shortcut hints, path (11).
    #[allow(dead_code)]
    pub start_small_size: f32,
    /// Button label font size (14).
    #[allow(dead_code)]
    pub start_btn_text_size: f32,
    /// Hint font size: drop zone, footnotes, dates, Clear missing (12).
    #[allow(dead_code)]
    pub start_hint_size: f32,
    /// Recent row name font size (14).
    #[allow(dead_code)]
    pub start_name_size: f32,
    /// MISSING pill font size (10.5).
    #[allow(dead_code)]
    pub start_pill_size: f32,
    /// Phosphor icon size in buttons and tiles (16).
    #[allow(dead_code)]
    pub start_icon_size: f32,
    /// Height of the recent-projects header row (22).
    #[allow(dead_code)]
    pub start_header_h: f32,
    /// Corner radius of a recent row and its tile (8).
    #[allow(dead_code)]
    pub start_row_radius: f32,
    /// Dash length of the drop-zone dashed border (4).
    #[allow(dead_code)]
    pub start_dash: f32,
    /// Gap between dashes of the drop-zone border (4).
    #[allow(dead_code)]
    pub start_dash_gap: f32,
}

impl Theme {
    /// Monospace font at `size`, used for every timecode readout.
    pub fn mono(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Monospace)
    }

    /// Medium-weight text font at `size`, used for row names and buttons.
    pub fn medium(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Name(MEDIUM_FAMILY.into()))
    }

    /// SemiBold heading font at `size`, used for headings and track names.
    pub fn semibold(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into()))
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            // Surfaces, from the ui-kit tokens (the ui_kit::palette mapping).
            app_bg: tokens::color::BG_GROUND,
            panel_bg: tokens::color::BG_PANEL,
            bar_bg: tokens::color::BG_GROUND,
            recessed_a: tokens::color::BG_CONTROL,
            // The ruler and the track lanes have no dedicated tokens yet;
            // these are the closest surfaces. Phase 11 restyles them.
            ruler_bg: tokens::color::BG_PANEL,
            border: tokens::color::STROKE_HAIRLINE,
            control_border: tokens::color::STROKE_HAIRLINE,
            control_fill: tokens::color::BG_CONTROL,
            control_hover: tokens::color::BG_HOVER,
            control_rounding: tokens::radius::SEGMENT,
            slider_track: tokens::color::BG_SELECTED,
            slider_fill: tokens::accent::ACCENT,
            slider_thumb: tokens::color::TEXT_PRIMARY,
            play_green: tokens::accent::ACCENT,
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
            track_slider_track_w: 4.0,
            track_slider_thumb: 12.0,
            compact_track_h: 64.0,
            record_red: tokens::color::SEM_RECORD,
            on_record_glyph: egui::Color32::WHITE,
            record_btn_d: 28.0,
            record_btn_glyph: 16.0,
            clip_body_alpha: 0x22,
            clip_header_alpha: 0x66,
            clip_name_pin: 16.0,
            clip_name_gap: 6.0,
            clip_name_min_w: 8.0,
            wave_lane_pad: 1.0,
            // Meters
            meter_bar_bg: tokens::color::BG_GROUND,
            meter_red: tokens::color::SEM_METER_CLIP,
            meter_yellow: tokens::color::SEM_METER_WARN,
            meter_green: tokens::color::SEM_METER_OK,
            meter_hold: egui::Color32::from_rgb(230, 234, 240),
            clip_light_on: tokens::color::SEM_METER_CLIP,
            clip_light_off: egui::Color32::from_rgb(0x3A, 0x22, 0x24),

            // Bin panel
            bin_row_h: 52.0,
            bin_row_gap: 2.0,
            bin_row_rounding: 6.0,
            bin_header_size: 10.5,
            bin_btn_h: 26.0,
            bin_btn_text_size: 12.0,
            bin_tile_sz: 30.0,
            bin_tile_icon_size: 16.0,
            bin_name_size: 12.5,
            bin_meta_size: 10.5,
            bin_pill_fill: egui::Color32::from_rgb(0x26, 0x2A, 0x31),
            bin_pill_h: 18.0,
            bin_pill_rounding: 9.0,
            bin_pill_text_size: 10.5,
            bin_drop_h: 44.0,
            bin_drop_text_size: 10.5,
            bin_progress_h: 2.0,
            bin_rename_pad: 40.0,
            bin_rename_min_w: 60.0,
            bin_offline: egui::Color32::from_rgb(210, 100, 100),
            // Errors / dialogs
            error: egui::Color32::from_rgb(225, 105, 105),
            error_soft: egui::Color32::from_rgb(235, 140, 140),
            // Spacing: 4/8 px grid, from the ui-kit space scale.
            space_xs: tokens::space::SCALE_4,
            space_sm: tokens::space::SCALE_4,
            space_md: tokens::space::WINDOW_GAP,
            space_lg: tokens::space::SCALE_16,
            // Shared shapes
            corner_r: 2.0,
            corner_xs: 1.0,
            hairline: 1.0,
            // Accent, text roles & dialogs, from the ui-kit tokens.
            accent: tokens::accent::ACCENT,
            accent_soft: accent_alpha(0x2E),
            text_primary: tokens::color::TEXT_PRIMARY,
            text_normal: tokens::color::TEXT_SECONDARY,
            text_muted: tokens::color::TEXT_TERTIARY,
            text_faint: tokens::color::TEXT_DISABLED,
            hover_overlay: accent_alpha(0x2E),
            dialog_w: 420.0,
            export_dialog_w: 480.0,
            modal_shade: egui::Color32::from_black_alpha(140),
            card_radius: tokens::radius::GROUP,
            card_pad: 20.0,
            card_shadow: egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: egui::Color32::from_black_alpha(90) },
            // Menus keep their pre-restyle popup look until phase 13 turns
            // them into ui-kit glass popovers; no token matches these yet.
            menu_radius: 8.0,
            menu_pad: 6.0,
            menu_item_radius: 5.0,
            menu_shadow: egui::Shadow { offset: [0, 4], blur: 12, spread: 0, color: egui::Color32::from_black_alpha(90) },
            menu_right_pad: 8.0,
            unsaved_dot_d: 6.0,
            field_h: 30.0,
            form_label_w: 140.0,
            form_row_gap: 10.0,
            segment_pad: 16.0,
            segment_pad_min: 8.0,
            value_box_w: 72.0,
            value_gap: 8.0,
            dialog_title_size: 15.0,
            dialog_desc_size: 12.0,
            dialog_label_size: 12.5,
            destructive: tokens::color::SEM_RECORD,
            accent_hover: tokens::accent::HOVER,
            // Start screen
            status_bar_h: tokens::size::STATUS_BAR_HEIGHT,
            start_card_margin: 48.0,
            start_panel_w: 560.0,
            start_panel_h: 560.0,
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
            start_drop_h: 52.0,
            start_drop_radius: 8.0,
            start_on_accent: egui::Color32::WHITE,
            start_row_gap: 2.0,
            start_row_h: 64.0,
            start_tile: 40.0,
            start_last_col_w: 96.0,
            start_x_btn: 28.0,
            start_missing_fill: egui::Color32::from_rgb(0x3A, 0x22, 0x24),
            start_row_recent: egui::Color32::from_rgb(0x1F, 0x23, 0x28),
            start_missing_text: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            start_missing_icon: egui::Color32::from_rgb(0x5A, 0x61, 0x6D),
            start_title_size: 24.0,
            start_small_size: 11.0,
            start_btn_text_size: 14.0,
            start_hint_size: 12.0,
            start_name_size: 14.0,
            start_pill_size: 10.5,
            start_icon_size: 16.0,
            start_header_h: 22.0,
            start_row_radius: 8.0,
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
            egui::FontFamily::Name(MEDIUM_FAMILY.into()),
            egui::FontFamily::Name(SEMIBOLD_FAMILY.into()),
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
        assert_eq!(t.space_xs, tokens::space::SCALE_4);
        assert_eq!(t.space_md, tokens::space::WINDOW_GAP);
        assert_eq!(t.accent, tokens::accent::ACCENT, "one accent for selection, lit toggles, range lines and focus");
        assert_eq!(t.accent_soft, accent_alpha(0x2E));
        assert_eq!(t.unsaved_dot_d, 6.0);
        assert_eq!(t.record_red, tokens::color::SEM_RECORD);
        assert_eq!(t.on_record_glyph, egui::Color32::WHITE);
        assert_eq!((t.record_btn_d, t.record_btn_glyph), (28.0, 16.0));
        assert_eq!((t.clip_body_alpha, t.clip_header_alpha), (0x22, 0x66));
        assert_eq!(t.menu_right_pad, 8.0);
        assert_eq!(t.app_bg, tokens::color::BG_GROUND);
        assert_eq!(t.control_rounding, tokens::radius::SEGMENT);
        assert_eq!(t.text_normal, tokens::color::TEXT_SECONDARY);
        assert_eq!(t.status_bar_h, tokens::size::STATUS_BAR_HEIGHT);
    }
}
