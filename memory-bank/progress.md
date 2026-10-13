# Progress

## What works - Task 2.5 complete (2026-10-12)
- Task 2.5: clipboard + range ops. clipboard.rs copy_range (relative starts, fades kept at untouched edges, packed selected tracks); ops::range delete_range/trim_to_selection (trim drops zero-length boundary pieces); Action::DeleteRange ("Delete") / TrimToSelection ("Trim to Selection"); Editor clipboard field + copy/cut/clipboard() (cut label "Cut"). Unit + roundtrip tests; cargo test 104 passed; clippy clean; fmt applied.
- Task 2.4: clip actions (place/move/trim/delete clips, gain, fades). Task 2.3: split/clear/place/shift primitives. Task 2.2: undo/redo + track ops. Task 2.1: model helpers + fixtures. Task 1.x: skeleton, io, Editor create/open/save, start screen, menus.

## What is left (SPEC "Build phases")
2. Model and undo core (continues: paste + marker ops).
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin each session).
- cargo stderr makes PowerShell report exit 1 even on success; read the output.
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md files via Set-Content; editor tool old_text can fail on CRLF/BOM files.

## Evolution of decisions
- 2026-10-12: copy_range ignores unknown track ids silently (returns Clipboard, not Result); delete_range/trim_to_selection return OpError::NotFound for unknown track ids.
- 2026-10-12: trim_to_selection retains only clips with len() > 0 (clear_range boundary case).
- 2026-10-12: History::push_undo always clears redo; undo/redo move the current project between stacks.
- 2026-10-12: Gesture preview without begin_gesture falls back to the current project as base.
