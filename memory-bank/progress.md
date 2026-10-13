# Progress

## What works - Task 2.6 complete (2026-10-12)
- Task 2.6: paste insert/replace. ops::paste paste_insert/paste_replace (receiving tracks target+k, auto-append "Track N", new clip ids; insert always shifts; replace ripple shifts by length difference, no ripple overwrites to max(e, s+len)); Editor::paste_insert_at / paste_replace_range with stored clipboard, label "Paste", error on empty clipboard. 10 editor roundtrip tests; cargo test 114 passed; clippy clean; fmt applied.
- Task 2.5: clipboard + range ops (copy_range, delete_range, trim_to_selection, Editor copy/cut). Task 2.4: clip actions (place/move/trim/delete clips, gain, fades). Task 2.3: split/clear/place/shift primitives. Task 2.2: undo/redo + track ops. Task 2.1: model helpers + fixtures. Task 1.x: skeleton, io, Editor create/open/save, start screen, menus.

## What is left (SPEC "Build phases")
2. Model and undo core (continues: marker ops).
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin each session).
- cargo stderr makes PowerShell report exit 1 even on success; read the output.
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md files via Set-Content; editor tool old_text can fail on CRLF/BOM files.

## Evolution of decisions
- 2026-10-12: paste ops take Arc<Clipboard> and target_track; missing receiving tracks are appended via Track::new("Track N"); empty clipboard (none stored or all track lists empty) is OpError::Invalid.
- 2026-10-12: copy_range ignores unknown track ids silently (returns Clipboard, not Result); delete_range/trim_to_selection return OpError::NotFound for unknown track ids.
- 2026-10-12: trim_to_selection retains only clips with len() > 0 (clear_range boundary case).
- 2026-10-12: History::push_undo always clears redo; undo/redo move the current project between stacks.
- 2026-10-12: Gesture preview without begin_gesture falls back to the current project as base.
