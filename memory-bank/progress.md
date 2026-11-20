# Progress

## Done
- S0: workspace, cutmark-hotkeys, marks, media-io, capture, recorder, video editor + video-ui, ui-kit base (tokens, palette, fonts, theme, appearance, meter), scripts/check.ps1.
- S1 phase 6c: core widgets part 1 in crates/ui-kit/src/widgets/ (panel, primary_button, icon_button, segmented, chip, switch + focus_ring, capsule_radius). `cargo test -p ui-kit` green including the style lint (widgets dir is in no_hardcoded_style.rs OPT_IN); scripts/check.ps1 ALL CHECKS PASSED.
- kittest interaction tests in crates/ui-kit/tests/widgets.rs (switch/chip toggles, second-segment click, Right-arrow selection, primary button click, icon button tooltip on hover) plus a unit test that the switch knob jumps in one frame when Theme.reduce_motion is true.
- S1 phase 7a: window chrome Windows half in crates/ui-kit/src/chrome/ (mod.rs + windows.rs), pure hit-test unit tests (edges, corners, maximize, caption, client, maximized, DPI 1.0/1.5/2.0), examples/chrome.rs hand-check app. windows + raw-window-handle now in [workspace.dependencies]. cargo test -p ui-kit green (27 lib tests), scripts/check.ps1 ALL CHECKS PASSED, example builds in release.

## Not started
- Apps using the new widgets; remaining Components rows; phase 7b+ (caption button drawing, inactive state); phase 8 media widgets.

## Known issues
- None open for ui-kit. The chrome's on-screen behaviors (Snap Layouts flyout, snap, drag) are only hand-checkable in the example; hit-test math is unit-tested.
