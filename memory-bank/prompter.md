# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1, V1: crates/prompter-platform (os::check_windows_11, build 22000 or later) and apps/prompter (cutmark-prompter): suite chrome, Violet appearance, Windows 11 dialog, placeholder panel.

## Next (V2)
- Waiting for the V2 prompt.

## Notes
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- chrome::install removes the native caption and V1 draws no main_toolbar, so the window has no title bar yet (Alt+F4 closes it).
- Library placeholder is replaced in V4.
- The Windows 11 dialog path has not been run on an older Windows.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile for other apps. Check the Cargo.lock diff.
