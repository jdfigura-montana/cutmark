# Prompter memory bank

## Track
- Worktree: cm_suite\cutmark-prompter
- Branch: prompter
- Spec: docs/suite/prompter.md

## Done (Phase 1, pending JD's check)
- V1, V2: crates/prompter-platform (os::check_windows_11; view::ViewWindow on NativeWindow(isize); PlatformError; non-Windows stubs return Unsupported) and apps/prompter (cutmark-prompter).
- V3: renderer.rs (DX12, DxgiFromVisual, logs surface alpha modes; PreMultiplied on JD's PC) and view.rs (one Floating view with sample text, drag bar, window setup, flag repair). White backing fix via clear_backing (JD approved the platform change).
- V4: Library shell in app.rs (title bar, Scripts, editor and Run panels, status bar, Appearance and About dialogs). menu.rs holds the menu list and enable rules. View controls are in the Run panel. Minimum window 900 x 480.

## State
- Phase 2 C1 done: crates/prompter-core. script::{parse, sections} and the block model; limits::{MAX_WORDS, split_at_heading}. Deps: pulldown-cmark (no default features), serde.
- C1 choices: Mark label keeps its prefix; the category name may end in a colon, a space or the end of the label. Cues in a heading do not split it. A split list item gives two ListItem blocks, so a renderer would draw two markers. Code blocks and code spans are plain text and their cues count. List depth starts at 0.

## Next
- C2.
- JD's check of V4 on screen is still pending.
- By hand: see-through, flash on open, move to another monitor or DPI, lock and sleep, scroll with the Library minimized, OBS capture.
- Open from V4: only Ctrl+Alt+C shows in the menus (V4 did not read the Hotkeys section). About notices are a short hand-written list.
- scripts\check.ps1 fails in this worktree at clippy cutmark-audio: FFmpeg headers not found.

## White layer findings
- The view's GDI redirection surface is opaque white after a show, under the DirectComposition visual. Who paints it is unknown. Only the area present at show was white.
- The window class background brush already reads 0. PatBlt BLACKNESS on the window DC fixes it, but does nothing on a hidden window.
- An outside hide and show needs a new clear; the once-a-second read-back catches it.
- A faint one-frame flash on open is not ruled out.

## Notes
- winit drops no-activate on every MousePassthrough toggle; view.rs re-applies it.
- egui-winit 0.36.2 ignores StartDrag without focus, so the view moves itself with OuterPosition.
- egui-winit sends Ctrl+Alt+C as Event::Copy, not a C key. app.rs puts the C key back, or menu_bar reads the Alt release as a tap and focuses File.
- The root viewport must be transparent or egui-wgpu uses alpha Auto for every viewport. clear_color keeps the Library opaque.
- To screenshot or pixel-test the Library or a view, use a temporary build with capture exclusion off.
- The shown alpha mode repeats egui-wgpu's rule (it does not expose its choice). Recheck on upgrade.
- "Primary monitor" size is the monitor the Library opens on; the platform crate has no monitor query.
- os::OsError is still separate from PlatformError.
- Prompter tasks never read or edit activeContext.md or progress.md.
- Only prompter-platform imports windows or other OS crates.
- The Windows 11 dialog path has not been run on an older Windows. The Linux stubs have not been compiled.
- Do not use cargo remove here: it deleted FilmCraft profile overrides from the root Cargo.toml.
- cargo add takes the newest release and can move the shared lockfile. Check the Cargo.lock diff.
