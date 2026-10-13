# Active Context

## Current state
Task 2.5 is DONE (2026-10-12). Clipboard + range ops: editor/clipboard.rs (Clipboard { length, tracks: Vec<Vec<Clip>> }, copy_range packing selected tracks top-to-bottom, per-clip pieces with fades kept only at untouched clip edges then clamp_fades; new clip ids), ops/range.rs delete_range (clear_range + optional ripple shift_from) and trim_to_selection (clear_range(0,s) + clear_range(e,u64::MAX) + retain len>0), Action::DeleteRange (label "Delete") / TrimToSelection (label "Trim to Selection"), Editor::clipboard field + copy (no undo/dirty) / cut (apply_labeled "Cut") / clipboard(). Tests: 4 clipboard unit tests, 6 range unit tests, 5 editor tests (3 roundtrips, cut undo+label, copy not dirty). cargo test 104 passed; clippy clean; fmt applied.

## Key learnings
- clear_range keeps a ZERO-LENGTH left/right piece when the range edge aligns exactly with a clip start/end (covers-both branch); trim must retain(|c| c.len() > 0) after clears.
- The editor tool intermittently drops old_text and insert_line-by-number shifts with earlier edits in the same batch: batched line-number inserts CORRUPTED action.rs and editor.rs; recovered via git checkout + PowerShell replacements. Prefer old_text anchors, one edit at a time, or Set-Content/regex replaces.
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files.
- clippy 1.99: collapsible_if suggests let-chains; Rust 2021 supports them.
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Binary crate: unused pub enum variants DO trigger dead_code; construct every variant somewhere.
- Arc<Project> != Arc<Project> uses inner PartialEq; EditorError has no PartialEq, use matches!.
- PowerShell `cargo` invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text. link.exe 1104 = stale test binary lock; wait and rerun.

## Next steps
- Await next task prompt (phase 2 continues: paste + marker ops per SPEC).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker sits in Editor::apply_labeled.
