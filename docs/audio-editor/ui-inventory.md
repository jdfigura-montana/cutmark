# Audio editor UI inventory

S1 phase 1, written Nov 19, 2026. Factual snapshot of the audio editor UI as it is
in this worktree (branch `s1`, cutmark-audio 1.1.0). The restyle phases (S1 spec,
docs/suite/S1-Audio-Editor-ui-kit.md, "Audio editor mapping") work from this file.
All `file:line` references point at the code on the day this was written.

## After S1

Added at the close of S1 (phase 15, cutmark-audio 1.2.0). The UI now uses ui-kit
throughout, and the theme shim is gone. The main screens are the title bar with
menu bar, the media panel, the timeline panel with the transport deck, the master
panel, the start screen, the help window, the tour and the About box. Everything
below this section is the historical record of the UI before S1 and no longer
describes the code.


## 1. Versions

| Where | egui | eframe | egui-phosphor |
| --- | --- | --- | --- |
| Root `Cargo.toml` [workspace.dependencies] (lines 37 to 40) | `0.36` | `0.36`, `default-features = false` | `0.14.0`, feature `fill` |
| `apps/audio-editor/Cargo.toml` (lines 21 to 22) | not declared (comes through eframe) | `workspace = true` + features `default_fonts`, `wgpu`, `accesskit` | `workspace = true` |
| `crates/ui-kit/Cargo.toml` (lines 22 to 24) | `workspace = true` | `workspace = true` | `workspace = true` |

- Locked in `Cargo.lock`: egui 0.36.2, eframe 0.36.2, egui-phosphor 0.14.0 (also egui-wgpu 0.36.2, egui-winit 0.36.2, wgpu 30.0.1).
- The audio editor uses the workspace versions: yes, for eframe and egui-phosphor. It has no direct egui dependency; egui reaches it through eframe at the same 0.36.2.
- Renderer features the audio editor enables: eframe `wgpu` (plus `default_fonts` and `accesskit`). No glow feature. `main.rs:23` also pins it in code: `renderer: eframe::Renderer::Wgpu`.
- `egui_kittest = "0.36"` sits in the workspace dependencies (root line 40); the audio editor does not use it (its headless tests drive a plain `egui::Context` with injected events through `ui/test_util.rs`, test_util.rs:80).

Note: docs/suite/S0-Workspace.md and the `scripts/check.ps1` comment still describe
cutmark-audio as an eframe-glow app. That is stale: the manifest and `main.rs` are
already on wgpu today.

## 2. Glow

Nothing in `apps/audio-editor` uses glow:

- No `glow` in any Cargo.toml in the workspace (searched all manifests; the only hits anywhere are `Cargo.lock` package entries and `THIRD-PARTY-NOTICES.txt` license lines, where glow 0.17.0 and egui_glow 0.36.2 appear as eframe optional dependencies).
- No `glow` import, no `PaintCallback`, no shaders, no `unsafe` GL code anywhere under `apps/audio-editor/src`.
- All painting is egui `Shape`s and widgets through `eframe::egui`. `main.rs:22-31` builds `eframe::NativeOptions` with `Renderer::Wgpu`.

Phase 2 (wgpu move) therefore has no glow drawing to port; its real work is confirming
this and cleaning up the stale docs.

## 3. UI files

One row per file under `apps/audio-editor/src/ui` plus `src/app.rs` and `src/main.rs`.

| File | Lines | What it draws | Main pub items |
| --- | --- | --- | --- |
| `ui/mod.rs` | 14 | Module declarations only. | - |
| `ui/theme.rs` | 1036 | Not drawn itself: the palette, sizes, fonts, `THEME` struct, egui Style setup, and a debug-only style window. | `THEME`, `apply(ctx)`, `menu_frame()`, `menu_item_style(style)`, `debug_window(ctx)`, `mono/medium/semibold(size)`, `Theme::track_color(i)` |
| `ui/menu.rs` | 686 | Menu bar (File/Edit/Process/View/Markers/Help), project status line with painted unsaved dot, and the dialogs it opens. | `ProjectMenu::show`, `project_status`, `modal_open`, `save`, `open_save_as`, `open_settings`, `open_export` |
| `ui/toolbar.rs` | 191 | Transport row: round Record button, Play/Stop, Pause, Home/End, timecode; Play/Stop end a take, other transport disabled while recording. | `show(ui, editor) -> Vec<Message>` |
| `ui/start_screen.rs` | 583 | Start screen: painted bar-logo, New/Open buttons, recent-project rows with painted tiles, drop zone (dashed painter rect), missing-file row, output device line. | `StartScreen::show -> Option<Editor>`, `with_new_dialog`, `with_open_request`, `dialog_is_open` |
| `ui/status_bar.rs` | 103 | Bottom bar: playhead timecode, selection length, sample rate/depth, RIPPLE and SNAP toggle readouts, export banner, device line. | `show`, `show_start` |
| `ui/bin_panel.rs` | 569 | Media bin: sort combo, two-line rows with usage pill and details, offline styling, inline rename, context menu, remove confirm, per-job progress bars, drop-to-import. | `BinPanel::show -> Vec<String>`, `pick_and_import`, `request_reveal` |
| `ui/master_strip.rs` | 157 | Right master strip: MASTER caption, master fader (app fader widget), horizontal master meters, clip lights. | `show(ui, editor, meters)` |
| `ui/test_util.rs` | 236 | Test-only headless egui harness (not drawn in the app): frame runner, click tap, text/rect/circle queries. | `Headless`, `Out`, `armed_editor`, `feed_take`, `wait_until` |
| `ui/timeline/mod.rs` | 642 | Timeline frame: ruler strip, header column, lanes area, hand-rolled h-scrollbar, wheel/zoom input, playhead follow paging, view-menu actions. | `Geom` (`sample_to_x`, `x_to_sample`, `zoom_at`, `zoom_to_fit`, `zoom_to_selection`, `tick_step`), `show` |
| `ui/timeline/ruler.rs` | 466 | Ruler drawing (adaptive ticks, time labels), marker triangles, marker context menu, scrub and marker drag interaction. | `draw`, `handle_markers`, `handle_scrub`, `draw_marker_menu` |
| `ui/timeline/track_header.rs` | 674 | Track headers: name (inline rename), M/S painted toggles, round Arm button, per-track meters and slider, volume/pan values, drag reorder, bottom resize. | `TrackHeaderState::show` |

| `ui/timeline/lane.rs` | 13 | One lane background: alternating shades plus separator. | `draw(painter, rect, is_even, even_bg, odd_bg, separator)` |
| `ui/timeline/clip_view.rs` | 603 | Clips: track-colored body and header strip, name and gain text, fade lines and handle squares, gain line, selection border, offline body, live recording clip. | `ClipView::show`, rect helpers for clips and the live take |
| `ui/timeline/waveform.rs` | 496 | Waveform painting per clip (min/max envelope per pixel column), center line, channel divider, offline hatch, live recording column. | `draw`, `draw_live`, `choose_level`, `level_minmax`, `samples_minmax`, `envelope`, `live_column` |
| `ui/timeline/interaction.rs` | 965 | Mouse/keyboard interaction over lanes and clips: selection, marquee (dashed rect), move/trim/edge gestures, snap line, range drawing, clip context menu, inline gain edit. | `handle_lane_input`, `handle_clip_input`, `draw_range`, `draw_clip_snap_line`, `draw_clip_menu`, `draw_gain_edit`, `gain_line_active` |
| `ui/widgets/meter.rs` | 269 | Hand-painted level meters: per-channel ballistics (RMS + peak, hold, clip light), horizontal strip meters for master, vertical bar meters in track headers. | `ChannelBallistics`, `MeterState`, `color_for(db)`, `header_meter`, `fill_zone`, `db_y` |
| `ui/widgets/fader.rs` | 159 | Hand-painted fader/slider: track, fill (optionally from center for pan), thumb, shadow cap. | `slider_bare`, `fader_bare`, `db_text`, `pan_text` |
| `ui/dialogs/mod.rs` | 309 | Shared dialog kit on `egui::Modal`: frame, title/description, form rows, text field, segmented control, footer buttons, browse row, dB/ms controls, warning title. | `modal`, `form_row`, `text_field`, `segmented`, `footer_button`, `db_control`, `ms_control`, `warning_title` |
| `ui/dialogs/audio_settings.rs` | 410 | Audio Settings dialog: output device list, buffer/rate controls, input device list with probe. | `AudioSettingsDialog::new_open`, `show`, `is_closed` |
| `ui/dialogs/confirm.rs` | 286 | ConfirmDialog (title + text, the messages dialog), CloseConfirmDialog (unsaved changes), ExportCloseDialog, RecordCloseDialog (recording in progress). | `ConfirmDialog`, `CloseConfirmDialog::show -> Option<CloseChoice>`, `ExportCloseDialog`, `RecordCloseDialog` |
| `ui/dialogs/export.rs` | 512 | Export dialog (format, folder, naming), export progress window, export banner in the status bar. | `ExportDialog::new_open`, `reopen`, `show`, `show_progress`, `ExportBanner` |
| `ui/dialogs/gain.rs` | 151 | Clip Gain / Process Gain dialog (gain dB, fade in/out mode). | `GainDialog`, `request`, `request_process`, `show` |
| `ui/dialogs/new_project.rs` | 119 | New Project dialog (name, sample rate, bit depth). | `NewProjectDialog::new_open`, `show -> Option<Editor>` |
| `ui/dialogs/normalize.rs` | 103 | Normalize dialog (target dBFS). | `NormalizeDialog::request`, `show` |
| `ui/dialogs/project_settings.rs` | 91 | Project Settings dialog (name, bit depth). | `ProjectSettingsDialog::new_open`, `show`, `is_closed` |
| `ui/dialogs/save_as.rs` | 124 | Save As dialog (name, folder picker). | `SaveAsDialog::new_open`, `show`, `is_closed` |
| `src/app.rs` | 1323 | App shell: Screen::Start / Screen::Editing switch, panel layout (menu bar, toolbar, bin, timeline, master strip, status bar), global keyboard handling, window close flow, drag-and-drop import, messages dialog, debug F12 window. | `App` (eframe::App), `APP_NAME` |
| `src/main.rs` | 49 | Entry point: eframe NativeOptions (Wgpu renderer, 1400x850 window, icon), calls `theme::apply`. | `main`, `window_icon` |

## 4. Theme

One theme module: `apps/audio-editor/src/ui/theme.rs`. Everything else in `src/ui`
reads `THEME` (a `LazyLock<Theme>`). Module-level constants:

| Const | Value |
| --- | --- |
| `ACCENT` (theme.rs:9) | `Color32::from_rgb(0x4C, 0x9B, 0xE8)` |
| `MEDIUM_FAMILY` (theme.rs:12) | `"PlexMedium"` |
| `SEMIBOLD_FAMILY` (theme.rs:15) | `"PlexSemiBold"` |
| `PHOSPHOR_FONT` (theme.rs:18) | `"phosphor"` |
| `PLEX_REGULAR/MEDIUM/SEMIBOLD`, `JB_MONO_REGULAR/MEDIUM` (theme.rs:21-29) | `include_bytes!` of the IBM Plex Sans and JetBrains Mono TTFs |
| `TRACK_COLORS` (theme.rs:32-41) | 8 colors: `0x4FB3A9, 0x9C8CE0, 0xD3A86A, 0xD97A8C, 0x7BC47F, 0x5FA8D3, 0xC98BC9, 0xB8B86A` |

`Theme` is one struct (theme.rs:48) with a single `Default` literal (theme.rs:556-802).
Every field and value, grouped as in the source:

- Surfaces: `app_bg #16181C`, `panel_bg #1A1D22`, `bar_bg #121418`, `recessed_a #0E1013`, `ruler_bg #16191D`, `lane_a #1B1E23`, `lane_b #1E2127`, `border #23272E`, `control_border #2B3038`, `control_fill #22262D`, `control_hover #2B3038`, `control_rounding 6.0`, `slider_track #2B3038`, `slider_fill #5A616D`, `slider_thumb #D7DAE0`, `play_green #5CC46E`
- Clip visuals: `header_strip_h 18.0`, `trim_edge_w 6.0`, `fade_handle_sz 8.0`, `fade_handle_grab 2.0`, `gain_hit_px 4.0`, `gain_text_w 56.0`, `clip_text_pad 4.0`, `clip_radius 5.0`, `clip_text_color #F2F4F7`, `clip_name_size 11.0`, `clip_gain_size 10.0`, `clip_margin 2.0`, `thin_clip_w 3.0`, `selection_border_w 2.0`, `selection_border ACCENT`, `offline_body rgba(150,62,62,220)`, `offline_header rgba(110,42,42,230)`, `offline_text WHITE`, `gain_line #F2F4F7`, `gain_curve rgba(#F2F4F7,178)`, `fade_shade rgba(0,0,0,90)`, `fade_handle_fill #E6E9EE`, `fade_handle_hover ACCENT`, `fade_handle_stroke rgb(18,20,23)`
- Waveform: `wave_width 1.0`, `wave_dot_r 1.5`
- Timeline frame: `ruler_h 30.0`, `header_col_w 208.0`, `scrollbar_h 12.0`, `scrollbar_thumb_min 20.0`, `ruler_tick_h 9.0`, `ruler_minor_tick_h 5.0`, `ruler_tick_color #3A404A`, `ruler_label_pad_x 4.0`, `ruler_label_pad_y 3.0`, `ruler_label_size 10.5`, `ruler_min_tick_px 60.0`, `marker_color #E8B44C`, `marker_tri_w 5.0`, `marker_tri_h 8.0`, `playhead_color #FF6B5C`, `playhead_width 1.5`, `playhead_tri_w 5.0`, `playhead_tri_h 10.0`
- Interaction: `snap_px 8.0`, `drag_start_px 3.0`, `boundary_grab_px 4.0`, `snap_line_color ACCENT`, `snap_line_w 2.0`, `range_line ACCENT`, `range_line_w 1.0`, `insert_line_color rgba(#4C9BE8,230)`, `insert_line_w 2.0`, `reorder_line_color ACCENT`, `reorder_line_w 2.0`, `ghost_fill rgba(#4C9BE8,90)`, `ghost_stroke rgba(#4C9BE8,220)`, `ghost_dash 6.0`, `ghost_gap 4.0`, `new_track_h 96.0`, `clip_menu_min_w 150.0`, `gain_edit_w 52.0`, `gain_edit_size 11.0`

- Track header: `track_resize_h 4.0`, `track_pad_v 9.0`, `track_pad_l 10.0`, `track_pad_r 8.0`, `track_row_gap 9.0`, `track_ms_w 22.0`, `track_ms_h 20.0`, `track_ms_gap 4.0`, `track_ms_radius 4.0`, `track_label_col_w 24.0`, `track_value_col_w 44.0`, `track_label_size 9.5`, `track_value_size 10.5`, `track_slider_track_w 4.0`, `track_slider_thumb 12.0`, `track_meter_bar_w 4.0`, `track_meter_gap 2.0`, `track_clip_light 4.0`, `compact_track_h 64.0`, `track_name_size 12.5`, `toggle_size 10.0`, `mute_color #E0A93A`, `solo_color #5CC46E`, `record_red #E5544B`, `arm_glyph_size 12.0`, `on_record_glyph WHITE`, `record_btn_d 28.0`, `record_btn_glyph 16.0`, `clip_body_alpha 0x22`, `clip_header_alpha 0x66`, `clip_name_pin 16.0`, `clip_name_gap 6.0`, `clip_name_min_w 8.0`, `wave_lane_pad 1.0`
- Meters: `meter_bar_bg #101216`, `meter_red #E5544B`, `meter_yellow #E3C04A`, `meter_green #3DBE6B`, `meter_hold rgb(230,234,240)`, `clip_light_on #E5544B`, `clip_light_off #3A2224`
- Bin panel: `bin_row_h 52.0`, `bin_row_gap 2.0`, `bin_row_rounding 6.0`, `bin_header_size 10.5`, `bin_btn_h 26.0`, `bin_btn_text_size 12.0`, `bin_tile_sz 30.0`, `bin_tile_icon_size 16.0`, `bin_name_size 12.5`, `bin_meta_size 10.5`, `bin_pill_fill #262A31`, `bin_pill_h 18.0`, `bin_pill_rounding 9.0`, `bin_pill_text_size 10.5`, `bin_drop_h 44.0`, `bin_drop_text_size 10.5`, `bin_progress_h 2.0`, `bin_rename_pad 40.0`, `bin_rename_min_w 60.0`, `bin_offline rgb(210,100,100)`
- Errors / dialogs: `error rgb(225,105,105)`, `error_soft rgb(235,140,140)`
- Spacing: `space_xs 4.0`, `space_sm 4.0`, `space_md 8.0`, `space_lg 16.0`
- Shared shapes: `corner_r 2.0`, `corner_round 3.0`, `corner_xs 1.0`, `hairline 1.0`

- Accent, text roles and dialogs: `accent ACCENT`, `accent_soft rgba(#4C9BE8,0x2E)`, `text_primary #E6E9EE`, `text_normal #C9CED6`, `text_muted #8A909A`, `text_faint #6E7581`, `hover_overlay rgba(#4C9BE8,0x2E)`, `dialog_w 420.0`, `export_dialog_w 480.0`, `modal_shade from_black_alpha(140)`, `card_radius 10.0`, `card_pad 20.0`, `card_shadow [0,4] blur 16 spread 0 black_alpha(90)`, `menu_radius 8.0`, `menu_pad 6.0`, `menu_item_radius 5.0`, `menu_shadow [0,4] blur 12 spread 0 black_alpha(90)`, `menu_right_pad 8.0`, `unsaved_dot_d 6.0`, `field_h 30.0`, `form_label_w 140.0`, `form_row_gap 10.0`, `segment_pad 16.0`, `segment_pad_min 8.0`, `value_box_w 72.0`, `value_gap 8.0`, `dialog_title_size 15.0`, `dialog_desc_size 12.0`, `dialog_label_size 12.5`, `destructive #E5544B`, `accent_hover #60AAEC`
- Status bar and start screen: `status_bar_h 26.0`, `start_card_w 1000.0`, `start_card_h 600.0`, `start_card_margin 48.0`, `start_card_min_w 760.0`, `start_card_radius 14.0`, `start_col_w 360.0`, `start_pad_top 36.0`, `start_pad_side 32.0`, `start_pad_bottom 28.0`, `start_pad_r_top 32.0`, `start_pad_r_side 28.0`, `start_pad_r_bottom 20.0`, `start_logo_h 40.0`, `start_bar_w 4.0`, `start_bar_pitch 6.0`, `start_bar_heights [8,16,26,34,22,30,38,24,14,28,36,20,30,18,10,24,32,16,8,12]`, `start_title_gap 20.0`, `start_buttons_gap 40.0`, `start_btn_h 52.0`, `start_btn_gap 10.0`, `start_btn_radius 8.0`, `start_drop_gap 20.0`, `start_drop_h 52.0`, `start_drop_radius 8.0`, `start_on_accent WHITE`, `start_row_gap 2.0`, `start_row_h 64.0`, `start_tile 40.0`, `start_last_col_w 96.0`, `start_x_btn 28.0`, `start_missing_fill #3A2224`, `start_row_recent #1F2328`, `start_missing_text #E5544B`, `start_missing_icon #5A616D`, `start_title_size 24.0`, `start_small_size 11.0`, `start_btn_text_size 14.0`, `start_hint_size 12.0`, `start_name_size 14.0`, `start_pill_size 10.5`, `start_icon_size 16.0`, `start_header_h 22.0`, `start_row_radius 8.0`, `start_dash 4.0`, `start_dash_gap 4.0`

Fonts (theme.rs:807-839): IBM Plex Sans Regular/Medium/SemiBold as the proportional
face (Medium inserted first), JetBrains Mono Regular/Medium as monospace, named
families `PlexMedium` and `PlexSemiBold`, phosphor Fill added as fallback in every
family. `apply(ctx)` (theme.rs:847) installs fonts, dark Visuals and 4/8 px spacing.

## 5. Hard-coded style outside theme.rs

The style lint targets hex colors, `Color32::from_rgb`, literal corner radii and font
sizes. Outside `theme.rs` the drawing code is clean of color literals; what remains is
literal alpha amounts layered on theme colors, and three literal font sizes.

`ui/start_screen.rs`
- 295: `from_rgba_unmultiplied(tc.r(), tc.g(), tc.b(), 0x1F)` and `..., 0x55)`: literal alphas over a track color (recent-project tile fill and border).

`ui/timeline/clip_view.rs`
- 215: `(0x55 as f32 * scale) as u8`: literal alpha on the clip stroke.
- 362: `(76.0 * scale) as u8`: literal alpha on the gain-line area fill.
- (185, 186, 263, 573, 574, 579: alphas come from `THEME.clip_body_alpha` / `clip_header_alpha` or are passed in; no color literal.)

`ui/timeline/waveform.rs`
- 141: `(0x33 as f32 * scale) as u8`: literal alpha on the waveform center line.
- 142: `(89.0 * scale) as u8`: literal alpha on the channel divider.

`ui/widgets/fader.rs`
- 147: `Color32::from_black_alpha(60)`: literal shadow under the fader cap.

`ui/timeline/ruler.rs`
- 87: `from_rgb(mix(r), mix(g), mix(b))`: derived hover mix, no literal color, listed for completeness.

Literal font sizes outside theme.rs:
- `ui/master_strip.rs:45`: `FontId::proportional(11.0)` (MASTER caption).
- `ui/dialogs/mod.rs:287`: `FontId::proportional(16.0)` (warning glyph).
- `ui/timeline/track_header.rs:227`: `FontId::proportional(14.0)` (track name rename editor).

Corner radii: none outside theme.rs. Every radius argument reads a `THEME` field
(`corner_r`, `corner_round`, `clip_radius`, `bin_row_rounding`, and so on).
`Color32::WHITE` appears outside theme.rs only in tests (for example
`ui/timeline/track_header.rs:541`). Phase 13's lint will need to decide whether
literal alpha amounts (this section) are in scope; the spec's lint scans for hex
colors, `from_rgb`, radii and font sizes, so the alpha literals above would pass it
as specified.

## 6. Custom widgets (hand-painted)

| Widget | Where | Signature | Paints |
| --- | --- | --- | --- |
| Level meter ballistics | `ui/widgets/meter.rs` | `ChannelBallistics::update(&mut self, now, dt, peak, sum_sq, frames, clipped)`, `rms_db()`, `color_for(db) -> Color32` (meter.rs:91) | Ballistics math and the green/yellow/red zone color for a dB value |
| Horizontal strip meter | `ui/widgets/meter.rs` | `MeterState::update(...)`, `header_meter(ui, id, rect, channels) -> bool` (meter.rs:171), `fill_zone(painter, bar, lo, hi, color)` (meter.rs:198) | Recessed bar, two channel strips, peak holds, clip lights; used in the master strip and per-track headers |
| Fader / slider | `ui/widgets/fader.rs` | `slider_bare(ui, id, rect, value, range, fill_from_center) -> SliderOutcome` (fader.rs:46), `fader_bare(ui, id, rect, value, range) -> SliderOutcome` (fader.rs:108) | Rect track, fill (center fill for pan), square thumb with shadow cap; pure painter, no egui Slider |
| Waveform | `ui/timeline/waveform.rs` | `draw(painter, clip, loaded, g, body, wave_scale, color, muted)` (waveform.rs:110), `draw_live(painter, status, g, body, wave_scale, color)` (waveform.rs:190) | Min/max envelope columns from peaks or samples, center line, channel divider, take waveform while recording |
| Ruler | `ui/timeline/ruler.rs` | `draw(...)` (ruler.rs:330), `handle_markers`, `handle_scrub` | Ticks and time labels at adaptive steps, marker triangles, faint marker lines, playhead handle, scrub |
| Track header toggles | `ui/timeline/track_header.rs` | `toggle_button(ui, rect, id, label, active) -> bool` (track_header.rs:402), arm button painter code (track_header.rs:421-440) | Painted M/S rounded squares, round Arm disc with phosphor record glyph |
| Clip visuals | `ui/timeline/clip_view.rs` | `ClipView::show` | Clip body/header from track color with alpha, fade triangles and handle squares, gain polyline, selection border, offline hatch, live take clip |
| Misc painter drawing | `ui/bin_panel.rs:391`, `ui/start_screen.rs:482`, `ui/menu.rs:46`, `ui/timeline/interaction.rs:155` | local fns | Dashed rectangles (bin drop zone, start drop zone), painted unsaved dot, marquee dashed rect |

## 7. Keyboard shortcuts

Global handling in `src/app.rs` (`handle_keys`, lines 416 to 570), firing only when no
text field has focus, the editor is not locked, and no take is recording.

| Shortcut | Action | file:line |
| --- | --- | --- |
| Ctrl+A | Select all | app.rs:452 |
| Ctrl+I | Import media (file picker) | app.rs:454 |
| Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y | Undo / redo / redo | app.rs:456-463 |
| Ctrl+N | New project (menu action) | app.rs:464 |
| Ctrl+O | Open project | app.rs:466 |
| Ctrl+Shift+S | Save As dialog | app.rs:468 |
| Ctrl+E | Export dialog | app.rs:470 |
| Ctrl+R | Start recording on the armed track | app.rs:472 |
| Ctrl+Shift+T | Add track | app.rs:478 |
| Ctrl+S | Save | app.rs:480 |
| Delete / Backspace | Delete selection (marker, clips or range) | app.rs:482 |
| Esc | Clear selection | app.rs:484 |
| R | Toggle ripple | app.rs:486 |
| N | Toggle snap | app.rs:488 |
| S | Split at playhead | app.rs:490 |
| Ctrl+C / Ctrl+X / Ctrl+V | Copy / cut / paste selection | app.rs:492-497 |
| Ctrl+T | Trim to selection | app.rs:498 |
| M | Add marker at playhead | app.rs:500 |
| , / . | Jump to previous / next marker | app.rs:502-505 |
| Ctrl+G | Process Gain dialog | app.rs:506 |
| Ctrl+Shift+N | Normalize dialog | app.rs:511 |
| Ctrl+Shift+I / Ctrl+Shift+O | Fade in / fade out | app.rs:513-520 |
| Ctrl+L | Silence selection | app.rs:521 |
| Space | Play / stop toggle | app.rs:535 |
| Shift+Space | Play selection once | app.rs:538 |
| P | Pause toggle | app.rs:547 |
| Home / End | Jump to start / end | app.rs:550-555 |

While a take is recording (`handle_recording_keys`, app.rs:577-598), only these work:
Ctrl+S save (app.rs:580), Space stop and keep the take (app.rs:587), Esc stop and
discard, consumed so it never also clears the selection (app.rs:590), M marker at the
record position (app.rs:593). Everything else is ignored.

Start screen (`handle_start_shortcuts`): Ctrl+N opens the New Project dialog
(app.rs:632), Ctrl+O the Open picker (app.rs:634).

Timeline (ui/timeline/mod.rs): `\` zoom to fit and Ctrl+`\` zoom to selection
(mod.rs:168-174), `+`/`=`/`-` zoom by 1.5 anchored on the playhead or view center
(mod.rs:318-331), Ctrl+wheel zoom at the mouse (mod.rs:280-288), Alt+wheel vertical
waveform scale (mod.rs:289-297), Shift+wheel horizontal scroll (mod.rs:298-306), plain
wheel vertical track scroll (mod.rs:307-314).

Local Esc/Enter handling (consume their key so the global handler does not also fire):
Esc cancels a marker drag (ruler.rs:113-120), a clip or lane gesture (interaction.rs:45,
718-739) and a track reorder (track_header.rs:74); Esc or a click closes the clip menu
(interaction.rs:848) and the marker menu (ruler.rs:293); Esc cancels the inline gain
edit (interaction.rs:949-951); Enter confirms the gain edit (interaction.rs:940) and a
track rename (track_header.rs:263); Ctrl+click on a track header moves the track
(track_header.rs:205) and makes solo exclusive (track_header.rs:290).

Debug builds only: F12 opens the style window (app.rs:121-123, theme.rs:941).

## 8. Dialogs and screens

Screens (app.rs `Screen` enum): the start screen (`start_screen.rs`) and the editing
screen. The editing screen lays out menu bar, toolbar, bin panel, timeline, master
strip and status bar in `App::ui` (app.rs, panel section around line 353).

| Dialog | Opened by |
| --- | --- |
| New Project (`dialogs/new_project.rs`) | Start screen button or Ctrl+N; Project > New in the menu (`ProjectMenuAction::New`, app.rs:464) |
| Open | Start screen button or Ctrl+O; Project > Open; native rfd picker, no custom dialog |
| Save As (`dialogs/save_as.rs`) | Ctrl+Shift+S or File > Save As (`menu.open_save_as`, menu.rs:433) |
| Export (`dialogs/export.rs`) | Ctrl+E or File > Export (`menu.open_export`, menu.rs:444); `show_progress` (export.rs:311) draws the progress window while the job runs; `ExportBanner` sits in the status bar |
| Audio Settings (`dialogs/audio_settings.rs`) | menu.open_settings (menu.rs:438) from the File menu |
| Project Settings (`dialogs/project_settings.rs`) | Project menu entry via `ProjectMenu` |
| Gain (`dialogs/gain.rs`) | Ctrl+G (`request_process`, gain.rs:71) or the clip gain line (`request`, gain.rs:55) |
| Normalize (`dialogs/normalize.rs`) | Ctrl+Shift+N (`request`, normalize.rs:28) |
| Messages / confirm (`confirm.rs` `ConfirmDialog`) | One dialog per frame for every op error, job error and notice (app.rs:560-568); title comes from `editor::message::dialog_title` |
| Unsaved changes (`confirm.rs` `CloseConfirmDialog`) | Window X / Alt+F4 / New / Open while the project is dirty (`pending_close`) |
| Export in progress close (`confirm.rs` `ExportCloseDialog`) | Close attempt while an export job runs |
| Recording in progress close (`confirm.rs` `RecordCloseDialog`) | Close attempt while a take is recording |
| Menus (`menu.rs` `ProjectMenu`) | egui menu bar at the top of the editing screen: File, Edit, Process, View, Markers, Help; the View menu returns `ViewMenuAction` applied by the timeline |
| Style debug window (theme.rs:941) | F12 in debug builds only |

## 9. ui-kit today

`crates/ui-kit` (package `ui-kit`) public items:

- `src/lib.rs`: `pub mod fonts`, `pub mod meter`, `pub mod palette`; `pub use meter::level_meter`; `pub fn apply(ctx: &egui::Context)` (lib.rs:42) installs fonts, Visuals and spacing from the palette.
- `src/fonts.rs`: `MEDIUM_FAMILY` ("PlexMedium"), `SEMIBOLD_FAMILY` ("PlexSemiBold"), `PHOSPHOR_FONT` ("phosphor"); `definitions() -> FontDefinitions`, `mono(size)`, `medium(size)`, `semibold(size)`. IBM Plex Sans Regular/Medium/SemiBold plus JetBrains Mono Regular/Medium, phosphor fallback in every family.
- `src/meter.rs`: `pub fn level_meter(ui, peak_dbfs, clipped, width) -> Response` (meter.rs:31): horizontal bar, -60 to 0 dBFS, green to -12, yellow to -3, red above, clip light.
- `src/palette.rs`: 22 pub constants, 21 colors plus `CONTROL_ROUNDING`: `APP_BG #16181C`, `PANEL_BG #1A1D22`, `BAR_BG #121418`, `RECESSED #0E1013`, `FAINT_BG #1E2127`, `BORDER #23272E`, `CONTROL_BORDER #2B3038`, `CONTROL_FILL #22262D`, `CONTROL_HOVER #2B3038`, `CONTROL_ROUNDING 6.0`, `SLIDER_TRACK #2B3038`, `SLIDER_FILL #5A616D`, `SLIDER_THUMB #D7DAE0`, `TEXT_PRIMARY #E6E9EE`, `TEXT_NORMAL #C9CED6`, `TEXT_MUTED #8A909A`, `ACCENT #4C9BE8`, `RECORD_RED #E5544B`, `PLAY_GREEN #5CC46E`, `METER_GREEN #3DBE6B`, `METER_YELLOW #E3C04A`, `METER_RED #E5544B`. (The S1 spec's starting-point table says "25 color constants plus CONTROL_ROUNDING"; the file holds 21 color constants, not 25.)

Places outside `crates/ui-kit` that use `ui_kit`: only `apps/recorder` (package
`cutmark-recorder`). The audio editor uses none of it (zero `ui_kit` hits under
`apps/audio-editor/src`).

| User | file:line | Item used |
| --- | --- | --- |
| apps/recorder/src/app.rs | 102 | `ui_kit::apply(&cc.egui_ctx)` |
| apps/recorder/src/app.rs | 292 | `ui_kit::palette::ACCENT` |
| apps/recorder/src/label_popup.rs | 102 | `use ui_kit::palette` |
| apps/recorder/src/marks_ui.rs | 5 | `palette::{ACCENT, METER_GREEN, METER_YELLOW, RECORD_RED, TEXT_MUTED, TEXT_NORMAL, TEXT_PRIMARY}` |
| apps/recorder/src/profile_ui.rs | 322, 339, 488, 573 | `ui_kit::palette::TEXT_MUTED` |
| apps/recorder/src/tracks_ui.rs | 9 | `ui_kit::palette::RECORD_RED` |
| apps/recorder/src/tracks_ui.rs | 240 | `ui_kit::level_meter(...)` |

So the Recorder already uses `apply`, `level_meter` and eight palette constants;
`fonts` items (`mono`, `medium`, `semibold`, the family consts) and the other palette
constants are currently unused outside the crate.

## 10. Gaps against the spec's "Audio editor mapping" table

Rows found in this inventory that the mapping table (S1 spec lines 103-123) does not
list. Per the spec, "anything it finds that is not here gets a row before Phase 10".

1. `ui/timeline/interaction.rs` (965 lines): gestures, marquee, snap line, range
   drawing, clip context menu, inline gain edit. The clip and playhead rows cover the
   static visuals, but the interaction visuals (marquee, snap line, ghost clip) have
   no row.
2. `ui/timeline/mod.rs` frame chrome: the hand-rolled horizontal scrollbar and the
   lane backgrounds are only implied by the lane/clip rows; the scrollbar has no row.
3. Track header meters and sliders (`track_header.rs` plus `widgets/meter.rs`
   `header_meter`): the mapping covers the master strip's meter and fader and deletes
   the app's meter and fader widgets, but the per-track header meters and sliders are
   not named anywhere.
4. `ui/dialogs/mod.rs` shared dialog kit (form rows, segmented control, footer
   buttons, dB/ms controls): the dialogs row maps to the ui-kit dialog frame, but this
   in-app helper layer (which the ui-kit widgets replace piece by piece) is not named.
5. Start screen painted logo (`start_bar_heights` bars) and the start screen status
   line (`status_bar::show_start`): covered only by the generic start screen row.
6. `ui/test_util.rs`, the headless test harness in `ui/`: not a visual, but it lives
   in `ui/` and survives the restyle; worth a "not restyled" note.
7. The debug-only F12 style window (`theme::debug_window`): disappears with the theme
   module per the mapping ("The app's own theme module: deleted"), but no row says so.
8. Window icon and title handling (main.rs:24-29, app.rs `update_title`): the custom
   chrome row covers the title bar visually, but the dynamic title text (dirty marker,
   project name) is app behavior the chrome must keep.
9. Toolbar zoom readout and the view toggles the View menu drives (ripple, snap, wave
   scale): the "Timeline panel toolbar" row names tools and zoom, but the status-bar
   RIPPLE/SNAP readouts and the Alt+wheel wave scale have no row.

Also noted, not a mapping gap: cutmark-audio is already on wgpu (sections 1 and 2),
so Phase 2's move may be a no-op beyond deleting stale glow notes, and the spec's
ui-kit starting-point count of palette constants (25) differs from the file (21 plus
`CONTROL_ROUNDING`).











