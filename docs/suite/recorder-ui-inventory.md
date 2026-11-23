# Recorder UI inventory (Phase 13, R1)

Read-only inventory of every UI file, control, hard-coded style value, `ui_kit` item and hotkey in
`apps/recorder`, as Phase 13 R1 requires. Line numbers are as of the s1 tag (Nov 22, 2026).
Nothing in `apps/recorder/src` was changed.

## 1. UI files

| File | Draws |
| --- | --- |
| apps/recorder/src/app.rs | The window frame: headings, warning banners, recovery banner, preview toggle, the recording and last-take panels, the countdown, hotkey and Control API sections, and it calls every other section in order |
| apps/recorder/src/profile_ui.rs | The Profile section: profile picker with New/Duplicate/Rename/Delete, every encoder field with inline errors, the Project and Naming template fields and the next-take path line |
| apps/recorder/src/webcam_ui.rs | The Webcam section: Enable switch, camera and format pickers, camera picture, camera control sliders, Camera ISO file group, chroma-key hand-off, camera sync test |
| apps/recorder/src/key_ui.rs | The Chroma key group inside the Webcam section: key color, eyedropper, matte view, similarity/smoothness/spill, garbage crop, mask rectangle |
| apps/recorder/src/layout_ui.rs | The Layout section: Screen only / Picture in picture / Cam only, and the whole picture-in-picture grid; then calls the Background section |
| apps/recorder/src/background_ui.rs | The Background section: None/Colour/Image/Video, the color picker, Browse/Clear for image and video, the Fit choice |
| apps/recorder/src/tracks_ui.rs | The Audio tracks panel: one row per track (name, device, gain, mute, offset, in-mixdown, level meter), Add track, Refresh devices, WAV sidecars, audio format |
| apps/recorder/src/marks_ui.rs | The Marks panel: the most recent marks as a timecode / category / label grid |
| apps/recorder/src/preview_ui.rs | The screen preview picture (texture upload, letterboxed draw) and the dark placeholder with a message |
| apps/recorder/src/status_ui.rs | The bottom status bar: live take numbers while recording, disk free while idle, finalizing progress |
| apps/recorder/src/preflight_ui.rs | The Pre-flight section: run button, Keep files checkbox, progress bar, pass/fail report, Open folder buttons |
| apps/recorder/src/webcam.rs | Draws the camera picture itself (`WebcamHost::show_preview`, via `PreviewView`); it also holds the camera settings logic |
| apps/recorder/src/countdown.rs | The countdown overlay: a native Win32 window (no egui), a big number on a translucent circle |
| apps/recorder/src/label_popup.rs | The Shift+F9 label popup: a native Win32 window (no egui) with category pills, an edit control and a hint line |
| apps/recorder/src/tray.rs | The tray icon (grey/red dot) and its menu, on its own thread via `tray-icon` |
| apps/recorder/src/sync_flash.rs | The sync flash: a native Win32 window that covers the monitor in white for the camera sync test |

Not UI: compositor.rs, config.rs, profiles.rs, recovery.rs, takes.rs, platform.rs, hotkeys.rs,
api/* and engine/* (logic and settings only; hotkeys.rs draws nothing, the hotkey grid lives in app.rs).

## 2. Controls of the main window, in on-screen order

The whole window is one scrolling column (CentralPanel in `app.rs::RecorderApp::ui`) plus a bottom
status bar. "State" is the field a change writes; `settings.*` is Settings (settings.json),
`profile.*` is the active Profile (profiles\<name>.json via the ProfileStore).

| # | On screen | Widget | Drawn by | State edited |
| --- | --- | --- | --- | --- |
| 1 | "Cutmark Recorder" heading, "Active profile: X" | heading, label | app.rs::ui | - |
| 2 | Warning banners (capture exclusion failed, recording error, audio notice, engine banners, hotkey problems, preview problem, webcam warning, API problem) | colored labels | app.rs::ui | - |
| 3 | "Recovered marks for N take(s)" | label + "Details"/"Hide details" + "Dismiss" small buttons | app.rs::recovery_banner | RecoveryReport, recovery_details (not saved) |
| 4 | "Preview" | checkbox | app.rs::ui | preview_enabled (not saved) |
| 5 | "Hide Recorder from screen captures" | checkbox | app.rs::ui | settings.hide_from_capture |
| 6 | Preview picture | texture draw / placeholder | preview_ui.rs::PreviewView::show, app.rs::show_preview | - (click support unused here) |
| 7 | Webcam "Enable" | checkbox | webcam_ui.rs::show | profile.webcam on/off (WebcamHost::enable/disable) |
| 8 | "Camera" | ComboBox + "Refresh" button | webcam_ui.rs::camera_row | profile.webcam.camera_id / camera_name |
| 9 | "Format" | ComboBox | webcam_ui.rs::format_row | profile.webcam.format |
| 10 | Camera picture | texture draw / placeholder, eyedropper click while armed | webcam.rs::WebcamHost::show_preview, webcam_ui.rs::preview_click | - (picks key color) |
| 11 | "Camera controls" (one row per supported property: exposure, focus, white balance, gain, brightness) | Slider + "Auto" checkbox | webcam_ui.rs::controls | profile.webcam.controls.<prop> (live via WebcamHost::set_control) |
| 12 | "Lock current values", "Reset to defaults" | buttons | webcam_ui.rs::controls | profile.webcam.controls (all) |
| 13 | "Camera ISO file": "Also save the raw camera as its own file" | checkbox | webcam_ui.rs::iso_group | profile.webcam.iso |
| 14 | Camera ISO rate control ("Rate control" mode + Quality / QP I,P / Target bitrate / Max bitrate) | ComboBox + Slider/DragValue | profile_ui.rs::rate_control_editor (from webcam_ui.rs::iso_group) | profile.webcam.iso_rc |
| 15 | Camera ISO "Codec" | label (text only) | webcam_ui.rs::iso_group | - |
| 16 | "Camera offset" | DragValue -500..=500 ms | webcam_ui.rs::iso_group | profile.webcam.offset_ms |
| 17 | "Chroma key": "Enable key" | checkbox | key_ui.rs::show | profile.webcam.key.enabled |
| 18 | "Key color" | color edit button | key_ui.rs::show | profile.webcam.key.color |
| 19 | "Eyedropper" / "Click the camera picture..." | selectable label | key_ui.rs::show | KeyUiState.eyedropper (not saved) |
| 20 | "Matte view" | checkbox (enabled only when key is on) | key_ui.rs::show | KeyUiState.matte (not saved) |
| 21 | "Similarity", "Smoothness", "Spill" | sliders 0..=1 | key_ui.rs::show | profile.webcam.key.similarity / smoothness / spill |
| 22 | "Garbage crop": "Left", "Right", "Top", "Bottom" | sliders 0..=90 % | key_ui.rs::show | profile.webcam.key.crop_*_pct |
| 23 | "Mask rectangle" | checkbox + "X", "Y", "Width", "Height" sliders 0..=100 % | key_ui.rs::show, key_ui.rs::mask_sliders | profile.webcam.key.mask_enabled / mask_x_pct / mask_y_pct / mask_w_pct / mask_h_pct |
| 24 | "Camera sync": "Sync test (12 s)" | button + hint labels + progress bar | webcam_ui.rs::sync_test | sends CommandKind::SyncTest |
| 25 | Sync result: measured offset, residual, verdict, "Apply as camera offset" | labels + button | webcam_ui.rs::sync_result | profile.webcam.offset_ms |
| 26 | "Layout": "Screen only", "Picture in picture", "Cam only" | selectable values | layout_ui.rs::show | profile.layout |
| 27 | "Corner" | ComboBox | layout_ui.rs::pip_controls | profile.pip.anchor |
| 28 | "X offset", "Y offset", "Scale" | sliders | layout_ui.rs::pip_controls | profile.pip.offset_x_pct / offset_y_pct / scale_pct |
| 29 | "Crop left/right/top/bottom" | sliders 0..=90 % | layout_ui.rs::pip_controls | profile.pip.crop_*_pct |
| 30 | "Border width" | slider 0..=40 px | layout_ui.rs::pip_controls | profile.pip.border_px |
| 31 | "Border color" | color edit button | layout_ui.rs::pip_controls | profile.pip.border_color |
| 32 | "Shape": "Rectangle", "Circle" | selectable values | layout_ui.rs::pip_controls | profile.pip.shape |
| 33 | "Corner radius" | slider 0..=100 px (Rectangle only) | layout_ui.rs::pip_controls | profile.pip.radius_px |
| 34 | "Background": "None", "Colour", "Image", "Video" | selectable values | background_ui.rs::show | profile.background.kind |
| 35 | "Colour" | color edit button | background_ui.rs::show | profile.background.color |
| 36 | "Image"/"Video" + file name | labels + "Browse..." and "Clear" buttons | background_ui.rs::show | profile.background.image_path / video_path (native file dialog via platform.rs) |
| 37 | "Fit": "Cover", "Contain", "Stretch" | selectable values | background_ui.rs::show | profile.background.fit |
| 38 | "Marks" | grid of recent marks (text only) | marks_ui.rs::show | - |
| 39 | "Recording"/"Paused" panel: Time, File, Folder, Size, Bitrate, Frames, Duplicated, Dropped, Captured | heading + grid (text only) | app.rs::recording_panel | - |
| 40 | "Starting in N" (countdown) | heading | app.rs::ui | - |
| 41 | "Idle" / "Idle. Starting is blocked..." | label / colored label | app.rs::ui | - |
| 42 | "Last take": File, Frames, Duplicated, Dropped, Size, Duration, Average bitrate | heading + grid (text only) | app.rs::summary_panel | - |

| 43 | Profile picker | ComboBox + "New", "Duplicate", "Rename", "Delete"/"Confirm delete" buttons (or a TextEdit + "OK"/"Cancel" while renaming) | profile_ui.rs::picker_row | settings.active_profile; profile files via profiles.rs |
| 44 | "Codec" | ComboBox (AV1/HEVC/H.264, gated by the AMF probe) | profile_ui.rs::fields | profile.codec |
| 45 | "Rate control" + Quality / QP / Target bitrate / Max bitrate | ComboBox + Slider/DragValue | profile_ui.rs::rate_control_editor | profile.rate_control |
| 46 | "Preset" | ComboBox (Speed/Balanced/Quality) | profile_ui.rs::fields | profile.preset |
| 47 | "Keyframe every" | DragValue 0..=60 s | profile_ui.rs::fields | profile.keyframe_secs |
| 48 | "Output size" | ComboBox (Source, 3840x2160, 2560x1440, 1920x1080, 1280x720, Custom) + two DragValues when Custom | profile_ui.rs::fields | profile.width / height |
| 49 | "Frame rate" | ComboBox (30, 60, Custom) + DragValue 1..=240 when Custom | profile_ui.rs::fields | profile.fps |
| 50 | "Container" | ComboBox (MKV/MP4/MOV) + MKV hint | profile_ui.rs::fields | profile.container |
| 51 | "Project" | single-line TextEdit | profile_ui.rs::naming_fields | settings.project |
| 52 | "Naming template" | single-line TextEdit + tokens hint | profile_ui.rs::naming_fields | settings.naming_template |
| 53 | "Next take: <path>" | label | profile_ui.rs::naming_fields | - |
| 54 | "Countdown before recording" | ComboBox (Off, 3 seconds, 5 seconds) | app.rs::countdown_section | settings.countdown_secs |
| 55 | "Hide my countdown when a prompter is linked" | checkbox | app.rs::countdown_section | settings.hide_countdown_when_linked |
| 56 | "Run pre-flight (5 s)" | button | preflight_ui.rs::PreflightSection::show | sends CommandKind::Preflight |
| 57 | "Keep files" | checkbox | preflight_ui.rs::PreflightSection::show | PreflightSection.keep_files (not saved) |
| 58 | Pre-flight progress and report | ProgressBar + grid (text only) + "Open notification settings" and "Open folder" buttons | preflight_ui.rs::PreflightSection::show, preflight_ui.rs::show_result | - |
| 59 | Control API "On" | checkbox | app.rs::api_section | settings.api.enabled |
| 60 | "Port" | DragValue 1024..=65535 | app.rs::api_section | settings.api.port |
| 61 | Listening/clients/"Not running", token | labels | app.rs::api_section | - |
| 62 | Token "Show"/"Hide", "Copy", "Regenerate" | buttons | app.rs::api_section | api_show_token (not saved); Regenerate replaces the Credential Manager token |
| 63 | Track rows: "On" | checkbox | tracks_ui.rs::track_row | settings.audio.tracks[i].enabled |
| 64 | Track name | single-line TextEdit | tracks_ui.rs::track_row | settings.audio.tracks[i].name |
| 65 | Track device | ComboBox, 280 wide | tracks_ui.rs::device_picker | settings.audio.tracks[i].device_id / device_name / kind |
| 66 | "Remove" | button | tracks_ui.rs::track_row | removes settings.audio.tracks[i] |
| 67 | "Gain" | slider -30..=12 dB | tracks_ui.rs::track_row | settings.audio.tracks[i].gain_db (live) |
| 68 | "Mute" | toggle | tracks_ui.rs::track_row | settings.audio.tracks[i].muted (live) |
| 69 | "Offset" | slider -500..=500 ms | tracks_ui.rs::track_row | settings.audio.tracks[i].offset_ms |
| 70 | "In mixdown" | checkbox | tracks_ui.rs::track_row | settings.audio.tracks[i].in_mixdown |
| 71 | Track level meter | ui_kit::level_meter | tracks_ui.rs::track_row | - |
| 72 | "Add track", "Refresh devices" | buttons | tracks_ui.rs::tracks_panel | appends a track; refreshes the device list |
| 73 | "Also save each track as a WAV file" | checkbox | tracks_ui.rs::tracks_panel | settings.audio.wav_sidecars |
| 74 | "Audio format": "AAC 320 kbps", "PCM 24-bit" | radio buttons | tracks_ui.rs::tracks_panel | settings.audio.codec |
| 75 | "Hotkeys" table: Chord, Action, Status | striped grid (text only) | app.rs::hotkey_grid | - |
| 76 | "Recent commands" | labels in a ScrollArea | app.rs::ui | - |
| - | Status bar (bottom panel): elapsed, size, bitrate, disk free, time left, duplicated, dropped, queue, camera line, or disk free while idle, or finalizing progress | labels + ProgressBar | status_ui.rs::show | - |

**Source pickers**

| Source | Where | Notes |
| --- | --- | --- |
| Display | none | There is no display picker yet; the recording and the preview use the primary monitor (app.rs preview_monitor is fixed at 0, never drawn) |
| Camera | webcam_ui.rs::camera_row (ComboBox "webcam-camera", section 8) | Lists cameras from WebcamHost; "Refresh" re-lists |
| Microphone (and any audio device) | tracks_ui.rs::device_picker (ComboBox per track, section 65) | One dropdown per track listing inputs as "Input: ..." and outputs as "Output (loopback): ..."; the mic is not special-cased in the UI |

## 3. Other windows and overlays

| Surface | Kind | File | Notes |
| --- | --- | --- | --- |
| Shift+F9 label popup | Native Win32 window, own thread, no egui | label_popup.rs | Topmost, hidden from capture; category pills (Highlight, Cut, Retake, Chapter, Note, B-roll), a text edit and a hint line; saves at the press QPC |
| Countdown overlay | Native Win32 window, own thread, no egui | countdown.rs | Big number on a translucent dark circle, topmost, hidden from capture; owns a global Esc hotkey while armed |
| Sync flash | Native Win32 window, own thread, no egui | sync_flash.rs | Covers the monitor in white; deliberately NOT hidden from capture |
| Tray icon | tray-icon menu on its own thread | tray.rs | "Show window", "Start or stop recording", "Quit"; grey idle, red recording; tooltip changes with state |
| Minimize to tray | behavior, not a window | app.rs::hide_to_tray_when_minimized | The window hides itself when minimized while recording (spec: minimizes to tray) |
| File pickers | Native dialogs | called as platform::pick_image_file / pick_video_file from background_ui.rs | - |
| egui popups, modals or dialogs | none | - | Every ComboBox popup is stock egui; there is no custom dialog, modal or toast in the app today |

## 4. Hard-coded style

egui side:

| File:Line | Value |
| --- | --- |
| preflight_ui.rs:11 | PASS_GREEN = Color32::from_rgb(70, 190, 100) |
| preview_ui.rs:7 | BACKDROP = Color32::from_gray(14) |
| preview_ui.rs:91 | Color32::WHITE as the image tint |
| preview_ui.rs:140 | FontId::proportional(16.0) for the placeholder message, Color32::GRAY text |
| webcam_ui.rs:26 | SYNC_GOOD = Color32::from_rgb(0x5C, 0xB8, 0x5C) (#5CB85C) |
| webcam_ui.rs:29 | SYNC_WARN = Color32::from_rgb(0xE6, 0xB4, 0x3C) (#E6B43C) |

`ui_kit::palette` uses (the deprecated direct-palette reads R10 removes):

| File:Line | Constants |
| --- | --- |
| marks_ui.rs:5 | ACCENT, METER_GREEN, METER_YELLOW, RECORD_RED, TEXT_MUTED, TEXT_NORMAL, TEXT_PRIMARY |
| tracks_ui.rs:9 | RECORD_RED (missing-device text) |
| app.rs:313 | ACCENT (recovery banner headline) |
| profile_ui.rs:289, 305, 454, 501 | TEXT_MUTED (locked note, tokens hint, "Checking encoders...", Source size caption) |
| countdown.rs:63, 394, 395, 408 | CONTROL_BORDER (pen), PANEL_BG (circle fill), TEXT_PRIMARY (number) |
| label_popup.rs:102, 285, 601, 602, 623, 635, 637, 642, 643 | PANEL_BG, CONTROL_BORDER, RECESSED, TEXT_PRIMARY, ACCENT, APP_BG, CONTROL_FILL, TEXT_NORMAL, TEXT_MUTED |

Native (GDI/Win32) side, outside the egui theme:

| File:Line | Value |
| --- | --- |
| countdown.rs:124 | KEY_COLOR = COLORREF(0x00FF_00FF) - the see-through colorkey, not a theme color |
| countdown.rs:310, 339 | Segoe UI bold, height = circle size * 11 / 20 |
| sync_flash.rs:65 | WHITE = COLORREF(0x00FF_FFFF) |
| tray.rs:24, 26 | IDLE_RGB [150,150,150], RECORDING_RGB [220,50,50] |
| label_popup.rs:211, 654 | corner radius s(6) fed to RoundRect |
| label_popup.rs:496, 498 | Segoe UI regular, 11 pt |

Literal sizes that are layout, not style (not counted as style debt): the width consts in
tracks_ui.rs (SLIDER_WIDTH 130, NAME_WIDTH 130, DEVICE_WIDTH 280, METER_WIDTH 180),
webcam_ui.rs (PREVIEW_MAX_WIDTH 360, SLIDER_WIDTH 220), key_ui.rs (SLIDER_WIDTH 220),
profile_ui.rs (240/320/180 TextEdit widths), preview_ui.rs (MIN_AREA_HEIGHT 180),
countdown.rs (CIRCLE_PX 260, OVERLAY_ALPHA 215), label_popup.rs layout numbers.

## 5. ui_kit items the Recorder uses

| Item | Where |
| --- | --- |
| ui_kit::apply(ctx) | app.rs:118 - installs the suite theme at startup; nothing re-applies it later |
| ui_kit::level_meter(ui, peak_dbfs, clipped, width) | tracks_ui.rs:240, one per audio track row |
| ui_kit::palette::ACCENT | app.rs:313, marks_ui.rs:5 (Chapter tag), label_popup.rs:635 (selected pill) |
| ui_kit::palette::METER_GREEN, METER_YELLOW, RECORD_RED, TEXT_MUTED, TEXT_NORMAL, TEXT_PRIMARY | marks_ui.rs:5 (mark category tags) |
| ui_kit::palette::RECORD_RED | tracks_ui.rs:9 (missing device text) |
| ui_kit::palette::TEXT_MUTED | profile_ui.rs:289, 305, 454, 501 |
| ui_kit::palette::PANEL_BG, CONTROL_BORDER, TEXT_PRIMARY | countdown.rs:394-408, label_popup.rs:601, 623 |
| ui_kit::palette::RECESSED | label_popup.rs:285, 602, 642 |
| ui_kit::palette::APP_BG, CONTROL_FILL, TEXT_NORMAL | label_popup.rs:635, 637 |
| ui_kit::palette::TEXT_MUTED (popup hint) | label_popup.rs:643 |

No ui_kit widget besides level_meter is used: no panel, primary_button, icon_button, segmented,
chip, switch, dialog, toast, transport, track_header, vertical meters, fader, shortcuts, tour,
help or about. Fonts, spacing and widget visuals are otherwise stock egui.

## 6. Hotkeys

Global hotkeys, registered through cutmark-hotkeys (owner "recorder", defaults in hotkeys.rs:18-25,
saved in the suite hotkey file; the spec table matches):

| Action | Default | Becomes |
| --- | --- | --- |
| start_stop | Ctrl+Alt+R | CommandKind::StartStop |
| pause_resume | Ctrl+Alt+P | CommandKind::PauseResume |
| mark | F9 | CommandKind::Mark(Silent) |
| mark_label | Shift+F9 | CommandKind::Mark(WithLabel), opens the label popup |
| highlight_last | Ctrl+F9 | CommandKind::Mark(HighlightLast) - the last 30 seconds |
| mute_mic | Ctrl+Alt+M | CommandKind::Mute { track: 0 } (track 0 assumed to be the mic, hotkeys.rs:15) |

Other keys:

| Key | Where | Notes |
| --- | --- | --- |
| Esc | countdown.rs (RegisterHotKey while the overlay is armed) | Global while a countdown runs; cancels it |
| Enter | label_popup.rs::edit_proc | Saves the labeled mark |
| Esc | label_popup.rs::edit_proc | Saves the mark with no label |
| Tab / Shift+Tab | label_popup.rs::edit_proc | Cycles the category pill forward / backward |
| 1..=6 | label_popup.rs::edit_proc | Picks a category while the label text is still empty |

egui-level shortcuts in the main window: none.

Settings UI for hotkeys: none today. The window shows a read-only status table only
(app.rs::hotkey_grid: chord, action, and OK / "AltGr types 'x'" / "Taken by another program");
there is no way to view, add or change a binding. The bindings do live in the shared suite
hotkey file that other apps may edit, and the Recorder reads it only at startup.

## 7. Test count

`cargo test -p cutmark-recorder -q` on Nov 22, 2026: **234 passed; 0 failed; 0 ignored**
(the FFmpeg "[av1_amf] / [mjpeg]" lines in the output are stderr chatter from passing tests).

## Findings for the mapping table (R1 notes)

Things on screen today that the Phase 13 mapping table does not name a row for:

- The warning and recovery banners (capture-exclusion failure, recording error, audio notice,
  engine banners, hotkey problems, preview and webcam problems, API problem, recovered-marks
  banner with Details/Dismiss).
- "Recent commands" (the last commands the engine received).
- The "Last take" summary panel.
- The "Preview" on/off checkbox and "Hide Recorder from screen captures" checkbox.
- The webcam "Enable" switch (the mapping starts at camera controls).
- "Refresh devices" next to "Add track", and the WAV sidecars / audio format choice.
- The profile rename flow (inline TextEdit with OK/Cancel) and the two-click "Confirm delete".
- The recording panel's "Captured" and "Folder" rows, and the status bar's finalizing progress
  and camera info line.
- The "Starting in N" countdown heading in the main column (the deck caption in the spec covers
  the number, not this line).

