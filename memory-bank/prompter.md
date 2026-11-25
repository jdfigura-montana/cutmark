# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done
- Phase 1, V1: crates/prompter-platform (os::check_windows_11, build 22000 or later) and apps/prompter (cutmark-prompter): suite chrome, Violet appearance, Windows 11 dialog, placeholder panel.
- Phase 1, V2: prompter_platform::view (ViewWindow on NativeWindow(isize): find_own_window, exclude_from_capture, set_no_activate, reassert_topmost, status) and PlatformError. Non-Windows stubs return Unsupported. The app does not call it yet.
- Phase 1, V3: renderer.rs (DX12, DxgiFromVisual, adapter selector logs the surface alpha modes) and view.rs (one Floating view with sample text, scroll, drag bar, window setup, flag repair). Temporary Show, Hide, Click-through buttons, Ctrl+Alt+C and a diagnostics line in app.rs.
- V3 run on JD_Admin's PC: surface offers Auto, Inherit, Opaque, PostMultiplied, PreMultiplied; PreMultiplied in use. Flags read back all yes on show; Library affinity 17.

## Next (V4, not blocked)
- By hand, not yet verified (capture exclusion blocks screenshots): per-pixel transparency, view still visible with click-through on (winit adds WS_EX_LAYERED), mouse passes through, drag bar (fast drags too), Ctrl+Alt+C, no focus steal on click, scroll stutter, view with the Library minimized, capture checklist in OBS.
- scripts\check.ps1 fails in this worktree at clippy cutmark-audio: FFmpeg headers not found.

## Notes
- V2: the ignored real-window test has not been run, and the Linux stubs have not been compiled.
- Confirmed: winit drops no-activate on every MousePassthrough toggle; view.rs re-applies it (5 frame read-back).
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy, not as a C key.
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- The shown alpha mode repeats egui-wgpu's rule (it does not expose its choice). Recheck on upgrade.
- "Primary monitor" size is the monitor the Library opens on; the platform crate has no monitor query.
- os::OsError is still separate from PlatformError.
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- chrome::install removes the native caption and V1 draws no main_toolbar, so the window has no title bar yet (Alt+F4 closes it).
- The temporary view controls are replaced in V4.
- The Windows 11 dialog path has not been run on an older Windows.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile for other apps. Check the Cargo.lock diff.
