# Active Context

## Current state
**Task 2.3 is DONE (2026-10-12).** Clip-editing primitives: ops/clip.rs split_clip_at (left keeps id, cut edges get fade 0, then clamp_fades), clear_range (5-way clip/range classification; covers-both splits with new id on right piece), place_overwrite (clear + insert + sort), shift_from (i128 math, Overflow on <0 or >u64::MAX), split_at(project, pos, track_ids) (None = all tracks, NotFound for unknown ids); Action::SplitAt{pos,track_ids} label "Split" dispatched to clip::split_at; 14 new tests in clip.rs + SplitAt roundtrip tests in editor.rs tests. cargo test 76 passed; clippy clean; fmt applied.

Previous state (task 2.2): undo/redo engine + track ops (see progress.md / git history).

## Key learnings
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files (editor tool old_text fails on them and on CRLF multi-line matches unpredictably).
- Editor tool multi-line old_text sometimes fails on CRLF files written by PowerShell; single-line matches always work.
- clippy 1.99: collapsible_if now suggests let-chains (`if let X = y && cond`); Rust 2021 supports let-chains (1.88+).
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Binary crate: unused pub enum variants DO trigger dead_code; construct every variant somewhere (Overflow = index past end, Invalid = empty track name).
- Arc<Project> != Arc<Project> uses inner PartialEq; EditorError has no PartialEq (IoError), so test failures with matches!.
- clear_range covers-both case: capture the original fade_out BEFORE zeroing the left piece's, or the right piece loses its kept fade.

## Next steps
- Await next task prompt (phase 2 continues: range/paste/marker ops + Actions per SPEC).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker sits in Editor::apply_labeled where the snapshot handoff goes.
