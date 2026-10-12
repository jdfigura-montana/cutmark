
# Active Context

## Current state
**Task 1.5 is DONE (2026-10-12).** Minimal Editor (src/editor/editor.rs): Editor{project: Arc<Project>, view, paths, dirty} with create/open/save; `// undo/redo: phase 2`; EditorError wrapping IoError; free recent_projects()/remove_recent() so UI never touches io::recent; recent writes best-effort (failure ignored). 6 tempdir tests (they touch + remove entries in the real %APPDATA% recent.json, cleaning up after themselves).

UI: ui/dialogs/new_project.rs NewProjectDialog (centered fixed egui::Window — NO egui::Modal in egui 0.36.2; no Label::truncate used) with name, Browse (rfd pick_folder), rate combo default 48 kHz, depth combo default 24-bit, Create disabled until name+folder set, red error label, returns Option<Editor>. ui/start_screen.rs StartScreen (cached recent list, greyed missing entries with Remove, red error label under buttons, returns Option<Editor>). app.rs: enum Screen{Start(StartScreen), Editing(Editor)}; Editing draws the task 1.1 panel layout (draw_editor_panels helper). main.rs: App::default().

Also fixed corrupted one-line mod.rs files (ui, ui/dialogs, editor, util, audio) that contained literal `r`n escapes — rewritten with real newlines. editor/mod.rs re-exports {Editor, recent_projects, remove_recent} and io::recent::RecentEntry; has #[allow(clippy::module_inception)] on pub mod editor (SPEC names editor/editor.rs).

## Key learnings
- egui 0.36.2: CentralPanel::show (show_inside deprecated); Window::show(ctx: &Context); Window::open(&mut bool) borrows, so Cancel must set a separate bool.
- clippy -D warnings catches module_inception, ptr_arg, deprecated, unused imports — all target-blocking.
- mod.rs files may still be corrupted single-line (literal `r`n) elsewhere; check Get-Content when a module unexpectedly fails to resolve.

## Next steps (rest of SPEC phase 1)
- Phase 1 check by hand: create a project, restart, open from Recent; delete folder, restart, greyed entry + Remove.
- No more phase-1 tasks known; await next task prompt (phase 2: model invariants + undo core).

## Active considerations
- Do NOT build ahead into later phases (clinerules). Remove `#![allow(dead_code)]` in main.rs in phase 10.
- Before any new egui/eframe/cpal/symphonia API use, verify docs.rs for the exact locked version.

