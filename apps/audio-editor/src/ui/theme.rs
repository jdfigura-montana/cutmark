//! ui::theme: the single source for every visual constant (colors, sizes,
//! fonts and spacing) and the egui Style/Visuals applied once at startup.

use std::sync::LazyLock;

use eframe::egui;

/// The single accent color: selection, lit toggles, range lines and focus.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4C, 0x9B, 0xE8);

/// Named family holding the IBM Plex Sans Medium face.
const MEDIUM_FAMILY: &str = "PlexMedium";

/// The named egui font family holding the IBM Plex Sans SemiBold face.
const SEMIBOLD_FAMILY: &str = "PlexSemiBold";

/// The font-data key egui-phosphor registers its icon font under.
const PHOSPHOR_FONT: &str = "phosphor";

/// IBM Plex Sans Regular (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_REGULAR: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf");
/// IBM Plex Sans Medium (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-Medium.ttf");
/// IBM Plex Sans SemiBold (SIL OFL 1.1, see assets/fonts/LICENSE-IBMPlex.txt).
const PLEX_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/IBMPlexSans-SemiBold.ttf");
/// JetBrains Mono Regular (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
/// JetBrains Mono Medium (SIL OFL 1.1, see assets/fonts/OFL-JetBrainsMono.txt).
const JB_MONO_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Medium.ttf");

/// The eight UI-only track colors, cycled by track index.
const TRACK_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x4F, 0xB3, 0xA9),
    egui::Color32::from_rgb(0x9C, 0x8C, 0xE0),
    egui::Color32::from_rgb(0xD3, 0xA8, 0x6A),
    egui::Color32::from_rgb(0xD9, 0x7A, 0x8C),
    egui::Color32::from_rgb(0x7B, 0xC4, 0x7F),
    egui::Color32::from_rgb(0x5F, 0xA8, 0xD3),
    egui::Color32::from_rgb(0xC9, 0x8B, 0xC9),
    egui::Color32::from_rgb(0xB8, 0xB8, 0x6A),
];

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
    /// Even track lane background.
    pub lane_a: egui::Color32,
    /// Odd track lane background.
    pub lane_b: egui::Color32,
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
    /// Font size of the clip name in the header strip.
    pub clip_name_size: f32,
    /// Font size of the clip gain value in the header strip.
    pub clip_gain_size: f32,
    /// Vertical inset of a clip rect from its track lane edges.
    pub clip_margin: f32,
    /// Clip width below which a clip draws as a single hairline.
    pub thin_clip_w: f32,
    /// Border width of a selected clip.
    pub selection_border_w: f32,
    /// Border color of a selected clip.
    pub selection_border: egui::Color32,
    /// Fill of an offline clip's body.
    pub offline_body: egui::Color32,
    /// Fill of an offline clip's header strip.
    pub offline_header: egui::Color32,
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
    /// Height of the ruler strip.
    pub ruler_h: f32,
    /// Width of the track header column.
    pub header_col_w: f32,
    /// Height of the horizontal scrollbar strip.
    pub scrollbar_h: f32,
    /// Minimum scrollbar thumb width.
    pub scrollbar_thumb_min: f32,
    /// Major ruler tick length.
    pub ruler_tick_h: f32,
    /// Minor ruler tick length.
    pub ruler_minor_tick_h: f32,
    /// Color of the ruler ticks.
    pub ruler_tick_color: egui::Color32,
    /// Horizontal padding of the ruler timecode labels.
    pub ruler_label_pad_x: f32,
    /// Vertical padding of the ruler timecode labels.
    pub ruler_label_pad_y: f32,
    /// Font size of the ruler timecode labels.
    pub ruler_label_size: f32,
    /// Minimum width in points a ruler tick must have to be drawn.
    pub ruler_min_tick_px: f32,
    /// Marker triangle color.
    pub marker_color: egui::Color32,
    /// Half width of the marker triangle.
    pub marker_tri_w: f32,
    /// Height of the marker triangle above the ruler bottom.
    pub marker_tri_h: f32,
    /// Playhead line and triangle color.
    pub playhead_color: egui::Color32,
    /// Playhead line width.
    pub playhead_width: f32,
    /// Half width of the playhead triangle.
    pub playhead_tri_w: f32,
    /// Height of the playhead triangle below the ruler top.
    pub playhead_tri_h: f32,

    // ---- Interaction ----
    /// Snap distance in points (the "8 px snap distance").
    pub snap_px: f32,
    /// Pointer travel that turns a press into a drag.
    pub drag_start_px: f32,
    /// Grab distance of a range boundary line.
    pub boundary_grab_px: f32,
    /// Bright vertical line drawn at the snap point while dragging.
    pub snap_line_color: egui::Color32,
    /// Width of the snap line.
    pub snap_line_w: f32,
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
    /// Font size of the inline gain edit field.
    pub gain_edit_size: f32,

    // ---- Track header ----
    /// Height of the resize strip at the header's bottom edge.
    pub track_resize_h: f32,
    /// Vertical padding (top and bottom) inside one track header.
    pub track_pad_v: f32,
    /// Left padding inside one track header, after the color stripe.
    pub track_pad_l: f32,
    /// Right padding inside one track header.
    pub track_pad_r: f32,
    /// Vertical gap between the header's control rows.
    pub track_row_gap: f32,
    /// Width of an M / S toggle button.
    pub track_ms_w: f32,
    /// Height of an M / S toggle button.
    pub track_ms_h: f32,
    /// Gap between the M and S buttons.
    pub track_ms_gap: f32,
    /// Corner radius of an M / S toggle button.
    pub track_ms_radius: f32,
    /// Width of the VOL / PAN label column.
    pub track_label_col_w: f32,
    /// Width of the fixed, right-aligned value column.
    pub track_value_col_w: f32,
    /// Font size of the VOL / PAN labels.
    pub track_label_size: f32,
    /// Font size of the volume / pan value readouts.
    pub track_value_size: f32,
    /// Thickness of the custom slider track.
    pub track_slider_track_w: f32,
    /// Diameter of the custom slider thumb.
    pub track_slider_thumb: f32,
    /// Width of one meter bar in a track header.
    pub track_meter_bar_w: f32,
    /// Gap between the two meter bars.
    pub track_meter_gap: f32,
    /// Side of the square track clip light.
    pub track_clip_light: f32,
    /// Font size of the track name.
    pub track_name_size: f32,
    /// Font size of the M / S toggle labels.
    pub toggle_size: f32,
    /// Header height below which the header uses the compact layout.
    pub compact_track_h: f32,
    /// Background of an active mute button.
    pub mute_color: egui::Color32,
    /// Background of an active solo button.
    pub solo_color: egui::Color32,
    /// Background of an armed record-arm button.
    pub record_red: egui::Color32,
    /// Font size of the record glyph on the arm button.
    pub arm_glyph_size: f32,
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
    /// Larger corner radius for thumbs and toggle buttons.
    pub corner_round: f32,
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
    pub menu_right_pad: f32,
    /// Diameter of the painted "unsaved changes" dot in the menu bar.
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
    /// Start screen card width (1000).
    pub start_card_w: f32,
    /// Start screen card height (600).
    pub start_card_h: f32,
    /// Minimum margin around the start screen card (48).
    pub start_card_margin: f32,
    /// Smallest start screen card width when the window is narrow (760).
    pub start_card_min_w: f32,
    /// Corner radius of the start screen card (14).
    pub start_card_radius: f32,
    /// Width of the start screen card's left column (360).
    pub start_col_w: f32,
    /// Top padding of the left column (36).
    pub start_pad_top: f32,
    /// Side padding of the left column (32).
    pub start_pad_side: f32,
    /// Bottom padding of the left column (28).
    pub start_pad_bottom: f32,
    /// Top padding of the right column (32).
    pub start_pad_r_top: f32,
    /// Side padding of the right column (28).
    pub start_pad_r_side: f32,
    /// Bottom padding of the right column (20).
    pub start_pad_r_bottom: f32,
    /// Height of the logo bar box (40).
    pub start_logo_h: f32,
    /// Width of one logo bar (4).
    pub start_bar_w: f32,
    /// Horizontal pitch between logo bars (6).
    pub start_bar_pitch: f32,
    /// Height of each of the 20 logo bars.
    pub start_bar_heights: [f32; 20],
    /// Gap between the logo and the title (20).
    pub start_title_gap: f32,
    /// Gap between the version line and the buttons (40).
    pub start_buttons_gap: f32,
    /// Height of the New/Open buttons (52).
    pub start_btn_h: f32,
    /// Gap between the two buttons (10).
    pub start_btn_gap: f32,
    /// Corner radius of the buttons (8).
    pub start_btn_radius: f32,
    /// Gap between the buttons and the drop zone (20).
    pub start_drop_gap: f32,
    /// Height of the dashed drop zone (52).
    pub start_drop_h: f32,
    /// Corner radius of the drop zone (8).
    pub start_drop_radius: f32,
    /// White text on the accent button.
    pub start_on_accent: egui::Color32,
    /// Gap between the recent rows (2).
    pub start_row_gap: f32,
    /// Height of one recent row (64).
    pub start_row_h: f32,
    /// Size of the recent row's project tile (40).
    pub start_tile: f32,
    /// Fixed width of the last-opened date column (96).
    pub start_last_col_w: f32,
    /// Size of the per-row remove button (28).
    pub start_x_btn: f32,
    /// Fill of the MISSING pill.
    pub start_missing_fill: egui::Color32,
    /// At-rest background of the most recently opened recent row (#1F2328).
    pub start_row_recent: egui::Color32,
    /// Text color of the MISSING pill.
    pub start_missing_text: egui::Color32,
    /// Icon color of a missing project's tile.
    pub start_missing_icon: egui::Color32,
    /// Title font size under the logo (24).
    pub start_title_size: f32,
    /// Small mono font size: version line, shortcut hints, path (11).
    pub start_small_size: f32,
    /// Button label font size (14).
    pub start_btn_text_size: f32,
    /// Hint font size: drop zone, footnotes, dates, Clear missing (12).
    pub start_hint_size: f32,
    /// Recent row name font size (14).
    pub start_name_size: f32,
    /// MISSING pill font size (10.5).
    pub start_pill_size: f32,
    /// Phosphor icon size in buttons and tiles (16).
    pub start_icon_size: f32,
    /// Height of the recent-projects header row (22).
    pub start_header_h: f32,
    /// Corner radius of a recent row and its tile (8).
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

    /// Medium-weight text font at `size`, used for row names and buttons.
    pub fn medium(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Name(MEDIUM_FAMILY.into()))
    }

    /// SemiBold heading font at `size`, used for headings and track names.
    pub fn semibold(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into()))
    }

    /// The UI-only track color for a track index (cycles through 8 colors).
    pub fn track_color(&self, index: usize) -> egui::Color32 {
        TRACK_COLORS[index % TRACK_COLORS.len()]
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme {
            // Surfaces
            app_bg: egui::Color32::from_rgb(0x16, 0x18, 0x1C),
            panel_bg: egui::Color32::from_rgb(0x1A, 0x1D, 0x22),
            bar_bg: egui::Color32::from_rgb(0x12, 0x14, 0x18),
            recessed_a: egui::Color32::from_rgb(0x0E, 0x10, 0x13),
            ruler_bg: egui::Color32::from_rgb(0x16, 0x19, 0x1D),
            lane_a: egui::Color32::from_rgb(0x1B, 0x1E, 0x23),
            lane_b: egui::Color32::from_rgb(0x1E, 0x21, 0x27),
            border: egui::Color32::from_rgb(0x23, 0x27, 0x2E),
            control_border: egui::Color32::from_rgb(0x2B, 0x30, 0x38),
            control_fill: egui::Color32::from_rgb(0x22, 0x26, 0x2D),
            control_hover: egui::Color32::from_rgb(0x2B, 0x30, 0x38),
            control_rounding: 6.0,
            slider_track: egui::Color32::from_rgb(0x2B, 0x30, 0x38),
            slider_fill: egui::Color32::from_rgb(0x5A, 0x61, 0x6D),
            slider_thumb: egui::Color32::from_rgb(0xD7, 0xDA, 0xE0),
            play_green: egui::Color32::from_rgb(0x5C, 0xC4, 0x6E),
            // Clip visuals
            header_strip_h: 18.0,
            trim_edge_w: 6.0,
            fade_handle_sz: 8.0,
            fade_handle_grab: 2.0,
            gain_hit_px: 4.0,
            gain_text_w: 56.0,
            clip_text_pad: 4.0,
            clip_radius: 5.0,
            clip_text_color: egui::Color32::from_rgb(0xF2, 0xF4, 0xF7),
            clip_name_size: 11.0,
            clip_gain_size: 10.0,
            clip_margin: 2.0,
            thin_clip_w: 3.0,
            selection_border_w: 2.0,
            selection_border: ACCENT,
            offline_body: egui::Color32::from_rgba_unmultiplied(150, 62, 62, 220),
            offline_header: egui::Color32::from_rgba_unmultiplied(110, 42, 42, 230),
            offline_text: egui::Color32::WHITE,
            gain_line: egui::Color32::from_rgb(0xF2, 0xF4, 0xF7),
            gain_curve: egui::Color32::from_rgba_unmultiplied(0xF2, 0xF4, 0xF7, 178),
            fade_shade: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 90),
            fade_handle_fill: egui::Color32::from_rgb(0xE6, 0xE9, 0xEE),
            fade_handle_hover: ACCENT,
            fade_handle_stroke: egui::Color32::from_rgb(18, 20, 23),
            // Waveform
            wave_width: 1.0,
            wave_dot_r: 1.5,
            // Timeline frame
            ruler_h: 30.0,
            header_col_w: 208.0,
            scrollbar_h: 12.0,
            scrollbar_thumb_min: 20.0,
            ruler_tick_h: 9.0,
            ruler_minor_tick_h: 5.0,
            ruler_tick_color: egui::Color32::from_rgb(0x3A, 0x40, 0x4A),
            ruler_label_pad_x: 4.0,
            ruler_label_pad_y: 3.0,
            ruler_label_size: 10.5,
            ruler_min_tick_px: 60.0,
            marker_color: egui::Color32::from_rgb(0xE8, 0xB4, 0x4C),
            marker_tri_w: 5.0,
            marker_tri_h: 8.0,
            playhead_color: egui::Color32::from_rgb(0xFF, 0x6B, 0x5C),
            playhead_width: 1.5,
            playhead_tri_w: 5.0,
            playhead_tri_h: 10.0,
            // Interaction
            snap_px: 8.0,
            drag_start_px: 3.0,
            boundary_grab_px: 4.0,
            snap_line_color: ACCENT,
            snap_line_w: 2.0,
            range_line: ACCENT,
            range_line_w: 1.0,
            insert_line_color: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 230),
            insert_line_w: 2.0,
            reorder_line_color: ACCENT,
            reorder_line_w: 2.0,
            ghost_fill: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 90),
            ghost_stroke: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 220),
            ghost_dash: 6.0,
            ghost_gap: 4.0,
            new_track_h: 96.0,
            clip_menu_min_w: 150.0,
            gain_edit_w: 52.0,
            gain_edit_size: 11.0,
            // Track header
            track_resize_h: 4.0,
            track_pad_v: 9.0,
            track_pad_l: 10.0,
            track_pad_r: 8.0,
            track_row_gap: 9.0,
            track_ms_w: 22.0,
            track_ms_h: 20.0,
            track_ms_gap: 4.0,
            track_ms_radius: 4.0,
            track_label_col_w: 24.0,
            track_value_col_w: 44.0,
            track_label_size: 9.5,
            track_value_size: 10.5,
            track_slider_track_w: 4.0,
            track_slider_thumb: 12.0,
            track_meter_bar_w: 4.0,
            track_meter_gap: 2.0,
            track_clip_light: 4.0,
            compact_track_h: 64.0,
            track_name_size: 12.5,
            toggle_size: 10.0,
            mute_color: egui::Color32::from_rgb(0xE0, 0xA9, 0x3A),
            solo_color: egui::Color32::from_rgb(0x5C, 0xC4, 0x6E),
            record_red: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            arm_glyph_size: 12.0,
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
            meter_bar_bg: egui::Color32::from_rgb(0x10, 0x12, 0x16),
            meter_red: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            meter_yellow: egui::Color32::from_rgb(0xE3, 0xC0, 0x4A),
            meter_green: egui::Color32::from_rgb(0x3D, 0xBE, 0x6B),
            meter_hold: egui::Color32::from_rgb(230, 234, 240),
            clip_light_on: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
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
            // Spacing: 4/8 px grid.
            space_xs: 4.0,
            space_sm: 4.0,
            space_md: 8.0,
            space_lg: 16.0,
            // Shared shapes
            corner_r: 2.0,
            corner_round: 3.0,
            corner_xs: 1.0,
            hairline: 1.0,
            // Accent, text roles & dialogs
            accent: ACCENT,
            accent_soft: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 0x2E),
            text_primary: egui::Color32::from_rgb(0xE6, 0xE9, 0xEE),
            text_normal: egui::Color32::from_rgb(0xC9, 0xCE, 0xD6),
            text_muted: egui::Color32::from_rgb(0x8A, 0x90, 0x9A),
            text_faint: egui::Color32::from_rgb(0x6E, 0x75, 0x81),
            hover_overlay: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 0x2E),
            dialog_w: 420.0,
            export_dialog_w: 480.0,
            modal_shade: egui::Color32::from_black_alpha(140),
            card_radius: 10.0,
            card_pad: 20.0,
            card_shadow: egui::Shadow { offset: [0, 4], blur: 16, spread: 0, color: egui::Color32::from_black_alpha(90) },
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
            destructive: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            accent_hover: egui::Color32::from_rgb(0x60, 0xAA, 0xEC),
            // Start screen
            status_bar_h: 26.0,
            start_card_w: 1000.0,
            start_card_h: 600.0,
            start_card_margin: 48.0,
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
            start_bar_heights: [8.0, 16.0, 26.0, 34.0, 22.0, 30.0, 38.0, 24.0, 14.0, 28.0, 36.0, 20.0, 30.0, 18.0, 10.0, 24.0, 32.0, 16.0, 8.0, 12.0],
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

/// Builds the font set: IBM Plex Sans (Proportional), JetBrains Mono
/// (Monospace), the named "PlexMedium"/"PlexSemiBold" families, and the
/// phosphor icon font as a glyph fallback for EVERY family, so an icon
/// renders wherever it is mixed into text.
fn font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Fill);
    for (key, bytes) in [
        ("IBMPlexSans-Regular", PLEX_REGULAR),
        ("IBMPlexSans-Medium", PLEX_MEDIUM),
        ("IBMPlexSans-SemiBold", PLEX_SEMIBOLD),
        ("JetBrainsMono-Regular", JB_MONO_REGULAR),
        ("JetBrainsMono-Medium", JB_MONO_MEDIUM),
    ] {
        fonts.font_data.insert(key.into(), egui::FontData::from_static(bytes).into());
    }
    // IBM Plex Sans is the proportional face, JetBrains Mono the monospace
    // face; phosphor stays in every family list as the glyph fallback.
    {
        let proportional = fonts.families.entry(egui::FontFamily::Proportional).or_default();
        proportional.insert(0, "IBMPlexSans-Medium".into());
        proportional.insert(0, "IBMPlexSans-Regular".into());
        let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
        monospace.insert(0, "JetBrainsMono-Medium".into());
        monospace.insert(0, "JetBrainsMono-Regular".into());
        monospace.push(PHOSPHOR_FONT.into());
    }
    fonts.families.insert(egui::FontFamily::Name(MEDIUM_FAMILY.into()), vec!["IBMPlexSans-Medium".into(), "IBMPlexSans-Regular".into(), PHOSPHOR_FONT.into()]);
    fonts
        .families
        .insert(egui::FontFamily::Name(SEMIBOLD_FAMILY.into()), vec!["IBMPlexSans-SemiBold".into(), "IBMPlexSans-Medium".into(), PHOSPHOR_FONT.into()]);
    fonts
}

/// Sets the egui Style/Visuals once at startup (called from `main`).
///
/// Owns the whole look: the dark palette surfaces, a single accent color for
/// selection/lit toggles/range lines/focus, IBM Plex Sans (Proportional) and
/// JetBrains Mono (Monospace) with a named "PlexSemiBold" family for headings
/// and track names, the phosphor icon font, and 4/8 px default spacing.
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(font_definitions());
    let t = &*THEME;
    let mut v = egui::Visuals::dark();

    // Surfaces from the palette.
    v.panel_fill = t.panel_bg;
    v.window_fill = t.panel_bg;
    v.extreme_bg_color = t.recessed_a; // recessed surfaces (fader slot, etc.)
    v.faint_bg_color = t.lane_b; // odd lane shade
    v.window_stroke = egui::Stroke::new(t.hairline, t.control_border);
    v.window_corner_radius = t.menu_radius.into();
    v.menu_corner_radius = t.menu_radius.into();
    v.popup_shadow = t.menu_shadow;

    // One accent for selection, focus and links.
    v.selection.stroke = egui::Stroke::new(t.hairline, t.accent);
    v.selection.bg_fill = t.accent;
    v.hyperlink_color = t.accent;

    // Widgets: every state takes its fills from the control tokens, its
    // strokes from the border tokens and its text from the text tokens.
    let text = t.text_normal;
    v.override_text_color = Some(text);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = t.control_rounding.into();
    }
    v.widgets.noninteractive.bg_fill = t.panel_bg;
    v.widgets.noninteractive.weak_bg_fill = t.panel_bg;
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(t.hairline, t.border);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(t.hairline, t.text_muted);
    v.widgets.inactive.bg_fill = t.control_fill;
    v.widgets.inactive.weak_bg_fill = t.control_fill;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(t.hairline, t.control_border);
    v.widgets.inactive.fg_stroke = egui::Stroke::new(t.hairline, t.text_normal);
    v.widgets.hovered.bg_fill = t.control_hover;
    v.widgets.hovered.weak_bg_fill = t.control_hover;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(t.hairline, t.control_border);
    v.widgets.hovered.fg_stroke = egui::Stroke::new(t.hairline, t.text_normal);
    v.widgets.active.bg_fill = t.control_hover;
    v.widgets.active.weak_bg_fill = t.control_hover;
    v.widgets.active.bg_stroke = egui::Stroke::new(t.hairline, t.control_border);
    v.widgets.active.fg_stroke = egui::Stroke::new(t.hairline, t.text_primary);
    v.widgets.open.bg_fill = t.control_fill;
    v.widgets.open.weak_bg_fill = t.control_fill;
    v.widgets.open.bg_stroke = egui::Stroke::new(t.hairline, t.control_border);
    v.widgets.open.fg_stroke = egui::Stroke::new(t.hairline, t.text_normal);

    // 4/8 px spacing grid; headings use the SemiBold face.
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(t.space_md, t.space_sm);
        style.spacing.button_padding = egui::vec2(t.space_md, t.space_sm);
        style.spacing.menu_margin = egui::Margin::same(t.menu_pad as i8);
        style.spacing.window_margin = t.space_md.into();
        let heading_size = style.text_styles.get(&egui::TextStyle::Heading).map(|font| font.size).unwrap_or(18.0);
        style.text_styles.insert(egui::TextStyle::Heading, egui::FontId::new(heading_size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into())));
    });

    ctx.set_visuals(v);
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

/// Debug-only helper window (toggled with F12) that shows `ctx.style_ui`.
///
/// The toggle lives in egui temp storage; F12 flips it. Release builds
/// compile this to nothing.
#[cfg(debug_assertions)]
pub fn debug_window(ctx: &egui::Context) {
    const KEY: &str = "theme_debug_window_open";
    if ctx.input(|i| i.key_pressed(egui::Key::F12)) {
        ctx.data_mut(|d| {
            let open = d.get_temp::<bool>(egui::Id::new(KEY)).unwrap_or(false);
            d.insert_temp(egui::Id::new(KEY), !open);
        });
    }
    let open = ctx.data(|d| d.get_temp::<bool>(egui::Id::new(KEY)).unwrap_or(false));
    if !open {
        return;
    }
    egui::Window::new("Style (F12)").default_width(420.0).show(ctx, |ui| {
        ctx.style_ui(ui, ctx.theme());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phosphor_icons_resolve_in_every_family() {
        use eframe::egui::epaint::text::{Fonts, TextOptions};
        let mut fonts = Fonts::new(TextOptions::default(), font_definitions());
        // Without the phosphor face, the icons must NOT resolve: this proves
        // phosphor itself (and not an egui default font) owns the glyphs.
        let mut bare = font_definitions();
        for keys in bare.families.values_mut() {
            keys.retain(|key| key != PHOSPHOR_FONT);
        }
        let mut bare_fonts = Fonts::new(TextOptions::default(), bare);
        let families = [
            egui::FontFamily::Proportional,
            egui::FontFamily::Monospace,
            egui::FontFamily::Name(MEDIUM_FAMILY.into()),
            egui::FontFamily::Name(SEMIBOLD_FAMILY.into()),
        ];
        for family in &families {
            let id = egui::FontId::new(16.0, family.clone());
            for (name, icon) in [
                ("FOLDER_OPEN", egui_phosphor::fill::FOLDER_OPEN),
                ("FILE_AUDIO", egui_phosphor::fill::FILE_AUDIO),
                ("PLUS", egui_phosphor::fill::PLUS),
                ("X", egui_phosphor::fill::X),
                ("WAVEFORM", egui_phosphor::fill::WAVEFORM),
            ] {
                let c = icon.chars().next().unwrap();
                assert!(!bare_fonts.has_glyph(&id, c), "{name} (U+{:04X}) resolves WITHOUT phosphor in {family:?}: the probe is blind", c as u32);
                assert!(fonts.has_glyph(&id, c), "{name} (U+{:04X}) missing in family {family:?}", c as u32);
            }
        }
    }

    #[test]
    fn theme_holds_the_spec_values() {
        let t = Theme::default();
        assert_eq!(t.header_strip_h, 18.0);
        assert_eq!(t.trim_edge_w, 6.0);
        assert_eq!(t.snap_px, 8.0);
        assert_eq!(t.fade_handle_sz, 8.0);
        assert_eq!(t.ruler_h, 30.0);
        assert_eq!(t.new_track_h, 96.0);
        assert_eq!(t.meter_red, egui::Color32::from_rgb(0xE5, 0x54, 0x4B));
        assert_eq!(t.meter_yellow, egui::Color32::from_rgb(0xE3, 0xC0, 0x4A));
        assert_eq!(t.meter_green, egui::Color32::from_rgb(0x3D, 0xBE, 0x6B));
        assert_eq!(t.space_xs, 4.0);
        assert_eq!(t.space_md, 8.0);
        assert_eq!(t.accent, egui::Color32::from_rgb(0x4C, 0x9B, 0xE8), "one accent for selection, lit toggles, range lines and focus");
        assert_eq!(t.accent_soft, egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 0x2E));
        assert_eq!(t.selection_border, t.accent);
        assert_eq!(t.snap_line_color, t.accent);
        assert_eq!(t.playhead_color, egui::Color32::from_rgb(0xFF, 0x6B, 0x5C));
        assert_eq!(t.marker_color, egui::Color32::from_rgb(0xE8, 0xB4, 0x4C));
        assert_eq!(t.unsaved_dot_d, 6.0);
        assert_eq!(t.record_red, egui::Color32::from_rgb(0xE5, 0x54, 0x4B));
        assert_eq!(t.on_record_glyph, egui::Color32::WHITE);
        assert_eq!((t.record_btn_d, t.record_btn_glyph), (28.0, 16.0));
        assert_eq!((t.clip_body_alpha, t.clip_header_alpha), (0x22, 0x66));
        assert_eq!((t.track_ms_w, t.track_ms_h, t.track_ms_gap), (22.0, 20.0, 4.0));
        assert_eq!(t.menu_right_pad, 8.0);
        assert_eq!(t.app_bg, egui::Color32::from_rgb(0x16, 0x18, 0x1C));
        assert_eq!(t.lane_a, egui::Color32::from_rgb(0x1B, 0x1E, 0x23));
        assert_eq!(t.lane_b, egui::Color32::from_rgb(0x1E, 0x21, 0x27));
        assert_eq!(t.control_rounding, 6.0);
        assert_eq!(t.text_normal, egui::Color32::from_rgb(0xC9, 0xCE, 0xD6));
    }

    #[test]
    fn track_color_cycles_by_index() {
        let t = Theme::default();
        assert_eq!(t.track_color(0), t.track_color(8));
        assert_eq!(t.track_color(3), t.track_color(11));
        assert_ne!(t.track_color(0), t.track_color(1));
    }
}
