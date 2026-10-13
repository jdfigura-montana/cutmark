# Active Context

## Current state
Task 2.7 is DONE (2026-10-12). Marker ops: ops::marker add_marker/move_marker (OpError::Invalid when a marker already sits at pos, list re-sorted by position), delete_markers (NotFound for unknown ids, validated before mutate), helpers prev_marker/next_marker (strictly before/after, no Result). Action variants AddMarker{pos}/MoveMarker{id,pos}/DeleteMarkers{ids}, labels "Add Marker"/"Move Marker"/"Delete Marker". 13 marker tests (9 in ops::marker, 5 roundtrip in editor.rs incl. add-at-existing-position error and ripple DeleteRange leaves markers untouched). cargo test 127 passed; clippy clean; fmt applied.

## Key learnings
- clear_range keeps a ZERO-LENGTH left/right piece when the range edge aligns exactly with a clip start/end (covers-both branch); trim must retain(|c| c.len() > 0) after clears.
- The editor tool often MANGLES insert_line edits and can mis-place old_text-based inserts into the middle of match arms: it corrupted action.rs twice; recovered with git checkout + full-file PowerShell here-string Set-Content. Prefer full-file rewrites via Set-Content for BOM files and single old_text anchor edits for the rest, one edit per call, verify after each.
- Action::DeleteRange/TrimToSelection track_ids are plain Vec<Uuid> (NOT Option); SplitAt uses Option.
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files.
- clippy 1.99: collapsible_if suggests let-chains; Rust 2021 supports them.
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Arc<Project> != Arc<Project>: deref both sides in assert_eq! (*a, *b).
- PowerShell cargo invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text.
- Test fixtures each make their own media id; pasting between two fixture projects needs the clipboard source media id replaced with the targets (see clipboard_of in editor.rs tests).
- Do NOT run multi-line/broken PowerShell; a mangled command can leave the shell interactive.

## Next steps
- Await next task prompt (phase 2: model/undo core is now complete per SPEC build phases).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker now sits in Editor::commit.
