# Active Context

Milestone: S1. Current phase: 6c (done). Branch: s1.

## What is built
- ui-kit: tokens (generated from design/tokens.json), palette, fonts, theme, appearance file/poll, level meter, accent derivation, style lint.
- S1 phase 6c: `crates/ui-kit/src/widgets/` with the first six components: panel, primary_button, icon_button, segmented, chip, switch. Shared helpers: `widgets::focus_ring` and `widgets::capsule_radius` (height / 2 rule). No app uses them yet.
- `Theme` now has `reduce_motion: bool` (default false, runtime-only, not persisted in appearance.json). Phase 7 (Win32 chrome) will set it from SPI_GETCLIENTAREAANIMATION. ui-kit animations (currently the switch knob) run 0ms when it is true.

## Decisions made while coding
- egui 0.36.2 has NO reduce-motion option (checked Options/InputOptions and the crate source). That is why reduce_motion lives in our Theme.
- egui 0.36 API gotchas: `Response::widget_info` is a method taking a closure (`response.widget_info(|| WidgetInfo::...())`), not a field. `Sense::click()` includes FOCUSABLE; clicked focusable widgets get keyboard focus.
- kittest/egui_kittest: no context injection for UI harnesses, and the first pass runs with default fonts, so `tests/widgets.rs` defers widget drawing by one pass while the closure installs `ui_kit::fonts::definitions()` (named Inter families panic otherwise). Harness exposes `pub ctx`, `fit_contents()`, `key_press(egui::Key)`, `Node::{click, focus, hover}`.
- Segmented uses one `ui.interact` per segment (id `ui.id().with("segmented_segment").with(i)`); arrows handled when a segment has focus. Switch's accessible name is the const "Switch".
- Segments' 1px shadow = glass shadow color rect offset 1px down under the segment (no RectShape shadow API used).

## Next steps
- More widgets in later 6x phases as the prompts name them; phase 7 wires `reduce_motion` from the OS inside the chrome module.
