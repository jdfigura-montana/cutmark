# Recorder Phase 14: Studio (spec)

Nov 27, 2026 · @JD · draft 1

Exported from the Claude Project "Creator Suite" ("Recorder Phase 14 Studio Spec"). The Project copy is the source. Builds on Phase 11 (input track, `events` and `cursor-render` crates).

## Overview

Studio is a fourth Recorder mode, next to Setup, Record and Review. It turns a Track take into a finished tutorial video: cursor styling, auto-zoom with an editable zoom lane, a shortcut overlay and frame styling. Everything previews live and exports as a normal video any editor can open (Final Cut, Premiere, DaVinci, or the CutMark editor). The clean take is never changed.

**Goals**

1. Track is useful with any editor, not only ours.
2. A good result with no effort (auto-zoom and sensible defaults), and full control when wanted (zoom lane).
3. Non-destructive: settings live in a small file next to the take; export any number of times.
4. Preview matches the export.

**Non-goals (v1)**

- Cutting or trimming the take (that is the editor's job; Studio exports the whole take).
- Placing the camera ISO in Studio (later).
- Exporting zooms as editor keyframes (planned with S2X, Final Cut first).
- Vertical (9:16) output (later; the frame model leaves room).

## Mode

- Mode switch: Setup | Record | Review | **Studio**. Disabled while recording, like Review.
- Review: Track takes get **Open in Studio** in Actions. This replaces Phase 11's Bake with cursor (Studio export covers it).

## Layout

- **Left:** takes with an events file (same list style as Review, Takes | Replays switch).
- **Center:** the live preview (output frame with all effects), the deck (Play, previous and next zoom, timecode, total), and under it the **timeline**:
  - Scrub bar with mark dots (as Review).
  - **Zoom lane:** one block per zoom segment.
  - **Activity lane:** thin ticks for clicks and shortcut chords, so the user sees why a zoom is where it is.
- **Right:** tool strip **Cursor | Zoom | Keys | Frame | Export**, each an inspector panel.

## Studio settings file

`{stem}.studio.json`, part of the take's file group (Rename, Delete). Versioned, readers ignore unknown fields. Holds every setting below plus the zoom segments. Missing file = defaults (and auto-zoom generated on first open).

## Cursor (panel)

| Setting | Values | Default |
| --- | --- | --- |
| Size | 100% to 300% | 150% |
| Smoothing | Off, Light, Strong | Light |
| Click effect | None, Ring, Ripple; color; size | Ring, white |
| Highlight | Off, Spotlight (soft circle), with radius | Off |
| Hide when idle | Off, or after 1 to 10 s | After 3 s |
| Cursor visible | On, Off | On |

## Zoom (panel and lane)

**Auto-zoom** (runs on first open and on "Regenerate"):

1. Activity points: clicks, shortcut chords, and cursor dwell (cursor still for 0.6 s or more after moving).
2. Group points less than 3 s apart into a cluster.
3. Each cluster becomes a segment from 0.5 s before its first point to 1.5 s after its last, merged with a neighbour closer than 2 s (so the view never zooms out and straight back in).
4. Segment zoom level: the panel's Amount (default 2.0x). Segments shorter than 1 s are dropped.
5. While zoomed, the view follows the cursor with a dead zone (the middle 40% of the zoomed view) and smoothing, and never shows outside the captured frame.

**Lane editing:** click a segment to select it; drag its body to move, its edges to trim; per-segment zoom level (1.25x to 4x) and "Follow cursor" on/off, or a fixed center set by dragging a box in the preview; Delete removes it; drag on an empty part of the lane adds one; Undo and Redo (Ctrl+Z, Ctrl+Y) for lane edits.

**Panel:** Auto-zoom on/off, Amount, Speed (ease in and out: 0.3 to 1.2 s, default 0.6 s), Regenerate (asks before replacing edited segments).

## Keys (panel)

Shortcut overlay from the recorded chords (only if keyboard shortcuts were recorded; otherwise the panel says so).

| Setting | Values | Default |
| --- | --- | --- |
| Show shortcuts | On, Off | On |
| Position | Bottom center, Bottom left, Bottom right, Top center | Bottom center |
| Size | Small, Medium, Large | Medium |
| Duration | 0.8 to 3 s | 1.5 s |

Drawn as key caps in the design language's style (rounded, dark glass), one chord at a time, newest replaces older.

## Frame (panel)

| Setting | Values | Default |
| --- | --- | --- |
| Frame | Off, On | Off |
| Background | Solid color, Gradient (6 presets), Image | Gradient preset 1 |
| Padding | 0% to 15% | 6% |
| Corner radius | 0 to 32 px (at 1080p, scaled) | 12 |
| Shadow | Off, Soft, Strong | Soft |

With Frame off, the output is the captured frame (zoom still applies).

## Export (panel)

- Output size: Source, 3840x2160, 2560x1440, 1920x1080 (keeps aspect; with Frame on, the frame fills the size).
- Frame rate: source.
- Codec and quality: the take's profile codec and rate control, with "Quality" one step higher by default (zoomed footage benefits).
- Audio: copied from the take.
- Writes `{stem}.studio.mp4` (part of the file group), with progress and Cancel; cancelled by starting a recording, like the bake. A toast with Show in folder when done.
- **Presets:** "Save as preset" stores the Cursor, Zoom (not segments), Keys, Frame and Export settings by name; "Default preset" is applied to takes opened in Studio for the first time.

## Rendering

A new `studio-render` crate, GPU (wgpu), used by both preview and export so they match:

1. Input: decoded source frame (texture), the time, the events reader, the studio settings.
2. Pass order: zoom (crop and scale of the source with easing), cursor (via `cursor-render`, moved to a wgpu path, sized relative to the zoom), click effects, highlight, frame (background, padding, rounded corners, shadow), shortcut overlay.
3. Preview renders at the preview size; export renders at the output size and reads back for the encoder (or hands a texture to the encoder when media-io supports it).
4. Zoom and cursor smoothing are computed from the events data ahead of time per take (a small cached path), so seeking is instant.

## Steps (one prompt each, sizes to be set in the prompt doc)

| Step | Work |
| --- | --- |
| T1 | Studio settings model, `{stem}.studio.json`, auto-zoom generator and follow-cursor path (pure logic, tests) |
| T2 | `studio-render` crate: zoom, frame, shortcut overlay; `cursor-render` wgpu path |
| T3 | Studio mode shell: mode switch, takes list, preview with render, deck, Open in Studio in Review |
| T4 | Panels: Cursor, Keys, Frame (live preview) |
| T5 | Timeline: scrub, zoom lane editing with undo, activity lane, Zoom panel |
| T6 | Export panel, export pipeline, presets |

## Acceptance

1. Opening a Track take in Studio shows auto-zoom segments that sit on the clicks, and a preview that already looks finished.
2. Lane edits (move, trim, level, delete, add, undo) show in the preview at once.
3. Export at 1080p plays in Final Cut, Premiere, DaVinci and VLC, and matches the preview.
4. Re-opening the take restores every setting and segment; the clean take is unchanged.
5. A 10 minute 1440p60 take exports in no more than about 1.5x its length on the RX 7700 XT.
