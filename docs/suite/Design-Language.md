# CutMark Design Language v1: Glass Deck

Nov 19, 2026 · @JD · rev 2: Phosphor icons, Scrollbar component, ui-kit origin (from the S1 spec)

Exported from the Claude Project "Creator Suite". The Project copy is the source; this file is replaced when the spec changes.

## Overview

One look for every app in the CutMark Creator Suite: Audio editor, Video editor, Recorder, Prompter, and TuteTool's editor chrome. Direction "E · Glass Deck, blue", chosen from the design canvas "Cutmark Look" (boards: E · Glass Deck, blue and E · Audio editor, blue). Inspired by Apple's current design language: the content is the hero, the chrome is monochrome, controls float, and nested shapes share a center.

**Goals**

- A user opening any CutMark app knows it is a CutMark app within one second.
- Calm, monochrome chrome. Color appears only where it carries meaning.
- One source of truth for every value (`tokens.json`), so a look change costs prompts, not rebuilds.
- Buildable in egui on wgpu with no custom shaders in v1.

**Non-goals (v1)**

- Light theme. Dark only.
- Real backdrop blur. Glass is painted (see Glass recipe).
- TuteTool course themes. Those stay in "TuteTool Spec Slide Design". This spec covers TuteTool's editor chrome only.

## Decisions

| Topic | Decision |
| --- | --- |
| Accent | User-defined with a color wheel, presets as quick swatches, derived shades computed automatically, readability enforced |
| Title bar | Custom-drawn, merged into the 52px main toolbar, on Windows and macOS |
| Theme | Dark only |
| Icons | Phosphor (MIT), through `egui-phosphor` |

## Principles

1. **Content first.** Video frames, waveforms and clips carry the color and contrast. Chrome steps back.
2. **One accent.** The accent marks what is active right now: Play, the playhead, the selection, switches that are on, the primary action, and the focus ring. Nothing decorative is ever accent colored.
3. **Meaning colors only.** Red is record, green and yellow are meter levels, pink is pending cuts, and marker categories keep their colors. Nothing else gets a hue.
4. **Concentric geometry.** A shape nested in another uses `inner radius = outer radius - padding` (minimum 6px). This is the suite's geometric signature.
5. **Progressive disclosure.** Rare controls show on hover or selection, not all the time.
6. **Floating transport.** Playback controls live in a glass deck over the content, bottom center, in every app that plays media.

## Tokens

All values live in `tokens.json`. Names below are the token names.

### Color: surfaces

| Token | Value | Use |
| --- | --- | --- |
| `bg.ground` | `#0A0A0B` | Window background, gaps between panels, main toolbar |
| `bg.panel` | `#161618` | Panels |
| `bg.group` | `#1E1E21` | Inspector groups, search fields, list footers |
| `bg.control` | `#232326` | Segmented control track, chips, toggle buttons |
| `bg.hover` | `#2C2C2E` | Hover fill, selected tool in a strip |
| `bg.selected` | `#3A3A3D` | Selected segment, toggle that is on |
| `bg.row.selected` | `#2A2A2D` | Selected list row |
| `bg.track.selected` | `#1C1C1F` | Selected track row |
| `bg.track.alt` | `rgba(255,255,255,.015)` | Every other track row |
| `bg.clip` | `#222226` | Clip body |
| `stroke.hairline` | `rgba(255,255,255,.06)` | Panel rim, ruler and header dividers |
| `stroke.separator` | `rgba(255,255,255,.05)` | Rows inside a group |
| `stroke.clip` | `rgba(255,255,255,.08)` | Clip rim |

### Color: text

| Token | Value | Use |
| --- | --- | --- |
| `text.primary` | `#F5F5F7` | Titles, values, active labels |
| `text.secondary` | `#C7C7CC` | Body text, idle buttons |
| `text.tertiary` | `#8E8E93` | Captions, track numbers, ruler labels, idle waveforms |
| `text.disabled` | `#6E6E73` | Disabled only. Never for readable text (fails 4.5:1) |

### Color: accent

The accent is one user-chosen color. Every other accent token is derived from it in OKLCH, so any choice looks consistent.

| Token | Default (Blue) | Derivation from the chosen color |
| --- | --- | --- |
| `accent` | `#5B9DFF` | The chosen color after the readability clamp below |
| `accent.hover` | `#7DB1FF` | L + 0.06 |
| `accent.pressed` | `#4A86E0` | L - 0.07 |
| `accent.on` | `#0A0A0B` | Fixed: text and icons on the accent are always near-black |
| `accent.tint` | `#1F2A3D` | L 0.27, C x 0.25 (selected clip body) |
| `accent.content` | `#C7D9F5` | L 0.88, C x 0.35 (waveform inside a selected clip) |

**Readability clamp** (applied to the chosen color before anything else):

1. Lightness is clamped to L 0.68 to 0.86. This keeps `accent.on` text at 4.5:1 or better on the accent, and the accent at 3:1 or better against `bg.panel` (needed for the focus ring and playhead).
2. Chroma is capped at 0.19, so neon picks don't vibrate against the dark chrome.
3. Gray picks (chroma below 0.02) are allowed. The accent then reads as white-on-gray, which is a valid look.
4. When the clamp changes the user's pick, the picker shows "Adjusted for readability" with the before and after swatches.
5. A pick whose hue is within 20° of `sem.record` shows a note: "Close to the record color. Record buttons will still show their R and dot." It is allowed.

**Presets** (one click swatches in the picker): Blue `#5B9DFF` (default), Violet `#A58BFF`, Teal `#45D3C7`, Orange `#FF9F0A`.

### Color: meaning

These never follow the accent.

| Token | Value | Use |
| --- | --- | --- |
| `sem.record` | `#FF453A` | Record button dot, record arm, recording state |
| `sem.meter.ok` | `#30D158` | Meter below -12 dBFS |
| `sem.meter.warn` | `#FFD60A` | Meter -12 to -3 dBFS |
| `sem.meter.clip` | `#FF453A` | Meter above -3 dBFS and the clip indicator |
| `sem.success` | `#30D158` | Finished stage check, "Ready" states |
| `sem.pending` | `#FF6F8E` | Pending cut bands and counts |
| `marker.note` / `retake` / `cut` / `broll` / `system` | `#B8C2D0` / `#FFB347` / `#FF6F8E` / `#B79CFF` / `#8A93A0` | Marker categories (from Video Editor UI v1 Spec) |

### Typography

Fonts: **Inter** for UI, **JetBrains Mono** for numbers that change (timecode, dB, coordinates). Both are bundled (SIL OFL).

| Token | Font | Size / weight | Use |
| --- | --- | --- | --- |
| `type.title` | Inter | 20 / 600, -0.01em | Inspector title (selected clip or track name) |
| `type.headline` | Inter | 14 / 600 | Track names, document name |
| `type.body` | Inter | 13 / 400 or 500 | Everything else |
| `type.caption` | Inter | 12 / 400 | Subtitles, status bar |
| `type.micro` | Inter | 11 / 500 | Tool strip labels, clip labels. Nothing smaller than 11 |
| `type.time.large` | JetBrains Mono | 17 / 600 | Transport timecode |
| `type.value` | JetBrains Mono | 12 / 500 | Values, ruler labels |

Rules: sentence case everywhere. No uppercase labels with wide letter spacing. Frames and hundredths in timecode use `text.tertiary`.

### Spacing and sizes

- Spacing scale: 2, 4, 6, 8, 10, 12, 16, 24.
- Window padding and panel gap: 8.
- Panel padding: 8.

| Element | Size |
| --- | --- |
| Main toolbar (includes the title bar) | 52 tall |
| Window caption buttons | 46 x 52 each |
| Status bar | 26 tall |
| Panel toolbar (timeline) | 44 tall |
| Ruler | 26 tall |
| Toolbar button, icon button | 32 |
| Segment, panel button | 28 tall |
| Chip | 26 tall |
| Inspector row | 42 tall |
| List row | 38 tall |
| Transport deck | 56 tall (buttons 44) |
| Switch | 40 x 24 |
| Scrollbar thumb | 6 wide |
| Track, video default | 46 (range 32 to 140) |
| Track, audio default | 96 (range 48 to 200) |

### Radii

| Token | Value | Use |
| --- | --- | --- |
| `radius.window` | OS default (8 on Windows 11, 10 on macOS) | Window corners, drawn by the OS |
| `radius.panel` | 18 | Panels, monitor frame |
| `radius.group` | 10 | Inspector groups, search field (panel 18 minus padding 8) |
| `radius.control` | 12 | Segmented control track |
| `radius.segment` | 9 | Segment inside its track (12 minus 3) |
| `radius.clip` | 10 audio, 8 video | Clips |
| `radius.capsule` | height / 2 | Chips, primary buttons, transport deck and its buttons, scrollbar thumb |

The concentric rule overrides any table value when a component is nested somewhere new.

### Elevation and glass

Panels are flat: `bg.panel` plus a 1px inside rim of `stroke.hairline`. No drop shadows on panels.

**Glass recipe** (transport deck, menus, popovers, toasts):

- Fill `rgba(34,34,38,.82)`.
- 1px inside top highlight `rgba(255,255,255,.14)`.
- 1px inside rim `rgba(255,255,255,.07)`.
- Shadow 0, 10, blur 30, `rgba(0,0,0,.5)`.

Real blur can replace the fill later behind the `glass` switch without changing any app.

### Icons

Phosphor (MIT), through `egui-phosphor`, already in `ui-kit`. Line icons use the Regular weight; Play, Stop, Record and frame step use Fill. Icons take the text color of their context. One icon set for the whole suite.

### Motion

| Event | Duration | Curve |
| --- | --- | --- |
| Hover fill | 120ms | ease-out |
| Segment and tab slide | 180ms | ease-out |
| Panel open and close | 200ms | ease-out |
| Transport deck fade | 200ms | ease-in-out |
| Accent change (live preview) | 150ms | ease-out |

The OS "reduce motion" setting turns all of these into instant changes.

## Window chrome

The window has no native title bar. The 52px main toolbar is the title bar.

**Windows 11**

- Caption buttons (minimize, maximize or restore, close) on the far right, 46 x 52 each, drawn by `ui-kit` with Segoe Fluent-style glyphs. Hover `bg.hover`; close hover `#C42B1C` with white glyph (the Windows standard).
- Hovering maximize shows the Windows 11 Snap Layouts flyout. This needs the maximize button to answer the OS hit test as the maximize button (`HTMAXBUTTON` in `WM_NCHITTEST`).
- Window corners and shadow come from DWM (`DWMWA_WINDOW_CORNER_PREFERENCE` set to round). Mica is not used, because `bg.ground` is a fixed color.
- Resize borders: 6px invisible hit area on every edge, handled by the OS.

**macOS (later)**

- Transparent title bar with full-size content. The traffic-light buttons sit inside the toolbar's left zone, vertically centered, with 78px reserved before the app mark.

**Both**

- Empty toolbar space drags the window. Double-clicking it maximizes (Windows) or zooms (macOS).
- Interactive toolbar elements never drag.
- When the window is inactive, toolbar text drops to `text.tertiary` and the primary button keeps its color.
- Full screen hides the caption buttons and keeps the toolbar.

## Components

Every component lives in `ui-kit` and reads only tokens.

| Component | Spec |
| --- | --- |
| Panel | `bg.panel`, `radius.panel`, hairline rim, padding 8 |
| Main toolbar | Title bar plus toolbar. Left: app mark, document name with a caption line (save state and format), Undo, Redo. Center: the mode switch (a segmented control). Right: icon buttons, the primary button, then the caption buttons. Background `bg.ground` |
| Segmented control | `bg.control` track, padding 3, selected segment `bg.selected` with a 1px shadow. Used for app modes, tabs, tool groups |
| Primary button | Capsule, `accent` fill, `accent.on` text, 13 / 600. One per screen (Export, Record, Go Live) |
| Chip | Capsule, `bg.control`. On state: `text.primary` fill with `bg.ground` text. Used for filters and editing toggles (Snap, Ripple, Linked) |
| Icon button | 32px circle, transparent, `bg.hover` on hover. Needs a tooltip and an accessible label |
| Transport deck | Glass capsule, bottom center over the content area, 16px from the bottom. Order: timecode, transport buttons with Play in the middle as a 56 x 44 accent capsule, total length. App-specific buttons (In, Out, Stop, Record, Loop) sit around Play. The deck can be dragged sideways and fades to 40% while the pointer is near it during scrubbing |
| Inspector header | `type.title` name, `type.caption` subtitle (track, source file, state) |
| Inspector group | `bg.group`, `radius.group`, rows separated by `stroke.separator`. Row: label, control, value in `type.value` |
| Switch | 40 x 24 capsule. Off `bg.selected`, on `accent`. White knob |
| Slider | Thin track, accent fill up to the value, white knob. Zoom sliders use a `text.primary` fill, because zoom is not a selection |
| Meter | Horizontal in inspectors and track headers, vertical in master columns. 6px bars, 3px radius, ok, warn and clip zones, dB scale in `type.value` |
| Scrollbar | Overlay, no track fill. 6px capsule thumb in `text.tertiary` at 40%, 70% on hover and while dragging |
| Tool strip | Icon over a `type.micro` label, 52 tall. Selected: `bg.hover` fill and `text.primary` |
| List row | 38 tall, 9 radius, hover `bg.group`, selected `bg.row.selected`. Leading 8px dot for category color, trailing value |
| Search field | `bg.group`, `radius.group`, leading magnifier icon, placeholder in `text.tertiary` |
| Track header | Track number in `type.value`, name in `type.headline`, toggles as 24 x 24 rounded squares (`bg.control`, on `bg.selected`). Record arm on: `sem.record` fill, white R |
| Track row | Alternating `bg.track.alt`. Selected track `bg.track.selected` |
| Clip | `bg.clip`, clip radius, `stroke.clip` rim, label at top-left in `type.micro`. Waveform `text.tertiary`. Video clips show thumbnails. Selected: `accent.tint` body, 2px `accent` rim, `accent.content` waveform. Muted or hidden: 45% opacity |
| Playhead | 1.5px `accent` line, rounded 16 x 20 head on the ruler |
| Marker | 7px dot on the ruler in its category color. User markers: 1.5px ring in `text.tertiary` |
| Pending cut band | Diagonal hatch of `sem.pending` at 28% and 6%, 6px radius |
| Status bar | `type.caption` in `text.tertiary`, no background |
| Menus and popovers | Glass recipe, `radius.group`, 6px padding, 32px rows, hover `bg.hover` |
| Dialogs | `bg.panel`, radius 18, centered, 24px padding, primary button bottom right |
| Toasts | Glass capsule, top center, 4 second auto-dismiss, one at a time |
| Color picker | See Accent picker |

### States

| State | Rule |
| --- | --- |
| Hover | `bg.hover` fill, or `accent.hover` on accent controls |
| Pressed | `bg.selected`, or `accent.pressed` |
| Focus | 2px `accent` ring, 1px offset, on every interactive element |
| Disabled | 40% opacity, no hover |
| Selected | As defined per component. Never more than one accent-filled element per group |
| Inactive window | Toolbar text `text.tertiary` |

## Accent picker

Lives in Settings, Appearance, in every app. One setting for the whole suite.

**Layout (top to bottom, in one inspector group)**

1. Swatch row: Blue, Violet, Teal, Orange, then Custom. 28px circles; the selected one gets a 2px `text.primary` ring.
2. Hue and saturation wheel, 200px. Picking on it switches the swatch to Custom.
3. Lightness slider under the wheel.
4. Hex field (`#RRGGBB`), accepts paste.
5. Live preview strip: a Play capsule, a switch that is on, a selected clip with a waveform, and a playhead, all in the derived colors.
6. "Adjusted for readability" note with before and after swatches, shown only when the clamp changed the pick.
7. "Reset to default" text button.

**Rules**

- Every change previews live in the open window (150ms ease) and applies on release.
- The setting is stored in a suite-wide file (`%APPDATA%\Cutmark\appearance.json` on Windows, `~/Library/Application Support/Cutmark/appearance.json` on macOS) as the user's raw pick plus the clamped result.
- Running CutMark apps watch that file and update within a second, so changing the accent in one app changes all of them.
- Exported media and published TuteTool courses never use the accent. It is chrome only.

## Layout rules

1. Window: `bg.ground`, 8px padding, panels separated by 8px gaps.
2. Main toolbar on top (52, includes the title bar), status bar at the bottom (26), panels between.
3. The mode switch is always centered in the window, not in the space left over by the side zones.
4. The transport deck always floats over the main content area: the monitor in the Video editor, the timeline in the Audio editor, the preview in the Recorder, the script in the Prompter.
5. Track controls beyond V, A (video) or M, S, R (audio) show only on the selected track or on hover.
6. Every panel boundary stays user-resizable, and workspace templates work as in the Video Editor UI v1 Spec.

## Per-app notes

| App | Mode switch | Transport deck over | Notes |
| --- | --- | --- | --- |
| Audio editor | Edit, Mix | Timeline | Pilot app for the restyle |
| Video editor | Import, Edit, Mix, Deliver | Monitor | See "Changes to the Video Editor UI v1 Spec" |
| Recorder | Setup, Record, Review | Preview | Record replaces Play as the center button; it uses `sem.record`, not the accent |
| Prompter | Script, Prompt | Script view | Mirror mode keeps the deck readable, never mirrored |
| TuteTool | Its own modes | Stage preview | Tauri CSS reads the same tokens as CSS custom properties. Tauri window uses `decorations: false` with the same caption buttons drawn in HTML |

### Changes to the Video Editor UI v1 Spec

These replace the matching lines in that spec when the restyle is done:

- The 36px title bar and the 46px workflow rail merge into the 52px main toolbar. The stages become the centered segmented control, and the stage status line becomes a short caption in the active segment ("2 cuts pending").
- Uppercase 11.5px panel headers are replaced by sentence case.
- Track toggles: V and A always show; lock, hide, M and S show on the selected track or on hover.
- Transport moves from a row under the monitor into the glass deck over the monitor.
- The marker category letter badges become 8px dots in lists and 7px dots on the ruler.

## Implementation

### One source of truth

1. `design/tokens.json` in the `cutmark` workspace holds every fixed value in this spec and the accent presets.
2. A build step (`build.rs` in `ui-kit`) generates `ui-kit/src/tokens.rs` as Rust constants.
3. The same step writes `design/tokens.css` as CSS custom properties for TuteTool's chrome. Accent variables are set at runtime from `appearance.json`.
4. Apps never hard-code a color, size, radius or font. A lint test greps the app crates for hex colors and literal radii and fails the build if it finds any.

### ui-kit

- Created during Recorder phase 1 (theme) and phase 3 (level meter), extended to this spec in S1. This spec replaces the S0 crate map's "egui dark theme" line.
- Owns: font loading (Inter, JetBrains Mono), the egui `Style` and `Visuals` built from tokens, every component above as a widget, the icon set, the glass painter, the accent derivation (OKLCH math, clamp, presets), the `appearance.json` watcher, and the window chrome.
- One `Theme` struct with two runtime inputs: `accent` (any color, derived and clamped) and `glass` (painted glass on, or solid `bg.group` fallback).
- Glass and concentric radii are painted with egui's `Shape` and `Shadow`. No custom wgpu passes in v1.
- Window chrome: the app window is created without decorations. The Windows hit test (drag areas, maximize button for Snap Layouts, resize edges) and DWM corner setting live in `ui-kit` behind `cfg(windows)`. S0 dependency rule 5 gains `ui-kit` in its list of crates allowed to hold Windows-only code.

### Style gallery

`cutmark-gallery`, a small binary in the workspace, shows every component in every state on one scrolling page, with the full accent picker and the glass switch at the top, inside the real custom window chrome. It is the acceptance test for `ui-kit`: the look gets approved in the gallery before any app is restyled.

## Milestones

| Milestone | Delivers | Done when |
| --- | --- | --- |
| G1 Tokens | `tokens.json`, generator, `tokens.rs`, `tokens.css`, hard-coded value lint, accent derivation and clamp with unit tests | Generated files build, the lint runs in CI, clamp tests pass for presets and edge picks (black, white, gray, neon) |
| G2 ui-kit core | Fonts, Visuals, window chrome (Windows), panel, main toolbar, buttons, segmented control, chip, switch, slider, fields, scrollbar | Window drags, snaps and resizes correctly on Windows 11 |
| G3 ui-kit media | Transport deck, meter, track header, clip, playhead, marker, pending band, glass painter, menus, toasts | Widgets render in tests |
| G4 Gallery | `cutmark-gallery` with the accent picker and `appearance.json` watcher | JD approves it on screen |
| G5 Audio editor pilot | Audio editor restyled with `ui-kit` | JD has used it for one week |
| G6 Rollout | Recorder, Prompter, Video editor, TuteTool chrome | Each app passes the lint and a screenshot review |

G1 to G5 are built as S1 (see docs/suite/S1-Audio-Editor-ui-kit.md).

## Accessibility

- Readable text meets 4.5:1 on its background. `text.disabled` is never used for readable text.
- The accent clamp guarantees 4.5:1 for text on the accent and 3:1 for the accent against panels, whatever the user picks.
- Click targets are at least 24 x 24; transport buttons are 44.
- Every control is reachable by keyboard, with the focus ring visible.
- Color never carries meaning alone: record arm also shows R, markers also have names, pending cuts also show a count.
- UI scale setting from 100% to 200%, tested at 1080p and 4K.

## Open questions

1. **App mark.** The circle with a notch is a placeholder until a real logo exists.
