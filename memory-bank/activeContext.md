# Active Context

## Current state
**Task 1.1 is DONE (2026-10-12).** The crate is scaffolded and the main editor window renders with all labeled empty panels. Next work is the rest of SPEC phase 1: start screen, New/Open/Save of an empty project folder, recent projects list.

## What Task 1.1 delivered
1. Environment set up from scratch: rustup + Rust 1.99.0 MSVC + VS 2022 Build Tools (VC++ workload) via winget.
2. `cargo init --name audio_editor` (edition 2021).
3. Deps via cargo add: eframe, serde(derive), serde_json, uuid(v4,serde), rfd, directories, anyhow, thiserror, tempfile(dev).
4. Every file in the SPEC source tree exists (~45 stubs, each only a `//!` doc comment; mod.rs per folder).
5. `src/main.rs`: `#![allow(dead_code)] // TEMP: remove in phase 10` on line 1, module decls, eframe window "Audio Editor" 1400x850.
6. `src/app.rs`: eframe App — menu bar (empty Project menu) + toolbar row, bottom Status, right Master (exact_size 90, not resizable), left Media Bin (resizable, default 220, ui.take_available_space()), center Timeline; grey labels.
7. Validated: cargo fmt, clippy -D warnings, cargo test, cargo build, exe smoke-launched and killed.

## Key learnings
- egui 0.36.2 broke the classic API: `eframe::App::ui(&mut self, ui, frame)` + `egui::Panel::left/right/top/bottom` with `.show(ui, ...)`; SidePanel/TopBottomPanel/menu_bar(ctx) no longer exist (see systemPatterns.md).
- clippy lint `default_constructed_unit_structs`: write `App`, not `App::default()`, for unit structs.
- cargo needs `%USERPROFILE%\.cargo\bin` prepended to PATH in fresh PowerShell shells; PowerShell surfaces cargo stderr as NativeCommandError noise — exit code 1 is not always failure, look for "Finished".
- winget "another installation is already in progress" (exit 1618): just retry.

## Next steps (rest of SPEC phase 1)
- Start screen vs editor state in app.rs.
- New Project dialog (name, parent folder, sample rate 44.1/48/88.2/96 kHz, bit depth 16/24/32f), project folder creation.
- Open project (project.json or folder), Save, Recent Projects in %APPDATA% via `directories`.
- Phase 1 check: create a project, close, reopen it from Recent.

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- Remove `#![allow(dead_code)]` in phase 10.
- Before any new egui/eframe/cpal/symphonia API use, verify docs.rs for the exact locked version.