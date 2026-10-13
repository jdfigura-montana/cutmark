# Progress

## What works - Task 2.1 complete (2026-10-12)
- Task 2.1: model helpers (Clip len/end/clamp_fades, Track::new/sort_clips), model::invariants (InvariantError a-g + check, 13 tests), model::fixtures (project_with_clips/spans/srcs, cfg(test)). cargo test 44 passed; clippy -D warnings clean; fmt applied.
- Task 1.7: Save As. io/project_file.rs::copy_project (recursive copy_tree, name validation, FolderExists error), Editor::save_as (copy, rename via Arc::make_mut, repoint paths, save, touch recent), ui/dialogs/save_as.rs SaveAsDialog (name prefilled, parent + Browse, red inline errors), ProjectMenu item + open_save_as, Ctrl+Shift+S shortcut in app.rs. 5 new tests (3 io, 1 editor save_as).
- Task 1.6: Project menu (New/Open/Save/Close) in ui/menu.rs, save-error modal in ui/dialogs/confirm.rs; Ctrl+N/O/S shortcuts; window title "[*]{name} - Audio Editor"; status bar sample rate. StartScreen with_new_dialog/with_open_request/dialog_is_open; NewProjectDialog::new_open.
- Task 1.5: minimal editor::Editor (create/open/save, dirty flag, recent-list touch), EditorError, free recent_projects/remove_recent; New Project dialog; start screen (New/Open Project/Open Folder/recent list with greyed missing + Remove); app.rs Screen{Start, Editing}.
- Task 1.4: io/recent.rs (RecentEntry/RecentList, default_path via directories, touch/remove, atomic save, 8 tests).
- Task 1.3: io::paths (ProjectPaths, validate_project_name, create_layout), io::project_file (FORMAT_VERSION=1, atomic save, load from folder or project.json), IoError enum.
- Task 1.2: model data types per SPEC "Timeline model" with serde; Project::new.
- Task 1.1: module tree, eframe window with labeled panel regions.
- cargo test: 29 passed; clippy -D warnings clean; cargo fmt applied.

## What is left (SPEC "Build phases")
1. **Skeleton** - DONE.
2. Model and undo core (invariants, Action, history, ops; Editor gains undo/redo fields per SPEC "Undo and redo").
3. Media import (decode/resample/cache/peaks, bin panel).
4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish (remove allow(dead_code)).

## Known issues / environment notes
- cargo not on default PATH (prepend $env:USERPROFILE\.cargo\bin).
- Keep no unwrap()/expect() outside tests and main.rs.
- memory-bank .md files carry a BOM; editor old_text matches may fail - rewrite via Set-Content.

## Evolution of decisions
- 2026-10-12: Save As copies the whole folder recursively, then saves the renamed project.json into the copy; old folder and old recent entry left as-is (SPEC: "Project menu").
- 2026-10-12: egui 0.36.2 API (App::ui, Panel::show, CentralPanel::show) adopted; egui::Modal does not exist in 0.36.2 - dialogs use centered fixed egui::Window.
- 2026-10-12: editor/mod.rs re-exports RecentEntry from io::recent so UI never imports io directly (architecture rule: ui calls only editor).
- 2026-10-12: Editor tests touch the real recent.json but remove their entries afterwards; serialized with a Mutex.
