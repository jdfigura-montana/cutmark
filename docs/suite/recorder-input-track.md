# Recorder Phase 11: Input track (spec)

Nov 27, 2026 · @JD · approved. Merges Phase 11 (events sidecar) and the Cursor Track spec. Where they differ, this file wins.

Exported from the Claude Project "Creator Suite" ("Recorder Phase 11 Input Track Spec"). The Project copy is the source.

## Overview

The Recorder can record the screen without the cursor and write the cursor, clicks, keyboard shortcuts and app switches to their own file next to the take. Review draws the cursor back during playback, and Bake writes a copy with the cursor burned in for Final Cut or any other editor. The video editor (S5) later adds cursor effects and auto-zoom from the same file.

**Goals**

1. Cursor lines up with the video within one frame, on the same QPC clock as capture.
2. Under 1% of one CPU core, no added GPU work while recording, a few MB per hour on disk.
3. Crash safe: a crashed take keeps its input data up to the last second.
4. One shared cursor renderer for Review, Bake and later the video editor.
5. Privacy: nothing typed is ever recorded; keyboard and app data are opt-in or minimal.

**Non-goals (v1)**

- Window titles (never recorded in v1).
- Typed text of any kind.
- Touch and pen.
- Cursor effects beyond the v1 style, and auto-zoom (S5).
- Input tracks for OBS recordings (Stream companion, later, same file format).

## Settings (Setup, Screen group; per profile)

| Setting | Values | Default |
| --- | --- | --- |
| Cursor | Show (burned in, today), Hide, Track | Show |
| Record clicks | on, off (only with Track) | on |
| Record keyboard shortcuts | on, off (only with Track) | off (opt-in) |
| Record app switches | on, off (only with Track) | on |

- Track is labelled "Track (draw it later)", with a caption that Review and Bake draw the cursor back.
- Keyboard shortcuts means chords with Ctrl, Alt or Win held (plus F1 to F24 alone). Plain letters, digits, Shift+letter and anything typed into a field are never recorded.
- App switches record the app's exe name and display name only.
- Tutorial capture preset: native resolution, 60 fps, quality one step higher than the profile default, Cursor Track, Record clicks on. Falls back to 30 fps when the encoder cannot do native at 60, and says so. Setup shows the expected size per hour.
- Track does not become the default for new profiles until the video editor draws cursors (S5); revisit then.

## Recording

1. **Clean capture:** with Track, Windows Graphics Capture runs with `IsCursorCaptureEnabled = false`. Nothing else in capture or encode changes.
2. **Cursor sampling:** a dedicated high-priority thread samples `GetCursorInfo` every 4 ms with a QPC stamp; writes a sample only on change (position, shape, visible) plus a keep-alive every 250 ms. Positions in physical virtual-desktop pixels (Per-Monitor DPI Aware v2).
3. **Clicks and wheel:** Raw Input (`RIDEV_INPUTSINK`) on a message-only window on that thread; left, right, middle, X1, X2 down and up, vertical and horizontal wheel with delta; each with QPC time and the latest position. No low-level mouse hook.
4. **Shapes:** cached by handle, read with `GetIconInfo` and drawn with `DrawIconEx` to 32-bit BGRA; monochrome and inverting cursors converted to dark with a light outline; deduplicated by pixel hash; animated cursors store frames when `GetCursorFrameInfo` is available; cap 256 by 256; cursor size setting and per-monitor DPI recorded at take start.
5. **Keyboard shortcuts (opt-in):** Raw Input keyboard on the same window. A chord is logged on key down when Ctrl, Alt or Win is held, or for F1 to F24 alone: modifiers plus the virtual key name. Nothing else from the keyboard is kept, not even in memory.
6. **App switches:** `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` out of context; log exe name and display name of the new foreground app, deduplicated (same app twice in a row is one event).
7. **Coordinates:** the file stores the capture source (monitor, region or window). Window capture also logs the window rectangle whenever it moves or resizes. Display changes (resolution, DPI, monitor removed) are logged as events with the new layout.
8. **Replay buffer:** while the buffer is on and Cursor is Track, the input logger keeps a ring of the same length; a saved replay gets its own `{stem}.events`.
9. **Failure:** if the logger cannot start, the take starts with the cursor shown and a toast says so. If it dies mid-take, recording continues, a toast warns, and Review marks the take "Input track incomplete".

## The events file

One binary file per take, `{stem}.events`, part of the take's file group (Review Rename and Delete, replay saves). Only the new `events` crate reads or writes it.

Header, then chunks; each chunk has a 4-byte type, a length and a CRC32, so readers stop cleanly at the first bad or partial chunk. Unknown types are skipped.

| Chunk | Contents |
| --- | --- |
| `HEAD` (first) | Magic `CMEVT`, schema_version 1, QPC frequency, QPC of the take's first video frame, capture source, monitor layout, cursor size, per-monitor DPI, which streams are on (cursor, clicks, keys, apps) |
| `SHAP` | One cursor shape: id, hotspot, size, frame count and times, BGRA pixels (LZ4) |
| `SAMP` | Cursor samples, delta-encoded, LZ4 |
| `MOUS` | Clicks and wheel |
| `KEYS` | Shortcut chords (only when opted in) |
| `APPS` | Foreground app changes |
| `GEOM` | Window rectangle and display layout changes |
| `TAIL` (last) | Counts and end QPC; missing means the take crashed, everything before it is still used |

Rules: times are QPC ticks in the file, converted by the crate to microseconds from the first video frame; the writer flushes every second; the file is never edited after the take ends.

Size: about 2 to 5 MB per hour with cursor motion; keys and apps add almost nothing.

## Cursor renderer (`cursor-render` crate)

Draws the cursor onto a frame from the events data and a style. Used by Review playback and Bake now, by the video editor later. v1 style: size 100% to 300%, light smoothing on or off, click ring on or off. The default style is real size, no smoothing, click ring off (the cursor exactly as recorded).

## Review

1. Takes with an events file show a cursor icon in their row.
2. Playback draws the cursor with the default style; "Show cursor" switch in the File group.
3. **Bake with cursor** in Actions: writes `{stem}.baked.mp4` with the cursor burned in, using the current style, through the same encoder; progress, Cancel; disabled while recording. Part of the take's file group.
4. Keys and app switches are not shown in Review in v1 (they are for the editor); the File group shows their counts.

## Steps (one prompt each)

| Step | Work |
| --- | --- |
| I1 | `capture`: cursor sampling thread, Raw Input mouse, shapes and cache, window and display events |
| I2 | `capture`: keyboard chords (opt-in) and foreground app events |
| I3 | New `events` crate: writer and reader, chunk format, crash recovery |
| I4 | Recorder: settings, Tutorial preset, logger with takes and the replay ring, file group, failure toasts |
| I5 | New `cursor-render` crate |
| I6 | Review: cursor icon, playback overlay, Show cursor, Bake with cursor |

## Acceptance

1. With Track the video has no cursor; with Show it does.
2. Cursor lines up within one frame at 60 fps at the start and end of a 60 minute take.
3. Clicks land on the right frame (click a stopwatch app's button).
4. I-beam, hand, resize and busy cursors draw with the right shape and hotspot; mixed 100% and 150% monitors line up.
5. Region and window capture map correctly, including a moving window; a resolution change mid-take still lines up.
6. Killing the Recorder mid-take leaves an events file readable up to the last second.
7. A replay save writes a matching events file; Rename and Delete include it.
8. With keyboard shortcuts off, the file has no KEYS chunk; with it on, typing a sentence logs nothing and Ctrl+S logs one chord.
9. Bake with cursor plays in Final Cut Pro on the Mac with the cursor visible.
10. Logger CPU under 1% of one core on the i3 laptop.
