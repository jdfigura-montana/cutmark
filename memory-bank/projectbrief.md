# Project Brief

## Source of truth
The project brief is **`docs/SPEC.md`** — read the sections named in each task before writing code. Never edit it; if something seems wrong or unclear, stop and ask. Also follow `clinerules` (per-task rules).

## Summary (see SPEC for details)
A personal, Windows-only, multitrack audio editor named **CutMark Audio Editor** in Rust that works like a video editor: import media into a project bin, place clips on tracks, edit non-destructively, and export one mixed WAV or MP3. Single executable, egui/eframe UI, pure testable core, full undo.

## SPEC section index (what each covers — details live in the SPEC)
- **Overview** — guiding principles: simplicity, non-destructive, undo-first, sample-accurate, testable core.
- **Platform and stack** — Windows 10/11 x64, Rust stable edition 2021, crate table.
- **Projects and project management** — project folders, project.json, start screen, Project menu, autosave/recovery.
- **Media bin** — bin item display, bin actions, missing media relink, cache.
- **Timeline model** — Project/MediaItem/Track/Clip/Marker structs and unit-tested invariants.
- **Waveform display and timeline UI** — window layout, ruler, peaks, zoom/scroll, clip anatomy, selection, snapping.
- **Transport and scrubbing** — play/stop/pause, play selection, playback-time edits, audible scrubbing.
- **Core editing** — split/trim/move/paste/delete/ripple and undo semantics.
- **Levels, processing, markers, export, keyboard shortcuts** — mixer controls, gain/normalize/fades, markers, two-pass WAV/MP3 export, shortcut table.
- **Code architecture** — five sealed layers, the src/ source tree (exact file list), layer rules, thread model.
- **Build phases** — 10 phases, each with a manual check; one phase at a time.
- **Out of scope and defaults to review** — v1 non-goals and chosen defaults.

## Build phases (progress tracked in progress.md)
1. Skeleton → 2. Model/undo core → 3. Media import → 4. Timeline view → 5. Playback → 6. Editing → 7. Levels/processing → 8. Markers/scrubbing → 9. Export → 10. Safety net/polish.