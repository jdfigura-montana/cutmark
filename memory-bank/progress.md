# Progress

## What works - Task 2.4 complete (2026-10-12)
- Task 2.4: clip actions. ops::clip place_clip/move_clips/trim_clip/delete_clips/set_clip_gain/set_clip_fades + Edge enum (uses 2.3 primitives split_clip_at/place_overwrite/shift_from); Action::PlaceClip/MoveClips/TrimClip/DeleteClips/SetClipGain/SetClipFades with spec labels; 13 roundtrip/error-path tests in editor.rs. cargo test 89 passed; clippy clean; fmt applied.
- Task 2.3: clip primitives split_clip_at/clear_range/place_overwrite/shift_from/split_at + Action::SplitAt (see git history).
- Task 2.2: undo/redo engine + track ops. Task 2.1: model helpers + invariants + fixtures.
- Task 1.x: skeleton, io, Editor create/open/save/save_as, start screen, menus.

## What is left (SPEC "Build phases")
2. Model and undo core (continues: range/paste/marker ops + Actions).
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin in each PowerShell command).
- cargo stderr makes PowerShell report exit 1 even on success; read the output.
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md files via Set-Content; editor tool multi-line matches can fail on CRLF files.

## Evolution of decisions
- 2026-10-12: OpError::Invalid for move_clips validation failures (start < 0 or track index out of range), validated before any mutation so the project is untouched on error.
- 2026-10-12: OpError::Overflow used for add/move track index past end, empty track names -> Invalid; shift_from reuses Overflow for start < 0 / > u64::MAX.
- 2026-10-12: History::push_undo always clears redo; undo_step/redo_step move the current project onto the opposite stack.
- 2026-10-12: Gesture preview without begin_gesture falls back to the current project as base (defensive).
