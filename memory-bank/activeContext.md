# Active Context

## Current state
**Task 1.2 is DONE (2026-10-12).** Model data types implemented (no edit logic). Next work is the rest of SPEC phase 1: start screen, New/Open/Save of an empty project folder, recent projects list. (Model/undo core is phase 2.)

## What Task 1.2 delivered
- src/model/: Project, BitDepth, ViewState (project.rs), MediaItem (media.rs), Track (track.rs), Clip (clip.rs), Marker (marker.rs) - exactly per SPEC "Timeline model" block; serde derive on all; Default for ViewState; Project::new. Re-exports in model/mod.rs.
- invariants.rs left as doc comment only (phase 2).
- Tests in project.rs: Project::new field check; serde_json round trip with one track/clip/media/marker. cargo test: 2 passed; clippy -D warnings clean; cargo fmt applied.

## What Task 1.1 delivered
1. Environment set up from scratch: rustup + Rust 1.99.0 MSVC + VS 2022 Build Tools (VC++ workload) via winget.
2. `cargo init --name audio_editor` (edition 2021).
3. Deps via cargo add: eframe, serde(derive), serde_json, uuid(v4,serde), rfd, directories, anyhow, thiserror, tempfile(dev).
4. Every file in the SPEC source tree exists (~45 stubs, each only a `//!` doc comment; mod.rs per folder).
5. `src/main.rs`: `#![allow(dead_code)] // TEMP: remove in phase 10` on line 1, module decls, eframe window "Audio Editor" 1400x850.
6. `src/app.rs`: eframe App - menu bar (empty Project menu) + toolbar row, bottom Status, right Master (exact_size 90, not resizable), left Media Bin (resizable, default 220, ui.take_available_space()), center Timeline; grey labels.

## Key learnings
- Stub files were written with literal `` `r`n `` escapes (single-line files). When rewriting, rewrite the whole file (Set-Content), not a substring replace - substring matches inside the escape run.
- Binary crate: `pub use` re-exports in a module trip clippy unused_imports; added `#[allow(unused_imports)]` + TEMP comment per re-export (remove in phase 10).
- egui 0.36.2 broke the classic API: `eframe::App::ui(&mut self, ui, frame)` + `egui::Panel::left/right/top/bottom` with `.show(ui, ...)` (see systemPatterns.md).
- cargo needs `%USERPROFILE%\.cargo\bin` prepended to PATH in fresh PowerShell shells; also all commands must be in ONE command string with `;` - each array element is a separate shell.

## Next steps (rest of SPEC phase 1)
- Start screen vs editor state in app.rs.
- New Project dialog (name, parent folder, sample rate 44.1/48/88.2/96 kHz, bit depth 16/24/32f), project folder creation.
- Open project (project.json or folder), Save, Recent Projects in %APPDATA% via `directories`.
- Phase 1 check: create a project, close, reopen it from Recent.

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- Remove `#![allow(dead_code)]` and the model/mod.rs re-export allows in phase 10.
- Before any new egui/eframe/cpal/symphonia API use, verify docs.rs for the exact locked version.
