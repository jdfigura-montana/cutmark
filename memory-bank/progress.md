# Progress

## What works - Task 2.7 complete (2026-10-12)
- Task 2.7: marker ops. ops::marker add_marker/move_marker/delete_markers + prev_marker/next_marker helpers; Action variants AddMarker/MoveMarker/DeleteMarkers with labels "Add Marker"/"Move Marker"/"Delete Marker". Duplicate-position rule: Invalid error, nothing changes. delete_markers validates all ids before removing (NotFound otherwise). 13 tests incl. roundtrips and ripple-does-not-move-markers. cargo test 127 passed; clippy clean; fmt applied.
- Task 2.6: paste insert/replace (receiving tracks target+k, auto-append "Track N", new clip ids). Task 2.5: clipboard + range ops. Task 2.4: clip actions. Task 2.3: split/clear/place/shift primitives. Task 2.2: undo/redo + track ops. Task 2.1: model helpers + fixtures. Task 1.x: skeleton, io, Editor create/open/save, start screen, menus.

## What is left (SPEC "Build phases")
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub (UI wiring). 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin each session).
- cargo stderr makes PowerShell report exit 1 even on success; read the output.
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md/.rs files via Set-Content; editor tool inserts are unreliable.

## Evolution of decisions
- 2026-10-12: marker duplicate rule = OpError::Invalid("marker already at position N") for both add and move; markers re-sorted after every add/move.
- 2026-10-12: delete_markers returns NotFound for any unknown id and validates all ids before mutating (all-or-nothing).
- 2026-10-12: paste ops take Arc<Clipboard> and target_track; missing receiving tracks are appended via Track::new("Track N"); empty clipboard is OpError::Invalid.
- 2026-10-12: copy_range ignores unknown track ids silently; delete_range/trim_to_selection return OpError::NotFound for unknown track ids.
- 2026-10-12: trim_to_selection retains only clips with len() > 0 (clear_range boundary case).
- 2026-10-12: History::push_undo always clears redo; undo/redo move the current project between stacks.
- 2026-10-12: Gesture preview without begin_gesture falls back to the current project as base.
