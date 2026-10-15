//! ui::theme: the single source for every visual constant (colors, sizes,
//! fonts and spacing) and the egui Style/Visuals applied once at startup.

use std::sync::LazyLock;

use eframe::egui;

/// The single accent color: selection, lit toggles, range lines and focus.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x4C, 0x9B, 0xE8);

/// The named egui font family holding the IBM Plex Sans SemiBold face.
const SEMIBOLD_FAMILY: &str = "PlexSemiBold";

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
    /// Recessed surface of the timecode / peak readout box.
    pub recessed_b: egui::Color32,
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
    /// Width of the dark under-shadow of the clip gain line.
    pub gain_shadow_w: f32,
    /// Dark under-shadow color of the clip gain line.
    pub gain_shadow: egui::Color32,
    /// Bright amber line color of the clip gain line.
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
    /// Faint center line color of each waveform lane.
    pub wave_center: egui::Color32,
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
    /// Width of the reorder grip at the header's left edge.
    pub track_grip_w: f32,
    /// Height of the resize strip at the header's bottom edge.
    pub track_resize_h: f32,
    /// Width of the stereo meter at the header's right edge.
    pub track_meter_w: f32,
    /// Height of the track name row.
    pub track_name_h: f32,
    /// Height of one control row (fader or pan).
    pub track_ctrl_h: f32,
    /// Width of an M / S toggle button.
    pub track_ms_w: f32,
    /// Header height below which the header uses the compact layout.
    pub compact_track_h: f32,
    /// Font size of the track name.
    pub track_name_size: f32,
    /// Font size of the M / S toggle labels.
    pub toggle_size: f32,
    /// Background of an active mute button.
    pub mute_color: egui::Color32,
    /// Background of an active solo button.
    pub solo_color: egui::Color32,
    /// Text color on an active mute/solo button.
    pub toggle_fg: egui::Color32,
    /// Vertical spacing between the reorder grip dots.
    pub track_grip_dots: f32,
    /// Half length of a reorder grip dot.
    pub track_grip_dot_len: f32,

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
    /// Peak-hold marker width.
    pub meter_hold_w: f32,
    /// Peak line width.
    pub meter_peak_w: f32,
    /// Clip light color while latched.
    pub clip_light_on: egui::Color32,
    /// Clip light color while clear.
    pub clip_light_off: egui::Color32,
    /// Height of the clip-light strip above the bars.
    pub clip_light_h: f32,
    /// Meter scale tick color.
    pub meter_tick: egui::Color32,
    /// Meter tick label color.
    pub meter_label: egui::Color32,
    /// Width of the tick-label gutter.
    pub meter_label_w: f32,
    /// Minimum bar-area height at which tick labels are drawn.
    pub meter_label_room: f32,
    /// Gap between the L and R bars.
    pub meter_bar_gap: f32,
    /// Font size of the meter tick labels.
    pub meter_label_size: f32,
    /// Horizontal padding of the meter tick labels.
    pub meter_label_pad_x: f32,

    // ---- Fader / master strip ----
    /// Width of the fader slot stroke.
    pub fader_slot_w: f32,
    /// Thickness of the fader handle across the slot.
    pub fader_handle_w: f32,
    /// Inset of the fader slot ends from the rect corners.
    pub fader_inset: f32,
    /// Fader value text color.
    pub fader_text: egui::Color32,
    /// Font size of the fader value text.
    pub fader_text_size: f32,
    /// Horizontal padding of the fader value text.
    pub fader_text_pad_x: f32,
    /// Vertical offset of the fader value text above the handle.
    pub fader_text_dy: f32,
    /// Vertical padding of the fader value text.
    pub fader_text_pad_y: f32,
    /// Width of the master meter bars area.
    pub master_meter_w: f32,
    /// Width of the vertical master fader.
    pub master_fader_w: f32,
    /// Height of the master peak-readout strip.
    pub master_readout_h: f32,
    /// Padding inside the master strip.
    pub master_pad: f32,

    // ---- Bin panel ----
    /// Height of one bin item row.
    pub bin_row_h: f32,
    /// Width of the selected-row accent bar.
    pub bin_sel_bar_w: f32,
    /// Height of the selected-row accent bar.
    pub bin_sel_bar_h: f32,
    /// Height of the preview progress bar under a bin row.
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
    /// Extra-large spacing (24).
    pub space_xl: f32,
    /// Extra-extra-large spacing (40).
    pub space_xxl: f32,
    /// Vertical grid spacing inside dialogs.
    pub grid_spacing: f32,

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
}

impl Theme {
    /// Monospace font at `size`, used for every timecode readout.
    pub fn mono(size: f32) -> egui::FontId {
        egui::FontId::new(size, egui::FontFamily::Monospace)
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
            recessed_b: egui::Color32::from_rgb(0x10, 0x12, 0x16),
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
            // Clip visuals
            header_strip_h: 18.0,
            trim_edge_w: 6.0,
            fade_handle_sz: 8.0,
            fade_handle_grab: 2.0,
            gain_hit_px: 4.0,
            gain_text_w: 56.0,
            clip_text_pad: 4.0,
            clip_name_size: 12.0,
            clip_gain_size: 11.0,
            clip_margin: 2.0,
            thin_clip_w: 3.0,
            selection_border_w: 2.0,
            selection_border: ACCENT,
            offline_body: egui::Color32::from_rgba_unmultiplied(150, 62, 62, 220),
            offline_header: egui::Color32::from_rgba_unmultiplied(110, 42, 42, 230),
            offline_text: egui::Color32::WHITE,
            gain_shadow_w: 3.0,
            gain_shadow: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            gain_line: ACCENT,
            gain_curve: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 210),
            fade_shade: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 90),
            fade_handle_fill: egui::Color32::from_rgba_unmultiplied(208, 212, 218, 230),
            fade_handle_hover: ACCENT,
            fade_handle_stroke: egui::Color32::from_rgb(18, 20, 23),
            // Waveform
            wave_width: 1.0,
            wave_center: egui::Color32::from_rgba_unmultiplied(255, 255, 255, 40),
            wave_dot_r: 1.5,
            // Timeline frame
            ruler_h: 28.0,
            header_col_w: 200.0,
            scrollbar_h: 12.0,
            scrollbar_thumb_min: 20.0,
            ruler_tick_h: 12.0,
            ruler_minor_tick_h: 5.0,
            ruler_label_pad_x: 4.0,
            ruler_label_pad_y: 3.0,
            ruler_label_size: 11.0,
            ruler_min_tick_px: 90.0,
            marker_color: egui::Color32::from_rgb(0xE8, 0xB4, 0x4C),
            marker_tri_w: 4.0,
            marker_tri_h: 7.0,
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
            range_line_w: 1.5,
            insert_line_color: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 230),
            insert_line_w: 2.0,
            reorder_line_color: ACCENT,
            reorder_line_w: 2.0,
            ghost_fill: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 90),
            ghost_stroke: egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 220),
            ghost_dash: 6.0,
            ghost_gap: 4.0,
            new_track_h: 80.0,
            clip_menu_min_w: 150.0,
            gain_edit_w: 52.0,
            gain_edit_size: 11.0,
            // Track header
            track_grip_w: 10.0,
            track_resize_h: 4.0,
            track_meter_w: 14.0,
            track_name_h: 18.0,
            track_ctrl_h: 14.0,
            track_ms_w: 16.0,
            compact_track_h: 64.0,
            track_name_size: 13.0,
            toggle_size: 10.0,
            mute_color: egui::Color32::from_rgb(0xE0, 0xA9, 0x3A),
            solo_color: egui::Color32::from_rgb(0x5C, 0xC4, 0x6E),
            toggle_fg: egui::Color32::from_rgb(18, 20, 23),
            track_grip_dots: 4.0,
            track_grip_dot_len: 3.0,
            // Meters
            meter_bar_bg: egui::Color32::from_rgb(0x0E, 0x10, 0x13),
            meter_red: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            meter_yellow: egui::Color32::from_rgb(0xE3, 0xC0, 0x4A),
            meter_green: egui::Color32::from_rgb(0x3D, 0xBE, 0x6B),
            meter_hold: egui::Color32::from_rgb(230, 234, 240),
            meter_hold_w: 2.0,
            meter_peak_w: 1.5,
            clip_light_on: egui::Color32::from_rgb(0xE5, 0x54, 0x4B),
            clip_light_off: egui::Color32::from_rgb(0x3A, 0x22, 0x24),
            clip_light_h: 8.0,
            meter_tick: egui::Color32::from_rgb(0x2B, 0x30, 0x38),
            meter_label: egui::Color32::from_rgb(0x8A, 0x90, 0x9A),
            meter_label_w: 24.0,
            meter_label_room: 90.0,
            meter_bar_gap: 2.0,
            meter_label_size: 9.0,
            meter_label_pad_x: 3.0,

            // Fader / master strip
            fader_slot_w: 2.0,
            fader_handle_w: 4.0,
            fader_inset: 3.0,
            fader_text: egui::Color32::from_rgb(170, 175, 180),
            fader_text_size: 9.0,
            fader_text_pad_x: 2.0,
            fader_text_dy: 7.0,
            fader_text_pad_y: 1.0,
            master_meter_w: 36.0,
            master_fader_w: 26.0,
            master_readout_h: 16.0,
            master_pad: 4.0,
            // Bin panel
            bin_row_h: 38.0,
            bin_sel_bar_w: 3.0,
            bin_sel_bar_h: 30.0,
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
            space_xl: 32.0,
            space_xxl: 40.0,
            grid_spacing: 8.0,
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
            dialog_w: 360.0,
            modal_shade: egui::Color32::from_black_alpha(110),
        }
    }
}

/// The shared theme instance every ui module reads.
pub static THEME: LazyLock<Theme> = LazyLock::new(Theme::default);

/// Sets the egui Style/Visuals once at startup (called from `main`).
///
/// Owns the whole look: the dark palette surfaces, a single accent color for
/// selection/lit toggles/range lines/focus, IBM Plex Sans (Proportional) and
/// JetBrains Mono (Monospace) with a named "PlexSemiBold" family for headings
/// and track names, the phosphor icon font, and 4/8 px default spacing.
pub fn apply(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Fill);
    for (key, bytes) in [
        ("IBMPlexSans-Regular", PLEX_REGULAR),
        ("IBMPlexSans-Medium", PLEX_MEDIUM),
        ("IBMPlexSans-SemiBold", PLEX_SEMIBOLD),
        ("JetBrainsMono-Regular", JB_MONO_REGULAR),
        ("JetBrainsMono-Medium", JB_MONO_MEDIUM),
    ] {
        fonts
            .font_data
            .insert(key.into(), egui::FontData::from_static(bytes).into());
    }
    // IBM Plex Sans is the proportional face, JetBrains Mono the monospace
    // face; phosphor stays in both lists as the glyph fallback.
    {
        let proportional = fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default();
        proportional.insert(0, "IBMPlexSans-Medium".into());
        proportional.insert(0, "IBMPlexSans-Regular".into());
        let monospace = fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default();
        monospace.insert(0, "JetBrainsMono-Medium".into());
        monospace.insert(0, "JetBrainsMono-Regular".into());
    }
    fonts.families.insert(
        egui::FontFamily::Name(SEMIBOLD_FAMILY.into()),
        vec!["IBMPlexSans-SemiBold".into(), "IBMPlexSans-Medium".into()],
    );
    ctx.set_fonts(fonts);
    let t = &*THEME;
    let mut v = egui::Visuals::dark();

    // Surfaces from the palette.
    v.panel_fill = t.panel_bg;
    v.window_fill = t.panel_bg;
    v.extreme_bg_color = t.recessed_a; // recessed surfaces (fader slot, etc.)
    v.faint_bg_color = t.lane_b; // odd lane shade
    v.window_stroke = egui::Stroke::new(t.hairline, t.border);
    v.window_corner_radius = t.corner_r.into();
    v.menu_corner_radius = t.corner_r.into();

    // One accent for selection, focus and links.
    v.selection.stroke = egui::Stroke::new(t.hairline, t.accent);
    v.selection.bg_fill = t.accent;
    v.hyperlink_color = t.accent;

    // Widgets: every state takes its fills from the control tokens, its
    // strokes from the border tokens and its text from the text tokens.
    let text = t.text_normal;
    v.override_text_color = Some(text);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
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
        style.spacing.menu_margin = t.space_sm.into();
        style.spacing.window_margin = t.space_md.into();
        let heading_size = style
            .text_styles
            .get(&egui::TextStyle::Heading)
            .map(|font| font.size)
            .unwrap_or(18.0);
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(heading_size, egui::FontFamily::Name(SEMIBOLD_FAMILY.into())),
        );
    });

    ctx.set_visuals(v);
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
    egui::Window::new("Style (F12)")
        .default_width(420.0)
        .show(ctx, |ui| {
            ctx.style_ui(ui, ctx.theme());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_holds_the_spec_values() {
        let t = Theme::default();
        assert_eq!(t.header_strip_h, 18.0);
        assert_eq!(t.trim_edge_w, 6.0);
        assert_eq!(t.snap_px, 8.0);
        assert_eq!(t.fade_handle_sz, 8.0);
        assert_eq!(t.ruler_h, 28.0);
        assert_eq!(t.new_track_h, 80.0);
        assert_eq!(t.meter_red, egui::Color32::from_rgb(0xE5, 0x54, 0x4B));
        assert_eq!(t.meter_yellow, egui::Color32::from_rgb(0xE3, 0xC0, 0x4A));
        assert_eq!(t.meter_green, egui::Color32::from_rgb(0x3D, 0xBE, 0x6B));
        assert_eq!(t.space_xs, 4.0);
        assert_eq!(t.space_md, 8.0);
        assert_eq!(t.grid_spacing, 8.0);
        assert_eq!(
            t.accent,
            egui::Color32::from_rgb(0x4C, 0x9B, 0xE8),
            "one accent for selection, lit toggles, range lines and focus"
        );
        assert_eq!(
            t.accent_soft,
            egui::Color32::from_rgba_unmultiplied(0x4C, 0x9B, 0xE8, 0x2E)
        );
        assert_eq!(t.selection_border, t.accent);
        assert_eq!(t.snap_line_color, t.accent);
        assert_eq!(t.playhead_color, egui::Color32::from_rgb(0xFF, 0x6B, 0x5C));
        assert_eq!(t.marker_color, egui::Color32::from_rgb(0xE8, 0xB4, 0x4C));
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
