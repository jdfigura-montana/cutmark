# Progress

## What works - Task 2.2 complete (2026-10-12)
- Task 2.2: undo/redo engine + track ops. ops::track add/delete/rename/move track, set volume/pan/mute/solo, solo_exclusive, set_master_volume (all clamped to SPEC ranges); ops::OpError; action::Action (10 track variants, label(), run()); history::History (1,000-step cap, push clears redo, undo_step/redo_step swap snapshots); editor::Editor apply/apply_labeled (clone > op > invariants > push undo > swap, dirty; publish-to-audio marker left), undo/redo + labels, gesture begin/preview/end/cancel; EditorError::Op/Invariant. roundtrip helper + all required tests. cargo test 62 passed; clippy clean; fmt applied.
- Task 2.1: model helpers + invariants + fixtures (see git/file history).
- Task 1.x: skeleton, io (paths/project_file/recent), Editor create/open/save/save_as, start screen, menus, Save As dialog.

## What is left (SPEC "Build phases")
2. Model and undo core (continues: clip ops, ranges, paste, markers ops + Actions).
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (prepend $env:USERPROFILE\.cargo\bin).
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md files via Set-Content; editor tool multi-line matches can fail on CRLF files.

## Evolution of decisions
- 2026-10-12: OpError::Overflow used for add/move track index past end; OpError::Invalid for empty track names (keeps all variants constructed, avoids bin-crate dead_code).
- 2026-10-12: History::push_undo always clears redo (SPEC step 4); undo_step/redo_step move the current project onto the opposite stack.
- 2026-10-12: Gesture preview without begin_gesture falls back to using the current project as base (defensive).
