# Progress

## What works - Task 2.8 complete (2026-10-12)
- Task 2.8: Project Settings. ops::set_project_settings validates the name (io::paths::validate_project_name) and sets name + export_bit_depth; never renames the project folder. Action::SetProjectSettings, label "Project Settings". Dialog ui::dialogs::project_settings (name, bit depth combo 16/24/32-bit float, OK/Cancel, red errors) opened from Project menu item "Project Settings..." via ProjectMenu::open_settings. 3 new tests (ops success + invalid-name-changes-nothing, editor.rs roundtrip). cargo test 130 passed; clippy clean; fmt applied.
- Task 2.7: marker ops. Task 2.6: paste insert/replace. Task 2.5: clipboard + range ops. Task 2.4: clip actions. Task 2.3: split/clear/place/shift primitives. Task 2.2: undo/redo + track ops. Task 2.1: model helpers + fixtures. Task 1.x: skeleton, io, Editor create/open/save, start screen, menus.

## What is left (SPEC "Build phases")
3. Media import. 4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub (UI wiring). 9. Export. 10. Safety net/polish.

## Known issues / environment notes
- cargo not on default PATH (append $env:USERPROFILE\.cargo\bin each session).
- cargo stderr makes PowerShell report exit 1 even on success; read the output.
- No unwrap()/expect() outside tests and main.rs.
- Rewrite BOM .md/.rs files via Set-Content; editor tool old_text anchors can fail on BOM files.

## Evolution of decisions
- 2026-10-12: set_project_settings maps IoError::InvalidProjectName to OpError::Invalid(error.to_string()); project folder is never renamed when the project name changes (SPEC: Save As is the only way to move/rename the folder).
- 2026-10-12: marker duplicate rule = OpError::Invalid("marker already at position N") for both add and move; markers re-sorted after every add/move.
- 2026-10-12: delete_markers returns NotFound for any unknown id and validates all ids before mutating (all-or-nothing).
- 2026-10-12: paste ops take Arc<Clipboard> and target_track; missing receiving tracks are appended via Track::new("Track N"); empty clipboard is OpError::Invalid.
- 2026-10-12: copy_range ignores unknown track ids silently; delete_range/trim_to_selection return OpError::NotFound for unknown track ids.
- 2026-10-12: trim_to_selection retains only clips with len() > 0 (clear_range boundary case).
- 2026-10-12: History::push_undo always clears redo; undo/redo move the current project between stacks.
- 2026-10-12: Gesture preview without begin_gesture falls back to the current project as base.
