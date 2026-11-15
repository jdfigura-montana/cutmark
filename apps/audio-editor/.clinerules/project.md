# Rust Audio Editor: rules for every task

## Source of truth
- docs/SPEC.md is the spec. Read the sections the task names before writing code.
- docs/SPEC.md outranks memory-bank/. If they disagree, follow the SPEC and fix the memory bank.
- Never edit docs/SPEC.md. If the spec seems wrong or unclear, stop and ask.
- Do only what the current prompt asks. Do not build ahead into later phases, even if memory-bank/progress.md lists them as next.
- Do not add features, crates, or files the prompt didn't ask for. Exception: files in memory-bank/.

## Memory bank
- Do not copy SPEC content into memory-bank/. Point to SPEC section names instead.
- Record only what the SPEC can't tell you: what's built so far, decisions made while coding, crate API gotchas, and open problems.
- Keep each memory bank file short. Replace stale lines instead of appending history.

## Environment
- Windows 10/11, x64. The terminal is PowerShell. Run one command at a time; do not chain with `&&`.
- Rust stable, edition 2021.
- Add crates only with `cargo add` (use `--features` and `--dev` as needed). Never type version numbers into Cargo.toml by hand.
- Before using a crate API you're unsure of, check the exact version in Cargo.lock and read the docs.rs page for that version. Do not guess from memory. egui/eframe, cpal and symphonia change their APIs between versions.

## Done means
Run these in order and fix every problem before saying you're finished:
1. cargo fmt
2. cargo clippy --all-targets -- -D warnings
3. cargo test
4. Update memory-bank/activeContext.md and memory-bank/progress.md.

Then reply with:
- Files created or changed, one line each
- How to check the result by hand

## Architecture (SPEC: "Code architecture")
- Top-level modules: model, editor, audio, io, ui, util. Calls flow downward only.
- ui calls only editor. ui may read model types to draw them.
- editor is the only code that changes a Project. editor calls audio and io.
- audio never touches files. io never plays audio. audio and io never call each other.
- model depends only on std, serde and uuid. util holds shared helpers only.
- Only ui, app.rs and main.rs import egui/eframe. Only audio imports cpal.
- Put code in the files named in the SPEC source tree. Don't invent new top-level modules.

## Code style
- Errors: thiserror enums in model, editor, audio and io. anyhow only in app.rs and main.rs.
- No unwrap() or expect() outside tests and main.rs.
- Timeline positions and lengths are u64 sample counts. Never store seconds as floats in the model.
- Small files, one concern each. Every pub item gets a one-line doc comment.
- Unit tests go in the same file under `#[cfg(test)] mod tests`.
- Files that matter (project.json, autosave, exports) are written to a temp file in the same folder, then renamed over the target.

## Audio thread (cpal callback), no exceptions
- No allocation or freeing.
- No locks, no file I/O, no logging.
- No panics, no unwrap, no indexing that can go out of bounds.
- Read shared state only through ArcSwap, atomics, or bounded crossbeam channels.
