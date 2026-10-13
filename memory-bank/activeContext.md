# Active Context

## Current state
Task 2.4 is DONE (2026-10-12). Clip actions in ops/clip.rs: place_clip (overwrite or insert = split_at + shift_from + push; returns new Uuid), move_clips (validate all starts/track indices first, Invalid on violation, then remove + place_overwrite per clip, ids kept), trim_clip (Edge::Start moves start+src_in together, Edge::End moves src_out; clamps: src_in>=0 via start>=old_start-old_src_in, src_out<=media_len, len>=1, neighbor bounds left/right; fades kept then clamp_fades), delete_clips (ripple: per track right-to-left remove + shift_from(end, -len)), set_clip_gain (clamp -60..=24), set_clip_fades (set then clamp_fades). Helper locate_clip returns (track_index, clip_index). Edge enum in ops/clip.rs. Action variants PlaceClip/MoveClips/TrimClip/DeleteClips/SetClipGain/SetClipFades with labels Add Clip/Move Clip/Trim Clip/Delete Clip/Clip Gain/Fades. 13 roundtrip/error tests in editor.rs incl. two error-path tests using new with_editor helper. cargo test 89 passed; clippy clean; fmt applied.

## Key learnings
- PowerShell here-string + Set-Content -Encoding UTF8 is the reliable way to rewrite BOM-carrying files (editor tool old_text fails on them and on CRLF multi-line matches unpredictably).
- Editor tool multi-line old_text sometimes fails on CRLF files written by PowerShell; single-line matches always work. Editor new_text must stay <= 6000 chars; split big additions into chunks inserted at the same anchor line.
- clippy 1.99: collapsible_if now suggests let-chains (`if let X = y && cond`); Rust 2021 supports let-chains (1.88+).
- `pub fn f(&mut v, v.field)` is an E0502 in tests: bind `let id = v.field;` first.
- Binary crate: unused pub enum variants DO trigger dead_code; construct every variant somewhere.
- Arc<Project> != Arc<Project> uses inner PartialEq; EditorError has no PartialEq (IoError), so test failures with matches!.
- clear_range covers-both case: capture the original fade_out BEFORE zeroing the left piece's fade.
- PowerShell `cargo` invocations report exit code 1 whenever cargo writes to stderr even on success; judge by output text, not exit code.

## Next steps
- Await next task prompt (phase 2 continues: range/paste/marker ops + Actions per SPEC).

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- `// publish to audio: phase 5` marker sits in Editor::apply_labeled where the snapshot handoff goes.
