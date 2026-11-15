# Cutmark Creator Suite

Commercial, Windows-only Rust suite. This repo is private.

## Layout
- Repo root: C:\Users\JD_Admin\Documents\Programming_Projects\cm_suite\cutmark (this workspace)
- apps/video-editor: the Cutmark Video app (binary cutmark-video)
- crates/video-ui: the video editor UI (egui), copied from FilmCraft's ui-egui and now ours
- docs/video-editor/ui-spec.md: source of truth for UI work
- third_party/filmcraft/: FilmCraft licenses, NOTICE and attribution. Keep intact.
- FilmCraft reference clone (read-only, not a workspace member): C:\Users\JD_Admin\Documents\Programming_Projects\cm_suite\filmcraft
- apps/audio-editor: the Cutmark Audio app, package cutmark-audio (arrives in S0 phase 2)
- crates/marks: shared sidecar schema, reader and writer; no UI, no audio, no video dependencies (arrives in S0 phase 4)
- third_party/ffmpeg: LGPL FFmpeg shared DLLs, fetch script, notices (arrives in S0 phase 3)
- docs/suite: suite specs exported as Markdown
- docs/audio-editor: audio editor spec and docs (arrives in S0 phase 2)
- scripts/check.ps1: runs fmt, clippy, tests and cargo deny (arrives in S0 phase 1, prompt 1e)

## Hard rules
1. Never edit FilmCraft engine code. It comes from git dependencies pinned in the root Cargo.toml. The cm_suite\filmcraft folder is read-only reference.
2. FilmCraft's own CLAUDE.md and AGENTS.md apply to their repo only. These rules win.
3. Video UI changes go only in crates/video-ui and apps/video-editor.
4. All edits to a video project go through FilmCraft engine commands. Video UI code never changes project state directly.
5. egui, eframe and wgpu versions must match the pinned FilmCraft commit. Never bump them alone.
6. No ArtCraft or FilmCraft names, logos or icons in anything user-visible.
7. No GPL or AGPL dependencies anywhere. LGPL is allowed only as the FFmpeg DLLs from third_party/ffmpeg, never linked statically.
8. One change per task. Do not refactor, rename or reformat code the task does not touch.
9. Before finishing: cargo build -p cutmark-video and cargo clippy -p cutmark-video -p cutmark-video-ui pass with no warnings in our crates.
10. No em dashes in docs or UI text.
11. Work comes from a spec. Read the spec sections the prompt names, in docs/suite/ or the app's docs folder. Do only what the prompt asks, and do not build ahead.
12. Done means scripts\check.ps1 passes, plus a release build of the app the task touched.
13. No CI changes without asking. No workflows, jobs, triggers, matrix entries, retries or schedules.
14. One command at a time in PowerShell. No && chaining.

## Code conventions
- Errors: thiserror enums in libraries. anyhow only in main.rs and app.rs. No unwrap or expect outside tests and main.rs; the lint enforces it in new crates.
- Logging: the log facade. Apps pick the backend. Nothing logs on an audio thread.
- Time: audio positions are u64 sample counts, marks are integer milliseconds, wall-clock time goes through chrono.
- Paths: suite-wide files live in %APPDATA%\Cutmark\, each app's settings in %APPDATA%\Cutmark\{app}\. Code gets them from the directories crate and never builds them by hand.
- Files that matter (projects, autosave, exports, sidecars) are written to a temp file in the same folder and renamed over the target.
- Dependencies are added with cargo add, never by typing a version.

## Git
- main stays buildable. S0 work happens on s0-workspace.
- Commit titles read "S0 phase N: <what>".
- The S0 milestone ends with the tag s0, after scripts\check.ps1 and one manual CI run are green.