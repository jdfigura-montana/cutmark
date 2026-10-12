# Active Context

## Current state
**Task 1.7 is DONE (2026-10-12).** io/project_file.rs::copy_project(from, parent, new_name) validates the name, errors FolderExists if the target exists, then recursively copies every file/folder (private copy_tree) and returns the new ProjectPaths. Editor::save_as(parent, new_name): copy_project, Arc::make_mut(project).name = new_name, repoint paths, save, touch_recent; old folder untouched. ui/dialogs/save_as.rs SaveAsDialog::new_open(current_name, current_root) — name prefilled, parent prefilled with current root's parent, Browse via rfd::pick_folder, red inline errors, Save disabled until name+parent; ProjectMenu holds save_as_dialog and open_save_as(editor), menu item "Save As... (Ctrl+Shift+S)". app.rs shortcut ctrl+shift+s checked BEFORE ctrl+s.

Previous state (task 1.6): ui/menu.rs: ProjectMenu.show(ui, editor) -> ProjectMenuAction{None,New,Open,Close}; Save calls Editor::save, failures open ConfirmDialog modal (ui/dialogs/confirm.rs, centered fixed Window, message + OK). app.rs: App{screen, menu, title}; ui() mem::replaces the screen out each frame so editor/menu/screen borrow disjointly; Project shortcuts Ctrl+N/O/S via ui.input key_pressed, guarded by ctx.egui_wants_keyboard_input() (+ dialog_is_open on the start screen); title via ViewportCommand::Title sent only when changed: "Audio Editor" / "[*]{name} - Audio Editor". status_bar.rs: show(ui, &project) right-aligned "{rate} Hz". ui/start_screen.rs: with_new_dialog()/with_open_request()/dialog_is_open(); pending_open triggers the file picker on first show. ui/dialogs/new_project.rs: new_open().

## Key learnings
- egui 0.36.2: CentralPanel::show (show_inside deprecated); Window::show(ctx: &Context); Window::open(&mut bool) borrows, so Cancel/OK must set a separate local bool inside the closure; Ui::close() closes menus (no close_menu); Context::egui_wants_keyboard_input(); ViewportCommand::Title via ctx.send_viewport_cmd; Ui::menu_button exists, closure gets a Ui.
- clippy -D warnings catches module_inception, ptr_arg, deprecated, unused imports - all target-blocking.
- mod.rs files may still be corrupted single-line (literal backtick-r backtick-n) elsewhere; check Get-Content when a module unexpectedly fails to resolve.
- Memory-bank .md files carry a BOM; editor old_text matches may fail - rewrite via Set-Content.

## Next steps (rest of SPEC phase 1)
- Phase 1 check by hand: create a project, Ctrl+S, Save As to a new name (both folders open from Recent with their own names), Project > Close, reopen from Recent; delete folder, restart, greyed entry + Remove.
- No more phase-1 tasks known; await next task prompt (phase 2: model invariants + undo core).

## Active considerations
- Do NOT build ahead into later phases (clinerules). Remove #![allow(dead_code)] in main.rs in phase 10.
- Before any new egui/eframe/cpal/symphonia API use, verify docs.rs for the exact locked version.
