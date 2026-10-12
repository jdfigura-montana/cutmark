# Progress

## What works - Task 1.3 complete (2026-10-12)
- Task 1.3: src/io/mod.rs IoError enum; io::paths (ProjectPaths, validate_project_name, create_layout) and io::project_file (FORMAT_VERSION=1, ProjectFile, atomic save via .tmp rename, load from folder or project.json). 8 tempdir tests added.
- Task 1.2: model data types per SPEC "Timeline model" with serde; Project::new; re-exports in model/mod.rs; invariants.rs untouched (phase 2).
- Task 1.1: Cargo binary crate `audio_editor` (edition 2021), full SPEC module tree, eframe 0.36.2 window with all labeled regions.
- cargo test: 10 passed; clippy -D warnings clean; cargo fmt applied.

## What is left (SPEC "Build phases")
1. **Skeleton** - IN PROGRESS: project folder + project.json done (task 1.3); start screen, New/Open UI wiring, recent projects list remain.
2. Model and undo core (invariants, Editor, Action, history, ops; cargo test coverage of apply/undo/compare)
3. Media import (decode/resample/cache/peaks, bin panel)
4. Timeline view (ruler, headers, lanes, clips, waveforms, zoom, drag-drop)
5. Playback (cpal engine, MixPlan, mixer, transport)
6. Editing (selections, move/trim/split/cut/copy/paste, ripple, snapping)
7. Levels and processing (volume/pan/mute/solo, meters, gain/normalize/fades/silence)
8. Markers and scrubbing (markers, variable-speed audible scrub)
9. Export (two-pass render, WAV + MP3, clipping warning)
10. Safety net and polish (autosave/recovery, relink, unsaved prompts; remove `allow(dead_code)` and model/mod.rs re-export allows)

## Known issues / environment notes
- cargo not on default PATH in PowerShell (prepend %USERPROFILE%\.cargo\bin; chain commands with `;` in one call).
- Keep no unwrap()/expect() outside tests and main.rs as code is filled in.

## Evolution of decisions
- 2026-10-12: Rust toolchain and VS Build Tools installed via winget (user-approved).
- 2026-10-12: Adopted egui 0.36.2 new API after compile errors, per clinerules "check docs.rs for the exact locked version".
- 2026-10-12: Memory bank uses docs/SPEC.md as the project brief (summarized + section pointers, not copied).
- 2026-10-12: Model structs copied field-for-field from SPEC "Timeline model"; ViewState defaults (1024 spp, snap on, ripple off).
- 2026-10-12: Task 1.3: IoError variants InvalidProjectName/FolderExists/NotAProject/UnsupportedVersion{found,max}; save is atomic (tmp + rename); load accepts folder or project.json and only rejects versions greater than 1 (forward-compat, per task).
