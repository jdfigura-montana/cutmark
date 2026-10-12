
# Progress

## What works - Task 1.6 complete (2026-10-12)
- Task 1.6: Project menu (New/Open/Save/Close) in ui/menu.rs, save-error modal in ui/dialogs/confirm.rs; Ctrl+N/O/S shortcuts (only when no text field focus); window title "[*]{name} - Audio Editor" / "Audio Editor"; status bar sample rate. StartScreen gained with_new_dialog/with_open_request; NewProjectDialog::new_open fixes a bug where the New Project button showed nothing. Corrupted mod.rs files (editor/ops, ui/timeline, ui/widgets) fixed; recent-list editor tests serialized with a Mutex.
- Task 1.5: minimal editor::Editor (create/open/save, dirty flag, recent-list touch), EditorError, free recent_projects/remove_recent; New Project dialog; start screen (New/Open Project/Open Folder/recent list with greyed missing + Remove); app.rs Screen{Start, Editing} switching to the task 1.1 panel layout. 6 new editor tests.
- Task 1.4: io/recent.rs (RecentEntry/RecentList, default_path via directories, touch/remove, atomic save, 8 tests).
- Task 1.3: io::paths (ProjectPaths, validate_project_name, create_layout), io::project_file (FORMAT_VERSION=1, atomic save, load from folder or project.json), IoError enum.
- Task 1.2: model data types per SPEC "Timeline model" with serde; Project::new.
- Task 1.1: module tree, eframe window with labeled panel regions (now shown when a project is open).
- cargo test: 25 passed; clippy -D warnings clean; cargo fmt applied.

## What is left (SPEC "Build phases")
1. **Skeleton** - DONE.
2. Model and undo core (invariants, Action, history, ops; Editor gains undo/redo fields per SPEC "Undo and redo").
3. Media import (decode/resample/cache/peaks, bin panel).
4. Timeline view. 5. Playback. 6. Editing. 7. Levels/processing. 8. Markers/scrub. 9. Export. 10. Safety net/polish (remove allow(dead_code)).

## Known issues / environment notes
- cargo not on default PATH (prepend %USERPROFILE%\cargo... i.e. $env:USERPROFILE\.cargo\bin; chain with ;).
- Keep no unwrap()/expect() outside tests and main.rs.
- memory-bank .md files carry a BOM; editor old_text matches may fail - rewrite via Set-Content.

## Evolution of decisions
- 2026-10-12: egui 0.36.2 API (App::ui, Panel::show, CentralPanel::show) adopted; egui::Modal does not exist in 0.36.2 - dialogs use centered fixed egui::Window.
- 2026-10-12: editor/mod.rs re-exports RecentEntry from io::recent so UI never imports io directly (architecture rule: ui calls only editor).
- 2026-10-12: Editor tests touch the real recent.json but remove their entries afterwards (create_layout/save paths need Editor::create which writes recent); now serialized with a Mutex because parallel tests raced on it.

