# Recorder Phase 13: UI restyle (spec)

Nov 22, 2026 · @JD · approved

Exported from the Claude Project "Creator Suite" ("Recorder Phase 13 UI Restyle Spec"). The Project copy is the source; this file is replaced when the spec changes.

## Overview

The Recorder's main window is one long scrolling column of every setting. Phase 13 moves it onto the `ui-kit` look and layout that S1 built: window chrome, a real menu bar, three modes (Setup, Record, Review), a preview with a floating deck, a tool strip for setup panels, and a Settings dialog for things set once.

**Visual reference:** the Cutmark Look canvas, boards "E2 · Recorder, Setup", "E2 · Recorder, Record" and "E2 · Recorder, Review". They are approved as a starting point. JD judges the result on the running app, and changes after use go into a later revision.

**Rules that apply:** docs/suite/Design-Language.md plus docs/suite/Design-Language-rev3.md (title bar, panel toolbar, panel header). Where they conflict, rev 3 wins.

**Goals**

1. Every control the Recorder has today still exists and works the same way. Nothing is removed.
2. No window scrolls as a whole. Only panel contents may scroll, with the `ui-kit` overlay scrollbar.
3. The Recorder joins the style lint: no hard-coded colors, radii or font sizes.

**Non-goals**

- No capture, encode or audio behavior changes.
- No buttons for features that do not exist yet. "Open in CutMark Editor" and "Send to Final Cut Pro" from the Review board stay hidden until their milestones ship.
- No workspace templates in the Recorder (panel widths are remembered, that is all).
- No Help topics. Recorder Help is its own phase and uses the S1 Phase 14 engine.
- No new `ui-kit` widgets unless a step needs one. If one is needed, it goes into `ui-kit` with tokens, never into the Recorder.

## Decisions

1. Review playback (R9) is built in Phase 13. If it runs long it ships as Phase 13b, and Review shows the take's first frame until then.
2. Delete take uses the `trash` crate (MIT), as one new dependency in the Recorder only.

## Window layout

```
Title bar            44   (rev 3)
Content              fills, 8px padding, 8px gaps
Status bar           26
```

**Title bar**

- Left: app mark, then the menu bar: File, Capture, View, Help.
- Center: project name in `type.body` 600, then a caption in `type.caption` `text.tertiary`. Setup: profile, size and frame rate. Record: "Recording takeNN · profile". Review: takes today and their total length.
- Right: the mode switch (segmented control: Setup, Record, Review), a 1px separator, then the caption buttons.
- Clicking the project name opens a small popover to rename the project or pick a recent one (same behavior as today's Project field).

**Menus**

| Menu | Items |
| --- | --- |
| File | New project, Open project folder, Settings (Ctrl+,), Exit |
| Capture | Start or Stop recording, Pause or Resume, Add mark, Add mark with label, Mute mic, Run pre-flight, Camera sync test |
| View | Setup, Record, Review (Ctrl+1, 2, 3), Matte view, Guides, Appearance |
| Help | Keyboard shortcuts, About (Help topics come with the Help phase) |

Menu items show the existing global hotkeys as their shortcuts. No hotkey changes.

## Modes

| Mode | Left panel | Center | Right panel |
| --- | --- | --- | --- |
| Setup | Sources, then the pre-flight card | Preview, deck | Tool strip and the open tool's panel |
| Record | none | Preview, large, red rim, REC pill, deck | Live: levels, encoder stats, marks, hotkey hints |
| Review | Takes for the current project | Take playback, scrub bar with mark dots, deck | Take info, marks, actions |

**Mode rules**

1. Starting a recording (button, hotkey, API or Prompter) switches to Record. While recording, Setup and Review are disabled in the mode switch.
2. Stopping returns to the mode that was open before recording, and shows a toast "takeNN saved" with a Review button.
3. The open mode and the open tool are remembered between launches.
4. Profiles switch only while idle, as today. The profile picker is disabled while recording.

## Setup mode

**Left panel: Sources** (header 44, "Sources", Add button)

- Video group: display and camera rows (icon, name, detail caption, status dot). Clicking a row opens its existing source picker as a popover.
- Audio group: one row per track with a compact `level_meter` and the peak value.
- Pre-flight card at the bottom (inspector group): result (Pass in `sem.meter.ok`, Fail in `sem.record`), one summary line, the Do Not Disturb line when it is off, "Run pre-flight (5 s)" button, and a Details link that opens a popover with the full result table, the Keep files switch and "Open notification settings".

**Center: Preview panel**

- Panel toolbar: "Profile" label, profile picker (dropdown), separator, chips Matte view and Guides, then on the right "Next take takeNN" (tooltip: the full next-take path).
- Preview: the existing live preview, letterboxed in a 16:9 box on `bg.ground`.
- Deck (glass, floating): timecode, Mark, Record (capsule in `sem.record` with white dot and "Record" label), Pause (disabled while idle), countdown caption.

**Right panel: tool strip** (Camera, Key, Layout, Audio, Output). Each tool's panel is inspector groups with rows: label, control, value.

| Tool | Groups |
| --- | --- |
| Camera | Image (exposure, white balance with Auto, gain, brightness, Lock current values, Reset to defaults), Sync (camera offset, Sync test with its hint) |
| Key | Chroma key (enable, key color with eyedropper, matte view, similarity, smoothness, spill), Garbage crop (left, right, top, bottom), Mask rectangle (enable, X, Y, width, height) |
| Layout | View (Screen only, Picture in picture, Cam only), Camera position (corner, X offset, Y offset, scale), Crop (left, right, top, bottom), Frame (shape, border width, border color), Background (None, Colour, Image, Video, with its caption) |
| Audio | The existing audio tracks section: each track's source, gain, mute, plus its meter |
| Output | Profile actions (New, Duplicate, Rename, Delete as a row of buttons), Encode (codec, rate control, quality, max bitrate with its AMD note, preset, keyframe interval), Format (output size, frame rate, container), Camera ISO file (switch, rate control, target and max bitrate, codec note) |

Groups that depend on a switch (key off, mask off, ISO off) show their rows disabled, not hidden.

## Record mode

- Center panel gets a 1px `sem.record` rim at 35%. REC pill top left (`sem.record` tint, dot, "REC").
- Deck: running timecode, Mark, Stop (capsule in `sem.record` with white square and "Stop"), Pause, Mute mic.
- Right panel "Live" (header shows takeNN in `sem.record`):
  - Levels: one row per track, `level_meter` and peak value.
  - Encoder: bitrate, file size, dropped and duplicated, disk time left.
  - Marks: live list rows (category dot, timecode, label), newest selected, auto-scrolls.
  - Footer: key caps for F9 and Shift+F9 with their actions.
- The existing warnings (low disk, dropped frames) show as toasts and keep their sounds and auto-stop.
- Tray behavior, the Shift+F9 popup and the countdown overlay are unchanged.

## Review mode

- Left panel "Takes": takes in the current project, grouped by day, newest first. Row: take name, time and mark count, duration. Header button opens the project folder.
- Center: playback of the selected take.
  - Video and audio, decoded through `media-io`. Play, pause, seek by clicking or dragging the scrub bar, previous and next mark, Add mark (writes to the take's sidecar through `marks`).
  - Scrub bar under the preview: progress in `accent`, playhead, mark dots in their category colors.
  - Deck: timecode, previous mark, Play (accent capsule), next mark, Add mark, total length.
- Right panel (header: take name and date):
  - File: duration, size, video format, camera ISO format, dropped frames.
  - Marks: list rows; clicking one seeks to it.
  - Actions: Show in folder, Rename (renames every file of the take together), Delete take (moves every file of the take to the Recycle Bin after a confirm dialog that lists the files).
- Hidden until their milestones: Open in CutMark Editor, Send to Final Cut Pro.

## Settings dialog

`ui-kit` dialog frame, sections in a left list:

| Section | Holds (moved from the main window) |
| --- | --- |
| General | Hide Recorder from screen captures |
| Recording | Root folder, naming template with the tokens hint, countdown (Off, 3, 5) and "Hide my countdown when a prompter is linked" |
| Hotkeys | The existing read-only hotkey status table (chord, action, status). No editing in Phase 13 |
| Diagnostics | Recent commands |
| Control API | On switch, port, listening address, connected clients, token with Show, Copy, Regenerate |
| Appearance | `ui-kit` accent picker and glass switch |

## Status bar

Left to right: state dot and label (Ready, Recording, Paused, Buffering later), disk free and time left at the current profile, codec warnings (for example the camera ISO HEVC note, in `sem.meter.warn`), right side: Control API clients, "Prompter linked" when linked.

## Panels

- Left and right panels are resizable by their dividers. Left 220 to 360 (default 270), right 300 to 440 (default 340). Widths are remembered per mode.
- The preview keeps its 16:9 box and scales into the space left.

## Mapping: today to after

| Today (main column) | After |
| --- | --- |
| Camera controls | Camera tool, Image group |
| Camera ISO file | Output tool, Camera ISO file group |
| Camera offset, Camera sync | Camera tool, Sync group |
| Chroma key, Garbage crop, Mask rectangle | Key tool. Matte view also as a preview chip (same state) |
| Layout | Layout tool |
| Background | Layout tool, Background group |
| Marks list | Record right panel, Review right panel |
| State line ("Idle") | Status bar |
| Profile block | Profile picker in the preview toolbar; fields and actions in the Output tool |
| Project field | Title bar project name popover |
| Naming template, next take path | Settings > Recording; next take in the preview toolbar |
| Countdown | Settings > Recording; caption on the deck |
| Pre-flight | Setup left panel card and its Details popover |
| Do Not Disturb row | Pre-flight card and Details popover |
| Control API | Settings > Control API; clients in the status bar |
| Audio tracks | Audio tool; meters in the Sources panel and the Record panel |
| Bottom bar (disk free, codec note) | Status bar |
| Webcam Enable, camera picker with Refresh, format picker | Camera tool, new Device group at the top. The Sources camera row opens the same picker |
| Camera picture (eyedropper target) | Top of the Camera tool and the Key tool, same widget. The eyedropper works on it in either |
| Focus control, sync result with "Apply as camera offset" | Camera tool, Image group and Sync group |
| PiP corner radius (Rectangle only) | Layout tool, Frame group |
| Background colour, Browse and Clear, Fit | Layout tool, Background group |
| Track rows (on, name, device, gain, mute, offset, in mixdown, remove), Add track, Refresh devices | Audio tool, one inspector group per track, then Add track and Refresh devices |
| WAV sidecars, audio format | Audio tool, Output group |
| Profile rename flow and two-click Confirm delete | Output tool, same flows |
| Display source | Sources panel, info row only ("Primary monitor"); no picker exists today |
| Preview on/off | View > Preview and a chip in the preview toolbar |
| Hide Recorder from screen captures | Settings > General |
| Hotkeys status table | Settings > Hotkeys, read-only as today (editing hotkeys is not in Phase 13) |
| Recent commands | Settings > Diagnostics |
| Warning banners (capture exclusion, recording error, audio, engine, hotkey, preview, webcam, API) | Banner strip at the top of the center panel: one row each, `sem.meter.warn` or `sem.record` dot, dismiss button where today's banner can be dismissed |
| Recovered marks banner with Details and Dismiss | Same banner strip; Details opens a popover |
| Recording panel (time, file, folder, size, bitrate, frames, duplicated, dropped, captured) | Record mode Live panel, Encoder group |
| "Starting in N" heading | Deck caption ("Starting in 3") |
| Idle line and "Starting is blocked" | Status bar state label; the blocked reason as a banner |
| Last take summary | Review right panel, File group. Until R8 lands, the "takeNN saved" toast plus a Last take group at the bottom of the Record Live panel |
| Status bar finalizing progress and camera line | Status bar, as today |

## Steps (one prompt each)

| Step | Work | Done when |
| --- | --- | --- |
| R1 | Inventory, read-only: `docs/suite/recorder-ui-inventory.md` listing every UI file, every control and where it is drawn, every hard-coded style value, every `ui_kit` item used, every hotkey, and the source pickers | File exists, no code changed |
| R2 | Shell: window chrome, title bar with menus and mode switch, status bar, three-panel layout, mode state. Today's whole column is parked inside the Setup right panel (scrolling) so nothing is lost | App runs, every old control still reachable, tests pass |
| R3 | Tool strip, Camera and Key panels; their controls leave the parked column | Rows match the mapping |
| R4 | Layout and Output panels, profile picker in the preview toolbar | Rows match the mapping |
| R5 | Sources panel, pre-flight card and Details popover, Audio tool | Rows match the mapping |
| R6 | Settings dialog (General, Recording, Hotkeys, Control API, Diagnostics, Appearance), banner strip. Only the recording panel, countdown heading, idle line and last take stay parked | Parked column holds only those four items |
| R7 | Preview panel and deck, Record mode, mode rules 1 to 3, toasts; the last parked items move and the parked column is deleted | Recording from the deck, hotkey, API and Prompter all switch modes correctly |
| R8 | Review mode: takes list, take info, marks, Show in folder, Rename, Delete | Rename and delete handle every file of a take |
| R9 | Review playback: decode, play, pause, seek, mark jumps, Add mark | A 10 minute AV1 take plays in sync and seeks to any mark |
| R10 | Recorder joins the style lint, deprecated `ui_kit::palette` uses removed, panel resize and remembered widths, inventory updated | Lint passes, clippy clean, test count at least the R1 count |

Each prompt runs only the Recorder's tests and clippy. `scripts\check.ps1` runs at JD's close-out. Steps that touch `ui-kit` follow the one-owner rule.

## Acceptance

1. Every row of the mapping table is in place and works.
2. The three modes match the boards in layout and look, minus hidden future buttons.
3. No whole-window scrolling at 1440 x 900 and at 1920 x 1080, at 100% and 150% scaling.
4. Changing the accent in any CutMark app recolors the Recorder within a second.
5. Recording, marks, pre-flight, the Control API and the Prompter link behave exactly as before.
6. Lint passes; test count at least the R1 count; one CI run green at the end.
