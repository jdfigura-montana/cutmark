# Active Context

## Current state
Task 2.6 is DONE (2026-10-12). Paste ops: ops/paste.rs paste_insert (split_clip_at + shift_from(+len) + insert clips at pos+start; always shifts) and paste_replace (ripple: clear_range(s,e) + shift_from(e, len-(e-s)) + insert at s; no ripple: clear_range(s, max(e, s+len)) + insert at s). Receiving tracks: clipboard track k -> target_track+k, missing tracks appended as Track::new("Track N"); every pasted clip gets a new uuid. Editor::paste_insert_at(pos, target_track) and paste_replace_range(s, e, target_track, ripple) use the stored clipboard, error OpError::Invalid("empty clipboard") when none or all tracks empty, commit path label "Paste" (new Editor::commit helper; apply_labeled now delegates to it). Tests: 10 editor paste tests (fixture clipboards built with copy_range sharing the target's media id; apply -> undo -> redo roundtrip). cargo test 114 passed; clippy clean; fmt applied.

## Key learnings
- clear_range keeps a ZERO-LENGTH left/right piece when the range edge aligns exactly with a clip start/end (covers-both branch); trim must retain(|c| c.len() > 0) after clears.
- The editor tool intermittently drops old_text and insert_line-by-number shifts with earlier edits in the same batch: batched line-number inserts CORRUPTED action.rs and editor.rs; recovered via git checkout + PowerShell replacements. Prefer old_text anchors, one edit at a time, or Set-Content/regex replaces.
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files.
- clippy 1.99: collapsible_if suggests let-chains; Rust 2021 supports them.
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Binary crate: unused pub enum variants DO trigger dead_code; construct every variant somewhere.
- Arc<Project> != Arc<Project> uses inner PartialEq; EditorError has no PartialEq, use matches!.
- PowerShell `cargo` invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text. link.exe 1104 = stale test binary lock; wait and rerun.
- Test fixtures each make their own media id; pasting between two fixture projects needs the clipboard source's media id replaced with the target's (see clipboard_of in editor.rs tests) or the invariant check fails MissingMedia.

## Next steps
- Await next task prompt (phase 2 continues: marker ops per SPEC).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker now sits in Editor::commit.
