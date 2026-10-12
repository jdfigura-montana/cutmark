# Progress

## What works — Task 1.1 complete (2026-10-12)
- Cargo binary crate `audio_editor`, edition 2021, full SPEC module tree (~45 doc-comment stub files with mod.rs per folder).
- eframe 0.36.2 window "Audio Editor" at 1400x850 with labeled regions: menu bar (empty Project menu), toolbar, resizable Media Bin (left, default 220 px), fixed Master (right, 90 px), Timeline (center), Status (bottom).
- Validation green: cargo fmt, clippy -D warnings, cargo test, cargo build, smoke launch of the exe.

## What is left (SPEC "Build phases")
1. **Skeleton** — IN PROGRESS: start screen, New/Open/Save project, recent projects list remain.
2. Model and undo core (Editor, Action, history, ops; cargo test coverage of apply/undo/compare)
3. Media import (decode/resample/cache/peaks, bin panel)
4. Timeline view (ruler, headers, lanes, clips, waveforms, zoom, drag-drop)
5. Playback (cpal engine, MixPlan, mixer, transport)
6. Editing (selections, move/trim/split/cut/copy/paste, ripple, snapping)
7. Levels and processing (volume/pan/mute/solo, meters, gain/normalize/fades/silence)
8. Markers and scrubbing (markers, variable-speed audible scrub)
9. Export (two-pass render, WAV + MP3, clipping warning)
10. Safety net and polish (autosave/recovery, relink, unsaved prompts; remove `allow(dead_code)`)

## Known issues / environment notes
- cargo not on default PATH in PowerShell (prepend %USERPROFILE%\.cargo\bin).
- egui 0.36.2 breaking API documented in systemPatterns.md; verify future crate APIs on docs.rs per locked version.
- Keep no unwrap()/expect() outside tests and main.rs as code is filled in.

## Evolution of decisions
- 2026-10-12: Rust toolchain and VS Build Tools installed via winget (user-approved) — nothing was present.
- 2026-10-12: Adopted egui 0.36.2 new API (App::ui / Panel) after compile errors, per clinerules' "check docs.rs for the exact locked version".
- 2026-10-12: Memory bank uses docs/SPEC.md as the project brief (summarized + section pointers, not copied).