# Progress

## What works - Task 2.3 complete (2026-10-12)
- Task 2.3: clip primitives. ops::clip split_clip_at/clear_range/place_overwrite/shift_from/split_at (fade rule: cut edges fade 0, untouched edges keep fades, then clamp_fades; gain unchanged on all fragments); Action::SplitAt "Split" dispatched to clip::split_at; SplitAt roundtrip tests for all-tracks and single-track. cargo test 76 passed; clippy clean; fmt applied.
- Task 2.2: undo/redo engine + track ops (see git/file history for detail).
- Task 2.1: model helpers + invariants + fixtures (see git/file history).
- Task 1.x: skeleton, io (paths/project_file/recent), Editor create/open/save/save_as, start screen, menus, Save As dialog.

## What is left (SPEC "Build phases")
2. Model and undo core (continues: range/paste/marker ops + Actions).
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (prepend $env:USERPROFILE\.cargo\bin).
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md files via Set-Content; editor tool multi-line matches can fail on CRLF files.

## Evolution of decisions
- 2026-10-12: OpError::Overflow used for add/move track index past end, empty track names -> Invalid; shift_from reuses Overflow for start < 0 / > u64::MAX (i128 arithmetic, no partial mutation). Keep all variants constructed (bin-crate dead_code).
- 2026-10-12: History::push_undo always clears redo (SPEC step 4); undo_step/redo_step move the current project onto the opposite stack.
- 2026-10-12: Gesture preview without begin_gesture falls back to using the current project as base (defensive).
