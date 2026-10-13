# Active Context

## Current state
**Task 2.2 is DONE (2026-10-12).** Undo/redo engine + track ops: ops/mod.rs OpError{NotFound(Uuid),Invalid(String),Overflow}; ops/track.rs all 10 track ops (clamps -60..+12 dB, pan -1..+1, Overflow for index past end, Invalid for empty names); action.rs Action enum (10 variants) + label() + run() dispatch; history.rs HistoryEntry{label,before:Arc<Project>} + History (undo/redo stacks, MAX 1,000, push_undo clears redo); editor.rs Editor gains history/gesture_base/gesture_label + with_project, apply/apply_labeled (SPEC 5 steps, `// publish to audio: phase 5` left in place), undo/redo/undo_label/redo_label, begin_gesture/preview/end_gesture/cancel_gesture; EditorError += Op/Invariant. roundtrip() test helper; 18 new tests. cargo test 62 passed; clippy -D warnings clean; fmt applied.

Previous state (task 2.1): model helpers + invariants complete (see progress.md).

## Key learnings
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files (editor tool old_text fails on them and on CRLF multi-line matches unpredictably).
- Editor tool multi-line old_text sometimes fails on CRLF files written by PowerShell; single-line matches always work.
- clippy 1.99: collapsible_if now suggests let-chains (`if let X = y && cond`); Rust 2021 supports let-chains (1.88+).
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Binary crate: unused pub enum variants DO trigger dead_code; construct every variant somewhere (Overflow = index past end, Invalid = empty track name).
- Arc<Project> != Arc<Project> uses inner PartialEq; EditorError has no PartialEq (IoError), so test failures with matches!.

## Next steps
- Await next task prompt (phase 2 continues: clip/range/paste/marker ops per SPEC "Undo and redo" testing note).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker sits in Editor::apply_labeled where the snapshot handoff goes.
