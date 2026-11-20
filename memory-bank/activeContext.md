# Active Context

Milestone: S1. Current phase: 7a (done). Branch: s1.

## What is built
- ui-kit: tokens (generated from design/tokens.json), palette, fonts, theme, appearance file/poll, level meter, accent derivation, style lint.
- S1 phase 6c: `crates/ui-kit/src/widgets/` with the first six components: panel, primary_button, icon_button, segmented, chip, switch. Shared helpers: `widgets::focus_ring` and `widgets::capsule_radius` (height / 2 rule). No app uses them yet.
- S1 phase 7a: `crates/ui-kit/src/chrome/` (mod.rs OS-neutral API + pure hit_test; windows.rs the only `#![allow(unsafe_code)]` file). `chrome::viewport` keeps decorations on with min size 640 x 400; `chrome::install` subclasses the eframe window (WM_NCCALCSIZE caption removal, WM_NCHITTEST from `chrome::publish` regions, HTMAXBUTTON hover + click, DWM round corners, SPI_GETCLIENTAREAANIMATION drives `Theme::reduce_motion` at install and on WM_SETTINGCHANGE). Example: `cargo run -p ui-kit --example chrome`.

## Decisions made while coding
- Phase 7a keeps the native frame (decorations on) and removes only the caption, so resize borders, Aero Snap, Win+arrows and double-click maximize stay OS-owned. Subclass id 0x4355_544D; one chrome window per process: `hook` refuses a second window (logs, leaves it the native title bar, never overwrites SHARED); SHARED clears on WM_NCDESTROY so a later window can be hooked.
- windows 0.62 API locations: TrackMouseEvent/TRACKMOUSEEVENT/TME_* live in Win32::UI::Input::KeyboardAndMouse (not WindowsAndMessaging); GetDpiForWindow and GetSystemMetricsForDpi in Win32::UI::HiDpi; PostMessageW takes Option<HWND>; TrackMouseEvent and SystemParametersInfoW return Result; DWM_WINDOW_CORNER_PREFERENCE(i32).
- eframe 0.36: `CreationContext` implements HasWindowHandle (HWND via raw_window_handle 0.6, added to workspace.dependencies); the App trait's required method is `fn ui(&mut self, ui: &mut Ui, frame)` (no update(ctx, frame)); CentralPanel::show takes &mut Ui.
- Hit test is a pure function in chrome/mod.rs (point, size, maximized, HitRegions in physical px, dpi) -> Zone, unit-tested on all platforms; windows.rs only maps Zone to HT*.
- Maximize button press handling: WM_NCLBUTTONDOWN sets a down flag, WM_NCLBUTTONUP over HTMAXBUTTON posts WM_SYSCOMMAND SC_MAXIMIZE/SC_RESTORE. Hover tracked via WM_NCMOUSEMOVE + TrackMouseEvent(TME_LEAVE|TME_NONCLIENT), requests repaint through the stored egui Context.

## Next steps
- Phase 7b/7c as prompted: caption button drawing (min/max/close glyphs, 46 x 52), inactive window state, then more widgets (phase 8).
- Phase 7a's manual Windows 11 hand-check items (drag, snap, Snap Layouts flyout, resize, round corners) still need on-screen verification in the example; unit tests cover the hit-test math only.
