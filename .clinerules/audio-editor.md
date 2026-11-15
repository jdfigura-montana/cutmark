# Audio editor rules (apps/audio-editor)

These add to the rules in the root CLAUDE.md. They apply to any task in apps/audio-editor.

## Source of truth
- docs/audio-editor/spec.md is the spec. Read the sections the task names before writing code.
- The spec outranks apps/audio-editor/memory-bank/. If they disagree, follow the spec and fix the memory bank.
- Do not edit the spec unless the prompt says so. If it seems wrong or unclear, stop and ask.
- Do only what the prompt asks. Do not build ahead, even if memory-bank/progress.md lists later phases as next.
- No behavior changes unless the task says so.
- Do not add features, crates or files the prompt did not ask for. Exception: files in the memory bank.
- Paths in the memory bank that start with docs/ now live in docs/audio-editor/.

## Memory bank
- At the start of a task, read only memory-bank/activeContext.md and memory-bank/progress.md. Read other memory bank files only if the task names them. At the end, update those two.
- Do not copy spec content into the memory bank. Point to spec section names.
- Record only what the spec cannot tell you: what is built, decisions made while coding, crate API gotchas, open problems. Keep files short. Replace stale lines instead of appending history.

## Environment
- Windows 11, x64, PowerShell. Edition 2024 (set by the workspace).
- Add crates only with cargo add. Never type a version into Cargo.toml.
- Before using a crate API you are unsure of, check the exact version in the root Cargo.lock and read the docs.rs page for that version. Do not guess from memory. egui/eframe, cpal and symphonia change their APIs often.

## Architecture (spec section "Code architecture")
- Top-level modules: model, editor, audio, io, ui, util. Calls flow downward only.
- ui calls only editor. ui may read model types to draw them.
- editor is the only code that changes a Project. editor calls audio and io.
- audio never touches files. io never plays audio. audio and io never call each other.
- model depends only on std, serde and uuid. util holds shared helpers only.
- Only ui, app.rs and main.rs import egui or eframe. Only audio imports cpal.
- Put code in the files named in the spec source tree. Do not invent new top-level modules.

## Code style
- Errors: thiserror enums in model, editor, audio and io. anyhow only in app.rs and main.rs.
- No unwrap() or expect() outside tests and main.rs.
- Timeline positions and lengths are u64 sample counts. Never store seconds as floats in the model.
- Small files, one concern each. Every pub item gets a one-line doc comment.
- Unit tests go in the same file under #[cfg(test)] mod tests.

## Audio thread (cpal callback), no exceptions
- No allocation or freeing.
- No locks, no file I/O, no logging.
- No panics, no unwrap, no indexing that can go out of bounds.
- Read shared state only through ArcSwap, atomics, or bounded crossbeam channels.

## UI rules
- Visual target is docs/audio-editor/ui-target.png.
- Every color, size and font comes from src/ui/theme.rs. No hardcoded Color32 or pixel values elsewhere in src/ui.
- Any right-aligned group must still read left to right on screen.

## egui 0.36 gotchas
- horizontal() and with_layout() rows using Align::Center stretch to the parent's full available height. Inside a Modal, always bound each row with an explicit max_rect.
- Panels use exact_size, not exact_height.
- The menu bar uses MenuBar::new().ui(...).

## Source encoding
- All .rs files are UTF-8 without a BOM.
- Write any non-ASCII character in a string literal as a \u{...} escape.
- Draw dots and icons with the painter or egui-phosphor, never as Unicode text glyphs.

## Done means
- Root rule 12 applies: scripts\check.ps1 passes, plus cargo build --release -p cutmark-audio. The release exe is target\release\cutmark-audio.exe in the workspace root, so the release build must be fresh.
- End every task with: files changed, the root cause of any bug, and a hand-check list.
