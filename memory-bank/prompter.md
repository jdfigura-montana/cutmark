# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1, V1: crates/prompter-platform (os::check_windows_11, build 22000 or later) and apps/prompter (cutmark-prompter): suite chrome, Violet appearance, Windows 11 dialog, placeholder panel.
- Phase 1, V2: prompter_platform::view (ViewWindow on NativeWindow(isize): find_own_window, exclude_from_capture, set_no_activate, reassert_topmost, status) and PlatformError. Non-Windows stubs return Unsupported. The app does not call it yet.

## Next (V3)
- Waiting for the V3 prompt. status() is meant for its diagnostics line.
- Ignored test, run by hand: cargo test -p prompter-platform -- --ignored hidden_window_takes_all_three_calls

## Notes
- V2: the ignored real-window test has not been run, and the Linux stubs have not been compiled.
- V3 check (from knowledge of winit, not verified in the pinned source): winit may rewrite GWL_EXSTYLE when a window flag changes, such as MousePassthrough, which would drop WS_EX_NOACTIVATE and WS_EX_TOOLWINDOW. Watch status() after toggling click-through; re-apply if needed.
- os::OsError is still separate from PlatformError.
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- chrome::install removes the native caption and V1 draws no main_toolbar, so the window has no title bar yet (Alt+F4 closes it).
- Library placeholder is replaced in V4.
- The Windows 11 dialog path has not been run on an older Windows.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile for other apps. Check the Cargo.lock diff.
