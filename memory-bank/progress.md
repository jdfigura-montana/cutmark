# Progress

## Done
- S0: workspace, cutmark-hotkeys, marks, media-io, capture, recorder, video editor + video-ui, ui-kit base (tokens, palette, fonts, theme, appearance, meter), scripts/check.ps1.
- S1 phase 6c: core widgets part 1 in crates/ui-kit/src/widgets/ (panel, primary_button, icon_button, segmented, chip, switch + focus_ring, capsule_radius). `cargo test -p ui-kit` green including the style lint (widgets dir is in no_hardcoded_style.rs OPT_IN); scripts/check.ps1 ALL CHECKS PASSED.
- kittest interaction tests in crates/ui-kit/tests/widgets.rs (switch/chip toggles, second-segment click, Right-arrow selection, primary button click, icon button tooltip on hover) plus a unit test that the switch knob jumps in one frame when Theme.reduce_motion is true.

## Not started
- Apps using the new widgets; remaining Components rows; phase 7 Win32 chrome (sets reduce_motion from SPI_GETCLIENTAREAANIMATION).

## Known issues
- None open for ui-kit.
