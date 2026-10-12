# Active Context

## Current state
**Task 1.3 is DONE (2026-10-12).** Project folders + project.json save/load done. Next work is rest of SPEC phase 1: start screen, New/Open wiring in UI, recent projects list (io/recent.rs still stub). (Model/undo core is phase 2.)

## What Task 1.3 delivered
- src/io/mod.rs: IoError thiserror enum (InvalidProjectName, FolderExists, NotAProject, UnsupportedVersion{found,max}, Io, Json) + pub(crate) is_project_json helper.
- src/io/paths.rs: ProjectPaths{root} with project_json()/media()/cache()/autosave()/exports(); validate_project_name (empty/whitespace/`< > : " / \ | ? *` rejected); create_layout (errors if folder exists, creates 5 dirs).
- src/io/project_file.rs: FORMAT_VERSION=1, ProjectFile{format_version, project, view} serde; save() pretty JSON via project.json.tmp + rename; load() accepts folder or project.json path, errors if format_version > 1.
- 8 new tests (tempdir based); cargo test 10 passed, clippy -D warnings clean, fmt applied.

## What Task 1.2 delivered
- src/model/: Project, BitDepth, ViewState (project.rs), MediaItem (media.rs), Track (track.rs), Clip (clip.rs), Marker (marker.rs) - exactly per SPEC "Timeline model" block; serde derive on all; Default for ViewState; Project::new. Re-exports in model/mod.rs.
- invariants.rs left as doc comment only (phase 2).
- Tests in project.rs: Project::new field check; serde_json round trip with one track/clip/media/marker.

## What Task 1.1 delivered
- Toolchain via winget (rustup, Rust 1.99.0 MSVC, VS 2022 Build Tools). cargo init (edition 2021). Deps via cargo add. Full SPEC module tree as ~45 one-line stubs. main.rs has TEMP `#![allow(dead_code)]` (remove in phase 10). app.rs: eframe App with all labeled regions (egui 0.36.2 new API: App::ui / egui::Panel).

## Key learnings
- Stub files were written with literal `` `r`n `` escapes (single-line files). When rewriting, rewrite the whole file (Set-Content with here-string), not substring replace or editor inserts.
- The editor tool call kept losing old_text during this task; prefer Set-Content here-strings for whole-file rewrites.
- cargo needs `%USERPROFILE%\.cargo\bin` prepended to PATH; chain commands with `;` in one command string (each array element is a separate shell). PowerShell reports exit code 1 for stderr noise; judge by the cargo output itself.

## Next steps (rest of SPEC phase 1)
- Start screen vs editor state in app.rs.
- New Project dialog (name, parent, sample rate, bit depth) wiring create_layout + save.
- Open project wiring load(); Recent Projects in %APPDATA% via `directories` (io/recent.rs).
- Phase 1 check: create a project, close, reopen it from Recent.

## Active considerations
- Do NOT build ahead into later phases (clinerules).
- Remove `#![allow(dead_code)]` and model/mod.rs re-export allows in phase 10.
- Before any new egui/eframe/cpal/symphonia API use, verify docs.rs for the exact locked version.
