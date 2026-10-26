# Cutmark Creator Suite

Commercial, Windows-only Rust suite. This repo is private.

## Layout
- Repo root: C:\Users\JD_Admin\Documents\Programming_Projects\cm_suite\cutmark (this workspace)
- apps/video-editor: the Cutmark Video app (binary cutmark-video)
- crates/video-ui: the video editor UI (egui), copied from FilmCraft's ui-egui and now ours
- docs/video-editor/ui-spec.md: source of truth for UI work
- third_party/filmcraft/: FilmCraft licenses, NOTICE and attribution. Keep intact.
- FilmCraft reference clone (read-only, not a workspace member): C:\Users\JD_Admin\Documents\Programming_Projects\cm_suite\filmcraft

## Hard rules
1. Never edit FilmCraft engine code. It comes from git dependencies pinned in the root Cargo.toml. The cm_suite\filmcraft folder is read-only reference.
2. FilmCraft's own CLAUDE.md and AGENTS.md apply to their repo only. These rules win.
3. UI changes go only in crates/video-ui and apps/video-editor.
4. All edits to a project go through FilmCraft engine commands. UI code never changes project state directly.
5. egui, eframe and wgpu versions must match the pinned FilmCraft commit. Never bump them alone.
6. No ArtCraft or FilmCraft names, logos or icons in anything user-visible.
7. No GPL or LGPL dependencies.
8. One change per task. Do not refactor, rename or reformat code the task does not touch.
9. Before finishing: cargo build -p cutmark-video and cargo clippy -p cutmark-video -p cutmark-video-ui pass with no warnings in our crates.
10. No em dashes in docs or UI text.